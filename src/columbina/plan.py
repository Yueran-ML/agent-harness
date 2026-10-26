"""Plan mode: the model looks and thinks, and changes nothing until the user says so.

This module is a plugin. Loading it has the logs of sessions take
:class:`PlanModeEvent`, and while a conversation is in plan mode:

- no call that writes or runs anything goes through, in the conversation or
  in a subagent that it handed a task to. The model is told why, and that
  the user takes the conversation out of plan mode when the plan is right;
- the model is told when a turn begins that it is to find out what it needs,
  ask what is open, and end with a plan of numbered steps. When the mode is
  left, it is told to carry out the plan that was agreed on;
- the requests go to another model, where the config names one: a model
  that thinks well for the plan, and the usual one for the work.

The command ``/plan`` goes into the mode and out of it, see
:mod:`columbina.commands.plan`. Whether a session is in the mode is written
into its log, so a session that is taken up again is in it where it was.
"""

from typing import ClassVar

from pydantic import BaseModel, field_validator

from columbina.agent import AGENTS, REQUEST, Agent, ContextSource, RequestDraft
from columbina.kernel import Context, Next
from columbina.llm import CallConfig
from columbina.session import SESSIONS, EventData, Session
from columbina.tools import TOOLS, Deny, Invocation

__all__ = [
    "IN_PLAN_MODE",
    "LEFT",
    "OUT_OF_PLAN_MODE",
    "PLANNING",
    "Config",
    "PlanModeEvent",
    "apply",
    "inject",
    "name",
    "planning",
    "set_planning",
]

name = "plan"
inject = (SESSIONS, AGENTS, TOOLS)

PLANNING = (
    "You are in plan mode. Find out what you need: read, search and ask the user what is "
    "open. Change nothing and run nothing, as nothing that writes or runs will go through. End "
    "your turn with a plan of numbered steps, with the files that each one changes. The user "
    "takes the conversation out of plan mode when the plan is right."
)
"""What the model is told while the conversation is in plan mode."""

LEFT = (
    "Plan mode is over. Carry out the plan that the user agreed to, as it stands after what "
    "they said of it."
)
"""What the model is told when the conversation leaves plan mode."""

IN_PLAN_MODE = "PLAN_MODE"
"""The code of a call that did not go through because of plan mode."""

OUT_OF_PLAN_MODE = (
    "the conversation is in plan mode, in which nothing is written or run. Write the plan "
    "instead: the user takes the conversation out of plan mode when it is right"
)


class PlanModeEvent(EventData):
    """The conversation went into plan mode, or out of it.

    Attributes:
        on: Whether it is in plan mode from here on.
    """

    event_type: ClassVar[str] = "plan/mode"
    ignorable: ClassVar[bool] = True

    on: bool


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        model: The model that the requests go to in plan mode, at the
            service of the conversation. Without it they go where they
            always go.
    """

    model: str | None = None

    @field_validator("model", mode="before")
    @classmethod
    def _named(cls, model: object) -> object:
        # A variable that is not set says nothing, and a profile writes it as nothing.
        return None if model == "" else model


def _last(session: Session) -> PlanModeEvent | None:
    for event in reversed(session.events):
        if isinstance(event.data, PlanModeEvent):
            return event.data
    return None


def planning(session: Session) -> bool:
    """Tell whether a session is in plan mode."""
    last = _last(session)
    return last is not None and last.on


def set_planning(session: Session, on: bool) -> bool:
    """Put a session into plan mode or out of it, and tell whether that changed anything."""
    if planning(session) == on:
        return False
    session.append(PlanModeEvent(on=on))
    return True


def apply(ctx: Context, config: Config) -> None:
    """Hold the calls and the requests of conversations in plan mode, and tell the model."""
    ctx.use(SESSIONS).declare(ctx, PlanModeEvent)
    agents = ctx.use(AGENTS)

    def held(session: Session) -> bool:
        """Tell whether a session is in plan mode, or one that handed it its task is."""
        while True:
            if planning(session):
                return True
            parent = None if session.parent is None else agents.live.get(session.parent)
            if parent is None:
                return False
            session = parent.session

    async def guard(caller: Context, invocation: Invocation) -> Deny | None:
        if invocation.tool.effect not in ("write", "run"):
            return None
        if not held(invocation.run.session):
            return None
        return Deny(OUT_OF_PLAN_MODE, IN_PLAN_MODE)

    async def told(agent: Agent) -> str | None:
        last = _last(agent.session)
        if last is None:
            return None
        return PLANNING if last.on else LEFT

    async def asked_of(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        model = await proceed(draft)
        if config.model is None or not planning(draft.agent.session):
            return model
        return model.model_copy(update={"model": config.model, "context_window": None})

    ctx.use(TOOLS).guards.add(ctx, "plan-mode", guard)
    agents.context.add(ctx, "plan", ContextSource(told, order=140, kind="append"))
    ctx.on(REQUEST, asked_of)
