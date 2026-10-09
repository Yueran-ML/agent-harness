"""The message surface: what the model sees, derived from the log."""

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    LlmFailure,
    Message,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    CORE_EVENTS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    InvalidEventError,
    Session,
    SessionEvent,
    StepStartEvent,
    Surface,
    SystemMessageEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnStartEvent,
    UserMessageEvent,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}

SYSTEM = SystemMessage(content=(TextBlock(text="You are helpful."),))
READ = ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}')
CALLING = AssistantMessage(content=(TextBlock(text="I will look."), READ))
RESULT = ToolMessage(call_id="call-1", content=(TextBlock(text="a very long file"),))
ANSWER = AssistantMessage(content=(TextBlock(text="It says hello."),))


def user(text: str) -> UserMessage:
    return UserMessage(content=(TextBlock(text=text),))


def said(message: AssistantMessage) -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=message,
        usage=Usage(),
        finish_reason="tool-calls" if message.tool_calls else "stop",
    )


def conversation(ctx: Context) -> Session:
    """A session with one turn: system 1, user 3, assistant 4, tool result 6, assistant 7."""
    session = Session(ctx, "s-1", KINDS)
    session.append(TurnStartEvent(turn=1))
    session.append(SystemMessageEvent(message=SYSTEM))
    session.append(StepStartEvent(turn=1, step=1))
    session.append(UserMessageEvent(message=user("Read a.txt.")))
    session.append(said(CALLING))
    session.append(ToolCallEvent(call_id="call-1", name="read", arguments=READ.arguments))
    session.append(ToolResultEvent(message=RESULT))
    session.append(said(ANSWER))
    return session


async def test_the_messages_are_the_message_events_in_order(ctx: Context) -> None:
    session = conversation(ctx)

    assert session.messages == (SYSTEM, user("Read a.txt."), CALLING, RESULT, ANSWER)
    assert session.surface == (1, 3, 4, 6, 7)


async def test_an_event_says_whether_it_is_on_the_surface(ctx: Context) -> None:
    session = conversation(ctx)

    assert [event.seq for event in session.events if event.on_surface] == [1, 3, 4, 6, 7]
    assert all(event.replaces == () for event in session.events)


async def test_an_empty_session_has_no_messages(ctx: Context) -> None:
    session = Session(ctx, "s-1", KINDS)

    assert session.messages == ()
    assert session.surface == ()


async def test_a_reply_without_content_stays_off_the_surface(ctx: Context) -> None:
    session = conversation(ctx)
    before = session.messages

    event = session.append(
        AssistantMessageEvent(
            message=AssistantMessage(content=()), usage=Usage(), finish_reason=None
        )
    )

    assert not event.on_surface
    assert session.messages == before


async def test_a_failed_attempt_stays_off_the_surface(ctx: Context) -> None:
    session = conversation(ctx)
    before = session.messages

    event = session.append(
        AssistantAttemptEvent(
            message=AssistantMessage(content=(TextBlock(text="It sa"),)),
            usage=Usage(),
            failure=LlmFailure(message="the stream ended without a finish", code="STREAM_CLOSED"),
        )
    )

    assert not event.on_surface
    assert session.messages == before


async def test_a_run_of_messages_can_be_replaced_by_one(ctx: Context) -> None:
    session = conversation(ctx)
    summary = user("Summary: the file a.txt says hello.")

    event = session.append(UserMessageEvent(message=summary, source="summary"), replaces=[3, 4, 6])

    assert event.on_surface
    assert event.replaces == (3, 4, 6)
    assert session.messages == (SYSTEM, summary, ANSWER)
    assert session.surface == (1, 8, 7)
    assert len(session) == 9
    assert session.events[4].data == said(CALLING)


async def test_a_single_message_can_be_replaced(ctx: Context) -> None:
    session = conversation(ctx)
    trimmed = ToolMessage(call_id="call-1", content=(TextBlock(text="a very [...] file"),))

    session.append(ToolResultEvent(message=trimmed), replaces=[6])

    assert session.messages == (SYSTEM, user("Read a.txt."), CALLING, trimmed, ANSWER)
    assert session.surface == (1, 3, 4, 8, 7)


async def test_new_messages_go_to_the_end_after_a_replacement(ctx: Context) -> None:
    session = conversation(ctx)
    summary = user("Summary: the file a.txt says hello.")
    session.append(UserMessageEvent(message=summary, source="summary"), replaces=[3, 4, 6])

    session.append(UserMessageEvent(message=user("Thanks.")))

    assert session.messages == (SYSTEM, summary, ANSWER, user("Thanks."))
    assert session.surface == (1, 8, 7, 9)


async def test_a_replacement_can_itself_be_replaced(ctx: Context) -> None:
    session = conversation(ctx)
    first = user("Summary: the file a.txt says hello.")
    session.append(UserMessageEvent(message=first, source="summary"), replaces=[3, 4, 6])
    second = user("Summary: a.txt was read and answered.")

    session.append(UserMessageEvent(message=second, source="summary"), replaces=[8, 7])

    assert session.messages == (SYSTEM, second)
    assert session.surface == (1, 9)


