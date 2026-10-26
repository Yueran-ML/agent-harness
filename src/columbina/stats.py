"""How the tasks of stored sessions went: how their turns ended, what they cost, what was wasted.

``columbina sessions stats`` reads the logs of the sessions of a working
directory, or of every one with ``--all``, and adds up:

- how the turns ended: done, stopped, blocked by a limit, failed, cut off by
  the length of a reply;
- what the people who asked said of them with ``/verdict``: right or wrong.
  That is the rate of success that counts, where enough was said;
- how the turns of subagents ended, which is how often a task that was handed
  on came back done;
- what the requests cost, as :mod:`columbina.prices` works it out;
- what was wasted: calls that failed, by their codes; calls that were made
  again in the same turn with the same arguments and nothing written in
  between; files that were read again in the same turn although nothing had
  written them since.

Only the turns that began in the time asked about are counted, and the
requests of those turns.
"""

import json
from collections import Counter
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import cast

from columbina.prices import Prices, Spent, spent
from columbina.session import (
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.verdicts import VerdictEvent

__all__ = ["Tally", "counted", "told"]

_READS = frozenset({"read"})
"""The tools whose calls read a file that their ``path`` names."""


@dataclass(slots=True)
class Tally:
    """What was counted.

    Attributes:
        sessions: How many conversations.
        subagent_sessions: How many sessions of subagents.
        turns: How the turns of conversations ended, by how.
        subagent_turns: How the turns of subagents ended, by how.
        verdicts: What people said of turns: ``right`` and ``wrong``.
        calls: How many calls of tools had a result.
        failed: The calls that failed, by their codes.
        repeated: Calls that were the same as one before them in their turn,
            with nothing written in between.
        read_again: Reads of a file that had been read in the same turn and
            not written since.
        spent: What the requests cost.
    """

    sessions: int = 0
    subagent_sessions: int = 0
    turns: Counter[str] = field(default_factory=Counter[str])
    subagent_turns: Counter[str] = field(default_factory=Counter[str])
    verdicts: Counter[str] = field(default_factory=Counter[str])
    calls: int = 0
    failed: Counter[str] = field(default_factory=Counter[str])
    repeated: int = 0
    read_again: int = 0
    spent: Spent = field(default_factory=Spent)

    def as_json(self) -> dict[str, object]:
        """Return what was counted, for another program."""
        return {
            "sessions": self.sessions,
            "subagent_sessions": self.subagent_sessions,
            "turns": dict(self.turns),
            "subagent_turns": dict(self.subagent_turns),
            "verdicts": dict(self.verdicts),
            "calls": self.calls,
            "failed": dict(self.failed),
            "repeated": self.repeated,
            "read_again": self.read_again,
            "spent": {key: round(value, 6) for key, value in self.spent.money.items()},
            "unpriced": list(self.spent.unpriced),
        }


def _same(call: ToolCallEvent) -> str:
    """Return what two calls share where they are the same."""
    try:
        arguments: object = json.loads(call.arguments)
    except ValueError:
        arguments = call.arguments
    return json.dumps([call.name, arguments], sort_keys=True, ensure_ascii=False)


def _path(call: ToolCallEvent) -> str | None:
    try:
        arguments: object = json.loads(call.arguments)
    except ValueError:
        return None
    if not isinstance(arguments, dict):
        return None
    path = cast("dict[str, object]", arguments).get("path")
    return path if isinstance(path, str) else None


def _counted_session(
    events: Sequence[SessionEvent[EventData]],
    tally: Tally,
    *,
    delegated: bool,
    since_ms: int,
    prices: Prices,
) -> None:
    turns = tally.subagent_turns if delegated else tally.turns
    counting = False
    first = len(events)
    made: set[str] = set()
    read: set[str] = set()
    calls: dict[str, ToolCallEvent] = {}
    verdicts: dict[int, bool] = {}
    for index, event in enumerate(events):
        data = event.data
        if isinstance(data, TurnStartEvent):
            counting = event.time >= since_ms
            if counting:
                first = min(first, index)
            made.clear()
            read.clear()
        if not counting:
            continue
        if isinstance(data, TurnEndEvent):
            turns[data.reason] += 1
        elif isinstance(data, VerdictEvent):
            verdicts[data.turn] = data.right
        elif isinstance(data, ToolCallEvent):
            calls[data.call_id] = data
            same = _same(data)
            if same in made:
                tally.repeated += 1
            made.add(same)
            path = _path(data) if data.name in _READS else None
            if path is not None:
                if path in read:
                    tally.read_again += 1
                read.add(path)
        elif isinstance(data, ToolResultEvent):
            tally.calls += 1
            if data.error is not None:
                tally.failed[data.error.code] += 1
            wrote = (data.meta or {}).get("wrote")
            if isinstance(wrote, str):
                made.clear()
                read.discard(wrote)
                call = calls.get(data.message.call_id)
                if call is not None:
                    read.discard(_path(call) or "")
    for right in verdicts.values():
        tally.verdicts["right" if right else "wrong"] += 1
    # The header of the requests of a turn may stand before the turn, so it is read from there.
    tally.spent.add(spent(_from(events, first), prices))


def _from(
    events: Sequence[SessionEvent[EventData]], first: int
) -> Iterable[SessionEvent[EventData]]:
    """Return the events from a turn on, with the headers of requests that stand before it."""
    for index, event in enumerate(events):
        if index >= first or isinstance(event.data, RequestHeaderEvent):
            yield event


def counted(
    sessions: Iterable[tuple[int, Sequence[SessionEvent[EventData]]]],
    *,
    since_ms: int,
    prices: Prices,
) -> Tally:
    """Count how the tasks of some sessions went.

    Args:
        sessions: Each session: how deep it was delegated, and its events.
        since_ms: The moment from which turns are counted.
        prices: The prices that the requests are worked out by.
    """
    tally = Tally()
    for depth, events in sessions:
        if not any(
            isinstance(event.data, TurnStartEvent) and event.time >= since_ms for event in events
        ):
            continue
        if depth:
            tally.subagent_sessions += 1
        else:
            tally.sessions += 1
        _counted_session(events, tally, delegated=depth > 0, since_ms=since_ms, prices=prices)
    return tally


_ENDS: Mapping[str, str] = {
    "completed": "done",
    "aborted": "stopped",
    "interrupted": "broken off",
    "blocked": "blocked by a limit",
    "error": "failed",
    "max-tokens": "cut off by the length of a reply",
    "forked": "taken on in a copy",
}


def _ended(turns: Counter[str]) -> str:
    total = sum(turns.values())
    parts = [f"{count} {_ENDS.get(reason, reason)}" for reason, count in turns.most_common()]
    return f"{total} ({', '.join(parts)})" if parts else "0"


def told(tally: Tally) -> str:
    """Say what was counted, for a person."""
    lines = [f"Conversations: {tally.sessions}. Turns: {_ended(tally.turns)}."]
    judged = tally.verdicts["right"] + tally.verdicts["wrong"]
    if judged:
        rate = round(100 * tally.verdicts["right"] / judged)
        lines.append(
            f"Said to be right: {tally.verdicts['right']} of the {judged} turns that were judged "
            f"({rate}%)."
        )
    else:
        lines.append("No turn was judged: say /verdict right or /verdict wrong after a turn.")
    if tally.subagent_sessions:
        done = tally.subagent_turns["completed"]
        all_of_them = sum(tally.subagent_turns.values())
        lines.append(
            f"Subagents: {tally.subagent_sessions}, whose turns came back done {done} times of "
            f"{all_of_them}."
        )
    money = tally.spent.told()
    lines.append(f"Spent: about {money}." if money else "Spent: nothing that is known.")
    if tally.spent.unpriced:
        lines.append(f"No price is known for {', '.join(tally.spent.unpriced)}.")
    lines.append(f"Calls of tools: {tally.calls}.")
    if tally.failed:
        failed = ", ".join(f"{code} {count}" for code, count in tally.failed.most_common())
        lines.append(f"Failed: {sum(tally.failed.values())} ({failed}).")
    lines.append(
        f"Made again with nothing changed: {tally.repeated}. Files read again unchanged: "
        f"{tally.read_again}."
    )
    return "\n".join(lines)
