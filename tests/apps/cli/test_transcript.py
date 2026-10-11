"""What a conversation looks like on the screen: the events of a session, as lines of text."""

import itertools
import json
from collections.abc import Sequence
from typing import Any

import pytest
from rich.text import Text

import columbina.approval
import columbina.retry
import columbina.session
from columbina.approval import ApprovalDecidedEvent
from columbina.apps.cli.transcript import Symbols, Transcript, on_screen, recap
from columbina.compaction import EVENTS as COMPACTION_EVENTS
from columbina.compaction import CompactionEndEvent, CompactionStartEvent, Outcome
from columbina.hooks import HookInvokedEvent, HookResultEvent
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    BlockEnd,
    BlockStart,
    Finish,
    LlmFailure,
    ReasoningBlock,
    ReasoningDelta,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    ToolCallDelta,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.retry import RetryEvent
from columbina.session import (
    SESSIONS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    Session,
    StepStartEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.subagents import SubagentEndedEvent, SubagentStartedEvent
from columbina.todo import TOOL as TODO_TOOL
from columbina.todo import Item, TodoWriteEvent

PLAIN = Symbols(call="*", done="ok", failed="failed", steer=">>")
STAND_IN = chr(0xFFFD)
OVERRIDE, POP = chr(0x202E), chr(0x202C)


@pytest.fixture
async def log(ctx: Context) -> Session:
    """A session in memory whose clock goes on by a quarter of a second with every event."""
    await ctx.plugin(columbina.session)
    await ctx.plugin(columbina.retry)
    await ctx.plugin(columbina.approval)
    sessions = ctx.use(SESSIONS)
    ticks = itertools.count(1_000_000, 250)
    sessions.clock = lambda: next(ticks)
    return sessions.create(ctx, ".", session_id="s-1")


def plain(texts: Sequence[Text]) -> list[str]:
    return [text.plain for text in texts]


def show(transcript: Transcript, log: Session, *happened: EventData) -> list[str]:
    """Write events to the log and return what the transcript shows for them."""
    shown: list[str] = []
    for data in happened:
        shown.extend(plain(transcript.event(log.append(data))))
    return shown


def said(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


def reply(*blocks: Any, finish: Any = "stop") -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=AssistantMessage(content=tuple(blocks)), usage=Usage(), finish_reason=finish
    )


def call(name: str, call_id: str = "c-1", **arguments: Any) -> ToolCallEvent:
    return ToolCallEvent(call_id=call_id, name=name, arguments=json.dumps(arguments))


def result(text: str, call_id: str = "c-1", error: str | None = None) -> ToolResultEvent:
    message = ToolMessage(
        call_id=call_id, content=(TextBlock(text=text),), is_error=error is not None
    )
    failed = None if error is None else ToolError(code=error, reason=text)
    return ToolResultEvent(message=message, error=failed)


# A reply that is being written


def test_the_text_of_a_reply_is_shown_line_by_line_as_it_arrives() -> None:
    transcript = Transcript(PLAIN)

    first = transcript.chunk(TextDelta(index=0, text="The file "))
    second = transcript.chunk(TextDelta(index=0, text="says 42.\nIt is "))

    assert plain(first) == []
    assert plain(second) == ["The file says 42."]
    assert transcript.partial.plain == "It is "


def test_the_line_that_is_not_complete_is_shown_when_its_block_ends() -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(TextDelta(index=0, text="One.\nTwo"))

    shown = transcript.chunk(BlockEnd(index=0))

    assert plain(shown) == ["Two"]
    assert transcript.partial.plain == ""


def test_while_the_model_thinks_or_writes_a_call_a_word_says_so() -> None:
    transcript = Transcript(PLAIN)

    transcript.chunk(BlockStart(index=0, kind="reasoning"))
    thinking = transcript.partial
    transcript.chunk(ReasoningDelta(index=0, text="Two and two."))
    still = transcript.partial.plain
    transcript.chunk(BlockEnd(index=0))
    between = transcript.partial.plain
    transcript.chunk(BlockStart(index=1, kind="tool-call", call_id="c-1", name="write"))
    transcript.chunk(ToolCallDelta(index=1, arguments='{"path": "a'))
    calling = transcript.partial.plain

    assert thinking.plain == "thinking..."
    assert thinking.style == "dim"
    assert (still, between) == ("thinking...", "")
    assert calling == "preparing a call of write..."


def test_text_takes_the_place_of_the_word_about_thinking() -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(BlockStart(index=0, kind="reasoning"))

    transcript.chunk(BlockStart(index=1, kind="text"))
    transcript.chunk(TextDelta(index=1, text="Four"))

    assert transcript.partial.plain == "Four"
    assert transcript.partial.style == ""


def test_text_takes_the_place_of_the_word_also_where_no_block_was_begun_for_it() -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(BlockStart(index=0, kind="reasoning"))

    shown = transcript.chunk(TextDelta(index=1, text="A line.\n"))

    # The line is complete and handed over. Nothing says any more that the model thinks.
    assert plain(shown) == ["A line."]
    assert transcript.partial.plain == ""


def test_a_line_that_is_not_complete_stays_in_front_of_a_word_about_what_comes_next() -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(TextDelta(index=0, text="Half a li"))

    transcript.chunk(BlockStart(index=1, kind="tool-call", call_id="c-1", name="read"))

    assert transcript.partial.plain == "Half a li"


def test_chunks_that_show_nothing_change_nothing() -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(TextDelta(index=0, text="Half"))

    assert transcript.chunk(Finish(reason="stop")) == []
    assert transcript.partial.plain == "Half"


async def test_a_reply_that_was_shown_as_it_arrived_is_not_shown_again(log: Session) -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(TextDelta(index=0, text="One.\nTwo"))

    shown = show(transcript, log, reply(TextBlock(text="One.\nTwo")))

    assert shown == ["Two"]
    assert transcript.partial.plain == ""


async def test_a_reply_that_did_not_arrive_in_pieces_is_shown_whole(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        reply(
            ReasoningBlock(text="Not shown."),
            TextBlock(text="One.\n\nTwo."),
            ToolCallBlock(call_id="c-1", name="read", arguments="{}"),
            finish="tool-calls",
        ),
    )

    assert shown == ["One.", "", "Two."]


async def test_each_reply_is_looked_at_by_itself(log: Session) -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(TextDelta(index=0, text="First.\n"))
    show(transcript, log, reply(TextBlock(text="First.\n")))

    shown = show(transcript, log, reply(TextBlock(text="Second.")))

    assert shown == ["Second."]


async def test_what_arrived_of_a_reply_that_failed_stays_and_is_closed(log: Session) -> None:
    transcript = Transcript(PLAIN)
    transcript.chunk(BlockStart(index=0, kind="reasoning"))
    transcript.chunk(TextDelta(index=1, text="The answer is"))
    failed = AssistantAttemptEvent(
        message=AssistantMessage(content=(TextBlock(text="The answer is"),)),
        usage=Usage(),
        failure=LlmFailure(message="the stream broke", code="STREAM_CLOSED"),
    )

    shown = show(transcript, log, failed)

    assert shown == ["The answer is"]
    assert transcript.partial.plain == ""


# Calls of tools


async def test_a_call_is_one_line_with_the_tool_and_what_it_acts_on(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, call("read", path="notes.txt", limit=20))

    assert shown == ["* read notes.txt"]


@pytest.mark.parametrize(
    ("arguments", "summary"),
    [
        ({"command": "git status", "timeout_seconds": 5}, "git status"),
        ({"pattern": "TODO", "path": "src"}, "src"),
        ({"pattern": "*.py"}, "*.py"),
        ({"query": "what is this"}, "what is this"),
        ({"label": "x", "depth": 2}, "x"),
        ({"depth": 2, "wide": True}, '{"depth": 2, "wide": true}'),
        ({"first": "a", "second": "b"}, '{"first": "a", "second": "b"}'),
        ({}, ""),
        ({"command": "echo one\necho   two"}, "echo one echo two"),
        ({"command": "x" * 200}, "x" * 97 + "..."),
    ],
)
async def test_what_a_call_acts_on_is_told_from_its_arguments(
    log: Session, arguments: dict[str, Any], summary: str
) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, call("tool", **arguments))

    assert shown[0] == f"* tool {summary}".rstrip()