async def test_the_system_message_can_be_replaced(ctx: Context) -> None:
    session = conversation(ctx)
    updated = SystemMessage(content=(TextBlock(text="You are terse."),))

    session.append(SystemMessageEvent(message=updated), replaces=[1])

    assert session.messages[0] == updated
    assert session.surface == (8, 3, 4, 6, 7)


@pytest.mark.parametrize(
    ("replaces", "complaint"),
    [
        ([2], "event 2 is not on the message surface"),
        ([99], "event 99 is not on the message surface"),
        ([-1], "event -1 is not on the message surface"),
        ([3, 6], "the events 3, 6 are not neighbours on the message surface"),
        ([4, 3], "the events 4, 3 are not neighbours on the message surface"),
        ([3, 3], "the events 3, 3 are not neighbours on the message surface"),
        ([6, 7, 8], "event 8 is not on the message surface"),
    ],
)
async def test_a_replacement_must_name_a_run_of_neighbours(
    ctx: Context, replaces: list[int], complaint: str
) -> None:
    session = conversation(ctx)

    with pytest.raises(InvalidEventError, match=complaint):
        session.append(UserMessageEvent(message=user("Summary.")), replaces=replaces)

    assert len(session) == 8
    assert session.surface == (1, 3, 4, 6, 7)


async def test_a_message_that_was_replaced_cannot_be_replaced_again(ctx: Context) -> None:
    session = conversation(ctx)
    session.append(UserMessageEvent(message=user("Summary.")), replaces=[3, 4, 6])

    with pytest.raises(InvalidEventError, match="event 4 is not on the message surface"):
        session.append(UserMessageEvent(message=user("Again.")), replaces=[4])


async def test_only_a_message_can_replace_messages(ctx: Context) -> None:
    session = conversation(ctx)

    with pytest.raises(
        InvalidEventError, match="a turn/start event is no message and cannot replace any"
    ):
        session.append(TurnStartEvent(turn=2), replaces=[3])

    assert len(session) == 8


async def test_a_session_counts_how_often_its_messages_were_replaced(ctx: Context) -> None:
    session = conversation(ctx)
    assert session.replaced == 0

    # One replacement, however many messages it takes the place of.
    session.append(UserMessageEvent(message=user("Summary."), source="summary"), replaces=[3, 4, 6])
    assert session.replaced == 1
    session.append(UserMessageEvent(message=user("Thanks.")))
    session.append(TurnStartEvent(turn=2))
    assert session.replaced == 1
    with pytest.raises(InvalidEventError):
        session.append(UserMessageEvent(message=user("Again.")), replaces=[4])
    assert session.replaced == 1
    session.append(SystemMessageEvent(message=SYSTEM), replaces=[1])
    assert session.replaced == 2

    # A log that is read again comes to the same number.
    assert Session(ctx, "s-2", KINDS, events=session.events).replaced == 2


async def test_a_surface_can_be_rebuilt_from_the_events(ctx: Context) -> None:
    session = conversation(ctx)
    session.append(UserMessageEvent(message=user("Summary.")), replaces=[3, 4, 6])
    session.append(UserMessageEvent(message=user("Thanks.")))

    rebuilt = Surface()
    for event in session.events:
        rebuilt.apply(event)

    assert rebuilt.seqs == session.surface
    assert rebuilt.messages == session.messages


def test_a_surface_refuses_an_event_that_is_marked_as_a_message_but_holds_none() -> None:
    event = SessionEvent(seq=0, time=0, data=TurnStartEvent(turn=1), on_surface=True)

    with pytest.raises(InvalidEventError, match="event 0 is marked as a message but holds none"):
        Surface().apply(event)


def test_a_surface_refuses_an_event_that_replaces_messages_without_being_one() -> None:
    event = SessionEvent(seq=0, time=0, data=TurnStartEvent(turn=1), replaces=(1,))

    with pytest.raises(InvalidEventError, match="event 0 replaces messages without being one"):
        Surface().apply(event)


@settings(max_examples=200, deadline=None)
@given(st.data())
def test_the_surface_always_matches_a_replay_of_the_log(data: st.DataObject) -> None:
    """Whatever is appended and replaced, the messages equal a from-scratch replay."""
    session = Session(Context(), "s-1", KINDS)
    expected: list[tuple[int, Message]] = []
    replacements = 0

    for step in range(data.draw(st.integers(min_value=0, max_value=30))):
        message = user(f"message {step}")
        if expected and data.draw(st.booleans()):
            start = data.draw(st.integers(min_value=0, max_value=len(expected) - 1))
            stop = data.draw(st.integers(min_value=start + 1, max_value=len(expected)))
            replaces = [seq for seq, _ in expected[start:stop]]
            event = session.append(UserMessageEvent(message=message), replaces=replaces)
            expected[start:stop] = [(event.seq, message)]
            replacements += 1
        elif data.draw(st.booleans()):
            event = session.append(UserMessageEvent(message=message))
            expected.append((event.seq, message))
        else:
            session.append(TurnStartEvent(turn=step))

    rebuilt = Surface()
    for event in session.events:
        rebuilt.apply(event)

    assert session.surface == tuple(seq for seq, _ in expected)
    assert session.messages == tuple(message for _, message in expected)
    assert rebuilt.replaced == session.replaced == replacements
    assert rebuilt.seqs == session.surface
    assert rebuilt.messages == session.messages
