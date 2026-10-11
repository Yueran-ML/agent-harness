"""``columbina serve`` as a process of its own, driven over real pipes.

Everything else that is tested of the service process is tested inside the
process of the tests. Here the program is started the way a desktop
application starts it, with the bundle that is shipped. Only the model is
another one: what it says is written down in the profile.
"""

import json
import os
import queue
import re
import subprocess
import sys
import threading
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import pytest

from columbina.builtin_tools import shell
from support.launch import profile
from support.protocol import DOCS

SIX_TIMES_SEVEN = "python -c 'print(6 * 7)'"


class Program:
    """The program, serving, and a test at the other end of its pipes.

    Attributes:
        heard: Every message that the program sent, in the order in which it
            was sent.
        raw: The lines that those messages came as.
    """

    def __init__(self, arguments: list[str], env: dict[str, str]) -> None:
        self._child = subprocess.Popen(
            [sys.executable, "-m", "columbina", "serve", *arguments],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self._lines: queue.Queue[bytes | None] = queue.Queue()
        self._errors: list[bytes] = []
        self._ids = iter(range(1, 1000))
        self.heard: list[dict[str, Any]] = []
        self.raw: list[bytes] = []
        self._threads = [
            threading.Thread(target=self._read, daemon=True),
            threading.Thread(target=self._log, daemon=True),
        ]
        for thread in self._threads:
            thread.start()

    def _read(self) -> None:
        assert self._child.stdout is not None
        for line in self._child.stdout:
            self._lines.put(line)
        self._lines.put(None)

    def _log(self) -> None:
        assert self._child.stderr is not None
        self._errors.append(self._child.stderr.read())

    def say(self, message: dict[str, Any]) -> None:
        assert self._child.stdin is not None
        self._child.stdin.write(json.dumps(message, ensure_ascii=False).encode() + b"\n")
        self._child.stdin.flush()

    def ask(self, method: str, params: dict[str, Any] | None = None) -> int:
        request: dict[str, Any] = {"jsonrpc": "2.0", "id": next(self._ids), "method": method}
        if params is not None:
            request["params"] = params
        self.say(request)
        return request["id"]

    def until(self, holds: Callable[[], bool], seconds: float = 60.0) -> None:
        """Read what the program sends until something holds, or fail with what was read."""
        while not holds():
            try:
                line = self._lines.get(timeout=seconds)
            except queue.Empty:
                raise AssertionError(f"nothing more came. So far: {self.kinds()}") from None
            assert line is not None, f"the program closed its output. So far: {self.kinds()}"
            self.raw.append(line)
            # Every line of the output is a message. Anything else fails here.
            message = json.loads(line)
            assert message["jsonrpc"] == "2.0"
            self.heard.append(message)

    def result(self, method: str, params: dict[str, Any] | None = None) -> Any:
        asked = self.ask(method, params)

        def answered() -> bool:
            return any("method" not in one and one.get("id") == asked for one in self.heard)

        self.until(answered)
        answer = next(one for one in self.heard if "method" not in one and one.get("id") == asked)
        assert "error" not in answer, answer
        return answer["result"]

    def told(self, method: str) -> list[dict[str, Any]]:
        return [one["params"] for one in self.heard if one.get("method") == method]

    def kinds(self) -> list[str]:
        """What the program sent, in short."""
        said: list[str] = []
        for one in self.heard:
            if one.get("method") == "session.event":
                said.append(f"session.event {one['params']['event']['type']}")
            elif one.get("method") == "session.status":
                said.append(f"session.status {one['params']['status']}")
            elif "method" in one:
                said.append(one["method"])
            else:
                said.append("answer")
        return said

    def left(self) -> tuple[int, list[bytes], str]:
        """Close the input of the program, and return its exit code and what it still said.

        Returns:
            The exit code, the lines that were in its output and not read
            yet, and everything that it wrote to its standard error.
        """
        assert self._child.stdin is not None
        self._child.stdin.close()
        code = self._child.wait(60)
        for thread in self._threads:
            thread.join(30)
        rest: list[bytes] = []
        while True:
            line = self._lines.get(timeout=30)
            if line is None:
                break
            rest.append(line)
        return code, rest, self._errors[0].decode("utf-8", "replace")

    def kill(self) -> None:
        self._child.kill()
        self._child.wait(60)
        for stream in (self._child.stdin, self._child.stdout, self._child.stderr):
            assert stream is not None
            stream.close()


@pytest.fixture
def started(home: Path, work: Path, tmp_path: Path) -> Iterator[Callable[..., Program]]:
    """Starts the program with a model that gives the replies of a test."""
    programs: list[Program] = []

    def start(*replies: dict[str, Any], extra: list[dict[str, Any]] | None = None) -> Program:
        chosen = profile(tmp_path, *replies, extra=extra, extends="serve")
        # No key of any service goes to a process that asks no service for anything.
        env = {
            name: value
            for name, value in os.environ.items()
            if not re.search("KEY|TOKEN|SECRET|PASSWORD", name.upper())
        }
        env["COLUMBINA_HOME"] = str(home)
        env["PYTHONPATH"] = os.pathsep.join([str(tmp_path), env.get("PYTHONPATH", "")])
        program = Program(["--profile", chosen, "--cwd", str(work)], env)
        programs.append(program)
        return program

    yield start
    for program in programs:
        program.kill()


def call(name: str, **arguments: object) -> dict[str, Any]:
    """A reply of the model that calls one tool."""
    return {"calls": [{"name": name, "arguments": arguments}]}


def statuses(program: Program) -> list[str]:
    return [told["status"] for told in program.told("session.status")]


def worked(program: Program) -> None:
    """Read until the agent has been at work and is at rest again."""
    program.until(lambda: "running" in statuses(program) and statuses(program)[-1] == "idle")


@pytest.mark.timeout(180)
def test_a_conversation_is_held_over_the_pipes_of_a_real_process(
    started: Callable[..., Program], home: Path, work: Path
) -> None:
    program = started(call(shell.NAME, command=SIX_TIMES_SEVEN), {"text": "It is 42."})

    hello = program.result("initialize", {"protocol": 1, "client": {"name": "a test"}})
    opened = program.result("session/open", {})
    program.ask("session/prompt", {"text": "What is six times seven?"})
    # The command needs a yes, and the program asks for it.
    program.until(lambda: "approval/ask" in program.kinds())
    question = next(one for one in program.heard if one.get("method") == "approval/ask")
    program.say({"jsonrpc": "2.0", "id": question["id"], "result": {"outcome": "allowed-once"}})
    worked(program)
    code, rest, errors = program.left()

    assert hello["cwd"] == str(work)
    assert (opened["resumed"], opened["events"]) == (False, 0)
    assert question["params"]["tool"] == shell.NAME
    assert question["params"]["target"] == SIX_TIMES_SEVEN
    assert question["params"]["session_id"] == opened["session_id"]
    events = [told["event"] for told in program.told("session.event")]
    # Every event of the turn, each once, in its order.
    assert [event["seq"] for event in events] == list(range(len(events)))
    results = [event["data"] for event in events if event["type"] == "tool/result"]
    assert [result["error"] for result in results] == [None]
    assert "42" in results[0]["message"]["content"][0]["text"]
    replies = [event["data"] for event in events if event["type"] == "assistant/message"]
    assert replies[-1]["message"]["content"] == [{"type": "text", "text": "It is 42."}]
    assert events[-1]["type"] == "turn/end"
    assert events[-1]["data"]["reason"] == "completed"
    assert statuses(program) == ["idle", "running", "idle"]
    # The program left by itself when its input was closed, and had nothing more to say.
    assert (code, rest) == (0, [])
    # What it told is what its log holds.
    (stored,) = home.rglob("session.v1.jsonl")
    lines = stored.read_bytes().splitlines()
    assert [json.loads(line) for line in lines[1 : len(events) + 1]] == events
    assert errors == ""


CHATTY = """
import os

from columbina.session import SESSION_EVENT

name = "chatty"


def apply(ctx, config):
    print("printed while the program loads")

    def heard(committed):
        if committed.event.type == "turn/start":
            print("printed in a turn")
            os.write(1, b"written to the standard output in a turn" + os.linesep.encode())

    ctx.on(SESSION_EVENT, heard)
"""


@pytest.mark.timeout(180)
def test_what_a_plugin_prints_does_not_stand_among_the_messages(
    started: Callable[..., Program], tmp_path: Path
) -> None:
    (tmp_path / "chatty_plugin.py").write_text(CHATTY, encoding="utf-8")
    chatty = {"insert": [{"id": "chatty", "plugin": "chatty_plugin"}]}
    program = started({"text": "Hello!"}, extra=[chatty])

    program.result("initialize", {"protocol": 1})
    program.result("session/open", {})
    program.ask("session/prompt", {"text": "Say hello."})
    worked(program)
    code, rest, errors = program.left()

    # Reading the output as messages did not fail, so there was nothing else in it.
    assert (code, rest) == (0, [])
    assert [line.strip() for line in errors.splitlines()] == [
        "printed while the program loads",
        "printed in a turn",
        "written to the standard output in a turn",
    ]


# The document


def turn_in_the_document() -> list[str]:
    """What section 12 of the document says is sent in the first turn of a session."""
    text = (DOCS / "README.md").read_text(encoding="utf-8")
    section = text[text.index("\n## 12. ") :]
    block = section[section.index("```\n") + 4 :]
    block = block[: block.index("```")]
    said: list[str] = []
    for line in block.splitlines():
        if "session/prompt" in line:
            continue
        if "→" in line:
            # The answer of the interface to the question.
            said.append("the yes")
            continue
        what = line.split("←", 1)[1].split("(", 1)[0].split()
        if what[0] == "session.event":
            said.extend(f"session.event {kind.rstrip(',')}" for kind in what[1:])
        elif what[0] == "session.status":
            said.append(f"session.status {what[1]}")
        else:
            said.append(what[0])
    return said


def in_short(kinds: list[str]) -> list[str]:
    """A list of what was sent, with every run of chunks as one."""
    short: list[str] = []
    for kind in kinds:
        if kind == "session.stream" and short and short[-1] == "session.stream":
            continue
        short.append(kind)
    return short


@pytest.mark.timeout(180)
def test_the_first_turn_of_a_session_is_told_the_way_the_document_says(
    started: Callable[..., Program], work: Path
) -> None:
    (work / "AGENTS.md").write_text("Answer in one sentence.\n", encoding="utf-8")
    program = started(
        {"reasoning": "A command will tell.", **call(shell.NAME, command=SIX_TIMES_SEVEN)},
        {"text": "It is 42."},
    )
    program.result("initialize", {"protocol": 1})
    program.result("session/open", {})
    # Where the agent stands is told once after the session is open. The turn comes after.
    program.until(lambda: statuses(program) == ["idle"])
    before = len(program.heard)

    asked = program.ask("session/prompt", {"text": "What is six times seven?"})
    program.until(lambda: "approval/ask" in program.kinds())
    question = next(one for one in program.heard if one.get("method") == "approval/ask")
    at_the_question = len(program.heard)
    program.say({"jsonrpc": "2.0", "id": question["id"], "result": {"outcome": "allowed-once"}})
    worked(program)
    program.left()

    kinds = program.kinds()
    told = [*kinds[before:at_the_question], "the yes", *kinds[at_the_question:]]
    # The answer to the request itself is the one thing that the document leaves out.
    answer = next(
        place
        for place, one in enumerate(program.heard)
        if "method" not in one and one.get("id") == asked
    )
    assert kinds[answer] == "answer"
    told.remove("answer")
    assert in_short(told) == in_short(turn_in_the_document())
