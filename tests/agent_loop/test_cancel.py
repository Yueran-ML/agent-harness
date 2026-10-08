"""The loop under stress: steering, cancelling, failing and closing in the middle of a turn."""

import asyncio
import logging

import pytest

from support.loop import SYSTEM, Harness, assistant, idle, types, user
from tack.agent import INBOX, Inbox, InboxItem
from tack.llm import AssistantMessage, Usage
from tack.llm.adapters.scripted import Reply
from tack.session import (
    AssistantMessageEvent,
    SessionStoreError,
    TurnEndEvent,
    UserMessageEvent,
)
from tack.tools import Tools

# Input that arrives while a turn runs


async def test_a_steer_reaches_a_running_turn_at_its_next_step(harness: Harness) -> None:
    agent = await harness.agent(
        Reply(text="Reading a.txt.", hold=True), Reply(text="Reading b.txt.")
    )
    agent.followup("Read a.txt.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.steer("No, read b.txt.")
    harness.model.release()
    await idle(agent)

    assert types(agent.session, 5) == [
        "step/start",
        "request/header",
        "agent/inbox/spliced",
        "assistant/message",
        "step/end",
        "agent/inbox/spliced",
        "user/message",
        "step/start",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    assert harness.model.requests[1].messages == (
        SYSTEM,
        user("Read a.txt."),
        assistant("Reading a.txt."),
        user("No, read b.txt."),
    )
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")
    steered = agent.session.events[-5].data
    assert isinstance(steered, UserMessageEvent)
    assert steered.source == "steer"


async def test_context_injected_into_a_running_turn_is_read_at_its_next_step(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="Working.", hold=True), Reply(text="Noted."))
    agent.followup("Do the work.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.inject("The disk is almost full.")
    harness.model.release()
    await idle(agent)

    assert harness.model.requests[1].messages[-1] == user("The disk is almost full.")
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_a_task_given_to_a_running_agent_waits_for_a_turn_of_its_own(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="First done.", hold=True), Reply(text="Second done."))
    agent.followup("First.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.followup("Second.")
    harness.model.release()
    await idle(agent)

    ends = [event.data for event in agent.session.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]
    assert harness.model.requests[0].messages[-1] == user("First.")
    assert harness.model.requests[1].messages[-1] == user("Second.")


# Limits


async def test_a_reply_cut_off_by_the_token_limit_ends_the_turn_for_that_reason(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="This sentence never", finish="max-tokens"))

    agent.followup("Write an essay.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="max-tokens")
    assert agent.session.messages[-1] == assistant("This sentence never")


async def test_a_turn_stays_cut_off_even_if_a_later_step_ends_well(harness: Harness) -> None:
    agent = await harness.agent(
        Reply(text="This sentence never", finish="max-tokens", hold=True), Reply(text="Sorry.")
    )
    agent.followup("Write an essay.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.steer("Shorter, please.")
    harness.model.release()
    await idle(agent)

    assert len(harness.model.requests) == 2
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="max-tokens")


# Cancelling


async def test_cancelling_keeps_what_had_arrived_of_the_reply(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="I am thinking", hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.cancel()
    await idle(agent)

    assert types(agent.session)[-3:] == ["assistant/message", "step/end", "turn/end"]
    assert agent.session.events[-3].data == AssistantMessageEvent(
        message=assistant("I am thinking"), usage=Usage(), finish_reason=None
    )
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert agent.session.messages[-1] == assistant("I am thinking")


async def test_a_reply_that_was_cancelled_before_anything_arrived_is_not_shown_to_the_model(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.cancel()
    await idle(agent)

    interrupted = agent.session.events[-3]
    assert interrupted.data == AssistantMessageEvent(
        message=AssistantMessage(content=()), usage=Usage(), finish_reason=None
    )
    assert not interrupted.on_surface
    assert agent.session.messages == (SYSTEM, user("Think."))


async def test_cancelling_says_who_cancelled(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.cancel("budget")
    agent.cancel("user")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="budget")


async def test_cancelling_drops_the_input_that_waits(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("First.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.followup("Second.")
    agent.steer("A hint.")

    agent.cancel()
    await idle(agent)

    assert INBOX.of(agent.session) == Inbox()
    assert len(harness.model.requests) == 1
    assert [e.data for e in agent.session.events if isinstance(e.data, TurnEndEvent)] == [
        TurnEndEvent(turn=1, reason="aborted", detail="user")
    ]


async def test_cancelling_can_keep_the_input_that_waits_until_the_agent_is_woken(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True), Reply(text="Second done."))
    agent.followup("First.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.followup("Second.")

    agent.cancel(keep_inbox=True)
    await idle(agent)

    assert INBOX.of(agent.session) == Inbox(next_turn=(InboxItem(message=user("Second.")),))
    assert len(harness.model.requests) == 1

    agent.wake()
    await idle(agent)

    assert harness.model.requests[1].messages[-1] == user("Second.")
    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")


async def test_cancelling_right_after_a_task_was_given_takes_the_task_back(
    harness: Harness,
) -> None:
    agent = await harness.agent()

    agent.followup("Never mind.")
    agent.cancel()
    await idle(agent)

    assert INBOX.of(agent.session) == Inbox()
    assert "turn/start" not in types(agent.session)
    assert harness.model.requests == []


async def test_cancelling_an_agent_at_rest_only_empties_its_inbox(harness: Harness) -> None:
    agent = await harness.agent()
    agent.inject("Some context.")

    agent.cancel()
    await idle(agent)

    assert INBOX.of(agent.session) == Inbox()
    assert "turn/start" not in types(agent.session)


async def test_an_agent_takes_a_new_task_after_a_cancellation(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True), Reply(text="Hello!"))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.cancel()
    await idle(agent)

    agent.followup("Say hello.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")
    assert harness.model.requests[1].messages == (
        SYSTEM,
        user("Think."),
        assistant("thinking"),
        user("Say hello."),
    )


# Closing


async def test_closing_an_agent_ends_the_turn_that_is_running(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.followup("Later.")

    await asyncio.wait_for(agent.close(), timeout=5)

    assert agent.session.closed
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="aborted", detail="disposed"
    )
    stored = harness.sessions.read(harness.cwd, "s-1")
    assert stored.events == list(agent.session.events)
    assert len(INBOX.of(agent.session).next_turn) == 1


async def test_an_agent_whose_tasks_are_cancelled_from_outside_closes_its_turn(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    others = [task for task in asyncio.all_tasks() if task is not asyncio.current_task()]

    for task in others:
        task.cancel()
    await asyncio.gather(*others, return_exceptions=True)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="aborted", detail="disposed"
    )
    await asyncio.wait_for(agent.close(), timeout=5)
    assert agent.session.closed


async def test_a_turn_that_alone_is_cancelled_from_outside_leaves_the_agent_usable(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True), Reply(text="Hello!"))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    (turn,) = [
        task
        for task in asyncio.all_tasks()
        if getattr(task.get_coro(), "__qualname__", "") == "LoopAgent._turn"
    ]

    turn.cancel()
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="aborted", detail="disposed"
    )

    agent.followup("Say hello.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")


# Failing


async def test_a_turn_that_breaks_ends_with_an_error_and_the_agent_goes_on(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    def broken(self: Tools) -> tuple[()]:
        raise RuntimeError("tools bug")

    agent = await harness.agent(Reply(text="Hello!"))
    monkeypatch.setattr(Tools, "schemas", broken)

    with caplog.at_level(logging.ERROR, logger="tack.agent_loop"):
        agent.followup("Say hello.")
        await idle(agent)
    monkeypatch.undo()

    assert types(agent.session)[-3:] == ["step/start", "step/end", "turn/end"]
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="RuntimeError: tools bug"
    )
    assert "turn 1 of the agent 's-1' failed" in caplog.text

    agent.followup("Again.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")


async def test_an_agent_that_cannot_write_its_log_stops_and_says_why(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Think.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    monkeypatch.setattr("tack.session.store.encode_event", full)

    harness.model.release()
    with (
        caplog.at_level(logging.ERROR, logger="tack.agent_loop"),
        pytest.raises(SessionStoreError, match="No space left on device"),
    ):
        await idle(agent)
    monkeypatch.undo()

    assert "the agent 's-1' has stopped" in caplog.text
    with pytest.raises(SessionStoreError):
        await idle(agent)
    agent.wake()
    with pytest.raises(SessionStoreError):
        await idle(agent)
    await asyncio.wait_for(agent.close(), timeout=5)


async def test_history_without_instructions_gets_none_later(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    first = await harness.agent(Reply(text="Hello!"), Reply(text="Hi."), system_prompt="")
    first.followup("Say hello.")
    await idle(first)
    await first.close()

    second = await harness.agents.resume(harness.ctx, harness.spec(), "s-1")
    with caplog.at_level(logging.WARNING, logger="tack.agent_loop"):
        second.followup("Again.")
        await idle(second)

    assert harness.model.requests[1].messages[0] == user("Say hello.")
    assert "has history without instructions" in caplog.text