@pytest.mark.parametrize("written", ['{"path": "a', "[1, 2]", '"text"', ""])
async def test_arguments_that_cannot_be_read_are_shown_as_they_are(
    log: Session, written: str
) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, ToolCallEvent(call_id="c-1", name="read", arguments=written))

    assert shown == [f"* read {written}".rstrip()]


async def test_the_list_of_things_to_do_is_shown_when_it_is_written(
    ctx: Context, log: Session
) -> None:
    ctx.use(SESSIONS).declare(ctx, TodoWriteEvent)
    transcript = Transcript(PLAIN)
    things = [
        {"content": "Read the check", "status": "completed"},
        {"content": "Mend calc.py", "status": "in_progress"},
        {"content": "Run the check"},
    ]
    items = (
        Item(content="Read the check", status="completed"),
        Item(content="Mend calc.py", status="in_progress"),
        Item(content="Run the check"),
    )

    # The call is told by its name alone: what it wrote follows, thing by thing.
    assert show(transcript, log, call(TODO_TOOL, items=things)) == ["* todo_write"]
    card = transcript.event(log.append(TodoWriteEvent(call_id="c-1", items=items)))

    assert plain(card) == [
        "    [x] Read the check",
        "    [>] Mend calc.py",
        "    [ ] Run the check",
    ]
    # What is done steps back, and what is being done stands out.
    assert [str(line.style) for line in card] == ["dim", "bold", ""]
    assert show(transcript, log, result("The list holds 3 things.")) == ["  ok 500 ms, 1 line"]


async def test_a_list_that_was_emptied_is_said_to_be_empty(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, TodoWriteEvent)
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, TodoWriteEvent(call_id="c-1", items=()))

    assert shown == ["    (the list of things to do is empty)"]


async def test_a_list_cannot_steer_the_terminal(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, TodoWriteEvent)
    transcript = Transcript(PLAIN)
    items = (Item(content="Ring\x07 the bell\x1b[2J"),)

    shown = show(transcript, log, TodoWriteEvent(call_id="c-1", items=items))

    assert shown == [f"    [ ] Ring{STAND_IN} the bell{STAND_IN}[2J"]


async def test_a_call_for_the_list_that_cannot_be_read_is_shown_as_it_is(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, ToolCallEvent(call_id="c-1", name=TODO_TOOL, arguments="[1"))

    assert shown == ["* todo_write [1"]


