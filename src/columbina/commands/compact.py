"""The command that makes the conversation shorter.

This module is a plugin. Loading it registers the command ``/compact``.

A conversation is made shorter by itself when it comes near what its model
takes. A person may want it sooner: before a new piece of work, or because
the old part has stopped being of use. The command asks the service that
does it, between two turns, and says what came of it.
"""

from columbina import cancellation
from columbina.agent import Agent, AgentBusyError
from columbina.commands import COMMANDS, Command
from columbina.compaction import COMPACTION, Outcome
from columbina.kernel import Context

__all__ = ["apply", "inject", "name"]

name = "command-compact"
inject = (COMMANDS, COMPACTION)

_BUSY = (
    "The agent is in the middle of something. Wait until it is at rest, or stop it, and ask again."
)
_STOPPED = "It was stopped. The conversation is as it was."


def _so_many(count: int, one: str, several: str) -> str:
    return f"1 {one}" if count == 1 else f"{count} {several}"


def _told(outcome: Outcome) -> str:
    """Say in a sentence or two what an attempt to make the conversation shorter came to."""
    if not outcome.changed:
        size = f"It comes to about {outcome.before} tokens."
        if outcome.error is None:
            return f"There is nothing in this conversation that could go yet. {size}"
        return f"The conversation is as it was: {outcome.error}. {size}"
    did: list[str] = []
    if outcome.summarised:
        gave = _so_many(outcome.summarised, "message gave", "messages gave")
        did.append(f"{gave} way to a summary")
    if outcome.trimmed:
        cut = _so_many(outcome.trimmed, "long result of a tool was", "long results of tools were")
        did.append(f"{cut} cut down to the beginning and the end")
    told = f"The conversation was made shorter: {' and '.join(did)}."
    sizes = f"It came to about {outcome.before} tokens and comes to about {outcome.after} now."
    if outcome.error is None:
        return f"{told} {sizes}"
    return f"{told} No summary came of it: {outcome.error}. {sizes}"


def apply(ctx: Context, config: object) -> None:
    """Register the command that makes the conversation shorter."""
    service = ctx.use(COMPACTION)

    async def run(agent: Agent, rest: str) -> str:
        async def work() -> Outcome:
            return await service.compact(agent, "manual")

        try:
            outcome = await cancellation.guard(
                agent.run_maintenance(work), what="making the conversation shorter"
            )
        except AgentBusyError:
            return _BUSY
        except cancellation.CancelledFromInside:
            # The agent was told to stop, and the work with it. Nobody stopped this command.
            return _STOPPED
        return _told(outcome)

    ctx.use(COMMANDS).registry.add(
        ctx, "compact", Command(run, "Sum up the older part of the conversation to make room.")
    )
