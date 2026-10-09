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
