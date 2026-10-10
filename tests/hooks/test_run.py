"""Running a hook: what it is told, how its answer is read, and what several answers come to."""

import json
import sys
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.hooks.config import Hook
from columbina.hooks.run import (
    MAX_COMMAND,
    MAX_OUTPUT_BYTES,
    Result,
    cut,
    from_clixml,
    merged,
    payload,
    read,
    run,
)
from support.hooks import SHELL, needs_shell, python_hook

NO_REASON = "A hook stopped this and gave no reason."
LIMIT = 10_000


def done(
    code: int | None = 0,
    out: str = "",
    err: str = "",
    *,
    timed_out: bool = False,
    omitted: int = 0,
) -> process.Completed:
    """How the command of a hook ended, as the one who ran it says it."""
    return process.Completed(
        None if timed_out else code,
        timed_out,
        out,
        len(out.encode()),
        omitted,
        0,
        err,
        len(err.encode()),
        0,
    )


def answered(event: str, fields: dict[str, Any], limit: int = LIMIT) -> Result:
    """What is read of a hook that ended well and wrote this JSON."""
    return read(event, done(0, json.dumps(fields, ensure_ascii=False) + "\n"), limit)


def about(event: str, **fields: Any) -> dict[str, Any]:
    return {"hookSpecificOutput": {"hookEventName": event, **fields}}


# What a hook is told


def test_a_hook_is_told_what_happens_in_one_line_of_json() -> None:
    told = payload(
        "PreToolUse",
        "s-1",
        Path("/home/sessions/s-1.jsonl"),
        Path("/work"),
        {"tool_name": "Bash", "tool_input": {"command": "echo 月亮"}, "tool_use_id": "call-1"},
    )

    assert told.endswith(b"\n")
    assert told.count(b"\n") == 1
    # What is no ASCII goes as it is, and not as numbers.
    assert "月亮".encode() in told
    assert json.loads(told) == {
        "session_id": "s-1",
        "transcript_path": str(Path("/home/sessions/s-1.jsonl")),
        "cwd": str(Path("/work")),
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo 月亮"},
        "tool_use_id": "call-1",
    }


# The plain answer: an exit code


def test_exit_code_two_stops_what_the_hook_was_asked_about() -> None:
    assert read("PreToolUse", done(2, "ignored", "  Not on a Friday.\n"), LIMIT) == Result(
        "blocked", "block", "Not on a Friday.", exit_code=2
    )


def test_a_hook_that_says_stop_and_gives_no_reason_stops_all_the_same() -> None:
    assert read("Stop", done(2, "", " \n"), LIMIT) == Result(
        "blocked", "block", NO_REASON, exit_code=2
    )


@pytest.mark.parametrize("event", ["SessionStart", "UserPromptSubmit"])
def test_what_a_hook_writes_at_these_moments_is_for_the_model_to_read(event: str) -> None:
    assert read(event, done(0, "  Today is a holiday.\n"), LIMIT) == Result(
        context="Today is a holiday.", exit_code=0
    )


@pytest.mark.parametrize("event", ["PreToolUse", "PostToolUse", "Stop"])
def test_what_a_hook_writes_at_the_other_moments_is_not_used(event: str) -> None:
    assert read(event, done(0, "formatted 3 files\n"), LIMIT) == Result(exit_code=0)
    assert read(event, done(0, ""), LIMIT) == Result(exit_code=0)


@pytest.mark.parametrize(
    ("code", "err", "why"),
    [
        (1, "", "it ended with the exit code 1"),
        (3, "Traceback\n  File x\n", "it ended with the exit code 3: Traceback"),
        (-9, "killed", "it ended with the exit code -9: killed"),
        (1, "x" * 500, "it ended with the exit code 1: " + "x" * 100),
    ],
)
def test_a_hook_that_fails_stops_nothing(code: int, err: str, why: str) -> None:
    result = read("PreToolUse", done(code, "{}", err), LIMIT)

    assert (result.status, result.decision, result.exit_code) == ("failed", None, code)
    (warning,) = result.warnings
    assert warning.startswith(why)
    assert len(warning) < 300


