"""Commands: what a person tells the program itself to do, by a line that begins with a slash.

This module is a plugin. Loading it provides :data:`COMMANDS`.

Most of what a person types goes to the model. A line that begins with a
slash does not: it names a command, and the command is carried out at once,
by the program. ``/help`` lists the commands, another one shows the tools or
changes what the session may do. A command gives back text for the person to
read. The model is not told about it.

Whoever owns a piece of state registers the commands that show or change it,
so a frontend need not know them. A plugin that leaves takes its commands
with it.

This plugin brings one command itself, ``/help``.
"""

import logging
from collections.abc import Awaitable, Callable
from dataclasses import dataclass

from columbina.agent import Agent
from columbina.kernel import Context, Registry, ServiceKey

__all__ = ["COMMANDS", "Command", "Commands", "apply", "is_command", "name"]

name = "commands"

logger = logging.getLogger(__name__)

_MARK = "/"


@dataclass(frozen=True, slots=True)
class Command:
    """Something that the program does when a person asks for it by name.

    Attributes:
        run: Carries the command out. It is given the agent that the person
            is talking to and what stood after the name of the command, and
            gives back text for the person.
        summary: One line that says what the command is for.
    """

    run: Callable[[Agent, str], Awaitable[str]]
    summary: str


def is_command(line: str) -> bool:
    """Tell whether a line that a person typed is a command and not meant for the model."""
    return line.startswith(_MARK)


class Commands:
    """The commands that a person can give.

    Attributes:
        registry: The commands, by their names without the slash.
    """

    __slots__ = ("registry",)

    def __init__(self) -> None:
        self.registry: Registry[Command] = Registry("command")

    async def run(self, agent: Agent, line: str) -> str:
        """Carry out the command that a line names, and return what it says.

        The name ends at the first space, and it is taken in lower case.
        What follows is handed to the command without the spaces around it.
        A name that no command has, and a command that fails, are answered
        with a sentence for the person. Neither stops the program.

        Args:
            agent: The agent that the person is talking to.
            line: The line as it was typed, with the slash.

        Raises:
            ValueError: The line does not begin with a slash.
        """
        if not is_command(line):
            raise ValueError(f"{line!r} is not a command: it does not begin with {_MARK}")
        named, _, rest = line[len(_MARK) :].partition(" ")
        command = self.registry.get(named.lower())
        if command is None:
            return f"There is no command {_MARK}{named}. {_MARK}help lists the commands."
        try:
            return await command.run(agent, rest.strip())
        except Exception as error:
            logger.error("the command %s%s failed", _MARK, named.lower(), exc_info=error)
            return f"The command {_MARK}{named.lower()} failed: {type(error).__name__}: {error}"

    async def help(self, agent: Agent, rest: str) -> str:
        """List the commands by their names, each with what it is for."""
        width = max(len(named) for named in self.registry)
        lines = [
            f"{_MARK}{named.ljust(width)}  {self.registry[named].summary}"
            for named in sorted(self.registry)
        ]
        lines.append("A line that does not begin with a slash goes to the agent.")
        return "\n".join(lines)


COMMANDS = ServiceKey[Commands]("commands")
"""The commands."""


def apply(ctx: Context, config: object) -> None:
    """Provide the commands, with the one that lists them."""
    commands = Commands()
    commands.registry.add(ctx, "help", Command(commands.help, "List the commands."))
    ctx.provide(COMMANDS, commands)
