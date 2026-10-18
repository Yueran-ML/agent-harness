"""How hard the model thinks: the levels that a user chooses between, and what each asks for.

A level is chosen with ``/effort``, in the terminal and in the service, and it holds from the
next request on. ``default`` asks for nothing and leaves it to the service. ``off``, ``low``,
``high`` and ``max`` ask the service for that much reasoning. ``ultra`` asks for the most
reasoning there is, and the agent that the user talks to is told to work as a team: to plan,
to hand the parts that stand alone to subagents that work side by side, and to have one of
them review the result before it answers.

Which levels a service offers goes by how its requests say how much to reason, which is the
reasoning style of its adapter. A service of the style of DeepSeek takes all of them. Where
the style is that of OpenAI, a model that does not reason refuses a request that names an
effort, and nothing tells the program which models reason. So there ``default`` and
``ultra`` are offered, and between them the level that the config asks for, if it asks for
one: whoever wrote it knows that the model takes it. ``ultra`` asks there for what the
config asks, and for the team. Where the style is none, only ``default`` and ``ultra`` are
offered, and ``ultra`` asks for the team alone.

Until a level is chosen in a run, a call asks what its config asks, whichever level that
stands for.
"""

from typing import Literal, get_args

from columbina.kernel import Context
from columbina.llm import LLM

__all__ = [
    "EFFORTS",
    "ULTRA",
    "Effort",
    "Reasoning",
    "Style",
    "effort_named",
    "effort_of",
    "offered",
    "reasoning_of",
    "style_of",
]

Effort = Literal["default", "off", "low", "high", "max", "ultra"]
"""A level that a user chooses."""

Reasoning = Literal["off", "low", "high", "max"]
"""What a call asks of a service, where it asks anything."""

Style = Literal["deepseek", "openai", "none"]
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

    ``None`` asks for nothing: the service decides. A service that is not of the style of
    DeepSeek offers no level of its own besides the default, so every other level asks it
    for what the config asks.

    Args:
        effort: The level that was chosen.
        style: How the service is told how much to reason.
        configured: What the config asks for.
    """
    if effort == "default":
        return None
    if style != "deepseek":
        return configured
    return "max" if effort == "ultra" else effort


def effort_of(configured: Reasoning | None, style: Style) -> Effort:
    """Return the level that what a config asks for stands for, among those that are offered.

    What a service of this style is not asked for is ``default``.
    """
    effort: Effort = "default" if configured is None else configured
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
