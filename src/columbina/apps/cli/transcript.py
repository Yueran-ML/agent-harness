"""What a conversation looks like on the screen.

A :class:`Transcript` turns what happens in a session into lines of text. It
is fed from two sides. The events of the session say what happened for good:
a call, its result, the end of a turn. The chunks of a reply that is still
being written say what is happening now, and only they are quick enough to
show a reply as it grows.

A reply is therefore shown from its chunks, line by line, and the event that
follows only closes it. A reply that came without chunks, as in a session
that is read from its file, is shown from the event. Either way the text is
shown once.

The lines are :class:`rich.text.Text`, so that they carry their colours with
them. :func:`on_screen` turns them into what a terminal is sent. Nothing that
a model or a tool wrote is ever read as markup, and nothing of it reaches the
terminal as a control character: the only sequences that a terminal is sent
are those of the colours that were chosen here.
"""

import io
import json
from collections.abc import Sequence
from dataclasses import dataclass
from typing import cast

from rich.console import Console
from rich.text import Text

from columbina.approval import ApprovalDecidedEvent
from columbina.apps.cli.lines import Lines
from columbina.apps.cli.terminal import Terminal
from columbina.apps.terminal_text import harmless
from columbina.llm import BlockEnd, BlockStart, Chunk, TextDelta
from columbina.retry import RetryEvent
from columbina.session import (
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)

__all__ = ["Symbols", "Transcript", "fit", "on_screen", "recap"]

_NOTE = "dim"
_BAD = "red"
_NAME = "bold"
_MARK = "cyan"
_GOES = "red"
_COMES = "green"

_SUMMARY_CHARACTERS = 100
_CHANGED_LINES = 6
_OUTPUT_LINES = 5
_ERROR_LINES = 3
_RECAP_LINES = 12
_NAMED_FIRST = ("command", "path", "pattern")
"""The arguments that say what a call acts on, the most telling one first."""


@dataclass(frozen=True, slots=True)
class Symbols:
    """The marks that stand before lines of the transcript.

    Attributes:
        call: Before a call of a tool.
        done: Before the result of a call that went well.
        failed: Before the result of a call that did not.
        steer: Before something that the user said while a turn was running.
    """

    call: str = chr(0x25CF)
    done: str = chr(0x2713)
    failed: str = chr(0x2717)
    steer: str = chr(0xBB)

    @classmethod
    def of(cls, terminal: Terminal) -> "Symbols":
        """Choose the marks that a terminal can show."""
        wanted = cls()
        return cls(
            call=terminal.symbol(wanted.call, "*"),
            done=terminal.symbol(wanted.done, "ok"),
            failed=terminal.symbol(wanted.failed, "failed"),
            steer=terminal.symbol(wanted.steer, ">>"),
        )


def on_screen(lines: Sequence[Text], *, colours: bool) -> str:
    """Return what a terminal is sent to show these lines, one below the other."""
    if not lines:
        return ""
    sent = io.StringIO()
    console = Console(
        file=sent,
        force_terminal=colours,
        color_system="standard" if colours else None,
        no_color=not colours,
        highlight=False,
        markup=False,
        emoji=False,
        legacy_windows=False,
    )
    for line in lines:
        # A long line is left whole: the terminal breaks it where its window ends.
        console.print(_shown_safely(line), soft_wrap=True)
    return sent.getvalue()


def _shown_safely(line: Text) -> Text:
    """Return a line that a terminal shows and does not act on, with its colours where they were."""
    safe = harmless(line.plain)
    if safe == line.plain:
        return line
    shown = line.copy()
    # The text keeps its length, so every colour stays on the characters that it was on.
    shown.plain = safe
    return shown


