"""What a model service reads from its cache when ``/effort`` changes the level, measured.

``columbina serve`` is started as a desktop application starts it, with what the command
line passes on, such as ``--provider`` and ``--model``. It finds the key of the service as it
always does, in the variable of the environment or among the keys that ``columbina setup``
stored; this script never reads a key and never prints one. One conversation is held in a
directory of its own that is empty. Each turn chooses a level with ``/effort`` and asks a
question that needs no tool, and the token counts of each request are read from the events
that the program tells. Each level is asked for twice in a row, so that a turn after a change
shows what the change cost and the one after it shows the cache when nothing changed.

A row tells, for one turn, how many tokens of input the service read from its cache, how many
it wrote to it, and how many it read anew, and what share of the input the cache gave. The
first request reads nothing: there is nothing in the cache yet. Run it with::

    uv run python -m bench.cache_levels --provider anthropic --model claude-opus-5-5
    uv run python -m bench.cache_levels --provider deepseek --levels default,max,max,ultra

It talks to the real service and costs what the turns cost: about the system prompt and the
tools once, and a few hundred tokens a turn beyond what the cache gives. A service with a
cache of a few minutes needs the turns to follow each other quickly, which they do.
"""

import argparse
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
from collections.abc import Callable, Sequence
from dataclasses import asdict, dataclass
from typing import Any

__all__ = ["Row", "Serving", "main", "measure", "plan", "row_of", "table"]

QUESTION = "Turn {turn}. Answer with one word and use no tool: what colour is the sky at noon?"
"""What each turn asks. A short answer keeps the turn cheap, and no tool keeps it one request."""

_ARGUMENTS = ("--provider", "--model", "--profile")
"""What the command line passes on to ``columbina serve``."""


def plan(offered: Sequence[str], named: Sequence[str] | None = None) -> list[str]:
    """Return the level of each turn: every level on offer twice, and the first again at the end.

    Args:
        offered: The levels that the service offers, the least first.
        named: The levels that the command line names, which are taken as they are.
    """
    if named:
        return list(named)
    first, *rest = offered
    return [first, first, *(level for level in rest for _ in range(2)), first, first]


@dataclass(frozen=True, slots=True)
class Row:
    """What the requests of one turn cost.

    Attributes:
        turn: The number of the turn, from 1.
        level: The level of effort that held for it.
        requests: How many requests the turn made.
        read: The tokens of input that the service read from its cache.
        written: The tokens of input that it wrote to its cache.
        fresh: The tokens of input that it read anew, without writing them.
        output: The tokens of the replies.
    """

    turn: int
    level: str
    requests: int
    read: int
    written: int
    fresh: int
    output: int

    @property
    def share(self) -> float:
        """What share of the input the cache gave."""
        total = self.read + self.written + self.fresh
        return self.read / total if total else 0.0


def row_of(turn: int, level: str, events: Sequence[dict[str, Any]]) -> Row:
    """Return what a turn cost, from the events that the program told of it.

    A reply that failed and was asked again counts with the one that came: both were paid for.
    """
    usages: list[dict[str, int]] = [
        event["data"].get("usage") or {}
        for event in events
        if event.get("type") in ("assistant/message", "assistant/attempt")
    ]
    return Row(
        turn=turn,
        level=level,
        requests=len(usages),
        read=sum(int(usage.get("cache_read_tokens", 0)) for usage in usages),
        written=sum(int(usage.get("cache_write_tokens", 0)) for usage in usages),
        fresh=sum(int(usage.get("input_tokens", 0)) for usage in usages),
        output=sum(int(usage.get("output_tokens", 0)) for usage in usages),
    )


def table(rows: Sequence[Row]) -> str:
    """Return the rows as a table to read."""
    head = f"{'turn':>4}  {'level':<8} {'requests':>8} {'read':>8} {'written':>8} {'fresh':>8}"
    lines = [f"{head} {'output':>7} {'cached':>7}"]
    lines.extend(
        f"{row.turn:>4}  {row.level:<8} {row.requests:>8} {row.read:>8} {row.written:>8} "
        f"{row.fresh:>8} {row.output:>7} {row.share:>7.1%}"
        for row in rows
    )
    return "\n".join(lines)


