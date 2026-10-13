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

A conversation that is made shorter says so: why it is, when it begins, and
what came of it, when it is over. Where the user asked for it, the command
that was given says what came of it, and nothing is said here a second time.

A task that is handed to a subagent is a call like any other, and what the
subagent does is not shown. Two lines say that it has begun, with the
session that holds its work, and how it ended.

A hook of the user is a command that may take its time. While one runs, a
word stands where a reply would grow and says which. What a hook stops is
told where that takes effect, and once: a call that it refused fails with its
reason, and a turn whose input it stopped ends with it. Only a hook that
keeps a turn going when the model was through would leave no trace, so that
is said in lines of its own, with the reason that the model is given.

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

from columbina.agent import CUT_OFF, replies
from columbina.approval import ApprovalDecidedEvent
from columbina.apps.cli.lines import Lines
from columbina.apps.cli.terminal import Terminal
from columbina.apps.common import how_it_ended
from columbina.apps.terminal_text import harmless
from columbina.commands.compact import told as what_came_of_it
from columbina.compaction import CompactionEndEvent, CompactionStartEvent, Why
from columbina.hooks import HookInvokedEvent, HookResultEvent
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
from columbina.subagents import SubagentEndedEvent, SubagentStartedEvent
from columbina.todo import TOOL as TODO_TOOL
from columbina.todo import TodoWriteEvent
from columbina.todo import shown as listed

__all__ = ["Symbols", "Transcript", "fit", "on_screen", "recap"]

_NOTE = "dim"
_BAD = "red"
_NAME = "bold"
_MARK = "cyan"
_DOING = "bold"
_GOES = "red"
_COMES = "green"

_SUMMARY_CHARACTERS = 100
_CHANGED_LINES = 6
_OUTPUT_LINES = 5
_ERROR_LINES = 3
_REASON_LINES = 3
_RECAP_LINES = 12
_NAMED_FIRST = ("command", "path", "pattern", "description")
"""The arguments that say what a call acts on or is about, the most telling one first."""

_SHORTER = "making the conversation shorter..."
"""What stands where a reply would grow, for as long as a conversation is being made shorter."""

_KEPT_GOING = "hook/Stop"
"""Where the words come from that a hook gives a model for going on with a turn."""
_GOES_ON = "(the reply was cut off at the limit of its length. The model goes on.)"
"""What stands where a reply was cut off and the model was told to go on from it."""

_WHY_SHORTER: dict[Why, str | None] = {
    "pressure": "(the conversation has come near what the model takes. Making it shorter.)",
    "overflow": "(the model refused the conversation as too long. Making it shorter.)",
    # The user has just asked for it.
    "manual": None,
}


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
    told = how_it_ended(data)
    if told is None:
        return []
    return [_note(told, _BAD if data.reason == "error" else _NOTE)]


