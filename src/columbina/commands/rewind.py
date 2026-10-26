"""The command that goes back to before a turn: its files, its conversation, or both.

This module is a plugin. Loading it registers the command ``/rewind``, which
needs :mod:`columbina.checkpoints`.

- ``/rewind`` lists the turns that can be gone back to, the last ten, with
  the beginning of what was said in each.
- ``/rewind N`` goes back to before turn N: the files are put back as they
  stood then, and the conversation goes on in a copy of the session that ends
  before that turn, as ``/fork`` copies one. The session as it stands is kept,
  and can be taken up again by its id. What was said in turn N is told, to
  be said again or otherwise.
- ``/rewind N files`` puts back the files alone, and ``/rewind N
  conversation`` goes back in the conversation alone.
- ``/rewind back`` puts the files back as they stood before they were last
  taken back, which undoes that.

The files are kept before they are put back, so going back can always be
undone with ``/rewind back``.
"""

import time
from collections.abc import Sequence

from columbina.agent import AGENTS, Agent, Agents, AgentSpec
from columbina.checkpoints import CHECKPOINTS, CheckpointError, CheckpointEvent, checkpoints_of
from columbina.commands import COMMANDS, SWITCH, Command
from columbina.kernel import Context
from columbina.session import (
    TURN,
    EventData,
    Session,
    SessionEvent,
    TurnStartEvent,
    UserMessageEvent,
)

__all__ = ["apply", "inject", "name", "said_in"]

name = "command-rewind"
inject = (COMMANDS, AGENTS, CHECKPOINTS)

_LISTED = 10
"""How many turns are listed."""

_NAMED = 10
"""How many files that were put back are named."""

_SAID = 70
"""How many characters of what was said in a turn are shown in the list."""

_USAGE = (
    "Say /rewind to list the turns, /rewind N to go back to before turn N, /rewind N files or "
    "/rewind N conversation for one of the two, and /rewind back to undo going back."
)
_RUNNING = "A turn is running. Going back waits until the agent is at rest."
_NONE = "No checkpoint was kept in this conversation, so there is nothing to go back to."


def said_in(session: Session, turn: int) -> str:
    """Return what the user said to begin a turn, or an empty text."""
    began = False
    for event in session.events:
        data = event.data
        if isinstance(data, TurnStartEvent):
            if began:
                break
            began = data.turn == turn
        elif began and isinstance(data, UserMessageEvent) and data.source == "user":
            return " ".join(getattr(block, "text", "") for block in data.message.content).strip()
    return ""


def _short(text: str) -> str:
    line = " ".join(text.split())
    return line if len(line) <= _SAID else line[: _SAID - 3] + "..."


def _listed(session: Session, kept: Sequence[CheckpointEvent], moments: dict[str, int]) -> str:
    turns = [checkpoint for checkpoint in kept if not checkpoint.before_going_back][-_LISTED:]
    lines = ["The turns that can be gone back to, the last ones last:"]
    for checkpoint in turns:
        when = time.strftime("%H:%M", time.localtime(moments[checkpoint.commit] / 1000))
        said = _short(said_in(session, checkpoint.turn)) or "(nothing was said)"
        lines.append(f"  {checkpoint.turn:>3}  {when}  {said}")
    lines.append("/rewind N goes back to before turn N.")
    return "\n".join(lines)


def _before_the_turn(events: Sequence[SessionEvent[EventData]], turn: int) -> int | None:
    """Return the position of the last event of the conversation before a turn began, if any.

    What was put in the inbox for the turn is left out with it: a copy that held
    it would begin the turn again by itself.
    """
    start = next(
        event.seq
        for event in events
        if isinstance(event.data, TurnStartEvent) and event.data.turn == turn
    )
    position = start - 1
    while position >= 0 and events[position].type.startswith("agent/inbox"):
        position -= 1
    return position if position >= 0 else None


def apply(ctx: Context, config: object) -> None:
    """Register the command that goes back to before a turn."""
    agents: Agents = ctx.use(AGENTS)
    checkpoints = ctx.use(CHECKPOINTS)

    async def files_back(agent: Agent, commit: str, turn: int) -> str:
        session = agent.session
        cwd = agent.spec.cwd
        try:
            now = await checkpoints.keep(cwd)
            session.append(
                CheckpointEvent(turn=TURN.of(session).turn, commit=now, before_going_back=True)
            )
            changed = await checkpoints.changed(cwd, commit)
            await checkpoints.go_back(cwd, commit)
        except CheckpointError as error:
            return f"The files were not put back: {error}."
        if not changed:
            return f"The files are as they were before turn {turn}: nothing was put back."
        named = ", ".join(changed[:_NAMED]) + (", ..." if len(changed) > _NAMED else "")
        counted = "1 file was" if len(changed) == 1 else f"{len(changed)} files were"
        return f"{counted} put back as they were before turn {turn}: {named}."

    async def conversation_back(agent: Agent, turn: int) -> str:
        switch = ctx.get(SWITCH)
        if switch is None:
            return "This program cannot go over to another session, so the conversation stays."
        session = agent.session
        upto = _before_the_turn(session.events, turn)
        if upto is None:
            return "Nothing stands before turn 1. /new begins another conversation."
        said = said_in(session, turn)

        async def copied(scope: Context, spec: AgentSpec) -> Agent:
            return await agents.fork(scope, spec, session, upto=upto)

        copy = await switch(copied)
        told = (
            f"The conversation goes on from before turn {turn}, in a copy of the session "
            f"{session.id}: {copy.id}. The session {session.id} stays as it was."
        )
        return f"{told}\nWhat was said in turn {turn}: {said}" if said else told

    async def run(agent: Agent, rest: str) -> str:
        words = rest.split()
        session = agent.session
        if TURN.of(session).in_turn:
            return _RUNNING
        kept = checkpoints_of(session)
        if not words:
            if not kept:
                return _NONE
            moments = {
                event.data.commit: event.time
                for event in session.events
                if isinstance(event.data, CheckpointEvent)
            }
            return _listed(session, kept, moments)
        if words == ["back"]:
            before = [checkpoint for checkpoint in kept if checkpoint.before_going_back]
            if not before:
                return "The files were never put back in this conversation."
            try:
                await checkpoints.go_back(agent.spec.cwd, before[-1].commit)
            except CheckpointError as error:
                return f"The files were not put back: {error}."
            return "The files are as they were before they were last put back."
        if not words[0].isdigit() or words[1:] not in ([], ["files"], ["conversation"]):
            return _USAGE
        turn = int(words[0])
        what = words[1] if len(words) == 2 else "both"
        found = [c for c in kept if c.turn == turn and not c.before_going_back]
        if not found:
            return f"No checkpoint was kept before turn {turn}."
        said: list[str] = []
        if what in ("files", "both"):
            said.append(await files_back(agent, found[0].commit, turn))
        if what in ("conversation", "both"):
            said.append(await conversation_back(agent, turn))
        return "\n".join(said)

    ctx.use(COMMANDS).registry.add(
        ctx,
        "rewind",
        Command(run, "Go back to before a turn: /rewind [N [files|conversation]], /rewind back."),
    )