def test_a_hook_that_takes_too_long_stops_nothing() -> None:
    assert read("PreToolUse", done(timed_out=True), LIMIT) == Result("timed-out")


def test_a_hook_that_writes_more_than_is_read_is_not_believed() -> None:
    # Half an answer may say the opposite of the whole one.
    cut_short = done(0, '{"decision": "block"', omitted=5)

    assert read("PreToolUse", cut_short, LIMIT) == Result(
        "failed",
        warnings=(f"it wrote more than the {MAX_OUTPUT_BYTES} bytes that are read",),
        exit_code=0,
    )


# The finer answer: JSON


def test_a_hook_can_stop_something_with_a_reason_in_json() -> None:
    assert answered("Stop", {"decision": "block", "reason": " The tests fail. "}) == Result(
        "blocked", "block", "The tests fail.", exit_code=0
    )
    assert answered("Stop", {"decision": "block"}) == Result(
        "blocked", "block", NO_REASON, exit_code=0
    )
    assert answered("Stop", {"decision": "block", "reason": 3}).reason == NO_REASON


def test_a_hook_can_say_that_it_has_nothing_against_it() -> None:
    assert answered("PreToolUse", {"decision": "approve", "reason": "Fine."}) == Result(
        decision="allow", exit_code=0
    )
    assert answered("PreToolUse", {}) == Result(exit_code=0)


def test_a_decision_that_is_not_known_is_none() -> None:
    assert answered("PreToolUse", {"decision": "maybe"}) == Result(
        warnings=("its answer has a decision that is neither block nor approve",), exit_code=0
    )


@pytest.mark.parametrize(
    ("wanted", "status", "decision", "reason"),
    [
        ("allow", "completed", "allow", ""),
        ("deny", "blocked", "block", "Not that file."),
        ("ask", "completed", "ask", "Not that file."),
    ],
)
def test_before_a_call_a_hook_can_let_it_through_stop_it_or_have_somebody_asked(
    wanted: str, status: str, decision: str, reason: str
) -> None:
    fields = about(
        "PreToolUse", permissionDecision=wanted, permissionDecisionReason="Not that file."
    )

    assert answered("PreToolUse", fields) == Result(status, decision, reason, exit_code=0)  # pyright: ignore[reportArgumentType]


def test_what_a_hook_says_about_one_call_counts_before_what_it_says_in_general() -> None:
    fields: dict[str, Any] = {
        "decision": "approve",
        **about("PreToolUse", permissionDecision="deny"),
    }

    assert answered("PreToolUse", fields) == Result("blocked", "block", NO_REASON, exit_code=0)


def test_what_a_hook_says_about_another_moment_is_passed_over() -> None:
    fields = about("PostToolUse", permissionDecision="deny", additionalContext="Mind the gap.")

    assert answered("PreToolUse", fields) == Result(
        warnings=(
            "its answer speaks of another moment than the one that it was asked about, and "
            "that part is passed over",
        ),
        exit_code=0,
    )


@pytest.mark.parametrize(
    ("event", "fields"),
    [
        ("PostToolUse", {"permissionDecision": "deny"}),
        ("PreToolUse", {"permissionDecision": "sometimes"}),
        ("PreToolUse", {"permissionDecision": ["deny"]}),
    ],
)
def test_a_wish_about_a_call_that_cannot_be_used_is_told(
    event: str, fields: dict[str, Any]
) -> None:
    assert answered(event, about(event, **fields)) == Result(
        warnings=("its answer has a permission decision that cannot be used here",), exit_code=0
    )


@pytest.mark.parametrize("event", ["SessionStart", "UserPromptSubmit", "PostToolUse"])
def test_a_hook_can_add_something_for_the_model_to_read(event: str) -> None:
    fields = about(event, additionalContext=" The branch is behind main. ")

    assert answered(event, fields) == Result(context="The branch is behind main.", exit_code=0)


@pytest.mark.parametrize("event", ["PreToolUse", "Stop"])
def test_at_the_other_moments_nothing_is_added_for_the_model(event: str) -> None:
    assert answered(event, about(event, additionalContext="Mind the gap.")) == Result(exit_code=0)