class Transcript:
    """The lines that a conversation is shown as, made as the conversation goes on."""

    __slots__ = (
        "_began",
        "_commands",
        "_handed",
        "_hooks",
        "_lines",
        "_shown",
        "_symbols",
        "_word",
    )

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
        # What each task is about that a subagent works on, by its call.
        self._handed: dict[str, str] = {}
        # The hooks that run, each by its moment and its command, the one that began last last.
        self._hooks: list[tuple[str, str]] = []

    @property
    def partial(self) -> Text:
        """The line of the reply that is not complete yet, or a word about what the model does."""
        if self._lines.partial:
            return fit(self._lines.partial)
        if self._hooks:
            moment, command = self._hooks[-1]
            return _note(f"running a {moment} hook ({_one_line(command)})...")
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
        """Take an event of the session and return the lines that show it.

        An event that takes the place of others shows nothing. What it stands
        for was shown when it happened: a result that is cut down later, to
        make the conversation shorter, is not a call that ends a second time.
        """
        if event.replaces:
            return []
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
            if data.source == _KEPT_GOING:
                return _kept_going(data)
            if data.source == CUT_OFF:
                # What follows is the same answer. The line says why it stopped in mid-air.
                return [_note(_GOES_ON)]
            if data.source != "steer":
                return []
            return [fit(f"{self._symbols.steer} {line}") for line in _said(data).splitlines()]
        if isinstance(data, TurnEndEvent):
            # No hook outlasts its turn. One whose end was never written is not waited for.
            self._hooks.clear()
            # An empty line sets the turn apart from what comes after it.
            return [*_end(data), Text()]
        if isinstance(data, RetryEvent):
            wait = _took(data.delay_ms)
            return [_note(f"(the request failed: {data.code}. Trying again in {wait})")]
        if isinstance(data, TodoWriteEvent):
            return _things(data)
        if isinstance(data, CompactionStartEvent):
            self._word = _SHORTER
            why = _WHY_SHORTER[data.why]
            return [] if why is None else [_note(why)]
        if isinstance(data, CompactionEndEvent):
            self._word = None
            if data.why == "manual":
                # The command that asked for it says what came of it.
                return []
            came = what_came_of_it(data.outcome)
            return [_note(f"({came[0].lower()}{came[1:]})")]
        if isinstance(data, SubagentStartedEvent):
            return self._handed_over(data)
        if isinstance(data, SubagentEndedEvent):
            return self._handed_back(data)
        if isinstance(data, HookInvokedEvent):
            self._hooks.append((data.event, data.command))
            return []
        if isinstance(data, HookResultEvent):
            if (data.event, data.command) in self._hooks:
                self._hooks.remove((data.event, data.command))
            return []
        if isinstance(data, ApprovalDecidedEvent) and data.call_id in self._began:
            # The time that somebody took to answer is not the time that the call took.
            self._began[data.call_id] = event.time
        return []

    def _handed_over(self, data: SubagentStartedEvent) -> list[Text]:
        about = _one_line(data.description)
        self._handed[data.call_id] = about
        who = "a subagent" if data.kind is None else f"a subagent of the kind {data.kind}"
        on = f' works on "{about}"' if about else " works"
        return [_note(f"    ({who}{on} in the session {data.session_id})")]

    def _handed_back(self, data: SubagentEndedEvent) -> list[Text]:
        # Several subagents may be at work at once, so each end says whose it is.
        about = self._handed.pop(data.call_id, "")
        who = f'the subagent for "{about}"' if about else "a subagent"
        steps = _counted(data.steps, "step")
        if data.reason == "completed":
            return [_note(f"    ({who} is through, after {steps})")]
        return [_note(f"    ({who} did not finish: {data.reason}, after {steps})")]

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
        if data.name == TODO_TOOL and arguments is not None:
            # The list that the call writes is shown when it is written, thing by thing.
            return [line]
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


def _things(data: TodoWriteEvent) -> list[Text]:
    """Show the list of things to do as the model has just written it."""
    if not data.items:
        return [_note("    (the list of things to do is empty)")]
    styles = {"pending": "", "in_progress": _DOING, "completed": _NOTE}
    return [
        fit(f"    {line}", styles[item.status])
        for item, line in zip(data.items, listed(data.items), strict=True)
    ]


def _said(data: UserMessageEvent) -> str:
    return "".join(block.text for block in data.message.content)


def _kept_going(data: UserMessageEvent) -> list[Text]:
    """Say that a hook keeps a turn going, with the beginning of the reason that it gave."""
    lines = _said(data).strip().splitlines()
    if not lines:
        return [_note("(a Stop hook keeps the turn going.)")]
    head = _note("(a Stop hook keeps the turn going. It says:)")
    return [head, *_beginning(lines, _REASON_LINES, "    ")]


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
    ended: list[Text] = []
    for event in events[starts[-1] :]:
        data = event.data
        if isinstance(data, UserMessageEvent) and data.source in ("user", "steer"):
            mark = ">" if data.source == "user" else symbols.steer
            shown.extend(_note(f"{mark} {line}") for line in _said(data).splitlines())
        elif isinstance(data, TurnEndEvent):
            ended = _end(data)
    # A reply that was cut off and taken up again is one answer with what went on from it.
    last = events[starts[-1] :]
    answers = [text for text in replies(event.data for event in last) if text.strip()]
    answer = answers[-1].splitlines() if answers else []
    return [*shown, *_ending(answer, _RECAP_LINES), *ended, Text()]
