"""Approval: asking somebody before something is done.

This module is a plugin. Loading it provides :data:`APPROVAL`.

Whoever decides that an action needs a yes asks here. The question goes to
the listeners of :data:`APPROVAL_ASK`, one after another, until one of them
answers. A terminal or a window that can put the question to a person
listens there. A program without anybody to ask has no listener, and then
the answer is that nobody is there.

The answers are a closed set.

``allowed-once``
    Yes, for this one time. Nothing else lets an action go ahead.
``rejected``
    No.
``cancelled``
    The question was withdrawn before it was answered.
``unavailable``
    Nobody could be asked: no listener answered, or the one that did failed,
    or it said something that is none of these four.

There is no answer that allows more than the one action that was asked
about. Who wants an action to go ahead every time writes a rule.

A session has a policy, which is written into its log and so comes back when
the session is taken up again. With ``ask``, questions are put. With
``never``, nobody is asked and every question is answered ``rejected``.

Every question is written into the log of its session, and so is its answer,
whatever it was.
"""

import logging
from dataclasses import dataclass
from typing import ClassVar, Literal, cast, get_args

from pydantic import BaseModel

from columbina.agent import RUNTIME_FACTS, Facts
from columbina.kernel import Context, SerialEvent, ServiceKey
from columbina.session import SESSIONS, EventData, Projection, Session, SessionEvent

__all__ = [
    "APPROVAL",
    "APPROVAL_ASK",
    "Approval",
    "ApprovalAskedEvent",
    "ApprovalDecidedEvent",
    "ApprovalPolicyEvent",
    "ApprovalRequest",
    "Config",
    "Outcome",
    "Policy",
    "apply",
    "inject",
    "name",
]

name = "approval"
inject = (SESSIONS,)

logger = logging.getLogger(__name__)

Policy = Literal["ask", "never"]
"""Whether questions are put at all."""

Outcome = Literal["allowed-once", "rejected", "cancelled", "unavailable"]
"""How a question ended."""

_OUTCOMES: tuple[str, ...] = get_args(Outcome)


class ApprovalPolicyEvent(EventData):
    """The policy of a session was set."""

    event_type: ClassVar[str] = "approval/policy"
    ignorable: ClassVar[bool] = True

    policy: Policy


class ApprovalAskedEvent(EventData):
    """A question was put."""

    event_type: ClassVar[str] = "approval/asked"
    ignorable: ClassVar[bool] = True

    call_id: str
    tool: str
    summary: str


class ApprovalDecidedEvent(EventData):
    """A question ended."""

    event_type: ClassVar[str] = "approval/decided"
    ignorable: ClassVar[bool] = True

    call_id: str
    outcome: Outcome


@dataclass(frozen=True, slots=True)
class ApprovalRequest:
    """A question about one call of a tool.

    Attributes:
        session: The session that the call belongs to.
        call_id: The id that the model gave the call.
        tool: The name of the tool.
        summary: One line for a person: the tool and what it acts on. A
            long text is cut short in it.
        reason: Why the question is put.
        target: What the call acts on, whole, as the tool says it, or
            ``None`` if the tool does not say. Whoever puts the question to
            a person shows this: nobody can say yes to a command of which
            only the beginning is seen.
        standing: What a yes for the rest of the session would cover, in
            words that finish "do not ask again about ...", or ``None`` if
            the call has to be asked about every time. The answer to the
            question is for this one call either way. Whoever offers the
            longer yes records it where the question came from.
    """

    session: Session
    call_id: str
    tool: str
    summary: str
    reason: str
    target: str | None = None
    standing: str | None = None


APPROVAL_ASK: SerialEvent[ApprovalRequest, Outcome] = SerialEvent("approval/ask")
"""Puts a question to whoever can answer it. A listener that cannot says nothing."""


_POLICIES: dict[Policy, str] = {
    "ask": "ask. A call that needs a yes is put to the user, if somebody is there to ask.",
    "never": "never. A call that needs a yes is refused without anybody being asked.",
}
"""What a model is told about each policy."""


def _last_policy(policy: Policy | None, event: SessionEvent[EventData]) -> Policy | None:
    data = event.data
    return data.policy if isinstance(data, ApprovalPolicyEvent) else policy


_POLICY: Projection[Policy | None] = Projection("approval-policy", None, _last_policy)


class Config(BaseModel):
    """The config of approval.

    Attributes:
        policy: The policy of a session whose log names none.
    """

    policy: Policy = "ask"


class Approval:
    """Puts questions about actions, and holds the policy of each session."""

    __slots__ = ("_default",)

    def __init__(self, default: Policy) -> None:
        self._default: Policy = default

    def recorded(self, session: Session) -> Policy | None:
        """Return the policy that the log of a session names, or ``None`` if it names none."""
        return _POLICY.of(session)

    def policy(self, session: Session) -> Policy:
        """Return the policy of a session: the one in its log, or else the one of the config."""
        return _POLICY.of(session) or self._default

    def set_policy(self, session: Session, policy: Policy) -> None:
        """Write the policy of a session into its log, unless the log names it already."""
        if _POLICY.of(session) != policy:
            session.append(ApprovalPolicyEvent(policy=policy))

    async def ask(self, ctx: Context, request: ApprovalRequest) -> Outcome:
        """Put a question, and return how it ended.

        The question and its answer are written into the log of the session.
        If the caller is cancelled while it waits, the answer in the log is
        ``cancelled``, and the cancellation goes on its way.

        Args:
            ctx: The context whose listeners are asked.
            request: The question.
        """
        session = request.session
        session.append(
            ApprovalAskedEvent(call_id=request.call_id, tool=request.tool, summary=request.summary)
        )
        outcome: Outcome = "cancelled"
        try:
            outcome = await self._answer(ctx, request)
        finally:
            session.append(ApprovalDecidedEvent(call_id=request.call_id, outcome=outcome))
        return outcome

    async def _answer(self, ctx: Context, request: ApprovalRequest) -> Outcome:
        if self.policy(request.session) == "never":
            return "rejected"
        try:
            answer = cast("object", await ctx.serial(APPROVAL_ASK, request))
        except Exception as error:
            logger.error("a listener of %r failed", APPROVAL_ASK.name, exc_info=error)
            return "unavailable"
        if answer is None:
            # Nobody is there to answer.
            return "unavailable"
        if not isinstance(answer, str) or answer not in _OUTCOMES:
            logger.error(
                "a listener of %r answered %r, which is no answer", APPROVAL_ASK.name, answer
            )
            return "unavailable"
        return cast("Outcome", answer)


APPROVAL = ServiceKey[Approval]("approval")
"""Approval."""


def apply(ctx: Context, config: Config) -> None:
    """Provide approval."""
    sessions = ctx.use(SESSIONS)
    for kind in (ApprovalPolicyEvent, ApprovalAskedEvent, ApprovalDecidedEvent):
        sessions.declare(ctx, kind)
    approval = Approval(config.policy)

    def facts(gathered: Facts) -> None:
        gathered.lines["approval"] = _POLICIES[approval.policy(gathered.agent.session)]

    ctx.on(RUNTIME_FACTS, facts)
    ctx.provide(APPROVAL, approval)
