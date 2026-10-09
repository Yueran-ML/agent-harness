"""The terminal that the interface talks to: found when there is one, and stood in for in tests."""

import sys
from typing import Any

import pytest
from prompt_toolkit.input import create_pipe_input
from prompt_toolkit.output import DummyOutput

from columbina.apps.cli import terminal
from columbina.apps.cli.terminal import Terminal
from support.terminal import desk

DOT = chr(0x25CF)
TICK = chr(0x2713)


class Stream:
    """A standard stream that says whether a terminal is behind it."""

    def __init__(self, at_a_terminal: bool, encoding: str | None = "utf-8") -> None:
        self._at_a_terminal = at_a_terminal
        self.encoding = encoding

    def isatty(self) -> bool:
        return self._at_a_terminal


def streams(monkeypatch: pytest.MonkeyPatch, *, keys: bool, screen: bool, **more: Any) -> None:
    monkeypatch.setattr(sys, "stdin", Stream(keys))
    monkeypatch.setattr(sys, "stdout", Stream(screen, **more))


@pytest.mark.parametrize(("keys", "screen"), [(False, True), (True, False), (False, False)])
def test_a_program_that_was_started_without_a_terminal_has_none(
    monkeypatch: pytest.MonkeyPatch, keys: bool, screen: bool
) -> None:
    streams(monkeypatch, keys=keys, screen=screen)

    assert terminal.real() is None


