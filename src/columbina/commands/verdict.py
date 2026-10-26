"""The command that says whether the last turn did what was asked.

This module is a plugin. Loading it registers the command ``/verdict``. What
it writes down is told in :mod:`columbina.verdicts`, which has to be loaded
for a log to take it.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.verdicts import judge

__all__ = ["apply", "inject", "name"]

name = "command-verdict"
inject = (COMMANDS,)

_USAGE = (
    "Say /verdict right or /verdict wrong, and a note after it if you like. "
    "#N after the word names turn N instead of the last one."
)


def apply(ctx: Context, config: object) -> None:
    """Register the command that writes down whether the last turn did what was asked."""

    async def run(agent: Agent, rest: str) -> str:
        word, _, note = rest.strip().partition(" ")
        if word not in ("right", "wrong"):
            return _USAGE
        named, _, after = note.strip().partition(" ")
        turn = None
        if named.startswith("#") and named[1:].isdigit():
            turn, note = int(named[1:]), after
        try:
            turn = judge(agent.session, word == "right", note.strip(), turn)
        except ValueError as error:
            return f"Nothing was written down: {error}."
        return f"Turn {turn} is written down as {word}."

    ctx.use(COMMANDS).registry.add(
        ctx,
        "verdict",
        Command(run, "Say whether the last turn did what was asked: /verdict right|wrong [NOTE]."),
    )
