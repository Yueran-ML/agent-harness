"""The inbox: input that waits for an agent, kept in the session log."""

from pathlib import Path

import tack.agent
import tack.session
from tack.agent import (
    INBOX,
    Inbox,
    InboxItem,
    InboxSplicedEvent,
    claim_for_step,
    claim_for_turn,
    clear_inbox,
    enqueue,
)
from tack.kernel import Context
from tack.llm import TextBlock, UserMessage
from tack.session import SESSION_EVENT, SESSIONS, Committed, Session


def item(text: str, source: str = "user") -> InboxItem:
    return InboxItem(message=UserMessage(content=(TextBlock(text=text),)), source=source)


async def open_session(ctx: Context, root: Path | None = None) -> Session:
    await ctx.plugin(tack.session, None if root is None else {"root": root})
    await ctx.plugin(tack.agent)
    return ctx.use(SESSIONS).create(ctx, Path.cwd(), session_id="s-1")


async def test_a_new_session_has_an_empty_inbox(ctx: Context) -> None:
    session = await open_session(ctx)

    assert INBOX.of(session) == Inbox()
    assert not INBOX.of(session)


async def test_input_waits_in_the_queue_it_was_put_in(ctx: Context) -> None:
    session = await open_session(ctx)

    enqueue(session, "next-turn", item("first task"))
    enqueue(session, "next-step", item("a hint", "steer"))
    enqueue(session, "next-turn", item("second task"))

    assert INBOX.of(session) == Inbox(
        next_turn=(item("first task"), item("second task")),
        next_step=(item("a hint", "steer"),),
    )
    assert INBOX.of(session)
    assert [event.type for event in session.events] == ["agent/inbox/spliced"] * 3
    assert session.events[0].data == InboxSplicedEvent(
        queue="next-turn", added=(item("first task"),)
    )


async def test_a_turn_takes_everything_for_the_next_step_and_one_task(ctx: Context) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-turn", item("first task"))
    enqueue(session, "next-step", item("a hint", "steer"))
    enqueue(session, "next-turn", item("second task"))
    enqueue(session, "next-step", item("some context", "inject"))

    claimed = claim_for_turn(session)

    assert claimed == (
        item("a hint", "steer"),
        item("some context", "inject"),
        item("first task"),
    )
    assert INBOX.of(session) == Inbox(next_turn=(item("second task"),))
    assert [event.data for event in session.events[4:]] == [
        InboxSplicedEvent(queue="next-step", removed=2),
        InboxSplicedEvent(queue="next-turn", removed=1),
    ]


async def test_a_turn_can_begin_with_only_one_of_the_two_queues(ctx: Context) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-step", item("a hint", "steer"))

    assert claim_for_turn(session) == (item("a hint", "steer"),)
    assert len(session) == 2

    enqueue(session, "next-turn", item("a task"))

    assert claim_for_turn(session) == (item("a task"),)
    assert len(session) == 4


async def test_a_step_takes_only_what_waits_for_the_next_step(ctx: Context) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-turn", item("a task"))
    enqueue(session, "next-step", item("a hint", "steer"))

    claimed = claim_for_step(session)

    assert claimed == (item("a hint", "steer"),)
    assert INBOX.of(session) == Inbox(next_turn=(item("a task"),))


async def test_taking_from_an_empty_inbox_leaves_the_log_alone(ctx: Context) -> None:
    session = await open_session(ctx)

    assert claim_for_turn(session) == ()
    assert claim_for_step(session) == ()
    clear_inbox(session)

    assert len(session) == 0


async def test_an_inbox_can_be_emptied(ctx: Context) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-turn", item("first task"))
    enqueue(session, "next-turn", item("second task"))
    enqueue(session, "next-step", item("a hint", "steer"))

    clear_inbox(session)

    assert INBOX.of(session) == Inbox()
    assert [event.data for event in session.events[3:]] == [
        InboxSplicedEvent(queue="next-step", removed=1),
        InboxSplicedEvent(queue="next-turn", removed=2),
    ]


# A listener that changes the inbox while it is being read


def at_the_next_removal(ctx: Context, session: Session, act: object) -> None:
    """Call ``act`` once, when the next event that takes input out of the inbox is written."""
    waiting = [act]

    def listener(committed: Committed) -> None:
        data = committed.event.data
        if waiting and isinstance(data, InboxSplicedEvent) and data.removed:
            waiting.pop()
            assert callable(act)
            act()

    ctx.on(SESSION_EVENT, listener)


async def test_a_task_that_is_dropped_while_a_turn_takes_its_input_is_not_taken(
    ctx: Context,
) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-step", item("some context", "inject"))
    enqueue(session, "next-turn", item("first task"))
    enqueue(session, "next-turn", item("second task"))
    # As the context is taken out, a listener cancels the agent and drops what waits.
    at_the_next_removal(ctx, session, lambda: clear_inbox(session))

    claimed = claim_for_turn(session)

    assert claimed == (item("some context", "inject"),)
    assert INBOX.of(session) == Inbox()
    assert [event.data for event in session.events[3:]] == [
        InboxSplicedEvent(queue="next-step", removed=1),
        InboxSplicedEvent(queue="next-turn", removed=2),
    ]


async def test_a_task_that_arrives_while_a_turn_takes_its_input_is_taken(ctx: Context) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-step", item("some context", "inject"))
    at_the_next_removal(
        ctx, session, lambda: enqueue(session, "next-turn", item("a task just in time"))
    )

    claimed = claim_for_turn(session)

    assert claimed == (item("some context", "inject"), item("a task just in time"))
    assert INBOX.of(session) == Inbox()


async def test_emptying_an_inbox_that_a_listener_empties_too_removes_nothing_twice(
    ctx: Context,
) -> None:
    session = await open_session(ctx)
    enqueue(session, "next-step", item("some context", "inject"))
    enqueue(session, "next-turn", item("first task"))
    at_the_next_removal(ctx, session, lambda: clear_inbox(session))

    clear_inbox(session)

    assert INBOX.of(session) == Inbox()
    assert [event.data for event in session.events[2:]] == [
        InboxSplicedEvent(queue="next-step", removed=1),
        InboxSplicedEvent(queue="next-turn", removed=1),
    ]


async def test_input_that_waited_is_still_there_after_a_resume(
    ctx: Context, tmp_path: Path
) -> None:
    session = await open_session(ctx, tmp_path)
    enqueue(session, "next-turn", item("a task"))
    enqueue(session, "next-step", item("a hint", "steer"))
    session.close()

    resumed = ctx.use(SESSIONS).resume(ctx, Path.cwd(), "s-1")

    assert INBOX.of(resumed) == Inbox(
        next_turn=(item("a task"),), next_step=(item("a hint", "steer"),)
    )
