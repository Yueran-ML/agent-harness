"""The real DeepSeek service with what stage 6 added. Skipped unless DEEPSEEK_API_KEY is set.

``tests/apps/test_extensions.py`` shows that a skill, hooks, subagents and a
server of tools work when a model uses them, and the model there is
scripted. These four tests show that a real model does use them. Each gives
the model one of the four and a task that calls for it. It looks at whether
the model took it up, and at nothing of how the model worded its answer.

The four runs cost a few thousand tokens each. Run them with::

    uv run pytest tests/live -m live
"""

import json
import os
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

from columbina.apps.launcher import main
from columbina.hooks import HookInvokedEvent, HookResultEvent
from columbina.session import EventData, StoredSession, read_session
from columbina.session.store import workspace_key
from columbina.subagents import SubagentEndedEvent, SubagentStartedEvent
from support.hooks import needs_shell, python_hook
from support.launch import KINDS

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(300),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

ALSO: tuple[type[EventData], ...] = (
    HookInvokedEvent,
    HookResultEvent,
    SubagentStartedEvent,
    SubagentEndedEvent,
)
"""The kinds of events that these runs leave beside those of every run."""


@pytest.fixture
def home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A data directory for Columbina that is thrown away after the test."""
    directory = tmp_path / "home"
    monkeypatch.setenv("COLUMBINA_HOME", str(directory))
    return directory


@pytest.fixture
def work(tmp_path: Path) -> Path:
    """A working directory with one file in it."""
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "notes.txt").write_text("The answer is 42.\n", encoding="utf-8", newline="\n")
    return directory


def run(*args: str, capsys: pytest.CaptureFixture[str]) -> list[dict[str, Any]]:
    """Run a task to its end, and return what the run reported."""
    code = main(["run", *args, "--json"])
    reported = [json.loads(line) for line in capsys.readouterr().out.splitlines()]
    assert code == 0
    assert reported[-1]["reason"] == "completed"
    return reported


def called(reported: list[dict[str, Any]]) -> list[list[tuple[str, dict[str, Any]]]]:
    """The calls of a run, reply by reply: the tool and the arguments of each."""
    replies: list[list[tuple[str, dict[str, Any]]]] = []
    for line in reported:
        if line["type"] == "status":
            replies.append([])
        elif line["type"] == "tool_call":
            replies[-1].append((line["name"], json.loads(line["arguments"])))
    return [reply for reply in replies if reply]


def answered(
    reported: list[dict[str, Any]], tool: str
) -> list[tuple[dict[str, Any], str, str | None]]:
    """The calls of one tool in a run: the arguments of each, what came of it, and its error."""
    came = {line["id"]: line for line in reported if line["type"] == "tool_result"}
    return [
        (json.loads(line["arguments"]), came[line["id"]]["text"], came[line["id"]]["error"])
        for line in reported
        if line["type"] == "tool_call" and line["name"] == tool
    ]


def left(home: Path, work: Path, reported: list[dict[str, Any]]) -> StoredSession:
    """Read the session that a run left behind."""
    kinds = {**KINDS, **{kind.event_type: kind for kind in ALSO}}
    return read_session(home / "sessions" / workspace_key(work) / reported[0]["id"], kinds)


def test_a_skill_that_fits_the_task_is_loaded_before_anything_else_is_done(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    directory = work / ".columbina" / "skills" / "release-notes"
    directory.mkdir(parents=True)
    (directory / "SKILL.md").write_text(
        "---\n"
        "name: release-notes\n"
        "description: Writes the notes of a release in the form that this project uses.\n"
        "---\n"
        "Write the notes as a list, one line for each change, under the codename of the release.\n"
        "The codenames are in codenames.txt.\n",
        encoding="utf-8",
        newline="\n",
    )
    (directory / "codenames.txt").write_text("1.4 is Columbine.\n", encoding="utf-8", newline="\n")
    task = (
        "Write the notes of release 1.4 into NOTES.md. Two things changed: the search is "
        "faster, and a long name no longer breaks the login."
    )

    reported = run(task, "--cwd", str(work), capsys=capsys)

    first, *_ = called(reported)
    # The model was told of the skill in one line. That was enough for it to load the skill
    # before it did anything else,
    assert first[0] == ("skill", {"name": "release-notes"})
    # and to follow it to the file that it points to: the codename stands nowhere else.
    assert "Columbine" in (work / "NOTES.md").read_text("utf-8")


BEFORE_THE_END = """\
import json, pathlib, sys

