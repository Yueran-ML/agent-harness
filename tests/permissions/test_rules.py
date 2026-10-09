"""Rules about calls of tools: how they are read, and what they speak about."""

import os
import time
from pathlib import Path

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.fs import locate
from columbina.permissions import rules
from columbina.permissions.rules import (
    Place,
    Rule,
    covers_command,
    covers_file,
    parse_rule,
    place_of,
    wild,
)

# How a rule is written


@pytest.mark.parametrize(
    ("text", "rule"),
    [
        ("grep", Rule("grep")),
        ("  grep  ", Rule("grep")),
        ("read(.env)", Rule("read", ".env")),
        ("write(docs/**)", Rule("write", "docs/**")),
        ("powershell(git status*)", Rule("powershell", "git status*")),
        ("powershell( git status* )", Rule("powershell", "git status*")),
        ("bash(echo (hi))", Rule("bash", "echo (hi)")),
        ("mcp__github__*", Rule("mcp__github__*")),
        ("my-tool.v2(x)", Rule("my-tool.v2", "x")),
        ("*", Rule("*")),
        ("*(secrets)", Rule("*", "secrets")),
    ],
)
def test_a_rule_is_a_name_or_a_name_with_a_pattern(text: str, rule: Rule) -> None:
    assert parse_rule(text) == rule


@pytest.mark.parametrize(
    ("text", "wrong"),
    [
        ("", "names no tool"),
        ("   ", "names no tool"),
        ("(x)", "names no tool"),
        ("read(", "does not close it"),
        ("read(.env", "does not close it"),
        ("read(.env) and more", "does not close it"),
        ("read()", "nothing between its parentheses"),
        ("read(  )", "nothing between its parentheses"),
        ("read file(x)", "a character that no name of a tool holds"),
        ("read;rm", "a character that no name of a tool holds"),
    ],
)
def test_what_is_no_rule_is_refused_in_words_that_say_what_is_wrong(text: str, wrong: str) -> None:
    with pytest.raises(ValueError, match=wrong) as refused:
        parse_rule(text)

    assert repr(text) in str(refused.value)


@pytest.mark.parametrize("rule", [Rule("grep"), Rule("read", ".env"), Rule("bash", "echo (hi)")])
def test_a_rule_is_written_the_way_it_is_read(rule: Rule) -> None:
    assert parse_rule(str(rule)) == rule


# A star


@pytest.mark.parametrize(
    ("pattern", "text", "fits"),
    [
        ("", "", True),
        ("", "a", False),
        ("a", "", False),
        ("*", "", True),
        ("*", "anything at all", True),
        ("git status", "git status", True),
        ("git status", "git status ", False),
        ("git status*", "git status", True),
        ("git status*", "git status --short", True),
        ("git status*", "git stash", False),
        ("*.py", "a.py", True),
        ("*.py", "a.pyc", False),
        ("a*b*c", "abc", True),
        ("a*b*c", "a-b-b-c", True),
        ("a*b*c", "a-c-b", False),
        ("**", "x", True),
        ("a**b", "ab", True),
        ("*a", "banana", True),
        ("*a", "bananas", False),
        ("a*", "banana", False),
        ("?", "?", True),
        ("?", "a", False),
        ("[ab]", "a", False),
    ],
)
def test_a_star_stands_for_any_run_of_characters_and_nothing_else_is_special(
    pattern: str, text: str, fits: bool
) -> None:
    assert wild(pattern, text) is fits


def by_definition(pattern: str, text: str) -> bool:
    """The same question, answered by trying every length for every star."""
    if not pattern:
        return not text
    if pattern[0] == "*":
        return any(by_definition(pattern[1:], text[skip:]) for skip in range(len(text) + 1))
    return bool(text) and pattern[0] == text[0] and by_definition(pattern[1:], text[1:])


@given(st.text(alphabet="ab*", max_size=7), st.text(alphabet="ab", max_size=8))
def test_a_pattern_fits_what_trying_every_way_says_it_fits(pattern: str, text: str) -> None:
    assert wild(pattern, text) is by_definition(pattern, text)


def test_a_pattern_with_many_stars_does_not_take_long_on_a_long_text() -> None:
    began = time.monotonic()

    assert not wild("*a" * 30 + "b", "a" * 5000)
    assert time.monotonic() - began < 2


# Which tools a rule speaks about


