"""The provider that runs a subagent in this process.

This module is a plugin. Loading it registers the provider ``spawn`` with the
subagents, see :mod:`columbina.subagents`.

A subagent that is spawned is an agent like any other. It is made under the
agent that hands it a task, so it ends when that one ends. It has a scope
of its own: what listens to the parent does not hear it, and so nothing of
its work shows up where the conversation of the parent is shown. And it has
a session of its own, which names the session of the parent and says how
deep the task was handed down. It begins with nothing of the conversation
of its parent: all that it knows of the task is what the task says.

What a subagent may do is fixed when it is made, before anything waits. The
mode of the sandbox is the one of its parent at that moment. The policy of
approval is ``never``: nobody watches a subagent, so nobody can be asked on
its behalf, and a call that needs a yes is refused. Both are written into
the log of the subagent. A yes that the parent was given for the rest of
its session is a yes for that session, and the subagent has another one.
The rules that the user wrote hold for both.

How its turn ended is read from its own log and from nothing else.
"""

import contextlib

from columbina.agent import AGENTS, Agent, Agents, CancelCause, replies
from columbina.approval import APPROVAL
from columbina.kernel import Context
from columbina.sandbox import SANDBOX
from columbina.session import (
    TURN,
    USAGE,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    Session,
    TurnEndEvent,
    TurnEndReason,
)
from columbina.subagents import SUBAGENTS, Child, Order, Outcome

__all__ = ["PROVIDER", "Spawn", "apply", "inject", "name", "outcome_of"]

name = "subagent-spawn"
inject = (AGENTS, SUBAGENTS)

PROVIDER = "spawn"
"""The name under which this provider is registered."""


def outcome_of(session: Session) -> Outcome:
    """Read from the log of a subagent how its turn ended and what it had said.

    A turn that came to its end gives the last reply that is not empty. One
    that did not gives the last words that arrived at all, also those of a
    request that failed: they are what the subagent had written until then.
    A reply that was cut off and taken up again is one reply with what went
    on from it.
    """
    reason: TurnEndReason | None = None
    detail: str | None = None
    code: str | None = None
    arrived = ""
    for event in session.events:
        data = event.data
        if isinstance(data, TurnEndEvent):
            reason, detail, code = data.reason, data.detail, data.code
        elif isinstance(data, AssistantMessageEvent):
            # Told below, with what it went on from.
            arrived = "" if data.message.text.strip() else arrived
        elif isinstance(data, AssistantAttemptEvent) and data.message.text.strip():
            arrived = data.message.text
    said = [text for text in replies(event.data for event in session.events) if text.strip()]
    replied = said[-1] if said else ""
    arrived = arrived or replied
    return Outcome(
        reason=reason,
        detail=detail,
        text=replied if reason == "completed" else arrived,
        steps=TURN.of(session).step,
        usage=USAGE.of(session).usage,
        code=code,
    )


class _Spawned:
    """A subagent that runs in this process."""

    __slots__ = ("_agent",)

    def __init__(self, agent: Agent) -> None:
        self._agent = agent

    @property
    def session_id(self) -> str:
        return self._agent.id

    async def ended(self) -> Outcome:
        # An agent that has stopped for good says so here. Its log says how far it came.
        with contextlib.suppress(Exception):
            await self._agent.when_idle()
        return outcome_of(self._agent.session)

    def stop(self, cause: CancelCause) -> None:
        self._agent.cancel(cause)

    async def close(self) -> None:
        await self._agent.close()


class Spawn:
    """Starts subagents as agents of this process."""

    __slots__ = ("_agents", "_ctx")

    def __init__(self, ctx: Context, agents: Agents) -> None:
        self._ctx = ctx
        self._agents = agents

    async def start(self, parent: Agent, order: Order) -> Child:
        """Make an agent under ``parent``, fix what it may do, and give it its task."""
        child = await self._agents.create(
            parent.ctx, order.spec, parent=parent.id, delegation_depth=order.depth
        )
        try:
            # Nothing has waited since the agent was made, so nothing of it has run.
            self._fix(parent, child)
            child.followup(order.task.prompt)
        except BaseException:
            await child.close()
            raise
        return _Spawned(child)

    def _fix(self, parent: Agent, child: Agent) -> None:
        """Write into the log of a subagent what it may do."""
        sandbox = self._ctx.get(SANDBOX)
        if sandbox is not None:
            sandbox.set_mode(child.session, sandbox.mode(parent.session))
        approval = self._ctx.get(APPROVAL)
        if approval is not None:
            approval.set_policy(child.session, "never")


def apply(ctx: Context, config: object) -> None:
    """Register the provider."""
    ctx.use(SUBAGENTS).providers.add(ctx, PROVIDER, Spawn(ctx, ctx.use(AGENTS)))
