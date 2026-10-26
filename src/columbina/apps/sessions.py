"""What ``columbina sessions`` does: the stored sessions of a directory, listed, shown and counted.

``list`` names the sessions that are stored for a working directory. ``show``
tells what happened in one of them, turn by turn: what the user said, the end
of what the model answered, and how the turn ended if it did not end well.
``fork`` copies one up to the last turn that came to its end, and names the
copy. The copy is a session of its own, to be taken up with ``--session-id``.
The one that it was made from is not changed, and may be open in a
conversation while it is copied: what is copied ends where its last whole
turn ends, whatever is being written after that. ``stats`` counts how the
tasks of the sessions went, see :mod:`columbina.stats`: those of this
directory, or those of all of them.

None of this holds a conversation, so none of it needs a model. What it needs
is what knows the log: the session service, and whatever declares events that
a reader may not pass over.

What comes back is text for a person. It holds what users and models wrote,
so whoever prints it makes it fit for a terminal first.
"""

import logging
import time
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Literal

from columbina.agent import replies
from columbina.apps.common import how_it_ended
from columbina.commands.sessions import listed
from columbina.commands.tools import used
from columbina.kernel import Context
from columbina.prices import SPENDING, Prices
from columbina.session import (
    SESSIONS,
    EventData,
    SessionError,
    SessionEvent,
    Sessions,
    StoredSession,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
    tools_used,
)
from columbina.session.store import SESSION_FILE, read_session, workspace_key
from columbina.stats import Tally, counted

logger = logging.getLogger("columbina.session")

__all__ = ["FORK", "LIST", "SHOW", "STATS", "What", "copied", "counted_sessions", "run", "shown"]

LIST = "list"
SHOW = "show"
FORK = "fork"
STATS = "stats"

What = Literal["list", "show", "fork", "stats"]
"""What is to be done with the stored sessions."""

_ANSWER_LINES = 12
_MARKS = {"user": ">", "steer": ">>"}
"""What stands before what the user said: as a task, and on the way."""


def _when(moment_ms: int) -> str:
    """Write a moment the way the clock of this machine shows it."""
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(moment_ms / 1000))


def _turn(events: list[SessionEvent[EventData]]) -> list[str]:
    """Tell one turn in short: what was said to the model, and the end of its last answer."""
    lines: list[str] = []
    ended: str | None = "(the turn did not come to its end)"
    for event in events:
        data = event.data
        if isinstance(data, UserMessageEvent) and data.source in _MARKS:
            said = "".join(block.text for block in data.message.content)
            lines.extend(f"{_MARKS[data.source]} {line}" for line in said.splitlines())
        elif isinstance(data, TurnEndEvent):
            ended = how_it_ended(data)
    # A reply that was cut off and taken up again is one answer with what went on from it.
    answers = [text for text in replies(event.data for event in events) if text.strip()]
    answer = answers[-1].splitlines() if answers else []
    if len(answer) > _ANSWER_LINES:
        earlier = len(answer) - _ANSWER_LINES
        lines.append(f"... {earlier} {'line' if earlier == 1 else 'lines'} before these")
    lines.extend(answer[-_ANSWER_LINES:])
    if ended is not None:
        lines.append(ended)
    return lines


def shown(stored: StoredSession) -> str:
    """Tell what a stored session holds, turn by turn."""
    header = stored.header
    lines = [f"session {header.id}, begun {_when(header.created_at)} in {header.cwd}"]
    if header.parent_session is not None:
        # A session that an agent delegated to is no copy of the one that delegated.
        what = "delegated by" if header.delegation_depth else "a copy of"
        lines.append(f"{what} the session {header.parent_session}")
    starts = [
        at for at, event in enumerate(stored.events) if isinstance(event.data, TurnStartEvent)
    ]
    if not starts:
        lines.append("(no turn has begun in this session)")
    # A turn reaches as far as the next one begins, and the last one to the end of the log.
    stops = [*starts[1:], len(stored.events)]
    for number, (start, stop) in enumerate(zip(starts, stops, strict=False), start=1):
        lines.extend(("", f"turn {number}", *_turn(stored.events[start:stop])))
    calls = tools_used(stored.events).tools
    if calls:
        lines.extend(("", *used(calls)))
    return "\n".join(lines)