def test_a_program_that_was_started_in_a_terminal_gets_it(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    with create_pipe_input() as keys:
        screen = DummyOutput()
        streams(monkeypatch, keys=True, screen=True, encoding="cp936")
        monkeypatch.setattr(terminal, "create_input", lambda: keys)
        monkeypatch.setattr(terminal, "create_output", lambda: screen)
        monkeypatch.delenv("NO_COLOR", raising=False)
        monkeypatch.setenv("WT_SESSION", "a terminal that shows every character")

        found = terminal.real()

    assert found == Terminal(keys, screen, encoding="cp936", colours=True)


def test_a_stream_that_names_no_encoding_is_taken_for_utf8(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    with create_pipe_input() as keys:
        streams(monkeypatch, keys=True, screen=True, encoding=None)
        monkeypatch.setattr(terminal, "create_input", lambda: keys)
        monkeypatch.setattr(terminal, "create_output", DummyOutput)
        monkeypatch.setenv("WT_SESSION", "a terminal that shows every character")

        found = terminal.real()

    assert found is not None
    assert found.encoding == "utf-8"


@pytest.mark.parametrize(
    ("platform", "environ", "encoding", "marks"),
    [
        ("linux", {}, "utf-8", "utf-8"),
        ("linux", {}, "iso8859-1", "iso8859-1"),
        ("darwin", {}, None, "utf-8"),
        ("win32", {"WT_SESSION": "x"}, "utf-8", "utf-8"),
        ("win32", {"WT_SESSION": "x"}, None, "utf-8"),
        ("win32", {}, "utf-8", "ascii"),
        ("win32", {"TERM": "xterm"}, "cp936", "ascii"),
    ],
)
def test_the_old_console_of_windows_is_shown_plain_marks(
    platform: str, environ: dict[str, str], encoding: str | None, marks: str
) -> None:
    assert terminal.marks_encoding(platform, environ, encoding) == marks


@pytest.mark.parametrize("value", ["1", "true", ""])
def test_no_colours_are_used_when_the_environment_asks_for_none(
    monkeypatch: pytest.MonkeyPatch, value: str
) -> None:
    with create_pipe_input() as keys:
        streams(monkeypatch, keys=True, screen=True)
        monkeypatch.setattr(terminal, "create_input", lambda: keys)
        monkeypatch.setattr(terminal, "create_output", DummyOutput)
        monkeypatch.setenv("NO_COLOR", value)

        found = terminal.real()

    # The convention says: present and not empty.
    assert found is not None
    assert found.colours is (value == "")


def test_a_window_that_is_no_console_counts_as_no_terminal(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class NotAConsoleError(Exception):
        pass

    def refuse() -> Any:
        raise NotAConsoleError

    with create_pipe_input() as keys:
        streams(monkeypatch, keys=True, screen=True)
        monkeypatch.setattr(terminal, "create_input", lambda: keys)
        monkeypatch.setattr(terminal, "create_output", refuse)
        monkeypatch.setattr(terminal, "NO_CONSOLE", (NotAConsoleError,))

        assert terminal.real() is None


def test_another_failure_to_reach_the_screen_is_not_hidden(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def broken() -> Any:
        raise RuntimeError("the screen is broken")

    with create_pipe_input() as keys:
        streams(monkeypatch, keys=True, screen=True)
        monkeypatch.setattr(terminal, "create_input", lambda: keys)
        monkeypatch.setattr(terminal, "create_output", broken)

        with pytest.raises(RuntimeError, match="the screen is broken"):
            terminal.real()


@pytest.mark.parametrize(
    ("encoding", "wanted", "shown"),
    [
        ("utf-8", TICK, TICK),
        ("UTF-8", DOT, DOT),
        ("gbk", DOT, DOT),
        ("gbk", TICK, "ok"),
        ("cp1252", TICK, "ok"),
        ("ascii", DOT, "ok"),
        ("no-such-encoding", DOT, "ok"),
    ],
)
def test_a_symbol_that_the_screen_cannot_show_is_written_plainly(
    encoding: str, wanted: str, shown: str
) -> None:
    with desk(encoding=encoding) as at:
        assert at.terminal.symbol(wanted, "ok") == shown


def test_the_terminal_of_a_test_takes_keys_and_keeps_what_is_shown() -> None:
    with desk() as at:
        at.type("ab")
        at.enter("c")
        keys = at.terminal.input.read_keys()
        at.terminal.output.write("shown\n")
        at.terminal.output.flush()

        assert [key.data for key in keys] == ["a", "b", "c", "\r"]
        assert at.shown() == "shown\n"
        assert at.terminal.colours is False


# Text from elsewhere


ESC = chr(0x1B)
STAND_IN = terminal.STAND_IN
OVERRIDE = chr(0x202E)
POP = chr(0x202C)


def test_ordinary_text_goes_to_the_terminal_as_it_is() -> None:
    joiner, accent, wide_space = chr(0x200D), chr(0x0301), chr(0x3000)
    accented, chinese = f"na{chr(0xEF)}ve caf{chr(0xE9)}", chr(0x5217) + chr(0x8868)
    text = f"{accented}, {chinese}{wide_space}e{accent} a{joiner}b"

    assert terminal.harmless(text) == text
    assert terminal.harmless("") == ""
    assert terminal.harmless("one\ttwo\nthree") == "one\ttwo\nthree"


@pytest.mark.parametrize(
    "code",
    [0x00, 0x01, 0x02, 0x07, 0x08, 0x0B, 0x0C, 0x0D, 0x1B, 0x7F, 0x85, 0x9B, 0x9D, 0x2028, 0x2029],
)
def test_a_character_that_a_terminal_acts_on_is_replaced_by_one_that_it_shows(code: int) -> None:
    assert terminal.harmless(f"a{chr(code)}b") == f"a{STAND_IN}b"


@pytest.mark.parametrize(
    "code", [0x061C, 0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066, 0x2069]
)
def test_a_character_that_turns_the_direction_of_text_is_replaced(code: int) -> None:
    assert terminal.harmless(f"a{chr(code)}b") == f"a{STAND_IN}b"


def test_half_of_a_pair_that_cannot_be_sent_is_replaced() -> None:
    assert terminal.harmless(f"a{chr(0xD800)}b") == f"a{STAND_IN}b"


def test_a_sequence_that_would_steer_the_terminal_is_shown_and_not_carried_out() -> None:
    clear, title = f"{ESC}[2J", f"{ESC}]2;hello{chr(7)}"

    assert terminal.harmless(f"before{clear}{title}after") == (
        f"before{STAND_IN}[2J{STAND_IN}]2;hello{STAND_IN}after"
    )


def test_text_that_is_made_harmless_keeps_its_length() -> None:
    text = "".join(chr(code) for code in range(0x250)) + OVERRIDE + POP

    assert len(terminal.harmless(text)) == len(text)


def test_what_cannot_be_seen_is_named_where_a_person_is_asked() -> None:
    zero_width, no_break, wide_space = chr(0x200B), chr(0xA0), chr(0x3000)

    assert terminal.spelled_out("python setup.py --dry-run") == "python setup.py --dry-run"
    assert terminal.spelled_out(f"python setup{zero_width}.py") == "python setup<U+200B>.py"
    assert terminal.spelled_out(f"rm{no_break}-rf{wide_space}x") == "rm<U+00A0>-rf<U+3000>x"
    assert terminal.spelled_out(f"git push {OVERRIDE}gro.live{POP}") == (
        "git push <U+202E>gro.live<U+202C>"
    )
    assert terminal.spelled_out(f"a{ESC}[2Jb\rc") == "a<U+001B>[2Jb<U+000D>c"
    assert terminal.spelled_out(f"a{chr(0xD800)}b") == "a<U+D800>b"


def test_spaces_tabs_and_line_feeds_are_not_named() -> None:
    assert terminal.spelled_out("a b\tc\nd") == "a b\tc\nd"


def test_whatever_a_command_is_refused_a_rule_for_is_named_in_the_question() -> None:
    """Where the splitter refuses a command for what cannot be seen in it, the question shows it."""
    from columbina.permissions.commands import split_bash, split_powershell

    for code in (
        0x00,
        0x1B,
        0x7F,
        0x85,
        0xA0,
        0xAD,
        0x200B,
        0x200D,
        0x202E,
        0x2028,
        0x3000,
        0xFEFF,
    ):
        command = f"git{chr(code)}status"
        assert split_bash(command) is None, hex(code)
        assert split_powershell(command) is None, hex(code)
        assert terminal.spelled_out(command) == f"git<U+{code:04X}>status", hex(code)