asked = json.load(sys.stdin)
if not (pathlib.Path(asked["cwd"]) / "CHANGES.md").is_file():
    sys.stderr.write("CHANGES.md has no line for what was changed. Write one.")
    sys.exit(2)
"""


@needs_shell
def test_what_a_hook_says_at_the_end_of_a_turn_is_acted_on(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    programs = tmp_path / "hooks"
    programs.mkdir()
    hook = {"type": "command", "command": python_hook(programs, BEFORE_THE_END)}
    home.mkdir(parents=True, exist_ok=True)
    (home / "hooks.json").write_text(
        json.dumps({"hooks": {"Stop": [{"hooks": [hook]}]}}), encoding="utf-8"
    )

    reported = run("Change the answer in notes.txt to 43.", "--cwd", str(work), capsys=capsys)

    assert "43" in (work / "notes.txt").read_text("utf-8")
    # Nothing in the task spoke of this file. The hook did, when the model was about to end.
    assert (work / "CHANGES.md").is_file()
    ended = [
        event.data.status
        for event in left(home, work, reported).events
        if isinstance(event.data, HookResultEvent)
    ]
    assert (ended[0], ended[-1]) == ("blocked", "completed")


def test_two_tasks_that_have_nothing_to_do_with_each_other_are_handed_out_in_one_reply(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (work / "a.txt").write_text("Alpha is 7391.\n", encoding="utf-8", newline="\n")
    (work / "b.txt").write_text("Beta is 2846.\n", encoding="utf-8", newline="\n")
    task = (
        "The files a.txt and b.txt have nothing to do with each other. Hand each of them to a "
        "subagent of its own, both at the same time, and then tell me what Alpha and Beta are."
    )

    reported = run(task, "--cwd", str(work), capsys=capsys)

    handed = [[tool for tool, _ in reply].count("subagent") for reply in called(reported)]
    assert max(handed) >= 2
    told = [
        type(event.data)
        for event in left(home, work, reported).events
        if isinstance(event.data, SubagentStartedEvent | SubagentEndedEvent)
    ]
    # Both were under way before either was done.
    assert told[:2] == [SubagentStartedEvent, SubagentStartedEvent]
    # Each file was read by a helper that came to its end, and gave back the number of that
    # file and not the other one: the one who asked did not have to read for them.
    found = [text for _, text, error in answered(reported, "subagent") if error is None]
    assert any("7391" in text and "2846" not in text for text in found)
    assert any("2846" in text and "7391" not in text for text in found)
    # What the helpers found came back to the one who asked.
    assert "7391" in reported[-1]["text"]
    assert "2846" in reported[-1]["text"]


def test_a_tool_of_a_server_is_called_where_the_server_says_what_it_is_for(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    program = Path(__file__).parents[1] / "fixtures" / "mcp_server.py"
    sums = {
        "id": "sums",
        "plugin": "columbina.mcp.server",
        "config": {
            "server": "sums",
            "command": sys.executable,
            "args": [str(program)],
            "env": {"STARTS": str(tmp_path / "starts.jsonl")},
        },
    }
    patch = tmp_path / "sums.yml"
    patch.write_text(yaml.safe_dump([{"insert": [sums]}]), encoding="utf-8")
    task = "What is 123456789 plus 987654321?"

    reported = run(
        task, "--cwd", str(work), "--patch", str(patch), "--allow", "mcp__sums__*", capsys=capsys
    )

    # The server says of itself that sums are done with its tool. The model calls the tool
    # with the two numbers, and the tool gives the sum: a call that failed would not do.
    sums_done = [
        (sorted(arguments.values()), text, error)
        for arguments, text, error in answered(reported, "mcp__sums__add")
    ]
    assert ([123456789, 987654321], "1111111110", None) in sums_done
    digits = "".join(sign for sign in reported[-1]["text"] if sign.isdigit())
    assert "1111111110" in digits
