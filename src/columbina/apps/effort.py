"""How hard the model thinks: the levels that a user chooses between, and what each asks for.

A level is chosen with ``/effort``, in the terminal and in the service, and it holds from the
next request on. ``default`` asks for nothing and leaves it to the service. ``off``, ``low``,
``high`` and ``max`` ask the service for that much reasoning. ``ultra`` asks for the most
reasoning there is, and the agent that the user talks to is told to work as a team: to plan,
to hand the parts that stand alone to subagents that work side by side, and to have one of
them review the result before it answers.

Which levels a service offers goes by how its requests say how much to reason, which is the
reasoning style of its adapter. A service of the style of DeepSeek takes all of them. One of
the style of Anthropic takes all but ``off``: not every model of Claude lets its thinking be
turned off, and ``ultra`` asks it for ``max``. Where
the style is that of OpenAI, a model that does not reason refuses a request that names an
effort, and nothing tells the program which models reason. So there ``default`` and
``ultra`` are offered, and between them the level that the config asks for, if it asks for
one: whoever wrote it knows that the model takes it. ``ultra`` asks there for what the
config asks, and for the team. Where the style is none, only ``default`` and ``ultra`` are
offered, and ``ultra`` asks for the team alone.

Until a level is chosen in a run, a call asks what its config asks, whichever level that
stands for.

What the agent is told at ``ultra`` is a message of its own, from the source of context
``effort``, and no part of the system prompt. The system prompt, the tools and the
conversation so far are then the same at every level: only what a request asks of the
reasoning differs, and what follows the conversation. How much of the rest a service reads
from its cache where the reasoning that it is asked for changes is the business of the
service. The message is written before the input of the first turn at
``ultra``, and stays where it was written. Where a summary has taken it from what the model
sees, it is written again, as the text of every source of context is. Once the level is
another, the model is told that the team is over, where it still sees the message that began
it. Only the agent that the user talks to is told: a subagent works as it is told by its
task. The message is no input of the user, so the terminal, a page that is exported and the
desktop application show nothing of it.
"""

from collections.abc import Awaitable, Callable
from typing import Literal, get_args

from columbina.agent import CONTEXT_PREFIX, Agent, ContextSource
from columbina.kernel import Context
from columbina.llm import LLM
from columbina.session import Session, UserMessageEvent

__all__ = [
    "EFFORTS",
    "LEFT",
    "SOURCE",
    "ULTRA",
    "Effort",
    "Reasoning",
    "Style",
    "effort_named",
    "effort_of",
    "offered",
    "reasoning_of",
    "style_of",
    "told_at",
    "told_by",
]

Effort = Literal["default", "off", "low", "high", "max", "ultra"]
"""A level that a user chooses."""

Reasoning = Literal["off", "low", "high", "max"]
"""What a call asks of a service, where it asks anything."""

Style = Literal["deepseek", "anthropic", "openai", "none"]
"""How the requests of a service say how much to reason."""

EFFORTS: tuple[Effort, ...] = get_args(Effort)
"""Every level, the least first."""


def style_of(ctx: Context, provider: str) -> Style:
    """Return how the service of ``provider`` is told how much to reason, as its adapter says.

    An adapter that says nothing of it, or a service that has none, is told nothing.
    """
    llm = ctx.get(LLM)
    adapter = None if llm is None else llm.adapters.get(provider)
    match getattr(adapter, "reasoning_style", None):
        case "deepseek":
            return "deepseek"
        case "anthropic":
            return "anthropic"
        case "openai":
            return "openai"
        case _:
            return "none"


def offered(style: Style, configured: Reasoning | None = None) -> tuple[Effort, ...]:
    """Return the levels that a service of this style offers, the least first.

    Args:
        style: How the service is told how much to reason.
        configured: What the config asks for, which a service of the style of OpenAI offers
            where it is a level of reasoning.
    """
    if style == "deepseek":
        return EFFORTS
    if style == "anthropic":
        return tuple(effort for effort in EFFORTS if effort != "off")
    if style == "openai" and configured is not None and configured != "off":
        return ("default", configured, "ultra")
    return ("default", "ultra")


def _listed(levels: tuple[Effort, ...]) -> str:
    return f"{', '.join(levels[:-1])} and {levels[-1]}"


