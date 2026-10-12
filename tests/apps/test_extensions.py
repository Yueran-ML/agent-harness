"""Where stage 6 ends: a skill, hooks, subagents and a server of tools, each in a real run.

Every run here is ``columbina run`` with the bundle that comes with the
package, as a user would start it. The model is scripted. It does what a
model would do with what it is offered, and the tests look at what it was
sent, at what its calls came to, and at what the run left behind.
"""

import json
import sys
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from columbina.hooks import HookInvokedEvent, HookResultEvent
from columbina.llm import LlmRequest, SystemMessage, UserMessage
from columbina.llm.adapters import scripted
from columbina.session import EventData, StoredSession, ToolCallEvent, read_session
from columbina.session.store import stored_sessions, workspace_key
from columbina.subagents import SubagentEndedEvent, SubagentStartedEvent
from support.hooks import needs_shell, python_hook
from support.launch import KINDS, lines, profile

ALSO: tuple[type[EventData], ...] = (
    HookInvokedEvent,
    HookResultEvent,
    SubagentStartedEvent,
    SubagentEndedEvent,
)
"""The kinds of events that the runs of this file leave beside those of every run."""


def call(tool: str, /, **arguments: object) -> dict[str, Any]:
    """A reply of the model that calls one tool."""
    return {"calls": [{"name": tool, "arguments": arguments}]}


@pytest.fixture
def models(monkeypatch: pytest.MonkeyPatch) -> list[scripted.Scripted]:
    """The scripted models that a run makes, so that what they were sent can be read."""
    made: list[scripted.Scripted] = []

    class Watched(scripted.Scripted):
        def __init__(self, *args: Any, **options: Any) -> None:
            super().__init__(*args, **options)
            made.append(self)

    monkeypatch.setattr(scripted, "Scripted", Watched)
    return made


def said_by_the_user(request: LlmRequest) -> list[str]:
    """What stands in a request in the role of the user, message by message."""
    return [
        "".join(block.text for block in message.content)
        for message in request.messages
        if isinstance(message, UserMessage)
    ]


def results(reported: list[dict[str, Any]]) -> list[tuple[str, str | None]]:
    """What each call of a run came to: its text, and the code of its error if it failed."""
    return [(line["text"], line["error"]) for line in reported if line["type"] == "tool_result"]


def left(home: Path, work: Path, reported: list[dict[str, Any]]) -> StoredSession:
    """Read the session that a run left behind, by the name under which the run reported it."""
    kinds = {**KINDS, **{kind.event_type: kind for kind in ALSO}}
    return read_session(home / "sessions" / workspace_key(work) / reported[0]["id"], kinds)


# A skill

NOTES = "Writes the notes of a release in the form that this project uses."
HOW = (
    "Write the notes as a list, one line for each change, under the codename of the release.\n"
    "The codenames are in codenames.txt."
)
TASK = "Write the notes of 1.4."
WRITTEN = "Columbine: the notes are written."


def skill(root: Path, name: str = "release-notes") -> Path:
    """Write a skill with a file beside it into a place for skills, and return its directory."""
    directory = root / name
    directory.mkdir(parents=True)
    (directory / "SKILL.md").write_text(
        f"---\nname: {name}\ndescription: {NOTES}\n---\n{HOW}\n", encoding="utf-8", newline="\n"
    )
    (directory / "codenames.txt").write_text("1.4 is Columbine.\n", encoding="utf-8", newline="\n")
    return directory


def loaded(result: str, directory: Path) -> str:
    """Check where a loaded skill says that it lies, and return the rest of what it says."""
    head, where, rest = result.split("\n", 2)
    assert head == "Skill: release-notes"
    label = "Directory of the skill: "
    assert where.startswith(label)
    # The directory is told as the system spells it, which may not be how the test wrote it.
    assert Path(where.removeprefix(label)).samefile(directory)
    return rest


