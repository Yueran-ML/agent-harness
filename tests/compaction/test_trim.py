"""Old results of tools are cut down to their two ends."""

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.compaction.basic.trim import (
    Cut,
    apply,
    capped,
    cuts,
    level,
    located,
    trim,
    unread,
    whole,
)
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
from columbina.token_meter import estimate_message

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

    assert trim(session, over=190, head=90, tail=90) == 0
    assert trim(session, over=190, head=60, tail=60) == 0
    # Ends that reach each other leave nothing out, whatever their sum says. An end that is
    # longer than the whole is no end of it: what it would make of the result would be short,
    # and would say that less than nothing was left out.
    assert trim(session, over=199, head=10, tail=300) == 0
    assert trim(session, over=199, head=300, tail=10) == 0
    assert trim(session, over=199, head=100, tail=100) == 0
    assert len(session) == count
    assert trim(session, over=190, head=50, tail=50) == 1


async def test_what_a_result_is_cut_down_to_is_never_long_itself(ctx: Context) -> None:
    # Ends that leave no room for the line between them: what they would make of a result
    # would be cut again the next time, down to other ends and with another count in it.
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", "x" * 1000)
    asked(session, say="Done.")
    count = len(session)

    assert trim(session, over=100, head=60, tail=39) == 0
    assert len(session) == count
    # With room for the line, it is cut once and stays as it is.
    assert trim(session, over=200, head=60, tail=39) == 1
    assert trim(session, over=200, head=60, tail=39) == 0
    assert shown(session, session.events[-1].seq) == f"{'x' * 60}\n{left_out(901)}\n{'x' * 39}"


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


async def test_what_would_be_cut_can_be_found_without_cutting_it(ctx: Context) -> None:
    session = begun(ctx)
    long = text_of(10_000)
    asked(session, "call-1", "call-2")
    first = answered(session, "call-1", long)
    answered(session, "call-2", "short")
    asked(session, say="Done.")
    count = len(session)

    found = cuts(session, **LIMITS)

    assert len(session) == count
    (cut,) = found
    assert cut.seq == first
    assert cut.whole == session.events[first].data
    assert cut.shorter.message.content == (
        TextBlock(text=f"{long[:4096]}{left_out(4880)}\n{long[-1024:]}"),
    )
    assert apply(session, found) == 1
    assert session.events[-1].data == cut.shorter
    assert session.events[-1].replaces == (first,)


WHERE = "(The whole result has 10000 bytes and is in spill/s-1/result.txt. Read it with read.)"


async def test_a_cut_can_say_where_the_whole_of_the_result_is(ctx: Context) -> None:
    session = begun(ctx)
    long = text_of(10_000)
    asked(session, "call-1")
    old = answered(session, "call-1", long)
    asked(session, say="Done.")
    (cut,) = cuts(session, **LIMITS)

    told = located(cut, WHERE, **LIMITS)

    # The line stands after the one that says what is left out, and the two ends are the same.
    assert told.seq == old
    assert told.whole == cut.whole
    (block,) = told.shorter.message.content
    assert block.text == f"{long[:4096]}{left_out(4880)}\n{WHERE}\n{long[-1024:]}"
    assert whole(cut) == long