@pytest.mark.parametrize(
    ("rule", "tool", "effect", "speaks"),
    [
        ("read", "read", "read", True),
        ("read", "grep", "read", True),
        ("read", "glob", "read", True),
        ("read", "edit", "write", False),
        ("read", "powershell", "run", False),
        ("write", "write", "write", True),
        ("write", "edit", "write", True),
        ("write", "read", "read", False),
        ("edit", "edit", "write", True),
        ("edit", "write", "write", False),
        ("grep", "grep", "read", True),
        ("grep", "read", "read", False),
        ("mcp__github__*", "mcp__github__list", "read", True),
        ("mcp__github__*", "mcp__gitlab__list", "read", False),
        ("*", "anything", "none", True),
        ("none", "todo", "none", False),
        ("run", "todo", "none", False),
    ],
)
def test_a_rule_speaks_about_a_tool_by_its_name_or_by_what_it_does(
    rule: str, tool: str, effect: str, speaks: bool
) -> None:
    assert covers_file(parse_rule(rule), tool, effect, None) is speaks  # pyright: ignore[reportArgumentType]


@pytest.mark.parametrize(
    ("rule", "tool", "speaks"),
    [
        ("run", "powershell", True),
        ("run", "bash", True),
        ("powershell", "powershell", True),
        ("bash", "powershell", False),
        ("read", "powershell", False),
        ("*", "powershell", True),
    ],
)
def test_a_rule_named_run_speaks_about_every_tool_that_runs_commands(
    rule: str, tool: str, speaks: bool
) -> None:
    assert covers_command(parse_rule(rule), tool, "git status") is speaks


# Commands


@pytest.mark.parametrize(
    ("rule", "command", "speaks"),
    [
        ("bash(git status*)", "git status", True),
        ("bash(git status*)", "  git status --short  ", True),
        ("bash(git status*)", "git push", False),
        ("bash(git status*)", "sudo git status", False),
        ("bash(git *)", "git", False),
        ("bash(git*)", "gitk", True),
        ("bash(*)", "anything; at all", True),
        ("bash(rm -rf /)", "rm -rf /", True),
        ("bash(Git status)", "git status", False),
        ("bash(a/b)", "a\\b", False),
    ],
)
def test_a_pattern_for_commands_is_matched_against_the_text_of_the_command(
    rule: str, command: str, speaks: bool
) -> None:
    assert covers_command(parse_rule(rule), "bash", command) is speaks


# Files


def inside(relative: str) -> Place:
    """A file in the working directory ``/work/project``."""
    absolute = "/work/project" if relative == "." else f"/work/project/{relative}"
    return Place(relative, absolute)


def outside(absolute: str, relative: str | None = None) -> Place:
    return Place(relative, absolute)


@pytest.mark.parametrize(
    ("pattern", "place", "speaks"),
    [
        # One name is looked for at every depth, and speaks about all below it.
        (".env", inside(".env"), True),
        (".env", inside("app/.env"), True),
        (".env", inside("app/.env.example"), False),
        (".env", inside(".env/keys"), True),
        ("secrets", inside("secrets/a/b.txt"), True),
        ("secrets", inside("app/secrets/b.txt"), True),
        ("secrets", inside("secrets.txt"), False),
        ("*.pem", inside("certs/server.pem"), True),
        ("*.pem", inside("certs/server.pem.bak"), False),
        (".env", outside("/home/me/other/.env", "../../home/me/other/.env"), True),
        ("*.pem", outside("/etc/ssl/server.pem", "../../etc/ssl/server.pem"), True),
        # Several names start at the working directory.
        ("docs/**", inside("docs/a.md"), True),
        ("docs/**", inside("docs"), True),
        ("docs/**", inside("app/docs/a.md"), False),
        ("docs", inside("docs/guide/a.md"), True),
        ("docs/*.md", inside("docs/a.md"), True),
        ("docs/*.md", inside("docs/guide/a.md"), False),
        ("docs/*.md", inside("docs/a.md/x"), True),
        ("src/**/test_*.py", inside("src/test_a.py"), True),
        ("src/**/test_*.py", inside("src/x/y/test_a.py"), True),
        ("src/**/test_*.py", inside("src/x/y/a_test.py"), False),
        ("src/a", outside("/elsewhere/src/a", "../../elsewhere/src/a"), False),
        # A dot and a slash pin a single name to the top.
        ("./.env", inside(".env"), True),
        ("./.env", inside("app/.env"), False),
        ("./.env", outside("/home/me/.env", "../../home/me/.env"), False),
        (".", inside("."), True),
        (".", inside("anything/at/all"), True),
        (".", outside("/elsewhere/a", "../../elsewhere/a"), False),
        ("**", inside("a/b"), True),
        ("**", outside("/elsewhere/a", "../../elsewhere/a"), True),
        # Two dots lead out of the working directory.
        ("../notes", outside("/work/notes/today.md", "../notes/today.md"), True),
        ("../notes", outside("/work/other/today.md", "../other/today.md"), False),
        ("../notes", inside("notes/today.md"), False),
        ("..", outside("/work/notes", "../notes"), True),
        ("..", inside("a.txt"), False),
        # A full path is matched against the full path, wherever the working directory is.
        ("/etc/**", outside("/etc/passwd", "../../etc/passwd"), True),
        ("/etc", outside("/etc/ssl/a.pem", "../../etc/ssl/a.pem"), True),
        ("/etc/**", outside("/var/etc/passwd", "../../var/etc/passwd"), False),
        ("/work/project/keys", inside("keys/id"), True),
        ("/work/project/keys", inside("app/keys/id"), False),
        ("/", outside("/anything", "../../anything"), True),
        ("C:/Users/me/.ssh", outside("C:/Users/me/.ssh/id_rsa"), True),
        ("C:\\Users\\me\\.ssh", outside("C:/Users/me/.ssh/id_rsa"), True),
        ("C:/Users/me/.ssh", outside("D:/Users/me/.ssh/id_rsa"), False),
        # Nothing leads from the working directory to a file on another drive.
        ("docs/a", outside("D:/docs/a"), False),
        ("./a", outside("D:/a"), False),
        ("a", outside("D:/docs/a"), True),
        ("//server/share/**", outside("//server/share/a.txt"), True),
        ("//server/share/**", outside("/server/share/a.txt"), False),
        # A backslash is a slash, and slashes at the end or doubled do not count.
        ("docs\\guide", inside("docs/guide/a.md"), True),
        ("docs//guide/", inside("docs/guide/a.md"), True),
    ],
)
def test_a_pattern_for_files_speaks_about_the_files_it_means(
    pattern: str, place: Place, speaks: bool
) -> None:
    assert covers_file(Rule("read", pattern), "read", "read", place) is speaks


