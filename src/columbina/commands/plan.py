"""The command that takes the conversation into plan mode and out of it.

This module is a plugin. Loading it registers the command ``/plan``, which
needs :mod:`columbina.plan`. ``/plan`` goes into the mode where the
conversation is not in it, and out of it where it is; ``/plan on`` and
``/plan off`` say which.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.plan import planning, set_planning

__all__ = ["apply", "inject", "name"]

name = "command-plan"
inject = (COMMANDS,)

_ON = (
    "Plan mode: the model looks and thinks, and changes nothing. Say what you want, and /plan "
    "again when the plan is right."
)
_OFF = "Plan mode is over: the model carries out the plan from the next turn."
_USAGE = "Say /plan, /plan on or /plan off."


def apply(ctx: Context, config: object) -> None:
    """Register the command that takes the conversation into plan mode and out of it."""

    async def run(agent: Agent, rest: str) -> str:
        word = rest.strip().lower()
        if word not in ("", "on", "off"):
            return _USAGE
        on = not planning(agent.session) if word == "" else word == "on"
        if not set_planning(agent.session, on):
            return "The conversation is in plan mode already." if on else "Plan mode is not on."
        return _ON if on else _OFF

    ctx.use(COMMANDS).registry.add(
        ctx, "plan", Command(run, "Go into plan mode, where nothing is changed, or out of it.")
    )
