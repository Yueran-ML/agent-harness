"""The command line: ``columbina run`` loads a profile, runs one task, and reports."""

import asyncio
import io
import itertools
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

import columbina
from columbina.apps.launcher import main
from columbina.apps.serve import wire as wire_module
from columbina.session import TurnEndEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE
from columbina.tools import pipeline
from support.launch import lines, profile, stored
from support.wires import Far

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

    left = stored(home, work)
    types = [event.type for event in left.events]
    assert types.count("tool/call") == 1
    assert types.count("tool/result") == 1
    assert left.events[-1].data == TurnEndEvent(turn=1, reason="completed")
    assert left.header.cwd == str(work)


def test_skills_are_looked_for_where_other_tools_keep_theirs_in_the_home_of_the_user(
    home: Path, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    user = tmp_path / "user"
    skill = user / ".agents" / "skills" / "release-notes"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(
        "---\nname: release-notes\ndescription: Write the notes of a release.\n---\nHow.\n",
        encoding="utf-8",
    )
    # Nothing names the place, so it is the one in the home of the user.
    monkeypatch.delenv("COLUMBINA_AGENTS_HOME")
    monkeypatch.setenv("HOME", str(user))
    monkeypatch.setenv("USERPROFILE", str(user))

    main(
        [
            "run",
            "Hello.",
            "--profile",
            profile(tmp_path, {"text": "Hello."}),
            "--cwd",
            str(work),
            "--session-id",
            "s-1",
        ]
    )

    told = [
        block.text
        for event in stored(home, work).events
        if isinstance(event.data, UserMessageEvent) and event.data.source == "context/skills"
        for block in event.data.message.content
    ]
    assert len(told) == 1
    assert "- release-notes: Write the notes of a release." in told[0]


def test_the_run_can_be_followed_as_lines_of_json(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # The clock that the runs of tools are timed with goes on by 7 milliseconds at each look.
    looks = itertools.count()
    monkeypatch.setattr(pipeline, "_now", lambda: next(looks) * 0.007)

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
            "code": None,
            "text": "The file says the answer is 42.",
            "calls": 2,
            "usage": {
                "input_tokens": 30,
                "output_tokens": 9,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
            },
            "tools": {"read": {"calls": 1, "failed": 0, "duration_ms": 7}},
        },
    ]