async def copied(
    ctx: Context, sessions: Sessions, cwd: Path, source: str, session_id: str | None = None
) -> str:
    """Copy a stored session up to its last whole turn, and return the id of the copy.

    Raises:
        SessionError: The session cannot be read, no turn of it has come to
            its end, or the copy cannot be written.
        ValueError: An id cannot name a session.
    """
    events = sessions.read(cwd, source).events
    ends = [at for at, event in enumerate(events) if isinstance(event.data, TurnEndEvent)]
    if not ends:
        raise SessionError(
            f"no turn of the session {source!r} has come to its end, so there is nothing to copy"
        )
    scope = ctx.scope("copy")
    try:
        copy = sessions.fork(scope, cwd, source, upto=ends[-1], session_id=session_id)
        await copy.flush()
        return copy.id
    finally:
        await scope.fiber.dispose()


async def run(
    ctx: Context, cwd: Path, what: What, source: str | None = None, copy_id: str | None = None
) -> str:
    """Do one of the three things, and return what is to be printed.

    Args:
        ctx: A context in which the session service is there.
        cwd: The working directory whose sessions are meant.
        what: What to do.
        source: The id of the session to show or to copy.
        copy_id: The id that the copy is to have. One is made up by default.

    Raises:
        SessionError: The sessions are kept in memory, the session that was
            named cannot be read, or there is nothing of it to copy.
        ValueError: An id cannot name a session.
    """
    sessions = ctx.use(SESSIONS)
    if what == LIST:
        return listed(sessions.stored(cwd))
    assert source is not None
    if what == SHOW:
        return shown(sessions.read(cwd, source))
    return await copied(ctx, sessions, cwd, source, copy_id)


_DAY_MS = 24 * 60 * 60 * 1000


def _stored(
    sessions: Sessions, places: Sequence[Path]
) -> Iterator[tuple[int, Sequence[SessionEvent[EventData]]]]:
    """Read the stored sessions of some directories of the store, but those that cannot be read."""
    for place in places:
        try:
            entries = sorted(place.iterdir())
        except (FileNotFoundError, NotADirectoryError):
            continue
        for entry in entries:
            if not (entry / SESSION_FILE).is_file():
                continue
            try:
                stored = read_session(entry, sessions.kinds)
            except (SessionError, OSError) as error:
                logger.warning("the session in %s is not counted: %s", entry, error)
                continue
            yield stored.header.delegation_depth, stored.events


def counted_sessions(ctx: Context, cwd: Path, *, everywhere: bool, days: int, now_ms: int) -> Tally:
    """Count how the tasks of the stored sessions went in the last few days.

    Args:
        ctx: A context in which the session service is there.
        cwd: The working directory whose sessions are counted.
        everywhere: Whether the sessions of every working directory are counted.
        days: How many days back turns are counted.
        now_ms: The moment from which the days are counted back.

    Raises:
        SessionError: The sessions are kept in memory.
    """
    sessions = ctx.use(SESSIONS)
    if sessions.root is None:
        raise SessionError("sessions are kept in memory, so there are none stored to count")
    if everywhere:
        try:
            places = sorted(path for path in sessions.root.iterdir() if path.is_dir())
        except FileNotFoundError:
            places = []
    else:
        places = [sessions.root / workspace_key(cwd)]
    spending = ctx.get(SPENDING)
    prices = Prices() if spending is None else spending.prices
    since = now_ms - days * _DAY_MS
    return counted(_stored(sessions, places), since_ms=since, prices=prices)
