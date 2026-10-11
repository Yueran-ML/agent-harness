"""The base program: what a run does once every plugin of the bundle is loaded.

Each test here runs ``columbina run`` on the bundle as it is shipped. Only the
model is another one: what it says is written down in the profile.
"""

import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

from columbina.approval import ApprovalAskedEvent, ApprovalDecidedEvent, ApprovalPolicyEvent
from columbina.apps.headless import DEFAULT_PROMPT
from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from columbina.retry import RetryEvent
from columbina.sandbox import SandboxModeEvent
from columbina.session import (
    RequestHeaderEvent,
    StoredSession,
    SystemMessageEvent,
    UserMessageEvent,
)
from support.hooks import needs_shell, python_hook
from support.launch import lines, profile, stored

SIX_TIMES_SEVEN = "python -c 'print(6 * 7)'"
MANY_WORDS = "python -c 'print(40000 * chr(119) + chr(10) + 20000 * (chr(111) + chr(32)))'"
"""A command that says more than a model is shown: a long line, and many short words."""
NOBODY_TO_ASK = (
    "The call was not allowed: No rule allows this command, and nothing holds it here. "
    "Nobody is here who could be asked, so it was not done."
)
RUN_BY = {
    "powershell": "the tool powershell, which is Windows PowerShell 5.1 and not bash",
    "bash": "the tool bash",
}


def call(name: str, **arguments: object) -> dict[str, Any]:
    """A reply of the model that calls one tool."""
    return {"calls": [{"name": name, "arguments": arguments}]}


@dataclass(frozen=True)
class Ran:
    """How a run went.

    Attributes:
        code: The exit code.
        results: The text and the error code of the result of each tool call.
        errors: What the program wrote beside its report.
    """

    code: int
    results: list[tuple[str, str | None]]
    errors: str


@dataclass(frozen=True)
class Program:
    """``columbina run`` in a working directory, with a home that is thrown away."""

    home: Path
    work: Path
    tmp_path: Path
    capsys: pytest.CaptureFixture[str]

    def run(
        self,
        *replies: dict[str, Any],
        flags: tuple[str, ...] = (),
        extra: list[dict[str, Any]] | None = None,
    ) -> Ran:
        """Run a task to which the model gives these replies and then says that it is done."""
        chosen = profile(self.tmp_path, *replies, {"text": "Done."}, extra=extra)
        run = ["run", "Do it.", "--profile", chosen, "--cwd", str(self.work)]
        code = main([*run, "--session-id", "s-1", "--json", *flags])
        captured = self.capsys.readouterr()
        results = [
            (line["text"], line["error"])
            for line in lines(captured.out)
            if line["type"] == "tool_result"
        ]
        return Ran(code, results, captured.err)

    def session(self) -> StoredSession:
        """The session that the run left behind."""
        return stored(self.home, self.work)

    def told(self, source: str) -> list[str]:
        """What a source of context told the model, without the frame around it."""
        return [
            "\n".join(event.data.message.content[0].text.split("\n")[1:-1])
            for event in self.session().events
            if isinstance(event.data, UserMessageEvent) and event.data.source == source
        ]

    def offered(self) -> list[str]:
        """The names of the tools that the model was offered."""
        headers = [
            event.data
            for event in self.session().events
            if isinstance(event.data, RequestHeaderEvent)
        ]
        return [tool.name for tool in headers[0].tools]


