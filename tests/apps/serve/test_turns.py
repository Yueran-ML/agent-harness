"""The service process: tasks, what is said on the way, stopping, and what is told of a turn."""

import asyncio
import json
import logging
from typing import Any

import pytest

from columbina.agent import STREAM, StreamFrame
from columbina.apps.serve.rpc import INVALID_PARAMS, STOPPED
from columbina.llm import BlockEnd, BlockStart, Finish, TextDelta, Usage
from columbina.llm.adapters.scripted import Reply
from columbina.session import Session, TurnEndEvent, UserMessageEvent, encode_event
from support.served import Served

LOGGER = "columbina.apps.serve.server"
HALF = chr(0xD83D)


def as_stored(session: Session) -> list[dict[str, Any]]:
    """The events of a session, each as the line of its log reads."""
    return [json.loads(encode_event(event)) for event in session.events]


def statuses(served: Served) -> list[str]:
    return [told["status"] for told in served.far.told("session.status")]


async def rested(served: Served) -> None:
    """Wait until the last word of where the agent stands is that it does not run."""
    await served.far.until(lambda: statuses(served)[-1] != "running")


def said(served: Served) -> list[tuple[str, str]]:
    """What stands in the session in the role of the user: where it came from, and the text."""
    return [
        (event.data.source, event.data.message.content[0].text)
        for event in served.agent().session.events
        if isinstance(event.data, UserMessageEvent)
    ]


# A task


async def test_a_task_opens_a_turn_and_every_event_of_it_is_told_in_its_order(
    served: Served,
) -> None:
    opened = await served.opened()
    served.harness.model.add(Reply(text="Hello!"))

    answer = await served.far.result("session/prompt", {"text": "Say hello."})
    await rested(served)

    told = served.far.told("session.event")
    session = served.agent().session
    assert answer == {}
    # The session was new, so everything that it holds was told, each event once.
    assert {one["session_id"] for one in told} == {opened["session_id"]}
    assert [one["event"] for one in told] == as_stored(session)
    assert [one["event"]["seq"] for one in told] == list(range(len(session.events)))
    assert [one["event"]["type"] for one in told][-1] == "turn/end"
    assert statuses(served) == ["idle", "running", "idle"]


async def test_a_reply_is_told_chunk_by_chunk_while_it_is_written(served: Served) -> None:
    opened = await served.opened()
    served.harness.model.add(
        Reply(text="Hello!", usage=Usage(input_tokens=7, output_tokens=2)),
    )

    await served.far.result("session/prompt", {"text": "Say hello."})
    await rested(served)

    frames = served.far.told("session.stream")
    assert {(one["session_id"], one["turn"], one["step"]) for one in frames} == {
        (opened["session_id"], 1, 1)
    }
    assert list(frames[0]) == ["session_id", "turn", "step", "chunk"]
    chunks = [one["chunk"] for one in frames]
    assert [chunk["type"] for chunk in chunks] == [
        "block-start",
        "text-delta",
        "block-end",
        "usage",
        "finish",
    ]
    assert chunks[0] == {
        "type": "block-start",
        "index": 0,
        "kind": "text",
        "call_id": None,
        "name": None,
    }
    assert chunks[1] == {"type": "text-delta", "index": 0, "text": "Hello!"}
    assert chunks[-1] == {"type": "finish", "reason": "stop", "failure": None}
    # The chunks are told before the event that holds the whole reply.
    heard = served.far.heard
    last_chunk = max(
        place for place, one in enumerate(heard) if one.get("method") == "session.stream"
    )
    whole = next(
        place
        for place, one in enumerate(heard)
        if one.get("method") == "session.event"
        and one["params"]["event"]["type"] == "assistant/message"
    )
    assert last_chunk < whole


async def test_half_a_character_in_a_chunk_is_told_as_it_came(served: Served) -> None:
    await served.opened()
    served.harness.model.add(
        [
            BlockStart(index=0, kind="text"),
            TextDelta(index=0, text=f"a{HALF}"),
            BlockEnd(index=0),
            Finish(reason="stop"),
        ]
    )

    await served.far.result("session/prompt", {"text": "Say it."})
    await rested(served)

    # No line of UTF-8 can hold it. The line spells it out, and the message is the same.
    texts = [
        one["chunk"]["text"]
        for one in served.far.told("session.stream")
        if one["chunk"]["type"] == "text-delta"
    ]
    assert texts == [f"a{HALF}"]


