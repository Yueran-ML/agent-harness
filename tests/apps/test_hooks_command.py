"""``columbina hooks``: the hooks of a directory, listed, trusted, and trusted no more."""

import json
import os
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.apps.terminal_text import STAND_IN
from columbina.hooks.config import digest
from columbina.hooks.run import MAX_COMMAND
from support.hooks import needs_shell, python_hook
from support.launch import profile

FROM_HOME = {
    "user_file": "${COLUMBINA_HOME}/hooks.json",
    "trust_file": "${COLUMBINA_HOME}/trusted-hooks.json",
}
"""The config that has the hooks read from the home of the program, the way a profile says it."""

NOT_TRUSTED = (
    "The file is not trusted as it is now, so none of these hooks runs. "
    "'columbina hooks trust' trusts it.\n"
)
TRUSTED = "The file is trusted as it is now, so these hooks run.\n"
UNTIL_IT_CHANGES = (
    "When the file changes, its hooks stop until it is trusted again. "
    "'columbina hooks distrust' takes the trust back.\n"
)
NO_HOOKS_RUN = (
    "columbina: the profile runs no hooks: none of its entries loads columbina.hooks, or the "
    "one that does is disabled. So there are no hooks to look at\n"
)
VERIFY = {"Stop": [{"hooks": [{"type": "command", "command": "pytest -q"}]}]}
"""What a hook file may hold: one command for the moment at which a turn would end."""


def chosen(tmp_path: Path, config: dict[str, Any] | None = None, **entry: Any) -> str:
    """Write a profile whose sessions run hooks, and return its path."""
    hooks = {
        "id": "hooks",
        "plugin": "columbina.hooks",
        "config": FROM_HOME if config is None else config,
        **entry,
    }
    return profile(tmp_path, {"text": "Done."}, extra=[{"insert": [hooks]}])


def asked(*arguments: str, capsys: pytest.CaptureFixture[str]) -> tuple[int, str, str]:
    capsys.readouterr()
    code = main(["hooks", *arguments])
    captured = capsys.readouterr()
    return code, captured.out, captured.err


def command(text: str, **more: Any) -> dict[str, Any]:
    return {"type": "command", "command": text, **more}


def written(path: Path, hooks: Any) -> bytes:
    """Write a hook file, and return what it holds."""
    content = json.dumps({"hooks": hooks}).encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return content


def theirs(work: Path) -> Path:
    """Where the hook file of a project lies, with every link on the way to it followed."""
    return Path(os.path.realpath(work)) / ".columbina" / "hooks.json"


def recorded(home: Path) -> Any:
    return json.loads((home / "trusted-hooks.json").read_bytes())


# Listing


def test_the_hooks_of_both_files_are_listed_with_their_moments_tools_and_times(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    mine = home / "hooks.json"
    written(
        mine,
        {
            "Stop": [{"hooks": [command("python check.py", timeout=1)]}],
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [command("python guard.py"), command("python log.py", timeout=0.5)],
                }
            ],
        },
    )
    written(
        theirs(work),
        {
            "PostToolUse": [
                {
                    "matcher": "Edit|Write",
                    "hooks": [command("ruff format\nruff check --fix", timeout=30)],
                },
                {"matcher": "*", "hooks": [command("echo done")]},
                {"hooks": [command("echo over")]},
            ],
            "SessionStart": [{"matcher": "Bash", "hooks": [command("git status")]}],
        },
    )

    code, out, err = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    assert out == (
        f"The hooks of the user, from {mine}:\n"
        "  at PreToolUse, for the tools Bash, within 60 seconds:\n"
        "      python guard.py\n"
        "  at PreToolUse, for the tools Bash, within 0.5 seconds:\n"
        "      python log.py\n"
        "  at Stop, within 1 second:\n"
        "      python check.py\n"
        "\n"
        f"The hooks of this project, from {theirs(work)}:\n"
        # Where no tool is at work, a matcher says nothing.
        "  at SessionStart, within 60 seconds:\n"
        "      git status\n"
        "  at PostToolUse, for the tools Edit|Write, within 30 seconds:\n"
        "      ruff format\n"
        "      ruff check --fix\n"
        "  at PostToolUse, for every tool, within 60 seconds:\n"
        "      echo done\n"
        "  at PostToolUse, for every tool, within 60 seconds:\n"
        "      echo over\n" + NOT_TRUSTED
    )