def test_a_hook_can_say_something_to_the_person() -> None:
    assert answered("Stop", {"systemMessage": " Formatted 3 files. "}) == Result(
        message="Formatted 3 files.", exit_code=0
    )


def test_a_field_that_cannot_be_honoured_is_told_and_takes_nothing_from_the_answer() -> None:
    fields: dict[str, Any] = {
        "decision": "block",
        "reason": "No.",
        "continue": False,
        "stopReason": "x",
        "updatedInput": {},
        "suppressOutput": True,
    }

    result = answered("PreToolUse", fields)

    assert (result.status, result.decision, result.reason) == ("blocked", "block", "No.")
    assert list(result.warnings) == [
        f"its answer has the field {name!r}, which is not honoured here"
        for name in ("continue", "stopReason", "updatedInput", "suppressOutput")
    ]


def test_a_field_that_nobody_knows_takes_nothing_from_the_answer() -> None:
    fields = {"decision": "block", "reason": "No.", "futureField": {"a": [1, 2]}}

    assert answered("PreToolUse", fields) == Result("blocked", "block", "No.", exit_code=0)


def test_output_that_begins_like_json_and_is_none_is_a_failure() -> None:
    assert read("PreToolUse", done(0, '{"decision": "block"'), LIMIT) == Result(
        "failed", warnings=("its output begins like JSON and is none",), exit_code=0
    )
    # Objects in objects, deeper than the reader of JSON goes.
    assert read("PreToolUse", done(0, '{"a":' * 100_000), LIMIT).status == "failed"


# How much of it


def test_a_text_that_is_too_long_loses_its_middle() -> None:
    assert cut("abcdefghij", 10) == "abcdefghij"
    assert cut("abcdefghijk", 10) == "abcde\n[1 characters are left out here]\nghijk"
    assert cut("a" * 30 + "b" * 30, 21) == (
        "a" * 10 + "\n[39 characters are left out here]\n" + "b" * 11
    )


def test_a_reason_and_what_is_added_for_the_model_are_kept_within_bounds() -> None:
    long = "x" * 500
    shortened = cut(long, 100)

    assert read("PreToolUse", done(2, "", long), 100).reason == shortened
    assert read("SessionStart", done(0, long), 100).context == shortened
    assert answered("Stop", {"decision": "block", "reason": long}, 100).reason == shortened
    assert answered("Stop", {"systemMessage": long}, 100).message == shortened
    assert answered("PostToolUse", about("PostToolUse", additionalContext=long), 100).context == (
        shortened
    )
    fields = about("PreToolUse", permissionDecision="ask", permissionDecisionReason=long)
    assert answered("PreToolUse", fields, 100).reason == shortened


# What the shell of Windows says about an error

CLIXML = (
    "#< CLIXML\r\n"
    '<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">'
    '<S S="Error">Get-Item : Cannot find path &apos;C:\\nope&apos; because it does not '
    "exist._x000D__x000A_</S>"
    '<S S="Error">At line:2 char:1_x000D__x000A_</S>'
    '<S S="Error">+ Get-Item &lt;nope&gt; &amp; more_x000D__x000A_</S>'
    "</Objs>"
)


def test_what_the_shell_of_windows_writes_about_an_error_is_turned_back_into_text() -> None:
    assert from_clixml(CLIXML) == (
        "Get-Item : Cannot find path 'C:\\nope' because it does not exist.\n"
        "At line:2 char:1\n"
        "+ Get-Item <nope> & more\n"
    )
    # What a program wrote there is text already.
    assert from_clixml('plain <S S="Error">text</S>') == 'plain <S S="Error">text</S>'
    assert from_clixml("") == ""


def test_the_reason_of_a_hook_is_read_from_what_the_shell_wrote_about_its_error() -> None:
    result = read("PreToolUse", done(2, "", CLIXML), LIMIT)

    assert result.reason.startswith("Get-Item : Cannot find path 'C:\\nope'")
    assert "CLIXML" not in result.reason
    assert read("PreToolUse", done(1, "", CLIXML), LIMIT).warnings == (
        "it ended with the exit code 1: Get-Item : Cannot find path 'C:\\nope' because it "
        "does not exist.",
    )