def test_a_tilde_stands_for_the_home_directory() -> None:
    home = Path.home().as_posix()

    assert covers_file(Rule("read", "~/.ssh"), "read", "read", outside(f"{home}/.ssh/id_rsa"))
    assert covers_file(Rule("read", "~"), "read", "read", outside(f"{home}/notes.txt"))
    assert not covers_file(Rule("read", "~/.ssh"), "read", "read", outside(f"{home}/.sshd/x"))
    assert not covers_file(Rule("read", "~x"), "read", "read", outside(f"{home}x"))


def test_upper_and_lower_case_count_where_the_disk_tells_them_apart() -> None:
    rule = Rule("read", "Secrets/*.PEM")

    assert covers_file(rule, "read", "read", inside("secrets/a.pem")) is rules.IGNORES_CASE
    assert covers_file(rule, "read", "read", inside("Secrets/a.PEM"))


def test_a_rule_with_a_pattern_does_not_speak_about_a_call_that_names_no_file() -> None:
    assert not covers_file(Rule("todo", "x"), "todo", "none", None)
    assert covers_file(Rule("todo"), "todo", "none", None)


def test_a_pattern_of_many_parts_and_a_path_of_many_names_are_matched_all_the_same() -> None:
    deep = "/".join(["d"] * 3000)
    began = time.monotonic()

    assert covers_file(Rule("read", "**/" * 500 + "*.env"), "read", "read", inside("app/x.env"))
    assert covers_file(Rule("read", "*.env"), "read", "read", inside(f"{deep}/x.env"))
    assert not covers_file(Rule("read", "*.env"), "read", "read", inside(f"{deep}/x.txt"))
    assert time.monotonic() - began < 5


# Where a file is, written both ways


def test_a_file_in_the_working_directory_is_written_from_there_and_in_full(tmp_path: Path) -> None:
    work = tmp_path / "work"
    (work / "sub").mkdir(parents=True)
    real = Path(os.path.realpath(work))

    assert place_of(locate(work, "sub/a.txt")) == Place(
        "sub/a.txt", (real / "sub/a.txt").as_posix()
    )
    assert place_of(locate(work, ".")) == Place(".", real.as_posix())


def test_a_file_outside_the_working_directory_is_written_with_the_steps_that_lead_to_it(
    tmp_path: Path,
) -> None:
    work = tmp_path / "work"
    work.mkdir()
    real = Path(os.path.realpath(tmp_path))

    assert place_of(locate(work, "../notes/a.txt")) == Place(
        "../notes/a.txt", (real / "notes/a.txt").as_posix()
    )


def test_a_file_that_no_path_leads_to_has_only_its_full_path(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def no_way(path: object, start: object) -> str:
        raise ValueError("path is on mount 'D:', start on mount 'C:'")

    work = tmp_path / "work"
    work.mkdir()
    where = locate(work, "../elsewhere.txt")
    monkeypatch.setattr(os.path, "relpath", no_way)

    assert place_of(where) == Place(None, where.path.as_posix())