def test_a_directory_without_hook_files_is_told_where_they_would_lie(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys) == (
        0,
        f"The user has no hook file. It would be {home / 'hooks.json'}.\n"
        "\n"
        f"This project has no hook file. It would be {theirs(work)}.\n",
        "",
    )
    # Looking makes nothing.
    assert not home.exists()
    assert not theirs(work).parent.exists()


def test_a_file_that_cannot_be_read_or_used_is_said_with_the_reason(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # A directory lies where the file of the user should, and the other file holds a list.
    (home / "hooks.json").mkdir(parents=True)
    theirs(work).parent.mkdir()
    theirs(work).write_bytes(b"[]")

    code, out, err = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    first, between, second = out.splitlines()
    assert first.startswith(f"The hook file of the user, {home / 'hooks.json'}, cannot be read: ")
    assert between == ""
    assert second == (
        f"The hook file of this project, {theirs(work)}, is not used: it holds a list and no object"
    )


def test_a_file_without_hooks_says_so_and_what_was_passed_over_in_it(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home / "hooks.json", {})
    written(theirs(work), {"Stop": [{"hooks": [{"type": "prompt", "prompt": "Is it done?"}]}]})

    code, out, err = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    # Nothing of the second file would run, so whether it is trusted is nothing to speak of.
    assert out == (
        f"The hook file of the user, {home / 'hooks.json'}, holds no hooks.\n"
        "\n"
        f"The hook file of this project, {theirs(work)}, holds no hooks.\n"
        "Not all of the file is used:\n"
        '  Stop, group 1, hook 1 is of the type "prompt". Only hooks of the type "command" are '
        "run, and this one is passed over\n"
    )


def test_what_was_passed_over_stands_after_the_hooks_that_count(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home / "hooks.json", {**VERIFY, "Later": []})

    _, out, _ = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert out.splitlines()[:5] == [
        f"The hooks of the user, from {home / 'hooks.json'}:",
        "  at Stop, within 60 seconds:",
        "      pytest -q",
        "Not all of the file is used:",
        '  "Later" is no moment at which hooks run here, and is passed over. The moments are: '
        "SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop",
    ]


def test_what_a_profile_leaves_out_is_said(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(theirs(work), VERIFY)

    # Neither a file of the user nor a record of trust.
    bare = profile(
        tmp_path, {"text": "Done."}, extra=[{"insert": [{"id": "h", "plugin": "columbina.hooks"}]}]
    )
    assert asked("list", "--profile", bare, "--cwd", str(work), capsys=capsys) == (
        0,
        "The profile names no hook file of the user.\n"
        "\n"
        f"The hooks of this project, from {theirs(work)}:\n"
        "  at Stop, within 60 seconds:\n"
        "      pytest -q\n"
        "The profile names no record of trust, so no file of a project is trusted and none of "
        "these hooks runs.\n",
        "",
    )

    apart = chosen(tmp_path, {**FROM_HOME, "project": False})
    assert asked("list", "--profile", apart, "--cwd", str(work), capsys=capsys) == (
        0,
        f"The user has no hook file. It would be {home / 'hooks.json'}.\n"
        "\n"
        "The profile does not read the hook file of a project.\n",
        "",
    )


def test_a_command_that_is_too_long_to_be_run_is_marked(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    longest = "x" * MAX_COMMAND
    written(home / "hooks.json", {"Stop": [{"hooks": [command(longest + "x"), command(longest)]}]})

    _, out, _ = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert out.splitlines()[1:7] == [
        "  at Stop, within 60 seconds:",
        f"      {longest}x",
        f"      (This command has more than {MAX_COMMAND} characters. One that long is not run.)",
        "  at Stop, within 60 seconds:",
        f"      {longest}",
        # Nothing is said about a command of just the length that is run.
        "",
    ]


def test_what_cannot_be_seen_in_a_file_is_named_and_nothing_of_it_moves_the_terminal(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    escape, hidden, turned = chr(0x1B), chr(0x200B), chr(0x202E)
    written(
        theirs(work),
        {
            "PreToolUse": [
                {
                    "matcher": f"Bash\n{turned}",
                    "hooks": [command(f"rm -rf ~{hidden}/x {escape}[2K\r\necho {turned}ko")],
                }
            ],
            f"Later{turned}": [],
        },
    )

    code, out, err = asked("list", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    # What would run is to be read as it is: a command that hides half of itself is no command
    # that anybody can say yes to.
    assert out.splitlines()[3:] == [
        "  at PreToolUse, for the tools Bash<U+000A><U+202E>, within 60 seconds:",
        "      rm -rf ~<U+200B>/x <U+001B>[2K<U+000D>",
        "      echo <U+202E>ko",
        "Not all of the file is used:",
        f'  "Later{STAND_IN}" is no moment at which hooks run here, and is passed over. The '
        "moments are: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop",
        NOT_TRUSTED.rstrip("\n"),
    ]
    assert not {escape, hidden, turned, "\r"} & set(out)


# Trusting


@needs_shell
def test_the_hooks_of_a_project_run_once_its_file_is_trusted(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    mark = tmp_path / "ran.txt"
    hook = python_hook(
        tmp_path,
        f"import pathlib\npathlib.Path({str(mark)!r}).write_text('it ran', encoding='utf-8')\n",
    )
    content = written(theirs(work), {"UserPromptSubmit": [{"hooks": [command(hook)]}]})
    which = chosen(tmp_path)
    task = ["run", "Say hello.", "--profile", which, "--cwd", str(work)]

    assert main(task) == 0

    # Whoever wrote the project has run nothing on this machine yet.
    assert not mark.exists()
    assert "the hooks of this project are not run" in capsys.readouterr().err

    code, out, err = asked("trust", "--profile", which, "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    assert out == (
        f"The hook file of this project, {theirs(work)}, is trusted as it is now. These hooks "
        "run from now on:\n"
        "  at UserPromptSubmit, within 60 seconds:\n"
        f"      {hook}\n" + UNTIL_IT_CHANGES
    )
    assert recorded(home) == {"version": 1, "files": {str(theirs(work)): digest(content)}}

    assert main(task) == 0

    # The session means the file that was trusted, by the same place.
    assert mark.read_text(encoding="utf-8") == "it ran"
    assert capsys.readouterr().err == ""


def test_a_file_that_changes_has_to_be_trusted_again(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(theirs(work), VERIFY)
    where = ("--profile", chosen(tmp_path), "--cwd", str(work))

    assert asked("list", *where, capsys=capsys)[1].endswith(NOT_TRUSTED)
    assert asked("trust", *where, capsys=capsys)[0] == 0
    assert asked("list", *where, capsys=capsys)[1].endswith(TRUSTED)

    changed = written(theirs(work), {"Stop": [{"hooks": [command("pytest -q; rm -rf .")]}]})

    assert asked("list", *where, capsys=capsys)[1].endswith(NOT_TRUSTED)
    assert asked("trust", *where, capsys=capsys)[0] == 0
    assert asked("list", *where, capsys=capsys)[1].endswith(TRUSTED)
    # The record knows one content of a file: the one that was looked at last.
    assert recorded(home) == {"version": 1, "files": {str(theirs(work)): digest(changed)}}
    # Nothing lies beside the record that was used to write it.
    assert [entry.name for entry in home.iterdir()] == ["trusted-hooks.json"]


def test_the_record_keeps_what_it_holds_about_other_projects(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    other = tmp_path / "other"
    other.mkdir()
    first = written(theirs(work), VERIFY)
    second = written(theirs(other), {})
    which = chosen(tmp_path)

    for project in (work, other):
        assert asked("trust", "--profile", which, "--cwd", str(project), capsys=capsys)[0] == 0

    assert recorded(home) == {
        "version": 1,
        "files": {str(theirs(work)): digest(first), str(theirs(other)): digest(second)},
    }

    assert asked("distrust", "--profile", which, "--cwd", str(work), capsys=capsys)[0] == 0

    assert recorded(home) == {"version": 1, "files": {str(theirs(other)): digest(second)}}


def test_a_file_without_hooks_is_trusted_all_the_same(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    held = {"Stop": [{"hooks": [{"type": "prompt", "prompt": "Is it done?"}]}]}
    content = written(theirs(work), held)

    code, out, err = asked(
        "trust", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys
    )

    assert (code, err) == (0, "")
    assert out == (
        f"The hook file of this project, {theirs(work)}, is trusted as it is now. It holds no "
        "hooks.\n"
        "Not all of the file is used:\n"
        '  Stop, group 1, hook 1 is of the type "prompt". Only hooks of the type "command" are '
        "run, and this one is passed over\n" + UNTIL_IT_CHANGES
    )
    assert recorded(home) == {"version": 1, "files": {str(theirs(work)): digest(content)}}


def test_without_a_hook_file_there_is_nothing_to_trust(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert asked("trust", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        "columbina: this project has no hook file, so there is nothing to trust. It would be "
        f"{theirs(work)}\n",
    )
    assert not home.exists()


@pytest.mark.parametrize(
    ("content", "why"),
    [
        (b"not json", "is not used: it is not JSON: "),
        (b"[]", "is not used: it holds a list and no object\n"),
        (None, "cannot be read: "),
    ],
)
def test_a_file_that_cannot_be_read_or_used_is_not_trusted(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    content: bytes | None,
    why: str,
) -> None:
    theirs(work).parent.mkdir()
    if content is None:
        # A directory lies where the file should.
        theirs(work).mkdir()
    else:
        theirs(work).write_bytes(content)

    code, out, err = asked(
        "trust", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys
    )

    assert (code, out) == (2, "")
    assert err.startswith(f"columbina: the hook file of this project, {theirs(work)}, {why}")
    assert not home.exists()


@pytest.mark.parametrize("what", ["trust", "distrust"])
def test_a_profile_that_keeps_no_record_trusts_no_project(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str], what: str
) -> None:
    written(theirs(work), VERIFY)
    without = chosen(tmp_path, {"user_file": "${COLUMBINA_HOME}/hooks.json"})

    assert asked(what, "--profile", without, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        "columbina: the profile names no record of trust, so the hook file of no project can "
        "be trusted\n",
    )


@pytest.mark.parametrize("what", ["trust", "distrust"])
def test_a_profile_that_reads_no_hook_file_of_a_project_has_none_to_trust(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str], what: str
) -> None:
    written(theirs(work), VERIFY)
    apart = chosen(tmp_path, {**FROM_HOME, "project": False})

    assert asked(what, "--profile", apart, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        "columbina: the profile does not read the hook file of a project, so there is nothing "
        "to trust\n",
    )
    assert not home.exists()


@pytest.mark.parametrize("in_the_way", ["the record", "its directory"])
def test_a_record_that_cannot_be_written_is_said_and_nothing_is_left_behind(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str], in_the_way: str
) -> None:
    written(theirs(work), VERIFY)
    if in_the_way == "the record":
        # A directory lies where the record should.
        (home / "trusted-hooks.json").mkdir(parents=True)
    else:
        # A file lies where the home of the program should.
        home.write_text("no directory", encoding="utf-8")
    which = chosen(tmp_path)

    code, out, err = asked("trust", "--profile", which, "--cwd", str(work), capsys=capsys)

    assert (code, out) == (2, "")
    assert err.startswith(
        f"columbina: the record of trust, {home / 'trusted-hooks.json'}, cannot be written: "
    )
    if in_the_way == "the record":
        assert [entry.name for entry in home.iterdir()] == ["trusted-hooks.json"]
    # Nobody was told that the file is trusted, and it is not.
    assert asked("list", "--profile", which, "--cwd", str(work), capsys=capsys)[1].endswith(
        NOT_TRUSTED
    )


# Taking trust back


def test_trust_is_taken_back(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(theirs(work), VERIFY)
    where = ("--profile", chosen(tmp_path), "--cwd", str(work))
    assert asked("trust", *where, capsys=capsys)[0] == 0

    assert asked("distrust", *where, capsys=capsys) == (
        0,
        f"The hook file of this project, {theirs(work)}, is trusted no more. Its hooks do not "
        "run.\n",
        "",
    )
    assert recorded(home) == {"version": 1, "files": {}}
    assert asked("list", *where, capsys=capsys)[1].endswith(NOT_TRUSTED)

    assert asked("distrust", *where, capsys=capsys) == (
        0,
        f"The hook file of this project, {theirs(work)}, was not trusted.\n",
        "",
    )
    assert [entry.name for entry in home.iterdir()] == ["trusted-hooks.json"]


def test_taking_back_what_was_never_given_writes_nothing(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert asked("distrust", "--profile", chosen(tmp_path), "--cwd", str(work), capsys=capsys) == (
        0,
        f"The hook file of this project, {theirs(work)}, was not trusted.\n",
        "",
    )
    assert not home.exists()


def test_trust_is_taken_back_whatever_has_become_of_the_file(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(theirs(work), VERIFY)
    where = ("--profile", chosen(tmp_path), "--cwd", str(work))
    assert asked("trust", *where, capsys=capsys)[0] == 0
    theirs(work).unlink()

    code, out, _ = asked("distrust", *where, capsys=capsys)

    assert code == 0
    assert out.endswith("is trusted no more. Its hooks do not run.\n")
    assert recorded(home) == {"version": 1, "files": {}}


# Which directory is meant


def test_the_working_directory_is_the_current_one_unless_another_is_named(
    home: Path,
    work: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    written(theirs(work), VERIFY)
    monkeypatch.chdir(work)

    code, out, _ = asked("list", "--profile", chosen(tmp_path), capsys=capsys)

    assert code == 0
    assert f"The hooks of this project, from {theirs(work)}:\n" in out


def test_the_hooks_of_a_project_are_meant_from_anywhere_inside_it(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (work / ".git").mkdir()
    deep = work / "src" / "package"
    deep.mkdir(parents=True)
    content = written(theirs(work), VERIFY)
    which = chosen(tmp_path)

    code, out, _ = asked("trust", "--profile", which, "--cwd", str(deep), capsys=capsys)

    assert code == 0
    assert out.startswith(f"The hook file of this project, {theirs(work)}, is trusted")
    assert recorded(home) == {"version": 1, "files": {str(theirs(work)): digest(content)}}
    for inside in (work, deep):
        told = asked("list", "--profile", which, "--cwd", str(inside), capsys=capsys)[1]
        assert f"The hooks of this project, from {theirs(work)}:\n" in told
        assert told.endswith(TRUSTED)

    code, out, _ = asked("distrust", "--profile", which, "--cwd", str(deep), capsys=capsys)

    assert code == 0
    assert out.startswith(f"The hook file of this project, {theirs(work)}, is trusted no more.")
    assert recorded(home) == {"version": 1, "files": {}}


def test_a_working_directory_that_is_none_is_refused(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    nowhere = str(tmp_path / "nowhere")

    assert asked("list", "--profile", chosen(tmp_path), "--cwd", nowhere, capsys=capsys) == (
        2,
        "",
        f"columbina: the working directory {nowhere!r} is not a directory\n",
    )


# Which profile is meant


def test_a_profile_that_runs_no_hooks_has_none_to_look_at(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    plain = profile(tmp_path, {"text": "Done."})
    assert asked("list", "--profile", plain, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        NO_HOOKS_RUN,
    )

    off = chosen(tmp_path, disabled=True)
    assert asked("trust", "--profile", off, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        NO_HOOKS_RUN,
    )


def test_the_entry_of_the_hooks_is_known_by_what_it_loads_and_not_by_its_name(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home / "hooks.json", VERIFY)

    renamed = chosen(tmp_path, id="formatters")
    code, out, _ = asked("list", "--profile", renamed, "--cwd", str(work), capsys=capsys)

    assert code == 0
    assert out.startswith(f"The hooks of the user, from {home / 'hooks.json'}:\n")

    # An entry of that name that loads something else says nothing about hooks.
    other = profile(
        tmp_path,
        {"text": "Done."},
        extra=[{"insert": [{"id": "hooks", "plugin": "columbina.todo", "config": FROM_HOME}]}],
    )
    assert asked("list", "--profile", other, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        NO_HOOKS_RUN,
    )


def test_a_config_of_the_hooks_that_is_wrong_is_said(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    wrong = chosen(tmp_path, {"user_file": "hooks.json"})

    code, out, err = asked("list", "--profile", wrong, "--cwd", str(work), capsys=capsys)

    assert (code, out) == (2, "")
    assert err.startswith("columbina: hooks (columbina.hooks) has an invalid config: ")
    assert "hooks.json does not begin at a root" in err


def test_a_variable_that_the_config_of_the_hooks_needs_and_is_not_set_is_said(
    home: Path,
    work: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.delenv("COLUMBINA_NOWHERE", raising=False)
    wrong = chosen(tmp_path, {"user_file": "${COLUMBINA_NOWHERE}/hooks.json"})

    assert asked("list", "--profile", wrong, "--cwd", str(work), capsys=capsys) == (
        2,
        "",
        "columbina: entry 'hooks': environment variable 'COLUMBINA_NOWHERE' is not set\n",
    )


def test_what_is_to_be_done_with_the_hooks_has_to_be_said(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    for arguments in (["hooks"], ["hooks", "run"]):
        with pytest.raises(SystemExit) as stopped:
            main(arguments)
        assert stopped.value.code == 2
    assert "list" in capsys.readouterr().err