async def test_a_call_that_changes_text_shows_what_goes_and_what_comes(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        call(
            "edit",
            path="calc.py",
            old="    return a / (b - 1)",
            new="    return a / b\n    # fixed",
        ),
    )

    assert shown == [
        "* edit calc.py",
        "    -     return a / (b - 1)",
        "    +     return a / b",
        "    +     # fixed",
    ]


async def test_a_long_change_is_shown_by_its_beginning(log: Session) -> None:
    transcript = Transcript(PLAIN)
    old = "\n".join(f"old {number}" for number in range(1, 10))

    shown = show(transcript, log, call("edit", path="a.txt", old=old, new="new"))

    assert shown == [
        "* edit a.txt",
        *(f"    - old {number}" for number in range(1, 7)),
        "    - ... 3 more lines",
        "    + new",
    ]


async def test_a_call_that_writes_a_file_says_how_much(log: Session) -> None:
    transcript = Transcript(PLAIN)

    one = show(transcript, log, call("write", "c-1", path="a.txt", content="one line"))
    more = show(transcript, log, call("write", "c-2", path="b.txt", content="one\ntwo\nthree\n"))
    none = show(transcript, log, call("write", "c-3", path="c.txt", content=""))

    assert one == ["* write a.txt (1 line)"]
    assert more == ["* write b.txt (3 lines)"]
    assert none == ["* write c.txt (0 lines)"]


# Results


async def test_a_result_says_how_long_the_call_took_and_how_much_came_back(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript, log, call("read", path="notes.txt"), result("     1\tone\n     2\ttwo")
    )

    assert shown == ["* read notes.txt", "  ok 250 ms, 2 lines"]


@pytest.mark.parametrize(
    ("ticks", "took"),
    [
        (1, "250 ms"),
        (3, "750 ms"),
        (4, "1.0 s"),
        (10, "2.5 s"),
        (240, "1 min 0 s"),
        (250, "1 min 2 s"),
    ],
)
async def test_the_time_of_a_call_is_told_in_the_unit_that_fits(
    log: Session, ticks: int, took: str
) -> None:
    transcript = Transcript(PLAIN)
    show(transcript, log, call("note"))
    for _ in range(ticks - 1):
        log.append(StepStartEvent(turn=1, step=1))

    shown = show(transcript, log, result(""))

    assert shown == [f"  ok {took}"]