async def test_a_cut_with_no_room_for_where_the_whole_is_stays_as_it_was(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", text_of(10_000))
    asked(session, say="Done.")
    (roomy,) = cuts(session, **LIMITS)
    (short,) = roomy.shorter.message.content
    tight = {**LIMITS, "over": len(short.text) + 10}
    (cut,) = cuts(session, **tight)

    # With the line, what takes the place of the result would be long enough to be cut again.
    assert located(cut, WHERE, **tight) == cut


async def test_where_the_whole_is_stands_in_every_block_that_was_cut(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    blocks = (
        TextBlock(text=text_of(9000)),
        TextBlock(text="short"),
        TextBlock(text=text_of(9500)),
    )
    session.append(ToolResultEvent(message=ToolMessage(call_id="call-1", content=blocks)))
    asked(session, say="Done.")
    (cut,) = cuts(session, **LIMITS)

    first, second, third = located(cut, WHERE, **LIMITS).shorter.message.content

    assert first.text.count(WHERE) == 1
    assert second.text == "short"
    assert third.text.count(WHERE) == 1
    # What is saved is all of it, block after block.
    assert whole(cut) == f"{text_of(9000)}\nshort\n{text_of(9500)}"


async def test_a_result_that_the_model_no_longer_sees_is_passed_over(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1", "call-2")
    gone = answered(session, "call-1", text_of(10_000))
    stays = answered(session, "call-2", text_of(10_000))
    asked(session, say="Done.")
    found = cuts(session, **LIMITS)
    assert [cut.seq for cut in found] == [gone, stays]
    # Something has taken the place of the first one since: a summary, or this very cut.
    assert apply(session, found[:1]) == 1
    count = len(session)

    assert apply(session, found) == 1

    assert len(session) == count + 1
    assert session.events[-1].replaces == (stays,)
    assert apply(session, [Cut(seq=10**6, whole=found[0].whole, shorter=found[0].shorter)]) == 0


def tokens(result: ToolResultEvent) -> int:
    return estimate_message(result.message)


def dense_of(size: int) -> str:
    """A text of so many characters that costs a model several times what one in English does."""
    lines = (f"第{number:06d}行这里写着一些字\n" for number in range(size // 15 + 1))
    return "".join(lines)[:size]


async def test_the_results_that_the_model_has_not_read_yet_are_those_after_its_last_reply(
    ctx: Context,
) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    first = answered(session, "call-1", text_of(10_000))
    asked(session, "call-2", "call-3")
    second = answered(session, "call-2", text_of(9000))
    answered(session, "call-3", "short")

    # The request that shows them to the model is still to come.
    assert [result.message.call_id for result in unread(session)] == ["call-2", "call-3"]
    assert [cut.seq for cut in capped(session, cap=100, size=tokens)] == [second]
    # Those that it has read are found as before, and no others with them.
    assert [cut.seq for cut in cuts(session, **LIMITS)] == [first]


async def test_once_the_model_has_replied_to_them_no_result_is_unread(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", text_of(10_000))
    asked(session, say="Done.")

    assert unread(session) == []
    assert capped(session, cap=0, size=tokens) == []


@pytest.mark.parametrize(
    ("sizes", "room", "most"),
    [
        # Each of four that are as large gets a quarter of the room.
        ([4000, 4000, 4000, 4000], 6000, 1500),
        # A small one stays as it is, and the others share what it leaves.
        ([100, 4000, 4000], 6000, 2950),
        ([4000, 100, 4000], 6000, 2950),
        ([10, 20, 5000], 1000, 970),
        # One that is just what its share would be stays whole too.
        ([500, 900, 900], 1500, 500),
        ([300, 900, 900], 1500, 600),
        # One result by itself has all the room.
        ([5000], 1200, 1200),
        # Rounded down.
        ([500, 500, 500], 1000, 333),
        # No room is no room.
        ([500, 700], 1, 0),
        ([500, 700], 0, 0),
        # All of them fit: none comes to more than the largest does.
        ([100, 200], 300, 200),
        ([], 10, 0),
    ],
)
def test_the_most_that_each_result_may_come_to_shares_out_their_room(
    sizes: list[int], room: int, most: int
) -> None:
    assert level(sizes, room) == most


@settings(max_examples=200, deadline=None)
@given(
    sizes=st.lists(st.integers(min_value=0, max_value=5000), min_size=1, max_size=8),
    room=st.integers(min_value=0, max_value=20_000),
)
def test_results_that_are_no_larger_than_the_most_fit_their_room_and_no_more_would_fit(
    sizes: list[int], room: int
) -> None:
    most = level(sizes, room)

    if sum(sizes) <= room:
        assert most == max(sizes)
        return
    assert sum(min(size, most) for size in sizes) <= room
    # With one token more for each, they would have no room.
    assert sum(min(size, most + 1) for size in sizes) > room


async def test_a_result_gives_way_by_what_it_costs_and_not_by_how_long_it_is(
    ctx: Context,
) -> None:
    session = begun(ctx)
    asked(session, "call-1", "call-2")
    plain, dense = text_of(6000), dense_of(6000)
    first = answered(session, "call-1", plain)
    second = answered(session, "call-2", dense)
    # Neither is long, as results are when they are cut by the number of their characters.
    assert len(plain) < LIMITS["over"] > len(dense)

    found = capped(session, cap=1000, size=tokens)

    assert [cut.seq for cut in found] == [first, second]
    for cut, whole_text in zip(found, (plain, dense), strict=True):
        assert cut.whole.message.content[0].text == whole_text
        # As much is kept as comes to the cap, and no less than has to go is left out.
        assert 990 < tokens(cut.shorter) <= 1000
        kept = cut.kept
        assert kept is not None
        head, tail = kept - kept // 5, kept // 5
        (block,) = cut.shorter.message.content
        between = "" if whole_text[:head].endswith("\n") else "\n"
        gap = left_out(len(whole_text) - kept)
        assert block.text == f"{whole_text[:head]}{between}{gap}\n{whole_text[-tail:]}"
    # What costs more by the character keeps fewer characters.
    assert (found[1].kept or 0) < (found[0].kept or 0)


async def test_a_result_that_comes_to_no_more_than_the_cap_stays_as_it_is(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1", "call-2")
    answered(session, "call-1", dense_of(3000))
    larger = answered(session, "call-2", dense_of(6000))
    (smaller, _) = unread(session)

    assert [cut.seq for cut in capped(session, cap=tokens(smaller), size=tokens)] == [larger]
    assert len(capped(session, cap=tokens(smaller) - 500, size=tokens)) == 2


async def test_only_the_text_of_a_result_that_gives_way_changes(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    error = ToolError(code="TOOL_ERROR", reason="it broke")
    answered(session, "call-1", dense_of(6000), error=error, meta={"took_ms": 12})

    (cut,) = capped(session, cap=1000, size=tokens)

    was, now = cut.whole, cut.shorter
    assert now.message.call_id == "call-1"
    assert now.message.is_error
    assert (now.error, now.meta) == (error, {"took_ms": 12})
    assert now == was.model_copy(update={"message": now.message})
    assert len(now.message.content) == 1


async def test_a_result_that_gives_way_may_come_to_just_what_the_cap_is(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", dense_of(6000))
    (cut,) = capped(session, cap=1000, size=tokens)
    exactly = tokens(cut.shorter)

    (again,) = capped(session, cap=exactly, size=tokens)

    assert tokens(again.shorter) == exactly


async def test_no_result_gives_way_for_the_sake_of_a_few_characters(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    text = text_of(6000)
    answered(session, "call-1", text)
    (result,) = unread(session)
    size = tokens(result)

    def saved(cap: int) -> int:
        """How many characters shorter the result is with such a cap: none if it stays whole."""
        found = capped(session, cap=cap, size=tokens)
        return len(text) - len(found[0].shorter.message.content[0].text) if found else 0

    assert saved(size) == 0
    # A cut that says where the whole is has two lines more than the result had. One that
    # took fewer characters away than those come to would leave the model no less to read,
    # and nowhere to put the line about the whole. So a result that gives way at all gives
    # way by some hundreds of characters, also where one token is all that is too much.
    least = saved(size - 1)
    assert 512 <= least < 520
    assert saved(size - 600) > 2 * least
    # So the line about the whole has room in it, and one more to stand on. A line that is
    # longer than what was taken away is left out: the result would be no shorter for it.
    (cut,) = capped(session, cap=size - 1, size=tokens)
    fits, too_long = "w" * (least - 2), "w" * (least - 1)
    (told,) = located(cut, fits, **LIMITS).shorter.message.content
    assert (fits in told.text, len(told.text)) == (True, len(text) - 1)
    assert located(cut, too_long, **LIMITS) == cut


async def test_a_result_that_was_cut_before_does_not_give_way_again(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1", "call-2")
    answered(session, "call-1", dense_of(6000))
    other = answered(session, "call-2", dense_of(6000))
    (cut, _) = capped(session, cap=1000, size=tokens)
    apply(session, [located(cut, WHERE, **LIMITS)])

    # It stands in the conversation as two ends, with a line between them about where its
    # whole is. Cut again, it would lose that line with its middle.
    assert [again.seq for again in capped(session, cap=200, size=tokens)] == [other]
    assert len(unread(session)) == 2


async def test_a_result_that_says_where_its_whole_is_keeps_that_line_however_low_the_cap(
    ctx: Context,
) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    text = f"{dense_of(5000)}\n{WHERE}"
    answered(session, "call-1", text)

    (roomy,) = capped(session, cap=600, size=tokens)
    (tight,) = capped(session, cap=0, size=tokens)

    # It was too large when it was made, and its last line says where all of it is. That
    # line is what the model finds the rest by, and it is kept whole.
    (block,) = roomy.shorter.message.content
    assert block.text.startswith(text[:100])
    assert block.text.endswith(f"\n{WHERE}")
    assert tight.kept == 0
    assert tight.shorter.message.content[0].text == f"{left_out(len(text) - len(WHERE))}\n{WHERE}"
    # A blank line after it is no reason to lose it.
    asked(session, "call-2")
    answered(session, "call-2", f"{text}\n\n")
    (ended,) = capped(session, cap=0, size=tokens)
    (block,) = ended.shorter.message.content
    assert block.text == f"{left_out(len(text) - len(WHERE))}\n{WHERE}\n\n"


async def test_with_no_room_at_all_a_result_is_the_line_about_what_is_left_out(
    ctx: Context,
) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    answered(session, "call-1", dense_of(5000))

    (cut,) = capped(session, cap=0, size=tokens)

    assert cut.kept == 0
    assert cut.shorter.message.content[0].text == f"{left_out(5000)}\n"


async def test_each_block_of_a_result_that_gives_way_keeps_as_much_as_the_others(
    ctx: Context,
) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    blocks = (
        TextBlock(text=dense_of(3000)),
        TextBlock(text="short"),
        TextBlock(text=dense_of(5000)),
    )
    seq = session.append(ToolResultEvent(message=ToolMessage(call_id="call-1", content=blocks))).seq

    (cut,) = capped(session, cap=1200, size=tokens)

    assert cut.seq == seq
    assert 1190 < tokens(cut.shorter) <= 1200
    first, second, third = cut.shorter.message.content
    kept = cut.kept
    assert kept is not None
    assert left_out(3000 - kept) in first.text
    assert second.text == "short"
    assert left_out(5000 - kept) in third.text


async def test_a_result_that_gave_way_can_say_where_its_whole_is(ctx: Context) -> None:
    session = begun(ctx)
    asked(session, "call-1")
    text = dense_of(6000)
    answered(session, "call-1", text)
    (cut,) = capped(session, cap=1000, size=tokens)
    kept = cut.kept
    assert kept is not None

    told = located(cut, WHERE, **LIMITS)

    # The line stands after the one about what is left out. As much of the two ends is kept
    # as before: the numbers of the config, by which read results are cut, do not count here.
    head, tail = kept - kept // 5, kept // 5
    (block,) = told.shorter.message.content
    between = "" if text[:head].endswith("\n") else "\n"
    assert block.text == (f"{text[:head]}{between}{left_out(6000 - kept)}\n{WHERE}\n{text[-tail:]}")
    assert (told.seq, told.whole, told.kept) == (cut.seq, cut.whole, kept)
    assert whole(cut) == text


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
            # Not long, or its ends with the line between them would be no shorter, or long.
            fits = size <= over or size <= head + tail
            least = head + tail + len(left_out(size - head - tail)) + 1
            assert fits or least + 1 >= size or least + 1 > over
            continue
        assert size > over
        assert size > head + tail
        assert len(text) < size
        assert len(text) <= over
        assert text.startswith(whole[:head])
        assert text.endswith(whole[size - tail :])
        assert left_out(size - head - tail) in text
