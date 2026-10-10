"""Old results of tools are cut down to their two ends."""

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.compaction.basic.trim import trim
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    CORE_EVENTS,
    AssistantMessageEvent,
    Session,
    SystemMessageEvent,
    ToolError,
    ToolResultEvent,
    TurnStartEvent,
    UserMessageEvent,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}
LIMITS = {"over": 8192, "head": 4096, "tail": 1024}


def text_of(size: int) -> str:
    """A text of so many characters in which every place can be told from the others."""
    return "".join(f"{number:07d}\n" for number in range(size // 8 + 1))[:size]


def asked(session: Session, *call_ids: str, say: str = "") -> None:
    """The model replies, and calls a tool once for each id."""
    calls = tuple(
        ToolCallBlock(call_id=call_id, name="read", arguments="{}") for call_id in call_ids
    )
    said = (TextBlock(text=say),) if say else ()
    message = AssistantMessage(content=(*said, *calls))
    session.append(
        AssistantMessageEvent(
            message=message, usage=Usage(), finish_reason="tool-calls" if calls else "stop"
        )
    )


def answered(session: Session, call_id: str, text: str, **more: object) -> int:
    """A tool gives its result. Returns where the result stands in the log."""
    message = ToolMessage(
        call_id=call_id, content=(TextBlock(text=text),), is_error="error" in more
    )
    return session.append(ToolResultEvent(message=message, **more)).seq  # pyright: ignore[reportArgumentType]


def begun(ctx: Context) -> Session:
    session = Session(ctx, "s-1", KINDS)
    session.append(TurnStartEvent(turn=1))
    session.append(
        SystemMessageEvent(message=SystemMessage(content=(TextBlock(text="Be helpful."),)))
    )
    session.append(UserMessageEvent(message=UserMessage(content=(TextBlock(text="Read them."),))))
    return session


def shown(session: Session, seq: int) -> str:
    """The text of a result as the model sees it now, wherever it stands in the log."""
    (text,) = (
        "".join(block.text for block in message.content if isinstance(block, TextBlock))
        for at, message in zip(session.surface, session.messages, strict=True)
        if at == seq
    )
    return text


def left_out(count: int) -> str:
    return f"[... {count} characters of this result were left out to shorten the conversation ...]"


async def test_an_old_result_that_is_long_is_cut_down_to_its_two_ends(ctx: Context) -> None:
    session = begun(ctx)
    long = text_of(10_000)
    asked(session, "call-1")
    old = answered(session, "call-1", long)
    asked(session, say="Done.")
    before = session.messages

    assert trim(session, **LIMITS) == 1

    cut = session.events[-1]
    assert cut.replaces == (old,)
    assert isinstance(cut.data, ToolResultEvent)
    assert shown(session, cut.seq) == f"{long[:4096]}{left_out(4880)}\n{long[-1024:]}"
    # Nothing else of what the model sees has changed, and the result stands where it stood.
    assert session.surface.index(cut.seq) == 3
    assert session.messages[:3] == before[:3]
    assert session.messages[4:] == before[4:]
    assert session.replaced == 1


async def test_the_line_about_what_is_left_out_stands_on_a_line_of_its_own(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1", "call-2")
    whole_lines = answered(session, "call-1", "x" * 4095 + "\n" + "y" * 6000)
    broken_line = answered(session, "call-2", "x" * 4096 + "y" * 6000)
    asked(session, say="Done.")

    assert trim(session, **LIMITS) == 2

    first, second = session.events[-2:]
    assert (first.replaces, second.replaces) == ((whole_lines,), (broken_line,))
    assert shown(session, first.seq) == f"{'x' * 4095}\n{left_out(4976)}\n{'y' * 1024}"
    assert shown(session, second.seq) == f"{'x' * 4096}\n{left_out(4976)}\n{'y' * 1024}"


async def test_a_result_that_the_model_has_not_read_yet_is_left_alone(ctx: Context) -> None:
    # The model reads a result in the request after it. What follows its last reply, it has
    # not read: cut down now, it would never have been read whole.
    session = begun(ctx)
    asked(session, "call-1")
    read = answered(session, "call-1", text_of(10_000))
    asked(session, "call-2", "call-3")
    unread = [answered(session, call_id, text_of(10_000)) for call_id in ("call-2", "call-3")]

    assert trim(session, **LIMITS) == 1

    assert session.events[-1].replaces == (read,)
    assert [shown(session, seq) for seq in unread] == [text_of(10_000)] * 2


async def test_before_the_model_has_replied_nothing_is_old(ctx: Context) -> None:
    session = begun(ctx)
    count = len(session)

    assert trim(session, **LIMITS) == 0
    assert len(session) == count


@pytest.mark.parametrize(("size", "cut"), [(8192, 0), (8193, 1)])
async def test_a_result_is_long_if_it_has_more_characters_than_the_limit(
    ctx: Context, size: int, cut: int
) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", text_of(size))
    asked(session, say="Done.")

    assert trim(session, **LIMITS) == cut


async def test_a_result_that_was_cut_is_not_cut_again(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", text_of(50_000))
    asked(session, say="Done.")

    assert trim(session, **LIMITS) == 1
    count = len(session)

    assert trim(session, **LIMITS) == 0
    assert len(session) == count


async def test_only_the_text_of_a_result_changes(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    error = ToolError(code="TOOL_ERROR", reason="it broke")
    old = answered(session, "call-1", text_of(10_000), error=error, meta={"took_ms": 12})
    asked(session, say="It failed.")

    assert trim(session, **LIMITS) == 1

    was, now = session.events[old].data, session.events[-1].data
    assert isinstance(was, ToolResultEvent)
    assert isinstance(now, ToolResultEvent)
    assert now.message.call_id == "call-1"
    assert now.message.is_error
    assert now.error == error
    assert now.meta == {"took_ms": 12}
    assert now == was.model_copy(update={"message": now.message})
    assert len(now.message.content) == 1


async def test_ends_that_are_no_shorter_than_the_whole_leave_a_result_as_it_is(
    ctx: Context,
) -> None:
    # The line between the ends has a length too. A result is replaced only by less.
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", text_of(200))
    asked(session, say="Done.")
    count = len(session)

    assert trim(session, over=100, head=90, tail=90) == 0
    # Ends that reach each other leave nothing out, whatever their sum says.
    assert trim(session, over=100, head=10, tail=300) == 0
    assert trim(session, over=100, head=300, tail=10) == 0
    assert trim(session, over=100, head=60, tail=60) == 0
    assert len(session) == count
    assert trim(session, over=100, head=50, tail=50) == 1


async def test_a_result_can_be_cut_down_to_its_beginning_alone(ctx: Context) -> None:
    session = begun(ctx)
    long = text_of(10_000)
    asked(session, "call-1")
    answered(session, "call-1", long)
    asked(session, say="Done.")

    assert trim(session, over=8192, head=4096, tail=0) == 1

    assert shown(session, session.events[-1].seq) == f"{long[:4096]}{left_out(5904)}\n"


async def test_each_block_of_text_of_a_result_is_cut_by_itself(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    blocks = (TextBlock(text="a short one"), TextBlock(text=text_of(10_000)))
    session.append(ToolResultEvent(message=ToolMessage(call_id="call-1", content=blocks)))
    asked(session, say="Done.")

    assert trim(session, **LIMITS) == 1

    now = session.events[-1].data
    assert isinstance(now, ToolResultEvent)
    assert now.message.content == (
        TextBlock(text="a short one"),
        TextBlock(text=f"{text_of(10_000)[:4096]}{left_out(4880)}\n{text_of(10_000)[-1024:]}"),
    )


async def test_messages_that_are_no_results_are_never_cut(ctx: Context) -> None:
    session = begun(ctx)
    session.append(
        UserMessageEvent(message=UserMessage(content=(TextBlock(text=text_of(20_000)),)))
    )
    asked(session, say=text_of(20_000))
    asked(session, say="Done.")
    count = len(session)

    assert trim(session, **LIMITS) == 0
    assert len(session) == count


@settings(max_examples=100, deadline=None)
@given(
    sizes=st.lists(st.integers(min_value=0, max_value=400), min_size=1, max_size=6),
    over=st.integers(min_value=0, max_value=300),
    head=st.integers(min_value=0, max_value=150),
    tail=st.integers(min_value=0, max_value=150),
)
def test_what_is_cut_is_shorter_and_keeps_its_ends(
    sizes: list[int], over: int, head: int, tail: int
) -> None:
    session = begun(Context())
    seqs: list[int] = []
    for number, size in enumerate(sizes):
        asked(session, f"call-{number}")
        seqs.append(answered(session, f"call-{number}", text_of(size)))
    asked(session, say="Done.")

    cut = trim(session, over=over, head=head, tail=tail)

    results = [
        (event.replaces[0] if event.replaces else event.seq, event.seq)
        for event in (session.events[seq] for seq in session.surface)
        if isinstance(event.data, ToolResultEvent)
    ]
    assert [original for original, _ in results] == seqs
    assert sum(1 for original, now in results if original != now) == cut
    for size, (original, now) in zip(sizes, results, strict=True):
        whole, text = text_of(size), shown(session, now)
        if original == now:
            # Not long, or its ends with the line between them would be no shorter.
            fits = size <= over or size <= head + tail
            assert fits or head + tail + len(left_out(size - head - tail)) + 2 >= size
            continue
        assert size > over
        assert size > head + tail
        assert len(text) < size
        assert text.startswith(whole[:head])
        assert text.endswith(whole[size - tail :])
        assert left_out(size - head - tail) in text