# Several hooks


def test_of_several_answers_the_strictest_counts_and_gives_the_reason() -> None:
    let = Result(decision="allow")
    ask = Result(decision="ask", reason="Ask about this.")
    stop = Result("blocked", "block", "Not this.")
    silent = Result(context="A note.")

    assert merged([let, silent]) == Result(decision="allow", context="A note.")
    assert merged([let, ask, silent]) == Result(
        decision="ask", reason="Ask about this.", context="A note."
    )
    assert merged([ask, stop, let]) == Result("blocked", "block", "Not this.")
    assert merged([stop, ask]) == Result("blocked", "block", "Not this.")
    # Hooks that want the same thing all give their reasons.
    again = Result("blocked", "block", "Nor that.")
    assert merged([stop, ask, again]).reason == "Not this.\n\nNor that."
    assert merged([silent, silent]) == Result(context="A note.\n\nA note.")
    assert merged([]) == Result()


def test_what_several_hooks_add_and_say_is_put_together_in_their_order() -> None:
    first = Result(context="One.", message="Formatted.", warnings=("a",), exit_code=0)
    failed = Result("failed", warnings=("b",), exit_code=1)
    last = Result(context="Two.", message="Linted.", exit_code=0)

    assert merged([first, failed, last]) == Result(
        context="One.\n\nTwo.", message="Formatted.\nLinted.", warnings=("a", "b")
    )


# With a real command


async def ran(command: str, place: Path, event: str = "PreToolUse", **more: Any) -> Result:
    assert SHELL is not None
    hook = Hook(command, more.pop("timeout", 30.0))
    told = payload(event, "s-1", place / "s-1.jsonl", place, more.pop("told", {}))
    project = more.pop("project", place)
    return await run(hook, event, told, shell=SHELL, cwd=place, project=project, limit=LIMIT)