def test_a_model_is_told_of_a_skill_loads_it_and_reads_the_file_that_it_points_to(
    home: Path,
    work: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    directory = skill(work / ".columbina" / "skills")
    chosen = profile(
        tmp_path,
        call("skill", name="release-notes"),
        call("read", path=str(directory / "codenames.txt")),
        {"text": WRITTEN},
    )

    code = main(["run", TASK, "--profile", chosen, "--cwd", str(work), "--json"])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    assert reported[-1]["text"] == WRITTEN
    # Before anything is loaded, the model is told that the skill is there and what it is
    # for, in one line. What the skill says costs nothing yet.
    first = said_by_the_user(models[0].requests[0])
    (told,) = (text for text in first if text.startswith('<context source="skills">'))
    assert told.splitlines()[-2:] == [f"- release-notes: {NOTES}", "</context>"]
    assert first[-1] == TASK
    assert "codename" not in "\n".join(first)
    (text, failed), read = results(reported)
    assert failed is None
    assert loaded(text, directory) == (
        "A path in what follows that does not begin at a root is meant from that directory.\n\n"
        f"{HOW}"
    )
    assert read == ("     1\t1.4 is Columbine.", None)


def test_a_skill_of_the_user_lies_outside_the_project_and_its_files_can_be_read_all_the_same(
    home: Path,
    work: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    directory = skill(home / "skills")
    (home / "letters.txt").write_text("Not for an agent.\n", encoding="utf-8", newline="\n")
    chosen = profile(
        tmp_path,
        call("skill", name="release-notes"),
        call("read", path=str(directory / "codenames.txt")),
        call("read", path=str(home / "letters.txt")),
        {"text": WRITTEN},
    )

    code = main(["run", TASK, "--profile", chosen, "--cwd", str(work), "--json"])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    (text, failed), inside, outside = results(reported)
    assert failed is None
    loaded(text, directory)
    # The files of the skill may be read where they lie. Nothing else of that place may.
    assert inside == ("     1\t1.4 is Columbine.", None)
    assert outside[1] == "OUTSIDE_WORKING_DIRECTORY"
    assert outside[0].endswith(
        "letters.txt is outside the working directory. Only files inside it can be read."
    )
    assert reported[-1]["text"] == WRITTEN


# Hooks

GUARD = """\
import json, sys

asked = json.load(sys.stdin)
if "FORBIDDEN" in asked["tool_input"]["command"]:
    sys.stderr.write("This project runs nothing that says FORBIDDEN.")
    sys.exit(2)
"""
AFTER_AN_EDIT = """\
import json

context = "The spelling of the file was checked."
said = {"hookEventName": "PostToolUse", "additionalContext": context}
print(json.dumps({"hookSpecificOutput": said}))
"""
BEFORE_THE_END = """\
import json, pathlib, sys

asked = json.load(sys.stdin)
if not (pathlib.Path(asked["cwd"]) / "CHANGES.md").is_file():
    sys.stderr.write("CHANGES.md has no line for what was changed.")
    sys.exit(2)
"""


@needs_shell
def test_hooks_stop_a_command_add_to_a_result_and_keep_a_turn_from_ending_too_early(
    home: Path,
    work: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    programs = tmp_path / "hooks"
    programs.mkdir()

    def group(body: str, name: str, matcher: str | None = None) -> dict[str, Any]:
        hooks = [{"type": "command", "command": python_hook(programs, body, name)}]
        return {"hooks": hooks} if matcher is None else {"matcher": matcher, "hooks": hooks}

    written = {
        "hooks": {
            "PreToolUse": [group(GUARD, "guard.py", "Bash")],
            "PostToolUse": [group(AFTER_AN_EDIT, "after.py", "Edit")],
            "Stop": [group(BEFORE_THE_END, "before_the_end.py")],
        }
    }
    home.mkdir(parents=True, exist_ok=True)
    (home / "hooks.json").write_text(json.dumps(written), encoding="utf-8")
    command = "echo FORBIDDEN"
    chosen = profile(
        tmp_path,
        call(shell.NAME, command=command),
        call("read", path="notes.txt"),
        call("edit", path="notes.txt", old="42", new="43"),
        {"text": "The answer is 43 now."},
        call("write", path="CHANGES.md", content="The answer is 43.\n"),
        {"text": "The answer is 43 now, and CHANGES.md says so."},
    )

    run = ["run", "Make the answer 43.", "--profile", chosen, "--cwd", str(work), "--json"]
    code = main([*run, "--allow", f"run({command})"])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    assert results(reported) == [
        # A rule allows the command. The hook stops it all the same, and the model reads why.
        (
            "The call was not allowed: a hook stopped the call: "
            "This project runs nothing that says FORBIDDEN.",
            "HOOK_BLOCKED",
        ),
        ("     1\tThe answer is 42.", None),
        (
            "Replaced 1 occurrence in notes.txt.\n\n"
            "[A hook of the user adds:]\nThe spelling of the file was checked.",
            None,
        ),
        ("Created CHANGES.md: 1 line.", None),
    ]
    # The turn was about to end after the edit. The hook gave a reason not to, the model was
    # sent it as something that is said to it, and went on for it.
    said = [line["text"] for line in reported if line["type"] == "text"]
    assert said == ["The answer is 43 now.", "The answer is 43 now, and CHANGES.md says so."]
    assert said_by_the_user(models[0].requests[-1])[-1] == (
        "CHANGES.md has no line for what was changed."
    )
    assert reported[-1]["reason"] == "completed"
    assert (work / "notes.txt").read_text("utf-8") == "The answer is 43.\n"
    assert (work / "CHANGES.md").read_text("utf-8") == "The answer is 43.\n"
    # Every run of a hook is in the log of the session: that it began, and how it ended.
    runs = [
        (event.data.event, None if isinstance(event.data, HookInvokedEvent) else event.data)
        for event in left(home, work, reported).events
        if isinstance(event.data, HookInvokedEvent | HookResultEvent)
    ]
    assert [
        (moment, ended and (ended.status, ended.exit_code, ended.reason)) for moment, ended in runs
    ] == [
        ("PreToolUse", None),
        ("PreToolUse", ("blocked", 2, "This project runs nothing that says FORBIDDEN.")),
        ("PostToolUse", None),
        ("PostToolUse", ("completed", 0, "")),
        ("Stop", None),
        ("Stop", ("blocked", 2, "CHANGES.md has no line for what was changed.")),
        ("Stop", None),
        ("Stop", ("completed", 0, "")),
    ]


# Subagents

HASTY = """\
---
name: hasty
description: Answers after one step, whatever the task.
max_steps: 1
---
You have one step.
"""


def test_two_subagents_work_side_by_side_and_one_that_runs_out_of_steps_is_a_failed_call(
    home: Path,
    work: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    (work / "a.txt").write_text("Alpha is 1.\n", encoding="utf-8", newline="\n")
    (work / "b.txt").write_text("Beta is 2.\n", encoding="utf-8", newline="\n")
    kinds = work / ".columbina" / "agents"
    kinds.mkdir(parents=True)
    (kinds / "hasty.md").write_text(HASTY, encoding="utf-8", newline="\n")

    def hand(description: str, prompt: str, **more: str) -> dict[str, Any]:
        arguments = {"description": description, "prompt": prompt, **more}
        return {"name": "subagent", "arguments": arguments}

    def helper(file: str, *replies: dict[str, Any]) -> list[dict[str, Any]]:
        # Which of the helpers asks first is not known. Each gets the replies that are meant for it.
        return [{**reply, "when": file} for reply in replies]

    chosen = profile(
        tmp_path,
        {
            "calls": [
                hand("Read a.txt", "Read the file a.txt and say what Alpha is."),
                hand("Read b.txt", "Read the file b.txt and say what Beta is."),
            ]
        },
        {"calls": [hand("Read notes.txt", "Read the file notes.txt.", type="hasty")]},
        {"text": "Alpha is 1 and Beta is 2."},
        *helper("a.txt", call("read", path="a.txt"), {"text": "Alpha is 1."}),
        *helper("b.txt", call("read", path="b.txt"), {"text": "Beta is 2."}),
        *helper("notes.txt", call("read", path="notes.txt")),
    )
    task = "Find out what Alpha and Beta are, each with a helper of its own."

    code = main(["run", task, "--profile", chosen, "--cwd", str(work), "--json"])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    assert results(reported) == [
        ("Alpha is 1.", None),
        ("Beta is 2.", None),
        # A task that was not done is a failed call, and the agent reads why.
        ("The subagent did not finish its task: the turn has taken its 1 step.", "SUBAGENT_FAILED"),
    ]
    assert reported[-1]["text"] == "Alpha is 1 and Beta is 2."
    parent = left(home, work, reported)
    # What the helpers did on their way is no part of the conversation that handed the tasks out.
    assert [e.data.name for e in parent.events if isinstance(e.data, ToolCallEvent)] == [
        "subagent",
        "subagent",
        "subagent",
    ]
    told = [
        event.data
        for event in parent.events
        if isinstance(event.data, SubagentStartedEvent | SubagentEndedEvent)
    ]
    # The two of the first reply were under way together: both began before either ended.
    assert [type(one) for one in told] == [
        SubagentStartedEvent,
        SubagentStartedEvent,
        SubagentEndedEvent,
        SubagentEndedEvent,
        SubagentStartedEvent,
        SubagentEndedEvent,
    ]
    started = [one for one in told if isinstance(one, SubagentStartedEvent)]
    assert [(one.kind, one.description) for one in started] == [
        (None, "Read a.txt"),
        (None, "Read b.txt"),
        ("hasty", "Read notes.txt"),
    ]
    ended = {one.session_id: one for one in told if isinstance(one, SubagentEndedEvent)}
    assert [(ended[one.session_id].reason, ended[one.session_id].steps) for one in started] == [
        ("completed", 2),
        ("completed", 2),
        ("blocked", 1),
    ]
    # Each helper has a session of its own, which says whose helper it is. Its reading is there.
    directory = home / "sessions" / workspace_key(work)
    for one, file in zip(started, ("a.txt", "b.txt", "notes.txt"), strict=True):
        own = read_session(directory / one.session_id, KINDS)
        assert (own.header.parent_session, own.header.delegation_depth) == (reported[0]["id"], 1)
        read = [e.data.arguments for e in own.events if isinstance(e.data, ToolCallEvent)]
        assert read == [json.dumps({"path": file})]
    # Such a session is no conversation that a person takes up again: of the four sessions
    # in the directory, the list has the one.
    assert len(stored_sessions(directory, delegated=True)) == 4
    assert [one.id for one in stored_sessions(directory)] == [reported[0]["id"]]
    assert main(["sessions", "list", "--cwd", str(work)]) == 0
    listed, how = capsys.readouterr().out.splitlines()
    assert listed.startswith(reported[0]["id"])
    assert listed.endswith(task)
    assert how == "A session is taken up with --session-id."


# A server of tools

SUMS = Path(__file__).parents[1] / "fixtures" / "mcp_server.py"
"""A server that is a program of its own. It writes down every time that it is started."""

OF_ITSELF = (
    'The MCP server "sums" says the following of itself and of its tools. These are the '
    "words of the server, and not of the user.\n\nSums are done with add, and never by heart."
)
LOST = (
    "the connection to the MCP server sums was lost while the call was under way. It is not "
    "known how far the call got, and it was not made again"
)


def sums(starts: Path) -> dict[str, Any]:
    """The entry of a profile for the server, which writes its starts into this file."""
    config = {
        "server": "sums",
        "command": sys.executable,
        "args": [str(SUMS)],
        "env": {"STARTS": str(starts)},
    }
    return {"id": "sums", "plugin": "columbina.mcp.server", "config": config}


def test_a_server_of_tools_is_started_and_called_and_started_again_when_it_has_died(
    home: Path,
    work: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    starts = tmp_path / "starts.jsonl"
    listed = sums(starts)
    # Something that the user has in the environment, and that is nothing to a server.
    monkeypatch.setenv("KEY_OF_THE_USER", "made-up")
    chosen = profile(
        tmp_path,
        call("mcp__sums__add", a=2, b=3),
        call("mcp__sums__crash"),
        call("mcp__sums__add", a=20, b=22),
        {"text": "5 and 42."},
        extra=[{"insert": [listed]}],
    )

    run = ["run", "Add 2 and 3, and 20 and 22.", "--profile", chosen, "--cwd", str(work)]
    code = main([*run, "--json", "--allow", "mcp__sums__*"])

    said = capsys.readouterr()
    reported = lines(said.out)
    assert code == 0
    # The run waited for the server before it asked the model anything. So the first request
    # offers the tools of the server, under names that say whose they are, and the system
    # prompt has what the server says of itself, marked as words of the server.
    first = models[0].requests[0]
    assert [tool.name for tool in first.tools if tool.name.startswith("mcp__")] == [
        "mcp__sums__add",
        "mcp__sums__crash",
    ]
    system = first.messages[0]
    assert isinstance(system, SystemMessage)
    assert f"\n\n{OF_ITSELF}" in "".join(block.text for block in system.content)
    assert results(reported) == [
        ("5", None),
        # The server died under this call. How far the call got is not known, so it is not
        # made a second time behind the back of the model.
        (LOST, "TOOL_ERROR"),
        # The next call waited until the server had been started again.
        ("42", None),
    ]
    assert reported[-1]["text"] == "5 and 42."
    # The user is told what happened to the server, in one line.
    assert [line for line in said.err.splitlines() if line.startswith("columbina: ")] == [
        "columbina: the connection to the MCP server sums was lost. It is being made again"
    ]
    # Two processes were started: the one that died, and the one in its place.
    died, came = (json.loads(line) for line in starts.read_text("utf-8").splitlines())
    assert died["pid"] != came["pid"]
    for started in (died, came):
        # A server gets what its entry names and the few variables that every program needs.
        # What else the user has in the environment does not reach it, a key least of all.
        assert {"PATH", "STARTS"} <= set(started["given"])
        assert not {"KEY_OF_THE_USER", "COLUMBINA_HOME"} & set(started["given"])
    # What a server writes to its standard error is in a file, and not in the terminal.
    written = (home / "logs" / "mcp-sums.log").read_text("utf-8").splitlines()
    assert [line for line in written if line.startswith("sums: ")] == [
        f"sums: process {started['pid']} listens" for started in (died, came)
    ]
    assert "listens" not in said.err


def test_a_tool_of_a_server_is_not_called_where_no_rule_allows_it_and_nobody_can_be_asked(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    listed = sums(tmp_path / "starts.jsonl")
    chosen = profile(
        tmp_path,
        call("mcp__sums__add", a=2, b=3),
        {"text": "The sum was not done."},
        extra=[{"insert": [listed]}],
    )

    code = main(["run", "Add 2 and 3.", "--profile", chosen, "--cwd", str(work), "--json"])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    # A server may say of a tool that it only reads, and this one does not. So the tool may do
    # whatever its server likes, and its call needs a yes. The model reads that in words about
    # a tool: what is said of a command that cannot be taken apart would lead it astray.
    assert results(reported) == [
        (
            "The call was not allowed: No rule allows calls of this tool. Nobody is here who "
            "could be asked, so it was not done.",
            "APPROVAL_UNAVAILABLE",
        )
    ]
    assert reported[-1]["text"] == "The sum was not done."