class Serving:
    """``columbina serve`` in a process of its own, and this script at the other end of its pipes.

    What the program writes to its standard error is not read: it goes where the error of
    this script goes, as the program wrote it.
    """

    def __init__(self, arguments: Sequence[str], cwd: str, *, seconds: float = 300.0) -> None:
        self._child = subprocess.Popen(
            [sys.executable, "-m", "columbina", "serve", *arguments, "--cwd", cwd],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
        )
        self._lines: queue.Queue[bytes | None] = queue.Queue()
        self._ids = iter(range(1, 1_000_000))
        self._seconds = seconds
        self.heard: list[dict[str, Any]] = []
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def _read(self) -> None:
        assert self._child.stdout is not None
        for line in self._child.stdout:
            self._lines.put(line)
        self._lines.put(None)

    def _say(self, message: dict[str, Any]) -> None:
        assert self._child.stdin is not None
        self._child.stdin.write(json.dumps(message, ensure_ascii=False).encode() + b"\n")
        self._child.stdin.flush()

    def until(self, holds: Callable[[], bool]) -> None:
        """Read what the program says until something holds, and answer what it asks meanwhile."""
        while not holds():
            line = self._lines.get(timeout=self._seconds)
            if line is None:
                raise RuntimeError("columbina serve ended before it answered")
            message: dict[str, Any] = json.loads(line)
            self.heard.append(message)
            if message.get("method") == "approval/ask":
                # Nothing that needs a yes is let through: the turns ask for no tool.
                answer = {"outcome": "rejected"}
                self._say({"jsonrpc": "2.0", "id": message["id"], "result": answer})

    def result(self, method: str, params: dict[str, Any] | None = None) -> Any:
        """Ask the program for something and return its answer.

        Raises:
            RuntimeError: The program answered with an error.
        """
        asked = next(self._ids)
        self._say({"jsonrpc": "2.0", "id": asked, "method": method, "params": params or {}})

        def answered() -> bool:
            return any("method" not in one and one.get("id") == asked for one in self.heard)

        self.until(answered)
        answer = next(one for one in self.heard if "method" not in one and one.get("id") == asked)
        if "error" in answer:
            raise RuntimeError(f"{method}: {answer['error'].get('message')}")
        return answer["result"]

    def told(self, method: str) -> list[dict[str, Any]]:
        """Return the parameters of what the program told by a method, in order."""
        return [one["params"] for one in self.heard if one.get("method") == method]

    def turn(self, text: str) -> list[dict[str, Any]]:
        """Give the agent a task, wait until it is at rest again, and return the events of it."""
        before = len(self.told("session.event"))
        self.result("session/prompt", {"text": text})

        def ended() -> bool:
            events = self.told("session.event")[before:]
            over = any(told["event"]["type"] == "turn/end" for told in events)
            return over and self.told("session.status")[-1]["status"] == "idle"

        self.until(ended)
        return [told["event"] for told in self.told("session.event")[before:]]

    def close(self) -> int:
        """Close the input of the program, and return its exit code."""
        assert self._child.stdin is not None
        self._child.stdin.close()
        try:
            code = self._child.wait(60)
        except subprocess.TimeoutExpired:
            self._child.kill()
            code = self._child.wait(60)
        # What it still said is not read: the conversation is over.
        self._reader.join(30)
        assert self._child.stdout is not None
        self._child.stdout.close()
        return code


def measure(serving: Serving, named: Sequence[str] | None = None) -> list[Row]:
    """Hold the conversation, a turn at each level, and return what each turn cost.

    Raises:
        RuntimeError: A level is not on offer, or a turn did not end well.
    """
    serving.result("initialize", {"protocol": 1, "client": {"name": "bench.cache_levels"}})
    serving.result("session/open", {})
    serving.until(lambda: bool(serving.told("session.status")))
    offered: list[str] = serving.told("session.status")[-1]["model"]["efforts"]
    rows: list[Row] = []
    for turn, level in enumerate(plan(offered, named), start=1):
        said = serving.result("session/command", {"line": f"/effort {level}"})["text"]
        if not said.startswith(f"The effort is {level}"):
            raise RuntimeError(said)
        events = serving.turn(QUESTION.format(turn=turn))
        end = next(event for event in events if event["type"] == "turn/end")["data"]
        if end["reason"] != "completed":
            raise RuntimeError(f"turn {turn} ended as {end['reason']}: {end.get('detail')}")
        rows.append(row_of(turn, level, events))
    return rows


def main(argv: Sequence[str]) -> int:
    """Measure, print the table or the rows as JSON, and return the exit code."""
    parser = argparse.ArgumentParser(prog="python -m bench.cache_levels", description=__doc__)
    for name in _ARGUMENTS:
        parser.add_argument(name, metavar=name.removeprefix("--").upper())
    parser.add_argument(
        "--levels",
        metavar="LIST",
        help="the level of each turn, separated by commas; every level on offer twice by default",
    )
    parser.add_argument("--json", action="store_true", help="print the rows as lines of JSON")
    given = parser.parse_args(argv)
    passed: list[str] = []
    for name in _ARGUMENTS:
        value = getattr(given, name.removeprefix("--"))
        if value is not None:
            passed.extend([name, value])
    named = [level.strip() for level in given.levels.split(",")] if given.levels else None
    with tempfile.TemporaryDirectory(prefix="columbina-cache-") as work:
        serving = Serving(passed, os.path.realpath(work))
        try:
            rows = measure(serving, named)
        except (RuntimeError, queue.Empty) as error:
            print(f"bench.cache_levels: {error or 'no answer in time'}", file=sys.stderr)
            serving.close()
            return 1
        serving.close()
    if given.json:
        for row in rows:
            print(json.dumps({**asdict(row), "share": round(row.share, 4)}))
    else:
        print(table(rows))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