@needs_shell
async def test_a_hook_reads_what_it_is_told_and_finds_the_project(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("MY_API_KEY", "not for a hook")
    monkeypatch.setenv("COLUMBINA_TEST_PLAIN", "for everybody")
    project = tmp_path / "project"
    project.mkdir()
    command = python_hook(
        tmp_path,
        "import json, os, sys\n"
        "told = json.load(sys.stdin.buffer)\n"
        "seen = {'told': told, 'cwd': os.getcwd(),\n"
        "        'claude': os.environ.get('CLAUDE_PROJECT_DIR'),\n"
        "        'columbina': os.environ.get('COLUMBINA_PROJECT_DIR'),\n"
        "        'key': os.environ.get('MY_API_KEY'),\n"
        "        'plain': os.environ.get('COLUMBINA_TEST_PLAIN')}\n"
        # Beside the program, wherever it runs: a hook that runs in the wrong directory
        # is to be found out, and is not to leave a file there.
        "beside = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'seen.json')\n"
        "open(beside, 'w', encoding='utf-8').write(json.dumps(seen))\n",
    )

    result = await ran(command, tmp_path, told={"prompt": "把月亮画出来 naïve"}, project=project)

    assert result == Result(exit_code=0)
    seen = json.loads((tmp_path / "seen.json").read_text(encoding="utf-8"))
    assert seen["told"] == {
        "session_id": "s-1",
        "transcript_path": str(tmp_path / "s-1.jsonl"),
        "cwd": str(tmp_path),
        "hook_event_name": "PreToolUse",
        "prompt": "把月亮画出来 naïve",
    }
    assert Path(seen["cwd"]).resolve() == tmp_path.resolve()
    assert (seen["claude"], seen["columbina"]) == (str(project), str(project))
    # What sounds like a secret stays behind. The rest of the environment comes along.
    assert (seen["key"], seen["plain"]) == (None, "for everybody")


@needs_shell
async def test_a_hook_stops_something_by_its_exit_code_and_says_why(tmp_path: Path) -> None:
    command = python_hook(
        tmp_path,
        "import sys\n"
        "sys.stdout.write('not the reason\\n')\n"
        "sys.stderr.buffer.write('不行 今天是星期五\\n'.encode('utf-8'))\n"
        "sys.exit(2)\n",
    )

    assert await ran(command, tmp_path) == Result(
        "blocked", "block", "不行 今天是星期五", exit_code=2
    )


@needs_shell
async def test_a_hook_answers_in_json_on_its_output(tmp_path: Path) -> None:
    answer = {"decision": "block", "reason": "测试没有通过", "systemMessage": "3 failed"}
    command = python_hook(
        tmp_path,
        "import json, sys\n"
        f"sys.stdout.buffer.write(json.dumps({answer!r}, ensure_ascii=False).encode('utf-8'))\n"
        "sys.stderr.write('a remark that nobody reads\\n')\n",
    )

    assert await ran(command, tmp_path, "Stop") == Result(
        "blocked", "block", "测试没有通过", message="3 failed", exit_code=0
    )


@needs_shell
async def test_a_hook_that_takes_too_long_is_stopped(tmp_path: Path) -> None:
    command = python_hook(tmp_path, "import time\ntime.sleep(60)\n")

    assert await ran(command, tmp_path, timeout=1.0) == Result("timed-out")


@needs_shell
async def test_a_hook_that_writes_too_much_is_not_believed(tmp_path: Path) -> None:
    command = python_hook(
        tmp_path, f"import sys\nsys.stdout.write('{{' + 'x' * {MAX_OUTPUT_BYTES + 10})\n"
    )

    result = await ran(command, tmp_path)

    assert (result.status, result.decision) == ("failed", None)
    assert result.warnings == (f"it wrote more than the {MAX_OUTPUT_BYTES} bytes that are read",)


async def test_a_command_that_is_too_long_is_not_run(tmp_path: Path) -> None:
    command = "echo " + "x" * MAX_COMMAND

    result = await run(
        Hook(command),
        "Stop",
        b"{}\n",
        shell=str(tmp_path / "no-such-shell"),
        cwd=tmp_path,
        project=tmp_path,
        limit=LIMIT,
    )

    assert result == Result(
        "failed",
        warnings=(f"its command has more than {MAX_COMMAND} characters, and was not run",),
    )


async def test_a_command_of_just_the_length_that_is_allowed_is_handed_to_the_shell(
    tmp_path: Path,
) -> None:
    command = "x" * MAX_COMMAND

    result = await run(
        Hook(command),
        "Stop",
        b"{}\n",
        shell=str(tmp_path / "no-such-shell"),
        cwd=tmp_path,
        project=tmp_path,
        limit=LIMIT,
    )

    # There is no such shell, which is another matter than the length.
    assert result.status == "failed"
    (warning,) = result.warnings
    assert warning.startswith("its command could not be started: ")


if sys.platform == "win32":

    @needs_shell
    async def test_an_error_of_powershell_itself_is_read_as_text(tmp_path: Path) -> None:
        result = await ran("Get-Item 'no-such-file-here' -ErrorAction Stop", tmp_path)

        assert (result.status, result.exit_code) == ("failed", 1)
        (warning,) = result.warnings
        assert warning.startswith("it ended with the exit code 1: Get-Item : ")
        assert "CLIXML" not in warning
        assert "_x000D_" not in warning

    @needs_shell
    async def test_powershell_reads_what_the_hook_is_told_and_passes_its_exit_code_on(
        tmp_path: Path,
    ) -> None:
        command = (
            "$told = [Console]::In.ReadToEnd() | ConvertFrom-Json; "
            "[Console]::Error.WriteLine($told.prompt); exit 2"
        )

        assert await ran(command, tmp_path, told={"prompt": "月亮"}) == Result(
            "blocked", "block", "月亮", exit_code=2
        )

else:

    @needs_shell
    async def test_bash_reads_what_the_hook_is_told_and_passes_its_exit_code_on(
        tmp_path: Path,
    ) -> None:
        command = "cat > told.json; echo 'not on a friday' >&2; exit 2"

        assert await ran(command, tmp_path, told={"prompt": "月亮"}) == Result(
            "blocked", "block", "not on a friday", exit_code=2
        )
        assert json.loads((tmp_path / "told.json").read_text(encoding="utf-8"))["prompt"] == "月亮"