async def test_the_time_that_an_answer_took_is_not_counted_as_time_of_the_call(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    show(transcript, log, call("bash", command="make"))
    for _ in range(40):
        log.append(StepStartEvent(turn=1, step=1))

    shown = show(
        transcript,
        log,
        ApprovalDecidedEvent(call_id="c-1", outcome="allowed-once"),
        ApprovalDecidedEvent(call_id="another", outcome="rejected"),
        result("done"),
    )

    assert shown == ["  ok 500 ms", "    done"]


async def test_the_output_of_a_command_is_shown_by_its_end(log: Session) -> None:
    transcript = Transcript(PLAIN)
    output = "\n".join(f"line {number}" for number in range(1, 9)) + "\n[exit code 1]"

    shown = show(transcript, log, call("bash", command="make"), result(output))

    # What went wrong, the summary and the exit code stand at the end of what a command
    # says, so the end is what is shown.
    assert shown == [
        "* bash make",
        "  ok 250 ms",
        "    ... 4 lines before these",
        *(f"    line {number}" for number in range(5, 9)),
        "    [exit code 1]",
    ]


async def test_one_line_too_many_of_the_output_of_a_command_is_told_as_one_line(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    output = "\n".join(f"line {number}" for number in range(1, 6)) + "\n[exit code 0]"

    shown = show(transcript, log, call("bash", command="make"), result(output))

    assert shown[2:4] == ["    ... 1 line before these", "    line 2"]


async def test_a_short_output_of_a_command_is_shown_whole(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, call("bash", command="make"), result("ok\n[exit code 0]"))

    assert shown == ["* bash make", "  ok 250 ms", "    ok", "    [exit code 0]"]


async def test_a_call_that_failed_shows_the_code_and_the_beginning_of_what_was_said(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    text = "The call was not allowed: The rule read(.env) is against it.\nSecond.\nThird.\nFourth."

    shown = show(transcript, log, call("read", path=".env"), result(text, error="DENIED"))

    assert shown == [
        "* read .env",
        "  failed DENIED, 250 ms",
        "    The call was not allowed: The rule read(.env) is against it.",
        "    Second.",
        "    Third.",
        "    ... 1 more line",
    ]


async def test_results_find_their_calls_by_the_id(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        call("bash", "c-1", command="make"),
        call("read", "c-2", path="a.txt"),
        result("one", "c-2"),
        result("done", "c-1"),
    )

    assert shown == [
        "* bash make",
        "* read a.txt",
        "  ok 250 ms, 1 line",
        "  ok 750 ms",
        "    done",
    ]


async def test_a_result_without_its_call_is_shown_without_a_time(log: Session) -> None:
    transcript = Transcript(PLAIN)

    assert show(transcript, log, result("", "nowhere")) == ["  ok"]
    assert show(transcript, log, result("x\ny", "nowhere", error="TOOL_ERROR")) == [
        "  failed TOOL_ERROR",
        "    x",
        "    y",
    ]


async def test_a_result_that_takes_the_place_of_another_is_nothing_new(log: Session) -> None:
    # A long result that the model has read is cut down later, to make the conversation
    # shorter. Shown again, it would look like the same call ending a second time.
    transcript = Transcript(PLAIN)
    show(transcript, log, call("bash", "c-1", command="make"))
    whole = log.append(result("one\ntwo\nthree", "c-1"))
    shown = plain(transcript.event(whole))

    shorter = log.append(result("one\n[...]\nthree", "c-1"), replaces=[whole.seq])

    assert shown == ["  ok 250 ms", "    one", "    two", "    three"]
    assert transcript.event(shorter) == []


# What else happens


async def test_what_the_user_says_is_not_shown_again_but_a_steer_is_when_it_is_read(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        said("Fix the parser."),
        said('<context source="runtime">\nfacts\n</context>', source="context/runtime"),
        said("No, the lexer.\nIt is in lex.py.", source="steer"),
    )

    assert shown == [">> No, the lexer.", ">> It is in lex.py."]


@pytest.mark.parametrize(
    ("end", "shown"),
    [
        (TurnEndEvent(turn=1, reason="completed"), []),
        (TurnEndEvent(turn=1, reason="aborted", detail="user"), ["(stopped)"]),
        (TurnEndEvent(turn=1, reason="aborted", detail="budget"), ["(stopped: budget)"]),
        (TurnEndEvent(turn=1, reason="aborted"), ["(stopped)"]),
        (
            TurnEndEvent(turn=1, reason="error", detail="AUTH: the key is not valid"),
            ["(the turn ended with an error: AUTH: the key is not valid)"],
        ),
        (
            TurnEndEvent(turn=1, reason="max-tokens"),
            ["(the reply was cut off: the model reached its limit of output)"],
        ),
        (
            TurnEndEvent(turn=1, reason="blocked", detail="the budget is spent"),
            ["(the turn was not allowed to go on: the budget is spent)"],
        ),
        (
            TurnEndEvent(turn=1, reason="interrupted"),
            ["(the turn was cut short when the program ended)"],
        ),
        (
            TurnEndEvent(turn=1, reason="forked"),
            ["(the turn was cut short where its session was copied)"],
        ),
    ],
)
async def test_a_turn_that_did_not_end_well_says_how_it_ended(
    log: Session, end: TurnEndEvent, shown: list[str]
) -> None:
    # The empty line sets the turn apart from what comes after it.
    assert show(Transcript(PLAIN), log, end) == [*shown, ""]


async def test_an_error_at_the_end_of_a_turn_stands_out(log: Session) -> None:
    transcript = Transcript(PLAIN)

    failed, _ = transcript.event(log.append(TurnEndEvent(turn=1, reason="error", detail="AUTH")))
    stopped, _ = transcript.event(log.append(TurnEndEvent(turn=2, reason="aborted")))

    assert (str(failed.style), str(stopped.style)) == ("red", "dim")


async def test_a_request_that_is_made_again_is_told(log: Session) -> None:
    transcript = Transcript(PLAIN)

    soon = show(transcript, log, RetryEvent(turn=1, step=1, attempt=1, code="SERVER", delay_ms=800))
    later = show(
        transcript, log, RetryEvent(turn=1, step=1, attempt=2, code="RATE_LIMIT", delay_ms=4000)
    )

    assert soon == ["(the request failed: SERVER. Trying again in 800 ms)"]
    assert later == ["(the request failed: RATE_LIMIT. Trying again in 4.0 s)"]


async def test_a_conversation_that_is_made_shorter_says_why_and_what_came_of_it(
    ctx: Context, log: Session
) -> None:
    ctx.use(SESSIONS).declare(ctx, *COMPACTION_EVENTS)
    transcript = Transcript(PLAIN)

    begun = show(transcript, log, CompactionStartEvent(why="pressure", tokens=15_200))
    # While it goes on, a word stands where a reply would grow.
    meanwhile = transcript.partial.plain
    over = show(
        transcript,
        log,
        CompactionEndEvent(why="pressure", outcome=Outcome(trimmed=2, before=15_200, after=6100)),
    )

    assert begun == ["(the conversation has come near what the model takes. Making it shorter.)"]
    assert meanwhile == "making the conversation shorter..."
    assert over == [
        "(the conversation was made shorter: 2 long results of tools were cut down to the "
        "beginning and the end. It came to about 15200 tokens and comes to about 6100 now.)"
    ]
    assert transcript.partial.plain == ""


async def test_a_conversation_that_the_model_refused_says_so(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, *COMPACTION_EVENTS)
    transcript = Transcript(PLAIN)
    failed = Outcome(before=900, after=900, error="the model wrote no summary")

    shown = show(
        transcript,
        log,
        CompactionStartEvent(why="overflow", tokens=900),
        CompactionEndEvent(why="overflow", outcome=failed),
    )

    assert shown == [
        "(the model refused the conversation as too long. Making it shorter.)",
        "(the conversation is as it was: the model wrote no summary. It comes to about 900 "
        "tokens.)",
    ]


async def test_what_the_user_asked_for_is_told_by_the_command_and_not_a_second_time(
    ctx: Context, log: Session
) -> None:
    ctx.use(SESSIONS).declare(ctx, *COMPACTION_EVENTS)
    transcript = Transcript(PLAIN)

    begun = show(transcript, log, CompactionStartEvent(why="manual", tokens=900))
    meanwhile = transcript.partial.plain
    over = show(
        transcript,
        log,
        CompactionEndEvent(why="manual", outcome=Outcome(summarised=4, before=900, after=300)),
    )

    assert (begun, over) == ([], [])
    assert meanwhile == "making the conversation shorter..."
    assert transcript.partial.plain == ""


async def test_the_bookkeeping_of_a_turn_is_not_shown(log: Session) -> None:
    transcript = Transcript(PLAIN)

    assert show(transcript, log, TurnStartEvent(turn=1), StepStartEvent(turn=1, step=1)) == []


# Looks


def test_symbols_are_chosen_by_what_the_screen_can_show() -> None:
    from support.terminal import desk

    with desk(encoding="utf-8") as wide, desk(encoding="cp1252") as narrow:
        assert Symbols.of(wide.terminal) == Symbols(
            call=chr(0x25CF), done=chr(0x2713), failed=chr(0x2717), steer=chr(0xBB)
        )
        assert Symbols.of(narrow.terminal) == Symbols(
            call="*", done="ok", failed="failed", steer=chr(0xBB)
        )


async def test_what_is_shown_has_colours_only_where_they_are_wanted(log: Session) -> None:
    transcript = Transcript(PLAIN)
    card = transcript.event(log.append(call("edit", path="a.txt", old="x", new="y")))

    coloured = on_screen(card, colours=True)
    bare = on_screen(card, colours=False)

    assert bare == "* edit a.txt\n    - x\n    + y\n"
    assert "\x1b[" in coloured
    assert "\x1b[" not in bare


def test_text_of_the_model_is_never_read_as_markup() -> None:
    transcript = Transcript(PLAIN)

    lines = transcript.chunk(TextDelta(index=0, text="[bold]not bold[/bold] :smile:\n"))

    assert on_screen(lines, colours=False) == "[bold]not bold[/bold] :smile:\n"


def test_what_a_model_or_a_tool_wrote_cannot_steer_the_terminal() -> None:
    transcript = Transcript(PLAIN)
    red, clear, title = "\x1b[31m", "\x01\x1b[2J\x02", "\x1b]2;mine\x07"
    hostile = f"a{red}b{clear}c{title}d\x9b2Je\x00f"

    lines = transcript.chunk(TextDelta(index=0, text=hostile + "\n"))
    sent = on_screen(lines, colours=True)

    # The line has no colour of its own, so nothing that begins a sequence may be sent at all.
    # What would have steered the terminal is shown as characters, the bell too.
    assert "\x1b" not in sent
    assert sent == (
        f"a{STAND_IN}[31mb{STAND_IN}{STAND_IN}[2J{STAND_IN}c{STAND_IN}]2;mine{STAND_IN}d"
        f"{STAND_IN}2Je{STAND_IN}f\n"
    )


DROPPED = [0x07, 0x08, 0x0B, 0x0C, 0x0D]
"""Control characters that the library which keeps the colours takes out of a text by itself."""


@pytest.mark.parametrize("code", DROPPED)
def test_no_character_goes_missing_from_a_reply_that_is_being_written(code: int) -> None:
    # A backspace that is taken out leaves two words standing as one. Whatever a terminal
    # would act on is shown in its place, so that nobody is shown less than was written.
    transcript = Transcript(PLAIN)
    odd = chr(code)

    lines = transcript.chunk(TextDelta(index=0, text=f"pass{odd}{odd}fail\nmo{odd}re"))

    assert on_screen(lines, colours=False) == f"pass{STAND_IN}{STAND_IN}fail\n"
    assert on_screen([transcript.partial], colours=False) == f"mo{STAND_IN}re\n"
    assert on_screen(transcript.chunk(BlockEnd(index=0)), colours=False) == f"mo{STAND_IN}re\n"


@pytest.mark.parametrize("code", [0x07, 0x08])
async def test_no_character_goes_missing_from_what_the_log_holds(log: Session, code: int) -> None:
    # The other three break a line where a text is taken apart into its lines.
    transcript = Transcript(PLAIN)
    odd = chr(code)

    shown = [
        *transcript.event(log.append(reply(TextBlock(text=f"a{odd}b")))),
        *transcript.event(log.append(call(f"re{odd}ad", path=f"x{odd}y"))),
        *transcript.event(log.append(result(f"no{odd}pe", error=f"BAD{odd}CODE"))),
        *transcript.event(log.append(said(f"st{odd}eer", source="steer"))),
        *transcript.event(log.append(TurnEndEvent(turn=1, reason="error", detail=f"o{odd}h"))),
    ]
    sent = on_screen(shown, colours=False).splitlines()

    assert sent[0] == f"a{STAND_IN}b"
    assert sent[1] == f"* re{STAND_IN}ad x{STAND_IN}y"
    assert sent[2].startswith(f"  failed BAD{STAND_IN}CODE")
    assert sent[3] == f"    no{STAND_IN}pe"
    assert sent[4] == f">> st{STAND_IN}eer"
    assert sent[5] == f"(the turn ended with an error: o{STAND_IN}h)"


@pytest.mark.parametrize("code", [0x07, 0x08])
async def test_no_character_goes_missing_from_the_last_turn_of_a_session_that_is_taken_up(
    log: Session, code: int
) -> None:
    odd = chr(code)
    for data in (
        TurnStartEvent(turn=1),
        said(f"he{odd}llo"),
        reply(TextBlock(text=f"wo{odd}rld")),
        TurnEndEvent(turn=1, reason="completed"),
    ):
        log.append(data)

    sent = on_screen(recap(log.events, PLAIN), colours=False).splitlines()

    assert sent[1:3] == [f"> he{STAND_IN}llo", f"wo{STAND_IN}rld"]


async def test_the_output_of_a_command_cannot_steer_the_terminal(log: Session) -> None:
    transcript = Transcript(PLAIN)
    show(transcript, log, call("bash", command="cat notes.txt"))

    lines = transcript.event(log.append(result("first\n\x1b[1A\x1b[2Kforged\n[exit code 0]")))
    sent = on_screen(lines, colours=False)

    assert sent.splitlines()[1:] == [
        "    first",
        f"    {STAND_IN}[1A{STAND_IN}[2Kforged",
        "    [exit code 0]",
    ]


def test_text_that_turns_its_direction_around_is_shown_in_the_order_it_has() -> None:
    line = Text(f"git push {OVERRIDE}gro.live{POP} --force")

    assert on_screen([line], colours=False) == f"git push {STAND_IN}gro.live{STAND_IN} --force\n"


async def test_a_line_keeps_its_colours_where_something_was_replaced(log: Session) -> None:
    transcript = Transcript(PLAIN)
    card = transcript.event(log.append(call("read", path="a\x1b.txt")))

    coloured = on_screen(card, colours=True)

    # The mark and the name of the tool are coloured as on any other line.
    usual = on_screen(
        transcript.event(log.append(call("read", "c-2", path="a?.txt"))), colours=True
    )
    assert coloured == usual.replace("a?.txt", f"a{STAND_IN}.txt")
    # The line is made fit where it is put together, before anything else is done with it.
    assert card[0].plain == f"* read a{STAND_IN}.txt"


def test_a_line_that_was_put_together_elsewhere_keeps_its_colours_too() -> None:
    # Whoever shows a line of their own gets the same care, on the way to the screen.
    line = Text("* ", style="cyan")
    line.append("read", style="bold")
    line.append(" a\x1b.txt")
    usual = Text("* ", style="cyan")
    usual.append("read", style="bold")
    usual.append(" a?.txt")

    coloured = on_screen([line], colours=True)

    assert coloured == on_screen([usual], colours=True).replace("a?.txt", f"a{STAND_IN}.txt")
    assert line.plain == "* read a\x1b.txt"


def test_a_long_line_is_left_to_the_screen_to_break() -> None:
    transcript = Transcript(PLAIN)
    long = "word " * 60

    lines = transcript.chunk(TextDelta(index=0, text=long + "\n"))

    assert on_screen(lines, colours=False) == long + "\n"


def test_nothing_to_show_is_no_text_at_all() -> None:
    assert on_screen([], colours=True) == ""


# A session that is taken up again


async def test_a_session_that_is_taken_up_again_shows_its_last_turn_in_short(log: Session) -> None:
    for data in (
        TurnStartEvent(turn=1),
        said("An earlier task."),
        reply(TextBlock(text="An earlier answer.")),
        TurnEndEvent(turn=1, reason="completed"),
        TurnStartEvent(turn=2),
        said("<context>facts</context>", source="context/runtime"),
        said("Fix the parser.\nIt fails on empty input."),
        reply(TextBlock(text="I will look."), finish="tool-calls"),
        said("And the lexer.", source="steer"),
        reply(TextBlock(text="Both are fixed.\nThe tests pass.")),
        TurnEndEvent(turn=2, reason="completed"),
    ):
        log.append(data)

    shown = plain(recap(log.events, PLAIN))

    assert shown == [
        "(this session is taken up again. Its last turn:)",
        "> Fix the parser.",
        "> It fails on empty input.",
        ">> And the lexer.",
        "Both are fixed.",
        "The tests pass.",
        "",
    ]


async def test_a_long_last_answer_is_shown_by_its_end(log: Session) -> None:
    answer = "\n".join(f"line {number}" for number in range(1, 31))
    for data in (TurnStartEvent(turn=1), said("Count."), reply(TextBlock(text=answer))):
        log.append(data)

    shown = plain(recap(log.events, PLAIN))

    assert shown == [
        "(this session is taken up again. Its last turn:)",
        "> Count.",
        "... 18 lines before these",
        *(f"line {number}" for number in range(19, 31)),
        "",
    ]


async def test_a_turn_that_ended_badly_says_so_when_the_session_is_taken_up(log: Session) -> None:
    for data in (
        TurnStartEvent(turn=1),
        said("Think."),
        TurnEndEvent(turn=1, reason="aborted", detail="user"),
    ):
        log.append(data)

    assert plain(recap(log.events, PLAIN)) == [
        "(this session is taken up again. Its last turn:)",
        "> Think.",
        "(stopped)",
        "",
    ]


async def test_the_last_words_of_the_model_are_shown_also_when_a_reply_without_words_followed(
    log: Session,
) -> None:
    for data in (
        TurnStartEvent(turn=1),
        said("Look."),
        reply(TextBlock(text="I will look."), finish="tool-calls"),
        reply(ToolCallBlock(call_id="c-1", name="read", arguments="{}"), finish="tool-calls"),
        TurnEndEvent(turn=1, reason="aborted", detail="user"),
    ):
        log.append(data)

    assert plain(recap(log.events, PLAIN)) == [
        "(this session is taken up again. Its last turn:)",
        "> Look.",
        "I will look.",
        "(stopped)",
        "",
    ]


async def test_a_session_without_a_turn_has_nothing_to_show(log: Session) -> None:
    assert recap(log.events, PLAIN) == []
    assert recap([], PLAIN) == []


# Subagents


@pytest.fixture
def subagents(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, SubagentStartedEvent, SubagentEndedEvent)


def started(
    description: str = "count files", call_id: str = "c-1", kind: str | None = None
) -> SubagentStartedEvent:
    return SubagentStartedEvent(
        call_id=call_id, session_id=f"s-of-{call_id}", kind=kind, description=description
    )


def ended(reason: str = "completed", steps: int = 12, call_id: str = "c-1") -> SubagentEndedEvent:
    return SubagentEndedEvent(
        call_id=call_id, session_id=f"s-of-{call_id}", reason=reason, detail=None, steps=steps
    )


def test_a_task_that_is_handed_to_a_subagent_is_shown_by_the_few_words_about_it(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    handed = call("subagent", description="count files", prompt="Count the files in src.")

    assert show(transcript, log, handed) == ["* subagent count files"]


@pytest.mark.usefixtures("subagents")
def test_a_subagent_that_begins_and_ends_is_told_in_two_lines(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, started(), ended())

    assert shown == [
        '    (a subagent works on "count files" in the session s-of-c-1)',
        '    (the subagent for "count files" is through, after 12 steps)',
    ]


@pytest.mark.usefixtures("subagents")
def test_the_kind_of_a_subagent_is_told_where_it_has_one(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, started(kind="reviewer"), ended(steps=1))

    assert shown == [
        '    (a subagent of the kind reviewer works on "count files" in the session s-of-c-1)',
        '    (the subagent for "count files" is through, after 1 step)',
    ]


@pytest.mark.usefixtures("subagents")
def test_a_subagent_that_did_not_finish_is_told_with_how_it_ended(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(transcript, log, started(), ended("blocked", 50))

    assert (
        shown[1] == '    (the subagent for "count files" did not finish: blocked, after 50 steps)'
    )


@pytest.mark.usefixtures("subagents")
def test_of_two_subagents_at_work_each_end_says_whose_it_is(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        started("count files", "c-1"),
        started("find the parser", "c-2"),
        ended(call_id="c-2", steps=3),
        ended(call_id="c-1", steps=7),
    )

    assert shown[2:] == [
        '    (the subagent for "find the parser" is through, after 3 steps)',
        '    (the subagent for "count files" is through, after 7 steps)',
    ]


@pytest.mark.usefixtures("subagents")
def test_the_end_of_a_subagent_whose_beginning_was_not_shown_names_no_task(log: Session) -> None:
    transcript = Transcript(PLAIN)

    assert show(transcript, log, ended(), ended("error", 2)) == [
        "    (a subagent is through, after 12 steps)",
        "    (a subagent did not finish: error, after 2 steps)",
    ]
    # And what is told of one task is told once.
    again = Transcript(PLAIN)
    assert show(again, log, started(), ended(), ended())[2:] == [
        "    (a subagent is through, after 12 steps)"
    ]


@pytest.mark.usefixtures("subagents")
def test_the_words_about_a_task_are_shown_on_one_line_and_do_nothing_to_the_terminal(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    words = "count\n the " + chr(0x1B) + "[2J files " + "x" * 200

    first, second = show(transcript, log, started(words), started("", "c-2"))

    assert first.startswith('    (a subagent works on "count the ' + STAND_IN + "[2J files xxx")
    assert first.endswith('..." in the session s-of-c-1)')
    assert chr(0x1B) not in first
    assert len(first) < 160
    assert second == "    (a subagent works in the session s-of-c-2)"


# Hooks


@pytest.fixture
def hooks(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, HookInvokedEvent, HookResultEvent)


def invoked(command: str = "pytest -q", event: str = "Stop") -> HookInvokedEvent:
    return HookInvokedEvent(event=event, command=command)


def ran(
    command: str = "pytest -q", event: str = "Stop", status: Any = "completed", reason: str = ""
) -> HookResultEvent:
    return HookResultEvent(
        event=event, command=command, status=status, exit_code=0, duration_ms=40, reason=reason
    )


@pytest.mark.usefixtures("hooks")
def test_while_a_hook_runs_a_word_says_what_is_waited_for(log: Session) -> None:
    transcript = Transcript(PLAIN)

    begun = show(transcript, log, invoked())
    meanwhile = transcript.partial
    over = show(transcript, log, ran())

    assert (begun, over) == ([], [])
    assert (meanwhile.plain, str(meanwhile.style)) == ("running a Stop hook (pytest -q)...", "dim")
    assert transcript.partial.plain == ""


@pytest.mark.usefixtures("hooks")
def test_of_hooks_that_run_at_once_the_word_tells_of_one_that_still_runs(log: Session) -> None:
    def word_after(*happened: EventData) -> str:
        transcript = Transcript(PLAIN)
        show(transcript, log, *happened)
        return transcript.partial.plain

    check, lint = ("check.py", "PreToolUse"), ("lint.py", "PreToolUse")
    checking = "running a PreToolUse hook (check.py)..."
    linting = "running a PreToolUse hook (lint.py)..."

    # The one that began last, for as long as it runs.
    assert word_after(invoked(*check), invoked(*lint)) == linting
    assert word_after(invoked(*check), invoked(*lint), ran(*lint)) == checking
    assert word_after(invoked(*check), invoked(*lint), ran(*check)) == linting
    assert word_after(invoked(*check), invoked(*lint), ran(*check), ran(*lint)) == ""
    # One hook that looks at two calls at once has ended when both of its runs have.
    assert word_after(invoked(*check), invoked(*check), ran(*check)) == checking
    assert word_after(invoked(*check), invoked(*check), ran(*check), ran(*check)) == ""
    # The same command at another moment is another hook.
    stop = ("check.py", "Stop")
    assert word_after(invoked(*check), invoked(*stop), ran(*stop)) == checking
    assert word_after(invoked(*check), invoked(*stop), ran(*check)) == (
        "running a Stop hook (check.py)..."
    )


@pytest.mark.usefixtures("hooks")
def test_when_a_hook_is_through_the_word_is_what_it_was_before(ctx: Context, log: Session) -> None:
    ctx.use(SESSIONS).declare(ctx, *COMPACTION_EVENTS)
    transcript = Transcript(PLAIN)

    show(transcript, log, CompactionStartEvent(why="manual", tokens=900), invoked())
    meanwhile = transcript.partial.plain
    show(transcript, log, ran())

    assert meanwhile == "running a Stop hook (pytest -q)..."
    assert transcript.partial.plain == "making the conversation shorter..."


@pytest.mark.usefixtures("hooks")
def test_a_reply_that_is_being_written_is_not_hidden_by_a_hook_that_runs(log: Session) -> None:
    transcript = Transcript(PLAIN)

    transcript.chunk(TextDelta(index=0, text="Half a li"))
    show(transcript, log, invoked())

    assert transcript.partial.plain == "Half a li"


@pytest.mark.usefixtures("hooks")
def test_the_command_of_a_hook_is_told_on_one_line_and_does_nothing_to_the_terminal(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)
    command = "python\n  check.py " + chr(0x1B) + "[2J " + "x" * 200

    show(transcript, log, invoked(command))
    word = transcript.partial.plain

    assert word.startswith("running a Stop hook (python check.py " + STAND_IN + "[2J xxx")
    assert word.endswith("xxx...)...")
    assert chr(0x1B) not in word
    assert len(word) == len("running a Stop hook (") + 100 + len(")...")
    # The run that ends is found by the command as it is, and not by what was shown of it.
    show(transcript, log, ran(command))
    assert transcript.partial.plain == ""


@pytest.mark.usefixtures("hooks")
def test_a_hook_whose_end_was_never_written_is_not_waited_for_once_the_turn_is_over(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)

    show(transcript, log, invoked(), TurnEndEvent(turn=1, reason="completed"))

    assert transcript.partial.plain == ""
    # And its end, should it come after all, is the end of nothing.
    assert show(transcript, log, ran()) == []
    assert transcript.partial.plain == ""


@pytest.mark.usefixtures("hooks")
def test_what_a_hook_stopped_is_told_where_it_takes_effect_and_not_twice(log: Session) -> None:
    transcript = Transcript(PLAIN)
    refused = ran("check.py", "PreToolUse", "blocked", "Leave the build alone.")

    shown = show(
        transcript,
        log,
        call("shell", command="rm -rf build"),
        invoked("check.py", "PreToolUse"),
        refused,
        result("a hook stopped the call: Leave the build alone.", error="HOOK_BLOCKED"),
    )

    assert shown == [
        "* shell rm -rf build",
        "  failed HOOK_BLOCKED, 750 ms",
        "    a hook stopped the call: Leave the build alone.",
    ]


@pytest.mark.usefixtures("hooks")
def test_a_hook_that_keeps_a_turn_going_is_told_with_what_it_says(log: Session) -> None:
    transcript = Transcript(PLAIN)
    reason = "The tests fail.\nRun them, and mend what fails."

    head, *lines = transcript.event(log.append(said(reason, source="hook/Stop")))

    assert (head.plain, str(head.style)) == ("(a Stop hook keeps the turn going. It says:)", "dim")
    assert plain(lines) == ["    The tests fail.", "    Run them, and mend what fails."]
    assert [str(line.style) for line in lines] == ["", ""]


@pytest.mark.usefixtures("hooks")
def test_a_long_reason_of_a_hook_is_shown_by_its_beginning(log: Session) -> None:
    transcript = Transcript(PLAIN)
    reason = "\n".join(f"FAILED test_{number}" for number in range(1, 6))

    assert show(transcript, log, said(reason, source="hook/Stop")) == [
        "(a Stop hook keeps the turn going. It says:)",
        "    FAILED test_1",
        "    FAILED test_2",
        "    FAILED test_3",
        "    ... 2 more lines",
    ]


@pytest.mark.usefixtures("hooks")
def test_a_hook_that_keeps_a_turn_going_and_says_nothing_is_told_all_the_same(
    log: Session,
) -> None:
    transcript = Transcript(PLAIN)

    assert show(transcript, log, said(" \n", source="hook/Stop")) == [
        "(a Stop hook keeps the turn going.)"
    ]


@pytest.mark.usefixtures("hooks")
def test_what_a_hook_adds_for_the_model_to_read_is_not_shown(log: Session) -> None:
    transcript = Transcript(PLAIN)

    shown = show(
        transcript,
        log,
        said("The branch is main.", source="hook/SessionStart"),
        said("Remember the changelog.", source="hook/UserPromptSubmit"),
        # An input that a hook stopped: the end of the turn says so.
        said("[A hook of the user stopped what was said here: no]", source="hook/UserPromptSubmit"),
    )

    assert shown == []
