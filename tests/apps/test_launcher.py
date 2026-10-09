"""The command line: ``tack run`` loads a profile, runs one task, and reports."""

import asyncio
import json
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

import tack
from tack.apps.launcher import main
from tack.session import CORE_EVENTS, TurnEndEvent, read_session
from tack.session.store import workspace_key

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}


@pytest.fixture
def home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A data directory for Tack that is thrown away after the test, and no API key."""
    directory = tmp_path / "home"
    monkeypatch.setenv("TACK_HOME", str(directory))
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.delenv("TACK_MODEL", raising=False)
    return directory


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "notes.txt").write_text("The answer is 42.\n", encoding="utf-8")
    return directory


def profile(
    tmp_path: Path, *replies: dict[str, Any], extra: list[dict[str, Any]] | None = None
) -> str:
    """Write a profile that is the headless bundle with a scripted model, and return its path."""
    scripted = {
        "id": "model",
        "plugin": "tack.llm.adapters.scripted",
        "config": {"provider": "deepseek", "replies": list(replies)},
    }
    layer = {"extends": "headless", "patch": [scripted, *(extra or [])]}
    path = tmp_path / "profile.yml"
    path.write_text(yaml.safe_dump(layer, sort_keys=False), encoding="utf-8")
    return str(path)


def lines(text: str) -> list[dict[str, Any]]:
    return [json.loads(line) for line in text.splitlines()]


READ_THEN_ANSWER: tuple[dict[str, Any], ...] = (
    {"text": "I will look.", "calls": [{"name": "read", "arguments": {"path": "notes.txt"}}]},
    {"text": "The file says the answer is 42.", "usage": {"input_tokens": 30, "output_tokens": 9}},
)


def test_a_task_is_run_and_the_answer_is_printed(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "run",
            "What does notes.txt say?",
            "--profile",
            profile(tmp_path, *READ_THEN_ANSWER),
            "--cwd",
            str(work),
        ]
    )

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == "The file says the answer is 42.\n"
    assert captured.err == ""


def test_the_run_leaves_a_session_behind(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    main(
        [
            "run",
            "What does notes.txt say?",
            "--profile",
            profile(tmp_path, *READ_THEN_ANSWER),
            "--cwd",
            str(work),
            "--session-id",
            "s-1",
        ]
    )

    stored = read_session(home / "sessions" / workspace_key(work) / "s-1", KINDS | _more_kinds())
    types = [event.type for event in stored.events]
    assert types.count("tool/call") == 1
    assert types.count("tool/result") == 1
    assert stored.events[-1].data == TurnEndEvent(turn=1, reason="completed")
    assert stored.header.cwd == str(work)


def _more_kinds() -> dict[str, Any]:
    from tack.agent import InboxSplicedEvent
    from tack.retry import RetryEvent

    return {kind.event_type: kind for kind in (InboxSplicedEvent, RetryEvent)}


def test_the_run_can_be_followed_as_lines_of_json(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "run",
            "What does notes.txt say?",
            "--profile",
            profile(tmp_path, *READ_THEN_ANSWER),
            "--cwd",
            str(work),
            "--session-id",
            "s-1",
            "--json",
        ]
    )

    captured = capsys.readouterr()
    assert code == 0
    assert captured.err == ""
    assert lines(captured.out) == [
        {
            "type": "session",
            "id": "s-1",
            "cwd": str(work),
            "provider": "deepseek",
            "model": "deepseek-flash",
        },
        {"type": "status", "turn": 1, "step": 1},
        {"type": "text", "text": "I will look."},
        {
            "type": "tool_call",
            "id": "call-1",
            "name": "read",
            "arguments": '{"path": "notes.txt"}',
        },
        {
            "type": "tool_result",
            "id": "call-1",
            "text": "     1\tThe answer is 42.",
            "error": None,
        },
        {"type": "status", "turn": 1, "step": 2},
        {"type": "text", "text": "The file says the answer is 42."},
        {
            "type": "final",
            "reason": "completed",
            "detail": None,
            "text": "The file says the answer is 42.",
            "calls": 2,
            "usage": {
                "input_tokens": 30,
                "output_tokens": 9,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
            },
        },
    ]


def test_reasoning_is_reported_as_thinking(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    reply = {"reasoning": "Two and two.", "text": "Four."}

    main(["run", "2+2?", "--profile", profile(tmp_path, reply), "--cwd", str(work), "--json"])

    kinds = [(line["type"], line.get("text")) for line in lines(capsys.readouterr().out)]
    assert kinds[2:4] == [("thinking", "Two and two."), ("text", "Four.")]


def test_a_run_that_fails_says_why_and_exits_with_one(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    failing = {"failure": {"message": "the key is not valid", "code": "AUTH", "status": 401}}

    code = main(["run", "Hello?", "--profile", profile(tmp_path, failing), "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == ""
    assert captured.err == "tack: the turn ended with an error: AUTH: the key is not valid\n"


def test_a_failed_run_ends_its_json_with_the_reason(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    failing = {"failure": {"message": "the key is not valid", "code": "AUTH", "status": 401}}

    code = main(
        ["run", "Hello?", "--profile", profile(tmp_path, failing), "--cwd", str(work), "--json"]
    )

    final = lines(capsys.readouterr().out)[-1]
    assert code == 1
    assert final["type"] == "final"
    assert final["reason"] == "error"
    assert final["detail"] == "AUTH: the key is not valid"
    assert final["text"] == ""


def test_a_reply_that_was_cut_off_is_printed_but_the_run_does_not_count_as_done(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    cut = {"text": "This sentence never", "finish": "max-tokens"}

    code = main(["run", "Write.", "--profile", profile(tmp_path, cut), "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == "This sentence never\n"
    assert captured.err == "tack: the turn ended early: max-tokens\n"


def test_a_session_can_be_continued_by_its_id(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    first = profile(tmp_path, {"text": "Hello."})
    main(["run", "Say hello.", "--profile", first, "--cwd", str(work), "--session-id", "s-1"])
    second = profile(tmp_path, {"text": "Hello again."})

    code = main(
        [
            "run",
            "Once more.",
            "--profile",
            second,
            "--cwd",
            str(work),
            "--session-id",
            "s-1",
            "--json",
        ]
    )

    captured = lines(capsys.readouterr().out.split("Hello.\n", 1)[1])
    assert code == 0
    assert captured[0]["id"] == "s-1"
    assert captured[1] == {"type": "status", "turn": 2, "step": 1}
    stored = read_session(home / "sessions" / workspace_key(work) / "s-1", KINDS | _more_kinds())
    ends = [event.data for event in stored.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]


def test_the_working_directory_defaults_to_the_current_one(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(work)

    main(
        [
            "run",
            "What does notes.txt say?",
            "--profile",
            profile(tmp_path, *READ_THEN_ANSWER),
            "--json",
        ]
    )

    session = lines(capsys.readouterr().out)[0]
    assert Path(session["cwd"]) == work


# What is loaded


def test_the_composition_can_be_printed_instead_of_run(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "run",
            "Do it.",
            "--profile",
            "headless",
            "--cwd",
            str(work),
            "--model",
            "deepseek-fast",
            "--dump-config",
        ]
    )

    captured = capsys.readouterr()
    entries = {entry["id"]: entry for entry in yaml.safe_load(captured.out)}
    assert code == 0
    assert list(entries) == [
        "session",
        "llm",
        "model",
        "tools",
        "tool-read",
        "agent",
        "agent-loop",
        "retry",
        "frontend",
    ]
    assert entries["session"] == {
        "id": "session",
        "plugin": "tack.session",
        "config": {"root": "${TACK_HOME}/sessions"},
    }
    assert entries["model"]["config"]["api_key_env"] == "DEEPSEEK_API_KEY"
    assert entries["frontend"]["config"] == {
        "model": {"provider": "deepseek", "model": "deepseek-fast"},
        "task": "Do it.",
        "cwd": str(work),
    }
    assert not home.exists()


CHOSEN_BY_THE_PROFILE = {
    "model": {"provider": "deepseek", "model": "deepseek-flash"},
    "cwd": "${PROJECT}/src",
    "json_lines": True,
    "session_id": "kept",
    "system_prompt": "Be brief.",
}


def test_the_command_line_changes_only_what_it_names(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(work)
    chosen = profile(tmp_path, extra=[{"id": "frontend", "config": CHOSEN_BY_THE_PROFILE}])

    main(["run", "Do it.", "--profile", chosen, "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["frontend"]["config"] == CHOSEN_BY_THE_PROFILE | {"task": "Do it."}


def test_what_the_command_line_names_wins_over_the_profile(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen = profile(tmp_path, extra=[{"id": "frontend", "config": CHOSEN_BY_THE_PROFILE}])

    main(
        [
            "run",
            "Do it.",
            "--profile",
            chosen,
            "--cwd",
            str(work),
            "--no-json",
            "--session-id",
            "mine",
            "--model",
            "deepseek-fast",
            "--dump-config",
        ]
    )

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["frontend"]["config"] == {
        "model": {"provider": "deepseek", "model": "deepseek-fast"},
        "cwd": str(work),
        "json_lines": False,
        "session_id": "mine",
        "system_prompt": "Be brief.",
        "task": "Do it.",
    }


def test_a_profile_can_ask_for_lines_of_json(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    always = {
        "id": "frontend",
        "config": {"model": {"provider": "deepseek", "model": "any"}, "json_lines": True},
    }

    chosen = profile(tmp_path, {"text": "Hello."}, extra=[always])

    code = main(["run", "Hi.", "--profile", chosen, "--cwd", str(work)])

    reported = lines(capsys.readouterr().out)
    assert code == 0
    assert [line["type"] for line in reported] == ["session", "status", "text", "final"]


def test_a_patch_changes_the_profile(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: retry\n  disabled: true\n", encoding="utf-8")

    main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["retry"] == {"id": "retry", "plugin": "tack.retry", "disabled": True}


def test_the_patch_of_the_user_comes_before_the_patches_of_the_command(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    home.mkdir()
    (home / "config.yml").write_text(
        "- id: tools\n  config:\n    timeout_seconds: 5\n- id: retry\n  disabled: true\n",
        encoding="utf-8",
    )
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: tools\n  config:\n    timeout_seconds: 9\n", encoding="utf-8")

    main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["tools"]["config"] == {"timeout_seconds": 9}
    assert entries["retry"]["disabled"] is True


def test_the_model_can_come_from_the_environment(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("TACK_MODEL", "deepseek-from-env")

    main(
        [
            "run",
            "Hi.",
            "--profile",
            profile(tmp_path, {"text": "Hi."}),
            "--cwd",
            str(work),
            "--json",
        ]
    )

    assert lines(capsys.readouterr().out)[0]["model"] == "deepseek-from-env"


# What can go wrong before a run


def test_a_profile_that_does_not_exist_is_refused(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["run", "Do it.", "--profile", "nowhere", "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == (
        "tack: there is no profile 'nowhere'. The profiles that come with Tack are: "
        "headless, minimal\n"
    )


def test_a_profile_without_a_frontend_cannot_run_a_task(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["run", "Do it.", "--profile", "minimal", "--cwd", str(work)])

    assert code == 2
    assert capsys.readouterr().err == (
        "tack: the profile has no entry 'frontend', so there is nothing to run the task\n"
    )


def test_a_frontend_that_is_switched_off_cannot_run_a_task(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: frontend\n  disabled: true\n", encoding="utf-8")

    code = main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch)])

    assert code == 2
    assert capsys.readouterr().err == (
        "tack: the entry 'frontend' is disabled, so there is nothing to run the task\n"
    )


def test_an_entry_called_frontend_that_is_no_frontend_cannot_run_a_task(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    (tmp_path / "idle_plugin.py").write_text(
        "name = 'idle'\n\n\ndef apply(ctx, config):\n    pass\n", encoding="utf-8"
    )
    monkeypatch.setattr(sys, "path", [str(tmp_path), *sys.path])
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: frontend\n  plugin: idle_plugin\n", encoding="utf-8")

    code = main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch)])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == (
        "tack: the entry 'frontend' loads idle_plugin, which provides nothing to run the task\n"
    )


def test_a_patch_that_cannot_be_applied_is_refused(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: nowhere\n  disabled: true\n", encoding="utf-8")

    code = main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch)])

    captured = capsys.readouterr()
    assert code == 2
    assert "nowhere" in captured.err
    assert captured.err.startswith("tack: ")


def test_a_patch_file_that_is_not_there_is_refused(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["run", "Do it.", "--cwd", str(work), "--patch", str(tmp_path / "missing.yml")])

    assert code == 2
    assert "missing.yml" in capsys.readouterr().err


def test_a_plugin_that_does_not_load_stops_the_run(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    broken = {"id": "tools", "config": {"timeout_seconds": -1}}

    code = main(
        [
            "run",
            "Do it.",
            "--profile",
            profile(tmp_path, {"text": "never"}, extra=[broken]),
            "--cwd",
            str(work),
        ]
    )

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert "tack: tools (tack.tools) failed: ValidationError" in captured.err
    assert "tack: tool-read (tack.builtin_tools.read) is waiting for the service 'tools'" in (
        captured.err
    )


def test_a_task_is_needed(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["run"])

    assert stopped.value.code == 2
    assert "TASK" in capsys.readouterr().err


def test_the_version_can_be_asked_for(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["--version"])

    assert stopped.value.code == 0
    assert capsys.readouterr().out == f"tack {tack.__version__}\n"


def test_without_a_command_the_help_is_printed(capsys: pytest.CaptureFixture[str]) -> None:
    code = main([])

    assert code == 2
    assert "usage: tack" in capsys.readouterr().err


def test_an_interrupted_run_exits_the_way_a_shell_expects(
    home: Path, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def interrupted(program: Any) -> int:
        program.close()
        raise KeyboardInterrupt

    monkeypatch.setattr(asyncio, "run", interrupted)

    code = main(
        ["run", "Do it.", "--profile", profile(tmp_path, {"text": "x"}), "--cwd", str(work)]
    )

    assert code == 130
