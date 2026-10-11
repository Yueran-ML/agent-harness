"""The tool that hands a task to a subagent, and what a model is told about subagents.

This module is a plugin. Loading it registers the tool ``subagent`` and two
sources of context. The work is done by :mod:`columbina.subagents`. Here it
is put before a model.

``subagent``
    Hands a task over and gives back what the subagent answered. A call
    waits until the subagent is through, and several calls of one reply run
    side by side. A task that was not done is a failed call, and the model
    reads why.
``agents``
    The kinds of subagent that there are, for an agent that can hand a task
    over. Nothing is said where there are none.
``delegated``
    For a subagent only: where it stands. Nobody watches it and nobody can
    be asked, and its last reply is all that comes of its work.
"""

from collections.abc import Sequence

from pydantic import BaseModel, Field

from columbina.agent import AGENTS, CONTEXT_PREFIX, Agent, Agents, ContextSource
from columbina.kernel import Context
from columbina.session import EventData, Projection, SessionEvent, UserMessageEvent
from columbina.subagents import SUBAGENTS, TOOL, SubagentFailed, Subagents, Task
from columbina.subagents.types import SubagentType
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "MAX_DESCRIPTION",
    "Config",
    "SubagentArgs",
    "apply",
    "catalog",
    "inject",
    "name",
    "subagent_tool",
]

name = "tool-subagent"
inject = (AGENTS, SUBAGENTS, TOOLS)

MAX_DESCRIPTION = 300
"""How many characters of what a kind of subagent is for a model is told at most."""

_SOURCE = CONTEXT_PREFIX + "agents"

_ABOUT = (
    "Hand a task to a subagent: another agent that works on it by itself, with tools of its "
    "own, and gives back what it found. It starts with nothing of this conversation, so the "
    "prompt has to hold all that it needs: what to do, where, and what to give back. Only its "
    "last answer comes back, not what it did on the way. Use it for work that stands by "
    "itself and would fill this conversation with what you do not need afterwards, such as "
    "searching a code base for something. Several calls in one reply run side by side. Do "
    "small things yourself: a subagent costs more than a few calls of your own."
)
_KINDS = (
    f"These kinds of subagent are there. Name one as `type` when you call the tool `{TOOL}`, "
    "or name none for one that has all your tools."
)
_GONE = (
    "No kind of subagent is there any more. The kinds that you were told of before cannot be "
    f"named. The tool `{TOOL}` works without a type."
)
_DELEGATED = (
    "You are a subagent. Another agent handed you this task, and it reads only your last "
    "reply: nothing else that you write or do reaches it. Nobody watches you work, so nobody "
    "can be asked anything, and a call that would need a yes from a person is refused. If "
    "something stays open or could not be done, say so in your last reply. Make that reply "
    "the whole answer to the task."
)


class Config(BaseModel):
    """The config of the tool that hands tasks to subagents.

    Attributes:
        timeout_seconds: How long a call may take. A subagent works through
            many requests of its own, so this is much longer than the limit
            of other tools.
        catalog_max_chars: How long the list of kinds that a model is given
            may be.
    """

    timeout_seconds: float = Field(default=1800, gt=0)
    catalog_max_chars: int = Field(default=6000, ge=500)


class SubagentArgs(BaseModel):
    """The arguments of the tool that hands a task to a subagent."""

    description: str = Field(description="What the subagent is for, in three to five words.")
    prompt: str = Field(
        min_length=1, description="The task, with all that the subagent needs to know."
    )
    type: str | None = Field(default=None, description="The kind of subagent, if one fits.")


def _given(state: bool, event: SessionEvent[EventData]) -> bool:
    data = event.data
    return state or (isinstance(data, UserMessageEvent) and data.source == _SOURCE)


_GIVEN: Projection[bool] = Projection("agents-given", False, _given)
"""Whether a session was ever told of kinds of subagent."""


def _line(kind: SubagentType) -> str:
    told = " ".join(kind.description.split())
    if len(told) > MAX_DESCRIPTION:
        told = f"{told[: MAX_DESCRIPTION - 1]}…"
    return f"- {kind.name}: {told}"


def catalog(kinds: Sequence[SubagentType], limit: int) -> str | None:
    """Return what a model is told about the kinds of subagent that it may name.

    Each kind has a line. If all lines together are too long, the last kinds
    are left out, and the model is told how many.

    Args:
        kinds: The kinds, in the order in which they are listed.
        limit: How many characters the lines may have together.

    Returns:
        The text, or ``None`` if there is no kind.
    """
    if not kinds:
        return None
    lines: list[str] = []
    used = 0
    for kind in kinds:
        line = _line(kind)
        if lines and used + len(line) > limit:
            break
        lines.append(line)
        used += len(line) + 1
    left = len(kinds) - len(lines)
    if left:
        more = "1 more kind is" if left == 1 else f"{left} more kinds are"
        lines.append(f"[{more} there and not listed, because the list would be too long.]")
    return "\n".join([_KINDS, *lines])


def subagent_tool(subagents: Subagents, agents: Agents, seconds: float) -> Tool[SubagentArgs, str]:
    """Make the tool that hands a task to a subagent."""

    async def hand_over(args: SubagentArgs, run: ToolRun) -> str:
        parent = agents.live.get(run.session.id)
        if parent is None:
            raise ToolFailure("No agent is there for a subagent to work for.", code="NO_AGENT")
        task = Task(run.call_id, args.description, args.prompt, args.type)
        try:
            return await subagents.run(parent, task)
        except SubagentFailed as failed:
            raise ToolFailure(str(failed), code=failed.code) from None

    return Tool(
        name=TOOL,
        description=_ABOUT,
        parameters=SubagentArgs,
        execute=hand_over,
        timeout_seconds=seconds,
        concurrency_safe=True,
    )


def apply(ctx: Context, config: Config) -> None:
    """Register the tool, and tell a model of the kinds and a subagent of where it stands."""
    subagents = ctx.use(SUBAGENTS)
    agents = ctx.use(AGENTS)

    async def kinds(agent: Agent) -> str | None:
        if agent.spec.tools is not None and TOOL not in agent.spec.tools:
            # An agent that cannot hand a task over has no use for the kinds.
            return None
        found = await subagents.found(agent.cwd)
        said = catalog(list(found.types.values()), config.catalog_max_chars)
        if said is None and _GIVEN.of(agent.session):
            return _GONE
        return said

    async def delegated(agent: Agent) -> str | None:
        return _DELEGATED if agent.session.delegation_depth else None

    ctx.use(TOOLS).add(ctx, subagent_tool(subagents, agents, config.timeout_seconds))
    # After what is told about the run, and before the skills.
    agents.context.add(ctx, "delegated", ContextSource(delegated, order=110, kind="append"))
    # After the skills, and before the list of things to do.
    agents.context.add(ctx, "agents", ContextSource(kinds, order=130, kind="replace"))