async def test_a_second_task_waits_for_the_first_and_the_agent_is_not_said_to_rest_between(
    served: Served,
) -> None:
    await served.opened()
    model = served.harness.model
    model.add(Reply(text="One.", hold=True), Reply(text="Two."))

    await served.far.result("session/prompt", {"text": "First."})
    await model.held()
    await served.far.result("session/prompt", {"text": "Second."})
    model.release()
    await rested(served)

    assert said(served) == [("user", "First."), ("user", "Second.")]
    ends = [
        one["event"]["data"]
        for one in served.far.told("session.event")
        if one["event"]["type"] == "turn/end"
    ]
    assert [end["turn"] for end in ends] == [1, 2]
    # Two turns, and one stretch of work.
    assert statuses(served) == ["idle", "running", "idle"]


@pytest.mark.parametrize("method", ["session/prompt", "session/steer"])
@pytest.mark.parametrize(
    ("params", "wrong"),
    [
        ({"text": ""}, "There is nothing in the text to give to the agent."),
        ({"text": "  \n\t "}, "There is nothing in the text to give to the agent."),
        ({}, "The params of {} are not right: text: Field required."),
        ({"text": 7}, "The params of {} are not right: text: Input should be a valid string."),
        ({"text": ["Hi"]}, "The params of {} are not right: text: Input should be a valid string."),
    ],
)
async def test_nothing_is_given_to_the_agent_where_there_is_nothing_to_give(
    served: Served, method: str, params: dict[str, object], wrong: str
) -> None:
    await served.opened()

    assert await served.far.refusal(method, params) == (INVALID_PARAMS, wrong.format(method))
    assert served.agent().at_rest
    assert len(served.agent().session.events) == 0


async def test_what_is_given_to_the_agent_is_the_text_without_the_space_around_it(
    served: Served,
) -> None:
    await served.opened()
    served.harness.model.add(Reply(text="Done."))

    await served.far.result("session/prompt", {"text": "  \n Fix the parser.\n\nAll of it. \n"})
    await rested(served)

    assert said(served) == [("user", "Fix the parser.\n\nAll of it.")]


async def test_a_task_that_begins_with_a_slash_is_a_task_and_no_command(served: Served) -> None:
    await served.opened()
    served.harness.model.add(Reply(text="That is a path."))

    await served.far.result("session/prompt", {"text": "/help is what I typed"})
    await rested(served)

    assert said(served) == [("user", "/help is what I typed")]
    assert len(served.harness.model.requests) == 1


# What is said on the way


async def test_what_is_said_to_a_turn_on_its_way_is_read_at_its_next_step(served: Served) -> None:
    await served.opened()
    model = served.harness.model
    model.add(Reply(text="Reading a.txt.", hold=True), Reply(text="Reading b.txt."))

    await served.far.result("session/prompt", {"text": "Read a.txt."})
    await model.held()
    answer = await served.far.result("session/steer", {"text": " No, read b.txt. "})
    model.release()
    await rested(served)

    assert answer == {}
    assert said(served) == [("user", "Read a.txt."), ("steer", "No, read b.txt.")]
    # One turn of two steps, and no second turn.
    turns = [
        one["event"]["data"]["turn"]
        for one in served.far.told("session.event")
        if one["event"]["type"] == "turn/end"
    ]
    assert turns == [1]
    assert len(model.requests) == 2


async def test_what_is_said_on_the_way_opens_a_turn_where_none_is_running(served: Served) -> None:
    await served.opened()
    served.harness.model.add(Reply(text="Hello!"))

    await served.far.result("session/steer", {"text": "Say hello."})
    await rested(served)

    assert said(served) == [("steer", "Say hello.")]
    assert statuses(served) == ["idle", "running", "idle"]


async def test_a_task_and_what_is_said_after_it_are_taken_in_the_order_in_which_they_came(
    served: Served,
) -> None:
    await served.opened()
    model = served.harness.model
    model.add(Reply(text="One."), Reply(text="Two."))

    # Neither waits for the answer to the one before it.
    first = served.far.ask("session/prompt", {"text": "A task."})
    second = served.far.ask("session/steer", {"text": "And this on the way."})
    await served.far.answer(first)
    await served.far.answer(second)
    await rested(served)

    assert said(served) == [("user", "A task."), ("steer", "And this on the way.")]