def effort_named(
    effort: Effort, levels: tuple[Effort, ...], rest: str
) -> tuple[Effort | None, str]:
    """Carry out ``/effort``: return the level that is chosen, if one is, and what is said.

    Args:
        effort: The level that holds now.
        levels: The levels that the service in use offers, the least first.
        rest: What follows the command: nothing, or the level that is chosen.
    """
    named = rest.strip().lower()
    if not named:
        said = f"The effort is {effort}. This service offers {_listed(levels)}."
        return None, f"{said} Another level is chosen with /effort LEVEL."
    for level in levels:
        if level == named:
            return level, f"The effort is {level} from the next request on."
    if named in EFFORTS:
        return None, f"This service does not take {named}. It offers {_listed(levels)}."
    return None, f"{rest.strip()} is no level of effort. This service offers {_listed(levels)}."


def reasoning_of(
    effort: Effort, style: Style, configured: Reasoning | None = None
) -> Reasoning | None:
    """Return what a call asks for at a level that was chosen, of a service of this style.

    ``None`` asks for nothing: the service decides. A service of the style of DeepSeek or of
    Anthropic is asked for the level, and for ``max`` at ``ultra``. Another service offers no
    level of its own besides the default, so every other level asks it for what the config
    asks.

    Args:
        effort: The level that was chosen.
        style: How the service is told how much to reason.
        configured: What the config asks for.
    """
    if effort == "default":
        return None
    if style not in ("deepseek", "anthropic"):
        return configured
    return "max" if effort == "ultra" else effort


def effort_of(configured: Reasoning | None, style: Style) -> Effort:
    """Return the level that what a config asks for stands for, among those that are offered.

    What a service of this style is not asked for is ``default``. A service of the style of
    Anthropic is asked for ``low`` where the config asks for ``off``, and so that is the level.
    """
    effort: Effort = "default" if configured is None else configured
    if style == "anthropic" and effort == "off":
        effort = "low"
    return effort if effort in offered(style, configured) else "default"


ULTRA = (
    "Work at full depth on every task that is more than a question. Begin with a short plan.\n"
    "Hand each part of the work that stands alone to a subagent, and hand several such parts "
    "over in the same step, so that they run side by side. Give each subagent a task that "
    "says everything it needs, because it sees nothing of this conversation.\n"
    "Put together what the subagents found, and do what is left yourself.\n"
    "Before you answer, hand one subagent the task of reviewing the work against what was "
    "asked: the changes, the reasoning and the claims. Put right what it finds, then answer.\n"
    "A simple question is answered as it is, without a plan or subagents."
)
"""What the agent that the user talks to is told at ``ultra``."""

LEFT = (
    "The effort is no longer ultra. What you were told about working at full depth with a "
    "plan, subagents and a review no longer holds: work as you did before it, and hand a part "
    "of the work to a subagent only where that helps."
)
"""What the agent is told when the level is no longer ``ultra``."""

SOURCE = "effort"
"""The name of the source of context that tells the agent how to work at ``ultra``."""

_ORDER = 160
"""Where the source stands among the others: after the list of things to do, and so last
before the input of the turn."""


def _team_in_view(session: Session) -> bool:
    """Tell whether the last message of the source that the model sees is the one of ``ultra``."""
    source = CONTEXT_PREFIX + SOURCE
    for seq in reversed(session.surface):
        data = session.events[seq].data
        if isinstance(data, UserMessageEvent) and data.source == source:
            return any(ULTRA in block.text for block in data.message.content)
    return False


def told_at(effort: Effort, session: Session) -> str | None:
    """Return what the agent of a session is told of how to work at a level, if anything.

    At ``ultra`` it is told to work as a team. At another level it is told that this is over,
    where the model still sees that it was told to, and nothing otherwise.
    """
    if effort == "ultra":
        return ULTRA
    return LEFT if _team_in_view(session) else None


def told_by(render: Callable[[Agent], Awaitable[str | None]]) -> ContextSource:
    """Return the source of context that tells the agent that the user talks to how to work.

    Its text holds until it changes: it is written where it differs from what the source gave
    last, after the conversation so far, and again where a summary took it from the model.

    Args:
        render: Returns what :func:`told_at` says for the agent that the user talks to, and
            ``None`` for every other agent.
    """
    return ContextSource(render, order=_ORDER, kind="append")
