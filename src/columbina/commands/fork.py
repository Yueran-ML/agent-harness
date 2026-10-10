"""The command that has the conversation go on in a copy of its session.

This module is a plugin. Loading it registers the command ``/fork``.

A conversation can be taken two ways from one point. The command copies the
session as it stands and has the conversation go on in the copy. The session
that it came from stays as it was, and can be taken up again by its id. Each
of the two goes its own way from there.

The whole session is copied. A command is carried out while the agent is at
rest, so the log ends between two turns: what stands after the last turn is
what was done to the session since, such as a summary that the user asked
for, and that belongs to how the session stands.

Going over to another session is up to whatever holds the conversation. A
frontend that can do it provides :data:`columbina.commands.SWITCH`. Where
none does, the command says so and copies nothing.
"""

from columbina.agent import AGENTS, Agent, Agents, AgentSpec
from columbina.commands import COMMANDS, SWITCH, Command
from columbina.kernel import Context
from columbina.session import TURN, TurnEndEvent

__all__ = ["apply", "inject", "name"]

name = "command-fork"
inject = (COMMANDS, AGENTS)

_NO_WAY = (
    "This program cannot go over to another session, so the session was not copied. "
    "`columbina sessions fork ID` copies a stored one."
)
_RUNNING = "A turn is running. The session can be copied when the agent is at rest."
_NOTHING = "No turn of this session has come to its end, so there is nothing to copy yet."


def apply(ctx: Context, config: object) -> None:
    """Register the command that has the conversation go on in a copy of its session."""
    agents: Agents = ctx.use(AGENTS)

    async def run(agent: Agent, rest: str) -> str:
        switch = ctx.get(SWITCH)
        if switch is None:
            return _NO_WAY
        session = agent.session
        if TURN.of(session).in_turn:
            return _RUNNING
        if not any(isinstance(event.data, TurnEndEvent) for event in session.events):
            return _NOTHING

        async def copied(scope: Context, spec: AgentSpec) -> Agent:
            return await agents.fork(scope, spec, session)

        copy = await switch(copied)
        return (
            f"The conversation goes on in a copy of the session {session.id}: {copy.id}\n"
            f"The session {session.id} stays as it was."
        )

    told = "Go on in a copy of this session, and leave the session as it is."
    ctx.use(COMMANDS).registry.add(ctx, "fork", Command(run, told))