@pytest.fixture
def program(home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> Program:
    return Program(home, work, tmp_path, capsys)


# The tools


def test_the_model_is_offered_the_tools_of_the_base_program(program: Program) -> None:
    ran = program.run()

    assert ran.code == 0
    assert ran.errors == ""
    assert program.offered() == [
        "read",
        "write",
        "edit",
        "glob",
        "grep",
        "todo_write",
        "skill",
        "subagent",
        shell.NAME,
    ]


def test_a_file_in_the_working_directory_can_be_made_and_changed(program: Program) -> None:
    ran = program.run(
        call("write", path="made/new.txt", content="one\ntwo\n"),
        call("read", path="notes.txt"),
        call("edit", path="notes.txt", old="42", new="43"),
    )

    assert ran.results == [
        ("Created made/new.txt: 2 lines.", None),
        ("     1\tThe answer is 42.", None),
        ("Replaced 1 occurrence in notes.txt.", None),
    ]
    assert (program.work / "made" / "new.txt").read_bytes() == b"one\ntwo\n"
    assert (program.work / "notes.txt").read_text(encoding="utf-8") == "The answer is 43.\n"


def test_a_file_that_the_model_has_not_seen_is_not_changed(program: Program) -> None:
    ran = program.run(call("edit", path="notes.txt", old="42", new="43"))

    assert [error for _, error in ran.results] == ["NOT_READ"]
    assert (program.work / "notes.txt").read_text(encoding="utf-8") == "The answer is 42.\n"


def test_files_are_found_by_their_names_and_by_what_they_hold(program: Program) -> None:
    (program.work / "src").mkdir()
    (program.work / "src" / "answer.py").write_text("ANSWER = 42\n", encoding="utf-8")

    ran = program.run(call("glob", pattern="**/*.py"), call("grep", pattern="ANSWER|answer"))

    assert ran.results == [
        ("src/answer.py", None),
        ("notes.txt:1:The answer is 42.\nsrc/answer.py:1:ANSWER = 42", None),
    ]


# What keeps the tools in check


def test_no_file_outside_the_working_directory_is_touched(program: Program) -> None:
    (program.tmp_path / "theirs.txt").write_text("theirs\n", encoding="utf-8")

    ran = program.run(
        call("read", path="../theirs.txt"),
        call("write", path="../mine.txt", content="mine"),
    )

    assert [error for _, error in ran.results] == [
        "OUTSIDE_WORKING_DIRECTORY",
        "OUTSIDE_WORKING_DIRECTORY",
    ]
    assert not (program.tmp_path / "mine.txt").exists()


def test_a_command_does_not_run_while_nobody_can_be_asked(program: Program) -> None:
    ran = program.run(call(shell.NAME, command=SIX_TIMES_SEVEN))

    assert ran.code == 0
    assert ran.results == [(NOBODY_TO_ASK, "APPROVAL_UNAVAILABLE")]
    asked = [
        event.data
        for event in program.session().events
        if isinstance(event.data, ApprovalAskedEvent | ApprovalDecidedEvent)
    ]
    assert asked == [
        ApprovalAskedEvent(
            call_id="call-1", tool=shell.NAME, summary=f"{shell.NAME}: {SIX_TIMES_SEVEN}"
        ),
        ApprovalDecidedEvent(call_id="call-1", outcome="unavailable"),
    ]


def test_a_rule_of_the_command_line_lets_a_command_run(program: Program) -> None:
    ran = program.run(
        call(shell.NAME, command=SIX_TIMES_SEVEN),
        call(shell.NAME, command="python --version"),
        flags=("--allow", "run(python -c *)"),
    )

    assert ran.results == [
        ("42\n[exit code 0]", None),
        (NOBODY_TO_ASK, "APPROVAL_UNAVAILABLE"),
    ]


def test_the_model_can_write_down_what_it_means_to_do(program: Program) -> None:
    things = [{"content": "Read the notes", "status": "in_progress"}, {"content": "Answer"}]

    ran = program.run(call("todo_write", items=things))

    assert ran.code == 0
    assert ran.results == [
        ("The list holds 2 things: 0 completed, 1 in progress, 1 pending.", None)
    ]
    assert "todo/write" in [event.type for event in program.session().events]


def test_what_a_command_says_is_kept_whole_and_the_model_is_shown_what_it_can_take(
    program: Program,
) -> None:
    ran = program.run(call(shell.NAME, command=MANY_WORDS), flags=("--allow", "run(python -c *)"))

    ((said, error),) = ran.results
    assert error is None
    # The program has a place for the whole of it, and the result says where that is.
    last = said.splitlines()[-1]
    assert last.startswith("(The whole output has ")
    where = Path(last.split(" bytes and is in ", 1)[1].split(". ", 1)[0])
    assert where.is_relative_to(program.home / "spill")
    kept = where.read_text(encoding="utf-8")
    assert kept.count("w") == 40000
    assert kept.count("o") == 20000
    # What the tool kept in memory is more than a result may come to, so it was cut again.
    assert "[... a part is left out here ...]" in said
    assert len(said) < 64_000
    assert said.startswith("wwww")


def test_a_session_begins_with_the_preset_of_the_bundle(program: Program) -> None:
    program.run()

    settings = [
        event.data
        for event in program.session().events
        if isinstance(event.data, SandboxModeEvent | ApprovalPolicyEvent)
    ]
    assert settings == [SandboxModeEvent(mode="workspace-write"), ApprovalPolicyEvent(policy="ask")]


def test_a_session_that_may_only_read_changes_nothing(program: Program) -> None:
    ran = program.run(
        call("write", path="new.txt", content="x"),
        call("read", path="notes.txt"),
        call("edit", path="notes.txt", old="42", new="43"),
        flags=("--permission", "read-only"),
    )

    assert [error for _, error in ran.results] == ["READ_ONLY", None, "READ_ONLY"]
    assert sorted(path.name for path in program.work.iterdir()) == ["notes.txt"]
    assert "sandbox mode: read-only. No file can be written." in program.told("context/runtime")[0]


def test_a_session_with_full_access_runs_a_command_without_a_rule(program: Program) -> None:
    ran = program.run(
        call(shell.NAME, command=SIX_TIMES_SEVEN),
        call("write", path="../beside.txt", content="beside"),
        flags=("--permission", "full-access"),
    )

    assert ran.results == [("42\n[exit code 0]", None), ("Created ../beside.txt: 1 line.", None)]
    assert (program.tmp_path / "beside.txt").read_text(encoding="utf-8") == "beside"


def test_the_rules_of_the_user_are_read_from_the_home_of_the_program(program: Program) -> None:
    program.home.mkdir()
    (program.home / "permissions.yml").write_text(
        "deny:\n  - read(notes.txt)\nallow:\n  - run(python -c *)\n", encoding="utf-8"
    )

    ran = program.run(call("read", path="notes.txt"), call(shell.NAME, command=SIX_TIMES_SEVEN))

    assert ran.errors == ""
    assert ran.results == [
        ("The call was not allowed: The rule read(notes.txt) is against it.", "DENIED"),
        ("42\n[exit code 0]", None),
    ]


def test_the_rules_of_a_project_refuse_and_never_allow(program: Program) -> None:
    (program.work / ".columbina").mkdir()
    (program.work / ".columbina" / "permissions.yml").write_text(
        "deny:\n  - read(notes.txt)\nallow:\n  - run\n", encoding="utf-8"
    )

    ran = program.run(call("read", path="notes.txt"), call(shell.NAME, command=SIX_TIMES_SEVEN))

    assert [error for _, error in ran.results] == ["DENIED", "APPROVAL_UNAVAILABLE"]
    assert ran.errors == (
        "columbina: the rules of the project in .columbina/permissions.yml hold rules under "
        "allow, which a project cannot give, so they were passed over\n"
    )


WAITING_FOR_PERMISSIONS = (
    "columbina: tool-shell (columbina.builtin_tools.shell) is waiting for the service "
    "'permissions'\n"
)
WAITING_FOR_A_SANDBOX = (
    "columbina: permissions (columbina.permissions) is waiting for the service 'sandbox'\n"
    "columbina: tool-shell (columbina.builtin_tools.shell) is waiting for the services "
    "'sandbox', 'permissions'\n"
)


@pytest.mark.parametrize(
    ("missing", "said"),
    [("permissions", WAITING_FOR_PERMISSIONS), ("sandbox", WAITING_FOR_A_SANDBOX)],
)
def test_the_program_does_not_start_with_a_shell_tool_that_nothing_keeps_in_check(
    program: Program, missing: str, said: str
) -> None:
    ran = program.run(
        call(shell.NAME, command=SIX_TIMES_SEVEN), extra=[{"id": missing, "disabled": True}]
    )

    assert ran.code == 2
    assert ran.results == []
    assert ran.errors == said
    assert not program.home.exists()


def test_without_the_permissions_and_the_shell_tool_the_other_tools_are_there(
    program: Program,
) -> None:
    off = [{"id": "permissions", "disabled": True}, {"id": "tool-shell", "disabled": True}]

    ran = program.run(
        call("write", path="new.txt", content="x"),
        call(shell.NAME, command=SIX_TIMES_SEVEN),
        extra=off,
    )

    assert ran.errors == ""
    assert [error for _, error in ran.results] == [None, "UNKNOWN_TOOL"]
    assert program.offered() == [
        "read",
        "write",
        "edit",
        "glob",
        "grep",
        "todo_write",
        "skill",
        "subagent",
    ]


def test_without_a_sandbox_nothing_is_written(program: Program) -> None:
    off = [{"id": name, "disabled": True} for name in ("sandbox", "permissions", "tool-shell")]

    ran = program.run(
        call("write", path="new.txt", content="x"), call("read", path="notes.txt"), extra=off
    )

    assert [error for _, error in ran.results] == ["NOT_WRITABLE", None]
    assert not (program.work / "new.txt").exists()


# What the model is told


def test_the_model_is_given_the_instructions_of_the_user_and_of_the_project(
    program: Program,
) -> None:
    program.home.mkdir()
    (program.home / "AGENTS.md").write_text("Answer in one sentence.\n", encoding="utf-8")
    (program.work / "AGENTS.md").write_text("Run the checks before you answer.\n", encoding="utf-8")

    program.run()

    assert program.told("context/instructions") == [
        "These files hold instructions for work in this directory. They come from the widest "
        "to the most specific one, and where two of them disagree, the later one counts.\n\n"
        f"==> {program.home / 'AGENTS.md'} <==\nAnswer in one sentence.\n\n"
        "==> AGENTS.md <==\nRun the checks before you answer."
    ]


def test_the_model_is_told_where_it_runs_and_what_it_may_do(program: Program) -> None:
    program.run()

    assert program.told("context/runtime") == [
        "approval: ask. A call that needs a yes is put to the user, if somebody is there to "
        "ask.\n"
        "commands are held: not at all. A command can do whatever the user of this machine can "
        "do.\n"
        f"commands are run by: {RUN_BY[shell.NAME]}\n"
        "sandbox mode: workspace-write. Files in the working directory can be written, except "
        "in .git and .columbina. Outside it no file can be read or written.\n"
        f"working directory: {program.work}"
    ]


def test_what_the_model_is_told_stands_before_the_task(program: Program) -> None:
    (program.work / "AGENTS.md").write_text("Run the checks before you answer.\n", encoding="utf-8")

    program.run()

    sources = [
        event.data.source
        for event in program.session().events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert sources == ["context/instructions", "context/runtime", "user"]


def test_a_plugin_adds_to_the_system_prompt(
    program: Program, monkeypatch: pytest.MonkeyPatch
) -> None:
    (program.tmp_path / "house_rules_plugin.py").write_text(
        "from columbina.system_prompt import SYSTEM_PROMPT, Section\n\n"
        "name = 'house-rules'\n"
        "inject = (SYSTEM_PROMPT,)\n\n\n"
        "def apply(ctx, config):\n"
        "    section = Section('Keep every answer short.', order=10)\n"
        "    ctx.use(SYSTEM_PROMPT).sections.add(ctx, 'house-rules', section)\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(sys, "path", [str(program.tmp_path), *sys.path])
    house = {"insert": [{"id": "house-rules", "plugin": "house_rules_plugin"}]}

    ran = program.run(extra=[house])

    systems = [
        event.data.message.content[0].text
        for event in program.session().events
        if isinstance(event.data, SystemMessageEvent)
    ]
    assert ran.errors == ""
    assert systems == [f"{DEFAULT_PROMPT}\n\nKeep every answer short."]


# What the user adds to the program


def test_the_skills_of_the_user_are_looked_for_in_the_home_of_the_program(
    program: Program,
) -> None:
    skill = program.home / "skills" / "release-notes"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(
        "---\nname: release-notes\ndescription: Write the notes of a release.\n---\n"
        "Take the changes from the log.\n",
        encoding="utf-8",
    )

    load = {"calls": [{"name": "skill", "arguments": {"name": "release-notes"}}]}

    ran = program.run(load)

    (told,) = program.told("context/skills")
    assert ran.errors == ""
    assert told.splitlines()[1:] == ["- release-notes: Write the notes of a release."]
    assert ran.results == [
        (
            "Skill: release-notes\n"
            f"Directory of the skill: {skill}\n"
            "A path in what follows that does not begin at a root is meant from that directory.\n"
            "\n"
            "Take the changes from the log.",
            None,
        )
    ]


def test_the_kinds_of_subagent_of_the_user_are_looked_for_in_the_home_of_the_program(
    program: Program,
) -> None:
    kinds = program.home / "agents"
    kinds.mkdir(parents=True)
    (kinds / "reviewer.md").write_text(
        "---\nname: reviewer\ndescription: Reviews a change.\n---\nYou review changes.\n",
        encoding="utf-8",
    )

    ran = program.run()

    (told,) = program.told("context/agents")
    assert ran.errors == ""
    assert told.splitlines()[1:] == ["- reviewer: Reviews a change."]


@needs_shell
def test_the_hooks_of_the_user_are_read_from_the_home_of_the_program(program: Program) -> None:
    program.home.mkdir()
    said = python_hook(program.tmp_path, "print('The branch is main.')")
    hooks = {"SessionStart": [{"hooks": [{"type": "command", "command": said}]}]}
    (program.home / "hooks.json").write_text(json.dumps({"hooks": hooks}), encoding="utf-8")

    ran = program.run()

    assert ran.errors == ""
    assert program.told("context/hooks") == ["The branch is main."]


@needs_shell
def test_the_hooks_of_a_project_are_run_once_the_user_has_trusted_them(
    program: Program,
) -> None:
    said = python_hook(program.tmp_path, "print('Run the checks with make.')")
    hooks = {"SessionStart": [{"hooks": [{"type": "command", "command": said}]}]}
    theirs = program.work / ".columbina" / "hooks.json"
    theirs.parent.mkdir()
    theirs.write_text(json.dumps({"hooks": hooks}), encoding="utf-8")

    # The way a user does it: with the program as it is shipped, in the directory of the project.
    assert main(["hooks", "trust", "--cwd", str(program.work)]) == 0
    program.capsys.readouterr()
    ran = program.run()

    assert ran.errors == ""
    assert program.told("context/hooks") == ["Run the checks with make."]


def test_the_hooks_of_a_project_that_nobody_trusted_are_not_run_and_that_is_said(
    program: Program,
) -> None:
    hooks = {"SessionStart": [{"hooks": [{"type": "command", "command": "exit 2"}]}]}
    theirs = program.work / ".columbina" / "hooks.json"
    theirs.parent.mkdir()
    theirs.write_text(json.dumps({"hooks": hooks}), encoding="utf-8")

    ran = program.run()

    assert ran.code == 0
    assert program.told("context/hooks") == []
    assert "the hooks of this project are not run" in ran.errors
    assert "'columbina hooks trust' trusts it" in ran.errors


# What goes wrong on the way


def test_a_request_that_fails_for_a_passing_reason_is_made_again(program: Program) -> None:
    busy = {"failure": {"message": "the service is busy", "code": "SERVER", "status": 503}}
    quick = {"base_delay_seconds": 0.001, "max_delay_seconds": 0.004, "jitter": 0}

    ran = program.run(busy, extra=[{"id": "retry", "config": quick}])

    retries = [
        event.data for event in program.session().events if isinstance(event.data, RetryEvent)
    ]
    assert ran.code == 0
    assert retries == [RetryEvent(turn=1, step=1, attempt=1, code="SERVER", delay_ms=1)]