def test_the_end_of_a_run_counts_the_calls_of_that_run_alone(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    finals: list[dict[str, Any]] = []
    for task in ("What does notes.txt say?", "And now?"):
        # Every run begins the script anew, so each has a script of its own.
        chosen = profile(tmp_path, *READ_THEN_ANSWER)
        argv = ["run", task, "--profile", chosen, "--cwd", str(work), "--session-id", "s-1"]
        assert main([*argv, "--json"]) == 0
        finals.append(lines(capsys.readouterr().out)[-1])

    assert [final["tools"]["read"]["calls"] for final in finals] == [1, 1]


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
    assert captured.err == "columbina: the turn ended with an error: AUTH: the key is not valid\n"


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
    # For a program: the code of the failure, as the service of the model gave it.
    assert final["code"] == "AUTH"
    assert final["text"] == ""


def test_a_reply_that_was_cut_off_is_printed_but_the_run_does_not_count_as_done(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # It is taken up three times. What arrived is printed as the one text that it is.
    pieces = ["This sentence", " never", " comes to", " its"]
    cut = [{"text": piece, "finish": "max-tokens"} for piece in pieces]

    code = main(["run", "Write.", "--profile", profile(tmp_path, *cut), "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == "This sentence never comes to its\n"
    assert captured.err == "columbina: the turn ended early: max-tokens\n"


def test_a_reply_that_was_cut_off_and_went_on_is_printed_whole_and_the_run_is_done(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    cut = {"text": "This sentence never", "finish": "max-tokens"}
    rest = {"text": " ends, they say."}

    code = main(["run", "Write.", "--profile", profile(tmp_path, cut, rest), "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == "This sentence never ends, they say.\n"
    assert captured.err == ""


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
    left = stored(home, work)
    ends = [event.data for event in left.events if isinstance(event.data, TurnEndEvent)]
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
        "fs",
        "tool-read",
        "agent",
        "agent-loop",
        "sandbox",
        "approval",
        "permissions",
        "tool-write",
        "tool-edit",
        "tool-glob",
        "tool-grep",
        "tool-shell",
        "github",
        "system-prompt",
        "system-prompt-conduct",
        "context-instructions",
        "context-runtime",
        "retry",
        "token-meter",
        "spill",
        "spill-policy",
        "compaction",
        "todo",
        "turn-budget",
        "skills",
        "hooks",
        "subagents",
        "subagent-spawn",
        "tool-subagent",
        "mcp",
        "frontend",
    ]
    assert entries["spill"] == {
        "id": "spill",
        "plugin": "columbina.spill.local",
        "config": {"root": "${COLUMBINA_HOME}/spill"},
    }
    assert entries["session"] == {
        "id": "session",
        "plugin": "columbina.session",
        "config": {"root": "${COLUMBINA_HOME}/sessions"},
    }
    assert entries["model"]["config"]["api_key_env"] == "DEEPSEEK_API_KEY"
    assert entries["permissions"] == {
        "id": "permissions",
        "plugin": "columbina.permissions",
        "config": {"preset": "workspace-write", "user_file": "${COLUMBINA_HOME}/permissions.yml"},
    }
    assert entries["context-instructions"]["config"] == {"user_file": "${COLUMBINA_HOME}/AGENTS.md"}
    # What the user adds to the program lies in the home of the program too. Skills are also
    # looked for where the user keeps those that other tools read.
    assert entries["skills"]["config"] == {
        "home": "${COLUMBINA_HOME}/skills",
        "user": "${COLUMBINA_AGENTS_HOME}/skills",
    }
    assert entries["hooks"]["config"] == {
        "user_file": "${COLUMBINA_HOME}/hooks.json",
        "trust_file": "${COLUMBINA_HOME}/trusted-hooks.json",
    }
    assert entries["subagents"]["config"] == {"home": "${COLUMBINA_HOME}/agents"}
    assert entries["mcp"]["config"] == {"log_dir": "${COLUMBINA_HOME}/logs"}
    # The model is another one. How much it takes stays as the profile has it.
    assert entries["frontend"]["config"] == {
        "model": {
            "provider": "deepseek",
            "model": "deepseek-fast",
            "context_window": "${COLUMBINA_CONTEXT_WINDOW:-1048576}",
        },
        "task": "Do it.",
        "cwd": str(work),
    }
    assert not home.exists()


def test_the_command_line_names_the_preset_and_adds_rules_that_allow(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    main(
        [
            "run",
            "Do it.",
            "--cwd",
            str(work),
            "--permission",
            "read-only",
            "--allow",
            "run(git status*)",
            "--allow",
            "grep",
            "--dump-config",
        ]
    )

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["permissions"]["config"] == {
        "preset": "read-only",
        "user_file": "${COLUMBINA_HOME}/permissions.yml",
        "allow": ["run(git status*)", "grep"],
    }


def test_the_rules_of_the_command_line_come_after_those_of_the_profile(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    mine = {
        "id": "permissions",
        "config": {"preset": "full-access", "allow": ["read(/etc/hosts)"], "deny": ["read(.env)"]},
    }
    chosen = profile(tmp_path, extra=[mine])

    main(
        [
            "run",
            "Do it.",
            "--profile",
            chosen,
            "--cwd",
            str(work),
            "--allow",
            "grep",
            "--dump-config",
        ]
    )

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["permissions"]["config"] == {
        "preset": "full-access",
        "allow": ["read(/etc/hosts)", "grep"],
        "deny": ["read(.env)"],
    }


def test_a_preset_can_be_named_for_permissions_that_have_no_config(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen = profile(tmp_path, extra=[{"id": "permissions", "config": None}])

    run = ["run", "Do it.", "--profile", chosen, "--cwd", str(work)]
    main([*run, "--permission", "full-access", "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["permissions"]["config"] == {"preset": "full-access"}


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
    assert entries["retry"] == {"id": "retry", "plugin": "columbina.retry", "disabled": True}


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
    monkeypatch.setenv("COLUMBINA_MODEL", "deepseek-from-env")

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
        "columbina: there is no profile 'nowhere'. The profiles that come with Columbina are: "
        "base, cli, headless, minimal, serve\n"
    )


def test_a_profile_without_a_frontend_cannot_run_a_task(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["run", "Do it.", "--profile", "minimal", "--cwd", str(work)])

    assert code == 2
    assert capsys.readouterr().err == (
        "columbina: the profile has no entry 'frontend', so there is nothing to run the task\n"
    )


WITHOUT_PERMISSIONS = [
    {"id": "tool-shell", "disabled": True},
    {"id": "permissions", "disabled": True},
]


def test_the_bundle_leaves_it_to_the_environment_what_runs_the_commands(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    main(["run", "Do it.", "--cwd", str(work), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["sandbox"]["config"] == {"provider": "${COLUMBINA_SANDBOX:-none}"}


@pytest.mark.parametrize("command", [["run", "Do it."], [], ["serve"]])
def test_the_command_line_names_what_runs_the_commands(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str], command: list[str]
) -> None:
    code = main([*command, "--cwd", str(work), "--sandbox", "docker", "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert entries["sandbox"]["config"] == {"provider": "docker"}


@pytest.mark.parametrize("command", [["run", "Do it."], [], ["serve"]])
def test_every_program_brings_the_tools_of_github(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str], command: list[str]
) -> None:
    code = main([*command, "--cwd", str(work), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert entries["github"] == {
        "id": "github",
        "plugin": "columbina.github.tools",
        "config": {"home": "${COLUMBINA_HOME}"},
    }


def test_what_runs_the_commands_is_named_for_a_sandbox_that_has_a_config_of_its_own(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    mine = {"id": "sandbox", "config": {"mode": "read-only", "docker": {"image": "busybox"}}}
    chosen = profile(tmp_path, extra=[mine])

    main(
        [
            "run",
            "Do it.",
            "--profile",
            chosen,
            "--cwd",
            str(work),
            "--sandbox",
            "a$b",
            "--dump-config",
        ]
    )

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    # A dollar sign in a name is a dollar sign, and no variable of the environment.
    assert entries["sandbox"]["config"] == {
        "mode": "read-only",
        "docker": {"image": "busybox"},
        "provider": "a$$b",
    }


WITHOUT_A_SANDBOX = [
    {"id": name, "disabled": True} for name in ("sandbox", "permissions", "tool-shell")
]


@pytest.mark.parametrize("extends", ["headless", "minimal"])
def test_what_runs_the_commands_cannot_be_named_for_a_program_without_a_sandbox(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    extends: str,
) -> None:
    # In one profile the sandbox is switched off, the other never had it.
    without = profile(
        tmp_path,
        {"text": "never"},
        extends=extends,
        extra=WITHOUT_A_SANDBOX if extends == "headless" else [],
    )

    code = main(["run", "Do it.", "--profile", without, "--cwd", str(work), "--sandbox", "docker"])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == (
        "columbina: the profile has no entry 'sandbox', or it is disabled, so --sandbox has "
        "nothing to change\n"
    )


@pytest.mark.parametrize("asked", [["--permission", "read-only"], ["--allow", "grep"]])
@pytest.mark.parametrize("extends", ["headless", "minimal"])
def test_a_preset_or_a_rule_needs_permissions_to_go_to(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    asked: list[str],
    extends: str,
) -> None:
    # In one profile the permissions are switched off, the other never had them.
    without = profile(
        tmp_path,
        {"text": "never"},
        extends=extends,
        extra=WITHOUT_PERMISSIONS if extends == "headless" else [],
    )

    code = main(["run", "Do it.", "--profile", without, "--cwd", str(work), *asked])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == (
        "columbina: the profile has no entry 'permissions', or it is disabled, so --permission "
        "and --allow have nothing to change\n"
    )


def test_a_preset_that_does_not_exist_is_refused(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["run", "Do it.", "--cwd", str(work), "--permission", "everything"])

    assert stopped.value.code == 2
    # The presets are named. How they are set apart differs from one Python to the next.
    said = capsys.readouterr().err
    assert "--permission" in said
    assert "everything" in said
    for preset in ("read-only", "workspace-write", "full-access"):
        assert preset in said


def test_a_rule_that_cannot_be_read_stops_the_run(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "run",
            "Do it.",
            "--profile",
            profile(tmp_path, {"text": "never"}),
            "--cwd",
            str(work),
            "--allow",
            "read(notes.txt",
        ]
    )

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert "columbina: permissions (columbina.permissions) failed: " in captured.err
    assert "read(notes.txt" in captured.err


@pytest.mark.parametrize(
    ("entry", "config", "said"),
    [
        ("permissions", ["grep"], "the config of the entry 'permissions' is not a mapping"),
        (
            "permissions",
            {"allow": "grep"},
            "the entry 'permissions' has rules under 'allow' that are not a list",
        ),
        ("frontend", "deepseek", "the config of the entry 'frontend' is not a mapping"),
    ],
)
def test_a_config_that_the_command_line_cannot_add_to_is_refused(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    entry: str,
    config: object,
    said: str,
) -> None:
    odd = profile(tmp_path, {"text": "never"}, extra=[{"id": entry, "config": config}])

    code = main(["run", "Do it.", "--profile", odd, "--cwd", str(work), "--allow", "glob"])

    assert code == 2
    assert capsys.readouterr().err == f"columbina: {said}\n"


def test_a_frontend_that_is_switched_off_cannot_run_a_task(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: frontend\n  disabled: true\n", encoding="utf-8")

    code = main(["run", "Do it.", "--cwd", str(work), "--patch", str(patch)])

    assert code == 2
    assert capsys.readouterr().err == (
        "columbina: the entry 'frontend' is disabled, so there is nothing to run the task\n"
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
        "columbina: the entry 'frontend' loads idle_plugin, "
        "which provides nothing to run the task\n"
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
    assert captured.err.startswith("columbina: ")


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
    assert "columbina: tools (columbina.tools) failed: ValidationError" in captured.err
    assert (
        "columbina: tool-read (columbina.builtin_tools.read) is waiting for the service 'tools'"
        in (captured.err)
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
    assert capsys.readouterr().out == f"columbina {columbina.__version__}\n"


NOT_A_TERMINAL = (
    "columbina: this is not a terminal, and the conversation needs one. "
    "`columbina run TASK` gives an agent one task without it.\n"
)


@pytest.mark.parametrize("given", [[], ["chat"], ["--model", "other"], ["chat", "--continue"]])
def test_without_a_command_a_conversation_is_meant_and_that_needs_a_terminal(
    home: Path,
    work: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    given: list[str],
) -> None:
    monkeypatch.chdir(work)

    code = main(given)

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == NOT_A_TERMINAL
    assert not home.exists()


def test_the_program_of_a_conversation_can_be_printed_instead_of_run(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["--cwd", str(work), "--continue", "--model", "other", "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert list(entries)[-10:] == [
        "commands",
        "command-tools",
        "command-permissions",
        "command-sessions",
        "command-compact",
        "command-todos",
        "command-fork",
        "command-skills",
        "command-mcp",
        "frontend",
    ]
    assert entries["frontend"] == {
        "id": "frontend",
        "plugin": "columbina.apps.cli",
        "config": {
            "model": {
                "provider": "deepseek",
                "model": "other",
                "context_window": "${COLUMBINA_CONTEXT_WINDOW:-1048576}",
            },
            "cwd": str(work),
            "resume": True,
        },
    }


def test_a_conversation_takes_the_preset_the_rules_and_the_session_of_the_command_line(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chat = ["chat", "--cwd", str(work), "--session-id", "mine"]
    main([*chat, "--permission", "read-only", "--allow", "grep", "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["frontend"]["config"] == {
        "model": {
            "provider": "deepseek",
            "model": "${COLUMBINA_MODEL:-deepseek-flash}",
            "context_window": "${COLUMBINA_CONTEXT_WINDOW:-1048576}",
        },
        "cwd": str(work),
        "session_id": "mine",
    }
    assert entries["permissions"]["config"]["preset"] == "read-only"
    assert entries["permissions"]["config"]["allow"] == ["grep"]


def test_a_session_cannot_be_both_named_and_looked_for(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["--cwd", str(work), "--continue", "--session-id", "mine"])

    assert code == 2
    assert capsys.readouterr().err == (
        "columbina: --continue takes up the session that was used last, and --session-id names "
        "one: give one of the two\n"
    )


def test_looking_for_the_last_session_counts_for_more_than_a_session_that_a_profile_names(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The command line changes what it names, and it names which session to take up.
    named = {"id": "frontend", "config": {"session_id": "old"}}
    chosen = profile(tmp_path, extends="cli", extra=[named])

    code = main(["--cwd", str(work), "--profile", chosen, "--continue", "--dump-config"])

    assert code == 0
    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert entries["frontend"]["config"]["resume"] is True
    assert "session_id" not in entries["frontend"]["config"]


def test_a_task_is_not_a_conversation_and_takes_none_of_its_arguments(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["run", "Do it.", "--continue"])

    assert stopped.value.code == 2
    assert "--continue" in capsys.readouterr().err


@pytest.mark.parametrize("flag", ["--help", "-h"])
def test_the_help_names_the_commands(capsys: pytest.CaptureFixture[str], flag: str) -> None:
    with pytest.raises(SystemExit) as stopped:
        main([flag])

    shown = capsys.readouterr().out
    assert stopped.value.code == 0
    # What each of the two commands is for, which only the help of the program says.
    assert "hold a conversation at the terminal" in shown
    assert "run one task and print the answer" in shown
    assert "hold a conversation for another program" in shown


def test_the_program_that_serves_can_be_printed_instead_of_run(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["serve", "--cwd", str(work), "--model", "other", "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert list(entries)[-9:] == [
        "commands",
        "command-tools",
        "command-permissions",
        "command-sessions",
        "command-compact",
        "command-todos",
        "command-skills",
        "command-mcp",
        "frontend",
    ]
    assert entries["frontend"] == {
        "id": "frontend",
        "plugin": "columbina.apps.serve",
        "config": {
            "model": {
                "provider": "deepseek",
                "model": "other",
                "context_window": "${COLUMBINA_CONTEXT_WINDOW:-1048576}",
            },
            "cwd": str(work),
        },
    }
    assert not home.exists()


def test_serving_takes_what_a_session_may_do_from_the_command_line_like_the_others(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        [
            "serve",
            "--cwd",
            str(work),
            "--permission",
            "read-only",
            "--allow",
            "run(git status*)",
            "--dump-config",
        ]
    )

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert entries["permissions"]["config"]["preset"] == "read-only"
    assert entries["permissions"]["config"]["allow"] == ["run(git status*)"]


HELLO = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": 1}}
OPEN = {"jsonrpc": "2.0", "id": 2, "method": "session/open", "params": {"session_id": "s-1"}}
LEAVE = {"jsonrpc": "2.0", "id": 3, "method": "shutdown"}


def test_the_program_that_serves_holds_a_conversation_over_the_wire_of_its_process(
    home: Path, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    far = Far()
    monkeypatch.setattr(wire_module, "taken", lambda: far)
    for message in (HELLO, OPEN, LEAVE):
        far.say(message)
    chosen = profile(tmp_path, extends="serve")

    code = main(["serve", "--profile", chosen, "--cwd", str(work), "--model", "swift-2"])

    assert code == 0
    hello, opened, status, left = far.heard
    assert hello["result"]["cwd"] == str(work)
    # The commands of the program that is shipped, without the one that copies a session.
    assert [command["name"] for command in hello["result"]["commands"]] == [
        "compact",
        "effort",
        "help",
        "mcp",
        "model",
        "permissions",
        "sessions",
        "skill",
        "skills",
        "todos",
        "tools",
    ]
    assert (opened["result"]["session_id"], opened["result"]["resumed"]) == ("s-1", False)
    assert status["params"] == {
        "session_id": "s-1",
        "status": "idle",
        "detail": None,
        "model": {
            "provider": "deepseek",
            "model": "swift-2",
            "context_window": 1048576,
            # The scripted model of the profile says nothing of how it is told to reason.
            "effort": "default",
            "efforts": ["default", "ultra"],
        },
        "sandbox": {"mode": "workspace-write", "strength": "none"},
        "approval": "ask",
    }
    assert left == {"jsonrpc": "2.0", "id": 3, "result": {}}
    assert far.closed


def test_a_process_without_standard_input_and_output_cannot_serve(
    home: Path, work: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def none() -> Far:
        raise OSError(9, "Bad file descriptor")

    monkeypatch.setattr(wire_module, "taken", none)

    code = main(["serve", "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err == (
        "columbina: 'columbina serve' talks over its standard input and output, and this "
        "process has none: [Errno 9] Bad file descriptor\n"
    )
    # Nothing was loaded for it.
    assert not home.exists()


@pytest.mark.parametrize("flag", [["--session-id", "s-1"], ["--continue"], ["--json"]])
def test_serving_names_no_session_and_no_task(
    capsys: pytest.CaptureFixture[str], flag: list[str]
) -> None:
    # Which session is open is said over the wire, by whoever is served.
    with pytest.raises(SystemExit) as stopped:
        main(["serve", *flag])

    assert stopped.value.code == 2
    assert flag[0] in capsys.readouterr().err


def test_a_task_loads_the_program_without_an_interface_unless_a_profile_is_named(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["run", "Do it.", "--cwd", str(work), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert entries["frontend"]["plugin"] == "columbina.apps.headless"
    assert "commands" not in entries


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


# The working directory


@pytest.mark.parametrize("command", [["run", "Do it."], ["chat"], []], ids=["run", "chat", "none"])
def test_a_working_directory_that_is_not_there_is_refused_before_anything_runs(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str], command: list[str]
) -> None:
    missing = tmp_path / "nowhere"
    a_file = tmp_path / "notes.txt"
    a_file.write_text("text", encoding="utf-8")

    codes = [
        main([*command, "--cwd", str(place), "--profile", profile(tmp_path, {"text": "never"})])
        for place in (missing, a_file)
    ]

    captured = capsys.readouterr()
    assert codes == [2, 2]
    assert captured.out == ""
    assert captured.err == (
        f"columbina: the working directory {str(missing)!r} is not a directory\n"
        f"columbina: the working directory {str(a_file)!r} is not a directory\n"
    )
    # No session was begun.
    assert not home.exists()


# What a terminal is sent


ESC = chr(0x1B)
STAND_IN = chr(0xFFFD)
OVERRIDE = chr(0x202E)


class Screen(io.StringIO):
    """A standard output that is a terminal."""

    def isatty(self) -> bool:
        return True


def test_what_is_said_about_a_profile_that_cannot_be_read_cannot_steer_a_terminal(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The name of a directory can turn the direction of the text, and what is said about a
    # file that is no YAML repeats a part of the file.
    odd = tmp_path / f"odd{OVERRIDE}dir"
    odd.mkdir()
    (odd / "bad.yml").write_text(f"- [not{OVERRIDE} a mapping", encoding="utf-8")

    code = main(["run", "Do it.", "--profile", str(odd / "bad.yml"), "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 2
    assert OVERRIDE not in captured.err
    assert f"odd{STAND_IN}dir" in captured.err
    assert f"- [not{STAND_IN} a mapping" in captured.err


def test_what_is_logged_during_a_run_cannot_steer_a_terminal(
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # The directory of the data can have any name. The log names a file in it when it cuts
    # off a line that a run before this one did not finish.
    home = tmp_path / f"home{OVERRIDE}x"
    monkeypatch.setenv("COLUMBINA_HOME", str(home))
    arguments = ["run", "Hello?", "--cwd", str(work), "--session-id", "s-1"]
    main([*arguments, "--profile", profile(tmp_path, {"text": "One."})])
    (file,) = home.rglob(SESSION_FILE)
    with file.open("ab") as log:
        log.write(b'{"seq": 99')
    capsys.readouterr()

    code = main([*arguments, "--profile", profile(tmp_path, {"text": "Two."})])

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == "Two." + chr(10)
    assert OVERRIDE not in captured.err
    assert "columbina: cut an unfinished line of 10 bytes off the end of " in captured.err
    assert f"home{STAND_IN}x" in captured.err


def test_what_is_logged_before_a_conversation_begins_cannot_steer_a_terminal(
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # A conversation shows what is logged above its prompt. Before there is a prompt, the
    # log goes where it goes for a run, and through the same filter. Here the rules of the
    # user cannot be read, and the log names their file.
    home = tmp_path / f"home{OVERRIDE}x"
    home.mkdir()
    (home / "permissions.yml").write_text("deny: [", encoding="utf-8")
    monkeypatch.setenv("COLUMBINA_HOME", str(home))

    code = main(["chat", "--cwd", str(work)])

    captured = capsys.readouterr()
    assert code == 2
    assert OVERRIDE not in captured.err
    assert "columbina: plugin 'permissions' failed to load" in captured.err
    assert f"home{STAND_IN}x" in captured.err


@pytest.mark.parametrize("command", [["chat"], ["run", "Do it."]])
def test_what_is_said_about_an_argument_that_is_not_known_cannot_steer_a_terminal(
    capsys: pytest.CaptureFixture[str], command: list[str]
) -> None:
    # Whatever calls the program may hand it a text that it got from somewhere else.
    with pytest.raises(SystemExit) as stopped:
        main([*command, f"--bad={ESC}[2J{OVERRIDE}"])

    said = capsys.readouterr().err
    assert stopped.value.code == 2
    assert ESC not in said
    assert OVERRIDE not in said
    assert f"--bad={STAND_IN}[2J{STAND_IN}" in said


def test_the_composition_is_printed_as_it_is_for_a_program_and_made_fit_for_a_terminal(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    said = f"a{OVERRIDE}b"
    arguments = ["run", "Do it.", "--cwd", str(work), "--dump-config"]
    arguments += ["--profile", profile(tmp_path, {"text": said})]

    main(arguments)
    for_a_program = capsys.readouterr().out
    screen = Screen()
    monkeypatch.setattr(sys, "stdout", screen)
    main(arguments)

    assert said in for_a_program
    assert screen.getvalue() == for_a_program.replace(OVERRIDE, STAND_IN)


def test_the_lines_of_json_of_a_run_cannot_steer_a_terminal(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    work = tmp_path / f"work{OVERRIDE}txt"
    work.mkdir()

    code = main(
        [
            "run",
            "Hello?",
            "--profile",
            profile(tmp_path, {"text": "Hi."}),
            "--cwd",
            str(work),
            "--json",
        ]
    )

    out = capsys.readouterr().out
    assert code == 0
    assert OVERRIDE not in out
    assert "work\\u202etxt" in out.splitlines()[0]
    assert lines(out)[0]["cwd"] == str(work)