def _took(milliseconds: int) -> str:
    if milliseconds < 1000:
        return f"{milliseconds} ms"
    if milliseconds < 60_000:
        return f"{milliseconds / 1000:.1f} s"
    minutes, rest = divmod(milliseconds // 1000, 60)
    return f"{minutes} min {rest} s"


def _counted(number: int, what: str) -> str:
    return f"{number} {what}" if number == 1 else f"{number} {what}s"


def fit(written: str, style: str = "") -> Text:
    """Return text from elsewhere as a line that is fit for a terminal.

    The library that keeps the colours takes some control characters out of
    a text by itself, and leaves no trace of them: the bell, the backspace,
    the carriage return, and two that move the paper on. A backspace that
    is gone leaves two words standing as one. So what a terminal would act
    on is replaced before the library is given the text, each character by
    one that is shown, and the text keeps its length.
    """
    return Text(harmless(written), style=style)


def _note(text: str, style: str = _NOTE) -> Text:
    return fit(text, style)


def _beginning(lines: list[str], at_most: int, before: str, style: str = "") -> list[Text]:
    """Return the first lines of many, each after ``before``, and a word about the rest."""
    shown = [fit(f"{before}{line}", style) for line in lines[:at_most]]
    if len(lines) > at_most:
        rest = _counted(len(lines) - at_most, "more line")
        shown.append(Text(f"{before}... {rest}", style=style or _NOTE))
    return shown


def _ending(lines: list[str], at_most: int, before: str = "") -> list[Text]:
    """Return the last lines of many, each after ``before``, and a word about those before them."""
    shown: list[Text] = []
    if len(lines) > at_most:
        earlier = _counted(len(lines) - at_most, "line")
        shown.append(_note(f"{before}... {earlier} before these"))
    shown.extend(fit(f"{before}{line}") for line in lines[-at_most:])
    return shown


def _one_line(text: str) -> str:
    """Return a text on one line and not too long, for the line that tells of a call."""
    joined = " ".join(text.split())
    if len(joined) > _SUMMARY_CHARACTERS:
        return joined[: _SUMMARY_CHARACTERS - 3] + "..."
    return joined


def _read(written: str) -> dict[str, object] | None:
    """Read the arguments of a call, or return ``None`` if they are not an object in JSON."""
    try:
        read: object = json.loads(written)
    except ValueError:
        return None
    return cast("dict[str, object]", read) if isinstance(read, dict) else None


def _acted_on(arguments: dict[str, object]) -> str:
    """Say in a few words what a call acts on, going by the names of its arguments."""
    for named in _NAMED_FIRST:
        value = arguments.get(named)
        if isinstance(value, str):
            return value
    texts = [value for value in arguments.values() if isinstance(value, str)]
    if len(texts) == 1:
        return texts[0]
    return json.dumps(arguments, ensure_ascii=False) if arguments else ""


def _end(data: TurnEndEvent) -> list[Text]:
    """Say how a turn ended, unless it ended the way turns are meant to."""
    detail = data.detail
    if data.reason == "aborted":
        told = "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"
        return [_note(told)]
    if data.reason == "error":
        return [_note(f"(the turn ended with an error: {detail})", _BAD)]
    if data.reason == "max-tokens":
        return [_note("(the reply was cut off: the model reached its limit of output)")]
    if data.reason == "blocked":
        return [_note(f"(the turn was not allowed to go on: {detail})")]
    if data.reason == "interrupted":
        return [_note("(the turn was cut short when the program ended)")]
    return []


class Transcript:
    """The lines that a conversation is shown as, made as the conversation goes on."""

    __slots__ = ("_began", "_commands", "_lines", "_shown", "_symbols", "_word")

    def __init__(self, symbols: Symbols) -> None:
        self._symbols = symbols
        self._lines = Lines()
        # Whether text of the reply that is being written was shown from its chunks.
        self._shown = False
        # What the model is doing while no text of it arrives.
        self._word: str | None = None
        # When each call was written down, and which calls run a command.
        self._began: dict[str, int] = {}
        self._commands: set[str] = set()

    @property
    def partial(self) -> Text:
        """The line of the reply that is not complete yet, or a word about what the model does."""
        if self._lines.partial:
            return fit(self._lines.partial)
        return Text() if self._word is None else _note(self._word)

    def chunk(self, chunk: Chunk) -> list[Text]:
        """Take a chunk of the reply that is being written, and return the lines it completes."""
        if isinstance(chunk, TextDelta):
            self._word = None
            self._shown = True
            return [fit(line) for line in self._lines.feed(chunk.text)]
        if isinstance(chunk, BlockStart):
            if chunk.kind == "reasoning":
                self._word = "thinking..."
            elif chunk.kind == "tool-call":
                self._word = f"preparing a call of {chunk.name}..."
            else:
                self._word = None
            return []
        if isinstance(chunk, BlockEnd):
            self._word = None
            return self._close()
        return []

    def event(self, event: SessionEvent[EventData]) -> list[Text]:
        """Take an event of the session and return the lines that show it."""
        data = event.data
        if isinstance(data, AssistantMessageEvent | AssistantAttemptEvent):
            shown = self._close()
            if not self._shown:
                shown.extend(fit(line) for line in data.message.text.splitlines())
            self._shown = False
            self._word = None
            return shown
        if isinstance(data, ToolCallEvent):
            return self._call(event.time, data)
        if isinstance(data, ToolResultEvent):
            return self._result(event.time, data)
        if isinstance(data, UserMessageEvent):
            if data.source != "steer":
                return []
            return [fit(f"{self._symbols.steer} {line}") for line in _said(data).splitlines()]
        if isinstance(data, TurnEndEvent):
            # An empty line sets the turn apart from what comes after it.
            return [*_end(data), Text()]
        if isinstance(data, RetryEvent):
            wait = _took(data.delay_ms)
            return [_note(f"(the request failed: {data.code}. Trying again in {wait})")]
        if isinstance(data, ApprovalDecidedEvent) and data.call_id in self._began:
            # The time that somebody took to answer is not the time that the call took.
            self._began[data.call_id] = event.time
        return []

    def _close(self) -> list[Text]:
        """Hand over the line that is not complete, if there is one."""
        last = self._lines.flush()
        return [] if last is None else [fit(last)]

    def _call(self, time: int, data: ToolCallEvent) -> list[Text]:
        self._began[data.call_id] = time
        arguments = _read(data.arguments)
        line = Text()
        line.append(f"{self._symbols.call} ", style=_MARK)
        line.append(harmless(data.name), style=_NAME)
        told = _one_line(data.arguments if arguments is None else _acted_on(arguments))
        if told:
            line.append(harmless(f" {told}"))
        shown = [line]
        if arguments is None:
            return shown
        if isinstance(arguments.get("command"), str):
            self._commands.add(data.call_id)
        content = arguments.get("content")
        if isinstance(content, str):
            line.append(f" ({_counted(len(content.splitlines()), 'line')})", style=_NOTE)
        old, new = arguments.get("old"), arguments.get("new")
        if isinstance(old, str) and isinstance(new, str):
            shown.extend(_beginning(old.splitlines(), _CHANGED_LINES, "    - ", _GOES))
            shown.extend(_beginning(new.splitlines(), _CHANGED_LINES, "    + ", _COMES))
        return shown

    def _result(self, time: int, data: ToolResultEvent) -> list[Text]:
        call_id = data.message.call_id
        began = self._began.pop(call_id, None)
        command = call_id in self._commands
        self._commands.discard(call_id)
        lines = "\n".join(block.text for block in data.message.content).splitlines()
        took = [] if began is None else [_took(time - began)]
        head = Text("  ")
        if data.error is not None:
            failed = ", ".join([f"{self._symbols.failed} {data.error.code}", *took])
            head.append(harmless(failed), style=_BAD)
            return [head, *_beginning(lines, _ERROR_LINES, "    ")]
        if lines and not command:
            took.append(_counted(len(lines), "line"))
        head.append(" ".join([self._symbols.done, ", ".join(took)]).rstrip(), style=_NOTE)
        if not command:
            return [head]
        # What went wrong and the exit code stand at the end of what a command says.
        return [head, *_ending(lines, _OUTPUT_LINES, "    ")]


def _said(data: UserMessageEvent) -> str:
    return "".join(block.text for block in data.message.content)


def recap(events: Sequence[SessionEvent[EventData]], symbols: Symbols) -> list[Text]:
    """Return what is shown of a session that is taken up again: its last turn, in short.

    That is what the user said in the turn, the end of the last thing the
    model answered, and how the turn ended if it did not end well. A session
    in which no turn has begun shows nothing.
    """
    starts = [
        position for position, event in enumerate(events) if isinstance(event.data, TurnStartEvent)
    ]
    if not starts:
        return []
    shown = [_note("(this session is taken up again. Its last turn:)")]
    answer: list[str] = []
    ended: list[Text] = []
    for event in events[starts[-1] :]:
        data = event.data
        if isinstance(data, UserMessageEvent) and data.source in ("user", "steer"):
            mark = ">" if data.source == "user" else symbols.steer
            shown.extend(_note(f"{mark} {line}") for line in _said(data).splitlines())
        elif isinstance(data, AssistantMessageEvent) and data.message.text.strip():
            answer = data.message.text.splitlines()
        elif isinstance(data, TurnEndEvent):
            ended = _end(data)
    return [*shown, *_ending(answer, _RECAP_LINES), *ended, Text()]