# Stopping


async def test_a_turn_is_stopped_and_what_waited_for_it_is_dropped(served: Served) -> None:
    await served.opened()
    model = served.harness.model
    model.add(Reply(text="Thinking...", hold=True), Reply(text="Never said."))

    await served.far.result("session/prompt", {"text": "Think."})
    await model.held()
    await served.far.result("session/prompt", {"text": "And then this."})
    answer = await served.far.result("session/cancel")
    await rested(served)

    # The answer says that it was asked for, and no more. The log says what came of it.
    assert answer == {}
    ended = [
        event.data
        for event in served.agent().session.events
        if isinstance(event.data, TurnEndEvent)
    ]
    assert ended == [TurnEndEvent(turn=1, reason="aborted", detail="user")]
    assert len(model.requests) == 1
    assert statuses(served) == ["idle", "running", "idle"]
    # The agent takes a task again.
    await served.far.result("session/prompt", {"text": "Now this."})
    await rested(served)
    assert said(served)[-1] == ("user", "Now this.")


async def test_stopping_does_nothing_where_nothing_runs(served: Served) -> None:
    await served.opened()

    assert await served.far.result("session/cancel") == {}

    assert len(served.agent().session.events) == 0
    assert statuses(served) == ["idle"]


# An agent that has stopped for good


async def stopped(served: Served, monkeypatch: pytest.MonkeyPatch) -> None:
    """Have the agent of the open session stop for good: its log cannot be written."""

    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    model = served.harness.model
    model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await model.held()
    monkeypatch.setattr("columbina.session.store.encode_event", full)
    model.release()
    await rested(served)
    monkeypatch.undo()


async def test_an_agent_that_has_stopped_for_good_is_said_to_have_and_takes_nothing_more(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await served.opened()

    with caplog.at_level(logging.CRITICAL):
        await stopped(served, monkeypatch)

    last = served.far.told("session.status")[-1]
    assert last["status"] == "stopped"
    assert "No space left on device" in last["detail"]
    why = f"The agent has stopped: {last['detail']}."
    assert await served.far.refusal("session/prompt", {"text": "Go on."}) == (STOPPED, why)
    assert await served.far.refusal("session/steer", {"text": "Go on."}) == (STOPPED, why)
    assert await served.far.refusal("session/command", {"line": "/help"}) == (STOPPED, why)
    # What it holds can still be read, and it can still be told to stop.
    assert (await served.far.result("session/events", {"from": 0}))["events"]
    assert await served.far.result("session/cancel") == {}
    # Leaving says so too, to whoever started the program.
    with caplog.at_level(logging.CRITICAL):
        served.far.hang_up()
        assert await served.left() == 1


async def test_another_session_can_be_opened_after_an_agent_has_stopped_for_good(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await served.opened("broken")
    with caplog.at_level(logging.CRITICAL):
        await stopped(served, monkeypatch)

    await served.far.result("session/open", {"session_id": "whole"})
    served.harness.model.add(Reply(text="Hello!"))
    await served.far.result("session/prompt", {"text": "Say hello."})
    await rested(served)

    assert [
        (told["session_id"], told["status"]) for told in served.far.told("session.status")[-3:]
    ] == [("whole", "idle"), ("whole", "running"), ("whole", "idle")]
    # The program leaves with the code that says a session could not be written to its
    # end, although the one that is open could.
    with caplog.at_level(logging.ERROR, logger=LOGGER):
        served.far.hang_up()
        assert await served.left() == 1
    assert [record.getMessage() for record in caplog.records if record.name == LOGGER] == [
        "the agent of the session broken stopped for good: "
        "the session 'broken' can no longer be written: [Errno 28] No space left on device"
    ]


# What is not this session's own


async def test_a_reply_that_is_written_elsewhere_is_not_told(served: Served) -> None:
    await served.opened("mine")
    harness = served.harness
    other = await harness.agents.create(harness.ctx, harness.spec(), session_id="other")

    # Outside every scope, where every listener hears it.
    frame = StreamFrame(agent=other, turn=1, step=1, chunk=TextDelta(index=0, text="Psst."))
    harness.ctx.emit(STREAM, frame)
    await asyncio.sleep(0.05)

    assert served.far.told("session.stream") == []
    await other.close()
