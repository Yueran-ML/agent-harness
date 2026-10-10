"""The command that shows the list of things to do.

This module is a plugin. Loading it registers the command ``/todos``.

The list is what the model has written down for itself, see
:mod:`columbina.todo`. It is shown in the conversation whenever it is
written. This command shows it as it stands, at any time.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.todo import TODOS, shown

__all__ = ["apply", "inject", "name"]

name = "command-todos"
inject = (COMMANDS,)

_NOTHING = "The agent has written down nothing to do."


async def _run(agent: Agent, rest: str) -> str:
    items = TODOS.of(agent.session).items
    return "\n".join(shown(items)) if items else _NOTHING


def apply(ctx: Context, config: object) -> None:
    """Register the command that shows the list of things to do."""
    told = "Show the list of things to do that the agent keeps."
    ctx.use(COMMANDS).registry.add(ctx, "todos", Command(_run, told))
