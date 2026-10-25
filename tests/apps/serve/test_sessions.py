"""The service process: the sessions of a directory, opening one, and reading what it holds."""

import asyncio
import json
import logging
import os
import re
import threading
import time
from typing import Any

import pytest

from columbina.agent.inbox import INBOX, InboxItem, InboxSplicedEvent
from columbina.apps.serve import server as server_module
from columbina.apps.serve.rpc import CANNOT_OPEN, INVALID_PARAMS, NO_SESSION
from columbina.llm import TextBlock, UserMessage
from columbina.llm.adapters.scripted import Reply
from columbina.session import (
    EventData,
    Listed,
    Session,
    SessionEvent,
    Sessions,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
    encode_event,
)
from columbina.session.store import SESSION_FILE
from support.served import Served
from support.terminal import until

LOGGER = "columbina.apps.serve.server"
SECOND = 1_000_000_000
NO_SESSION_OPEN = (NO_SESSION, "No session is open. session/open opens one.")


def said(text: str) -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)))


async def keep(served: Served, session_id: str, *events: EventData, at: int = 1000) -> int:
    """Store a session that holds these events and was last written at second ``at``.

    Returns:
        When the session was created.
    """
    harness = served.harness
    session = harness.sessions.create(harness.ctx, harness.cwd, session_id=session_id)
    for event in events:
        session.append(event)
    await session.flush()
    session.close()
    file = harness.sessions.directory(harness.cwd, session_id) / SESSION_FILE
    os.utime(file, ns=(at * SECOND, at * SECOND))
    return session.created_at


def as_stored(session: Session) -> list[dict[str, Any]]:
    """The events of a session, each as the line of its log reads."""
    return [json.loads(encode_event(event)) for event in session.events]


def after(served: Served, answer: dict[str, Any]) -> list[dict[str, Any]]:
    """What the process sent after one of its messages."""
    heard = served.far.heard
    return heard[heard.index(answer) + 1 :]


# The sessions of the directory


async def test_the_stored_sessions_of_the_directory_are_listed_the_last_used_first(
    served: Served,
) -> None:
    await keep(served, "older", said("Fix the parser."), at=1000)
    await keep(served, "newer", said("Write the notes.\nAnd the changelog."), at=3000)
    await keep(served, "silent", TurnStartEvent(turn=1), at=2000)
    await served.greeted()

    # No session has to be open for this.
    assert await served.far.result("session/list") == {
        "sessions": [
            {"id": "newer", "modified_ms": 3_000_000, "first_input": "Write the notes."},
            {"id": "silent", "modified_ms": 2_000_000, "first_input": None},
            {"id": "older", "modified_ms": 1_000_000, "first_input": "Fix the parser."},
        ]
    }


async def test_the_session_of_a_subagent_is_not_listed_among_the_conversations(
    served: Served,
) -> None:
    harness = served.harness
    await keep(served, "talk", said("Fix the parser."))
    child = harness.sessions.create(
        harness.ctx, harness.cwd, session_id="child", parent="talk", delegation_depth=1
    )
    child.append(said("Find where the parser is."))
    await child.flush()
    child.close()
    await served.greeted()

    listed = await served.far.result("session/list")

    assert [one["id"] for one in listed["sessions"]] == ["talk"]


async def test_a_list_that_takes_long_does_not_hold_up_what_is_asked_for_after_it(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    await served.greeted()
    looked = threading.Event()
    let_go = threading.Event()
    listed = Sessions.stored

    def slowly(self: Sessions, cwd: object) -> list[Listed]:
        looked.set()
        let_go.wait(10)
        return listed(self, cwd)  # pyright: ignore[reportArgumentType]

    monkeypatch.setattr(Sessions, "stored", slowly)

    slow = served.far.ask("session/list")
    await asyncio.to_thread(looked.wait, 10)
    opened = await served.far.result("session/open", {"session_id": "meanwhile"})
    before = served.far.answers(slow)
    let_go.set()
    answer = await served.far.answer(slow)

    assert opened["session_id"] == "meanwhile"
    assert before == []
    assert answer["result"] == {"sessions": []}


# Opening one


async def test_a_session_is_opened_anew_and_where_its_agent_stands_is_told_after_the_answer(
    served: Served,
) -> None:
    before = time.time() * 1000
    await served.greeted()

    asked = served.far.ask("session/open", {})
    answer = await served.far.answer(asked)
    await served.far.until(lambda: bool(served.far.told("session.status")))

    opened = answer["result"]
    assert re.fullmatch(r"\d{8}T\d{6}-[0-9a-f]{8}", opened["session_id"])
    assert (opened["resumed"], opened["events"]) == (False, 0)
    assert before - 1000 <= opened["created_at"] <= time.time() * 1000 + 1000
    assert list(opened) == ["session_id", "resumed", "created_at", "events"]
    # The answer comes first. Then, once, where the agent stands.
    assert after(served, answer) == [
        {
            "jsonrpc": "2.0",
            "method": "session.status",
            "params": {
                "session_id": opened["session_id"],
                "status": "idle",
                "detail": None,
                "model": {
                    "provider": "scripted",
                    "model": "script",
                    "context_window": None,
                    "effort": "default",
                    "efforts": ["default", "ultra"],
                },
                "sandbox": None,
                "approval": None,
            },
        }
    ]
    assert served.agent().id == opened["session_id"]


async def test_a_session_is_opened_under_the_id_that_is_asked_for(served: Served) -> None:
    opened = await served.opened("mine-1")

    assert (opened["session_id"], opened["resumed"], opened["events"]) == ("mine-1", False, 0)


async def test_a_stored_session_is_taken_up_and_what_it_holds_can_be_read(served: Served) -> None:
    created = await keep(
        served,
        "s-old",
        TurnStartEvent(turn=1),
        said("Fix the parser."),
        TurnEndEvent(turn=1, reason="completed"),
    )

    opened = await served.opened("s-old")
    held = await served.far.result("session/events", {"from": 0})

    # Three events were stored, and taking the session up wrote down that it was taken up.
    assert opened == {"session_id": "s-old", "resumed": True, "created_at": created, "events": 4}
    assert held == {"events": as_stored(served.agent().session), "next": None}
    assert [event["type"] for event in held["events"]] == [
        "turn/start",
        "user/message",
        "turn/end",
        "session/end-seed",
    ]


@pytest.mark.parametrize("session_id", ["../escape", "", "a b", "-a", "x" * 101])
async def test_an_id_that_cannot_name_a_session_is_refused(served: Served, session_id: str) -> None:
    await served.greeted()

    code, message = await served.far.refusal("session/open", {"session_id": session_id})

    assert code == INVALID_PARAMS
    assert "is not a usable session id" in message
    assert list(served.harness.agents.live) == []


@pytest.mark.parametrize("session_id", [7, True, ["a"]])
async def test_an_id_that_is_no_text_is_refused(served: Served, session_id: object) -> None:
    await served.greeted()

    code, message = await served.far.refusal("session/open", {"session_id": session_id})

    assert code == INVALID_PARAMS
    assert message.startswith(
        "The params of session/open are not right: session_id: Input should be a valid string"
    )


async def test_a_session_that_is_open_elsewhere_cannot_be_opened_and_the_other_one_stays(
    served: Served,
) -> None:
    harness = served.harness
    await served.opened("mine")
    theirs = harness.sessions.create(harness.ctx, harness.cwd, session_id="theirs")
    theirs.append(said("Somebody else is at work here."))

    code, message = await served.far.refusal("session/open", {"session_id": "theirs"})

    assert code == CANNOT_OPEN
    assert message.startswith("The session cannot be opened: ")
    assert "theirs" in message
    # The conversation is where it was, and goes on there.
    harness.model.add(Reply(text="Still here."))
    assert await served.far.result("session/prompt", {"text": "Are you there?"}) == {}
    await served.far.until(
        lambda: (
            served.far.told("session.status")[-1]["status"] == "idle"
            and len(served.far.told("session.status")) >= 3
        )
    )
    assert {told["session_id"] for told in served.far.told("session.event")} == {"mine"}
    theirs.close()


async def test_a_session_that_cannot_be_read_cannot_be_opened(served: Served) -> None:
    harness = served.harness
    await keep(served, "torn", said("Fix the parser."))
    file = harness.sessions.directory(harness.cwd, "torn") / SESSION_FILE
    file.write_bytes(b"this is no session\n")
    await served.greeted()

    code, message = await served.far.refusal("session/open", {"session_id": "torn"})

    assert code == CANNOT_OPEN
    assert message.startswith("The session cannot be opened: ")
    assert list(harness.agents.live) == []
    # And nothing is open, so nothing can be asked of a session.
    assert await served.far.refusal("session/events", {"from": 0}) == NO_SESSION_OPEN


async def test_opening_another_session_ends_the_one_that_was_open_and_tells_no_more_of_it(
    served: Served,
) -> None:
    harness = served.harness
    await served.opened("first")
    harness.model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await harness.model.held()

    asked = served.far.ask("session/open", {"session_id": "second"})
    answer = await served.far.answer(asked)

    def ended() -> list[TurnEndEvent]:
        return [
            event.data
            for event in harness.sessions.read(harness.cwd, "first").events
            if isinstance(event.data, TurnEndEvent)
        ]

    # The first one is ended after the answer, on the side.
    await until(lambda: list(harness.agents.live) == ["second"] and ended() != [])

    assert answer["result"]["session_id"] == "second"
    # The turn of the first one was stopped, and its session is closed and stored.
    assert ended() == [TurnEndEvent(turn=1, reason="aborted", detail="user")]
    # Nothing of that was told: from the answer on, the conversation is the second one.
    assert {message["params"]["session_id"] for message in after(served, answer)} == {"second"}
    assert [message["method"] for message in after(served, answer)] == ["session.status"]


def waits_in(served: Served, session_id: str) -> list[str]:
    """What waits unread in a stored session, as its log has it: tasks first, then the rest."""
    waiting: dict[str, tuple[InboxItem, ...]] = {"next-turn": (), "next-step": ()}
    for event in served.harness.sessions.read(served.harness.cwd, session_id).events:
        data = event.data
        if isinstance(data, InboxSplicedEvent):
            waiting[data.queue] = (*waiting[data.queue][data.removed :], *data.added)
    items = (*waiting["next-turn"], *waiting["next-step"])
    return [block.text for item in items for block in item.message.content]


async def test_what_waited_in_a_session_that_is_left_in_the_middle_of_a_turn_is_dropped_untold(
    served: Served,
) -> None:
    harness = served.harness
    await served.opened("first")
    harness.model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await harness.model.held()
    await served.far.result("session/steer", {"text": "Think of this too."})
    await until(lambda: INBOX.of(harness.agents.live["first"].session).next_step != ())

    answer = await served.far.answer(served.far.ask("session/open", {"session_id": "second"}))
    await until(lambda: list(harness.agents.live) == ["second"])

    # The turn was stopped as a turn is stopped, and what waited went with it.
    assert waits_in(served, "first") == []
    changes = [
        (event.data.removed, len(event.data.added))
        for event in harness.sessions.read(harness.cwd, "first").events
        if isinstance(event.data, InboxSplicedEvent) and event.data.queue == "next-step"
    ]
    assert changes == [(0, 1), (1, 0)]
    # Whoever holds the conversation is not told. What waited is theirs to remember.
    assert {message["params"]["session_id"] for message in after(served, answer)} == {"second"}


async def test_what_waits_in_a_session_at_rest_still_waits_after_the_session_is_left(
    served: Served,
) -> None:
    harness = served.harness
    waiting = InboxItem(
        message=UserMessage(content=(TextBlock(text="Read this next."),)), source="steer"
    )
    # A run that was killed leaves this behind: something was said and never read.
    await keep(
        served, "first", said("Hello."), InboxSplicedEvent(queue="next-step", added=(waiting,))
    )
    await served.opened("first")
    assert harness.agents.live["first"].at_rest

    await served.opened("second")
    await until(lambda: list(harness.agents.live) == ["second"])

    assert waits_in(served, "first") == ["Read this next."]


async def test_the_session_that_is_open_can_be_opened_again(served: Served) -> None:
    harness = served.harness
    await served.opened("mine")
    harness.model.add(Reply(text="Hello!"))
    await served.far.result("session/prompt", {"text": "Say hello."})
    await served.far.until(lambda: served.far.told("session.status")[-1]["status"] == "idle")
    first = served.agent()
    stored = len(first.session.events)

    again = await served.far.result("session/open", {"session_id": "mine"})

    # It was closed and taken up, which is written down as one more event.
    assert again == {
        "session_id": "mine",
        "resumed": True,
        "created_at": first.session.created_at,
        "events": stored + 1,
    }
    assert served.agent() is not first
    assert first.session.closed


async def test_a_session_can_be_opened_right_after_the_one_before_it(served: Served) -> None:
    await served.greeted()

    # Three in a row, and none waits for the answer to the one before.
    asked = [served.far.ask("session/open", {"session_id": name}) for name in ("a", "b", "a")]
    answers = [await served.far.answer(one) for one in asked]

    assert [answer["result"]["session_id"] for answer in answers] == ["a", "b", "a"]
    # Nothing was said in the first one, so nothing of it was stored to be taken up.
    assert [answer["result"]["resumed"] for answer in answers] == [False, False, False]
    await until(lambda: list(served.harness.agents.live) == ["a"])


async def test_the_program_can_leave_while_the_session_before_is_still_being_ended(
    served: Served,
) -> None:
    harness = served.harness
    await served.opened("first")
    harness.model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await harness.model.held()

    # Neither waits for the answer to the other.
    opening = served.far.ask("session/open", {"session_id": "second"})
    leaving = served.far.ask("shutdown")

    assert await served.left() == 0
    assert served.far.answers(opening)[0]["result"]["session_id"] == "second"
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]
    # Both are closed, and the turn of the first one was written down as stopped.
    assert list(harness.agents.live) == []
    ended = [
        event.data
        for event in harness.sessions.read(harness.cwd, "first").events
        if isinstance(event.data, TurnEndEvent)
    ]
    assert ended == [TurnEndEvent(turn=1, reason="aborted", detail="user")]


async def test_the_program_does_not_leave_before_a_session_that_was_left_has_come_to_its_end(
    served: Served,
) -> None:
    await served.opened("first")
    first = served.agent()
    let_go = asyncio.Event()

    async def slowly() -> None:
        await let_go.wait()

    # Something of the first session takes its time to be put away.
    first.ctx.effect(lambda: slowly)

    await served.far.result("session/open", {"session_id": "second"})
    leaving = served.far.ask("shutdown")
    await asyncio.sleep(0.1)
    still_there = not served.running.done()
    let_go.set()

    assert await served.left() == 0
    # The program waited for the first session, although the conversation had left it.
    assert still_there
    assert first.session.closed
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]


async def test_a_session_that_could_not_be_ended_properly_is_told_by_the_exit_code(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    harness = served.harness
    await served.opened("first")
    harness.model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await harness.model.held()

    def full(event: SessionEvent[EventData]) -> str:
        # The end of the turn that is stopped cannot be written. Everything else can.
        if isinstance(event.data, TurnEndEvent):
            raise OSError(28, "No space left on device")
        return encode_event(event)

    monkeypatch.setattr("columbina.session.store.encode_event", full)

    with caplog.at_level(logging.CRITICAL), caplog.at_level(logging.ERROR, logger=LOGGER):
        # The first session is ended on the side, when nothing of it is told any more.
        opened = await served.far.result("session/open", {"session_id": "second"})
        await until(lambda: list(harness.agents.live) == ["second"])
        told = [one["status"] for one in served.far.told("session.status")]
        served.far.hang_up()
        code = await served.left()

    assert opened["session_id"] == "second"
    # Nobody was told that the first one stopped for good: the conversation had left it.
    assert "stopped" not in told
    # So the exit code tells it, and the log says which session it was.
    assert code == 1
    assert [record.getMessage() for record in caplog.records if record.name == LOGGER] == [
        "the agent of the session first stopped for good: "
        "the session 'first' cannot be written: [Errno 28] No space left on device"
    ]


def disk_full(monkeypatch: pytest.MonkeyPatch) -> None:
    """Have nothing be written to any session from now on."""

    def refuse(event: SessionEvent[EventData]) -> str:
        raise OSError(28, "No space left on device")

    monkeypatch.setattr("columbina.session.store.encode_event", refuse)


async def thinking_with_a_task_waiting(served: Served) -> None:
    """Have the agent of the open session be in a turn, with a second task waiting for it."""
    model = served.harness.model
    model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await model.held()
    # Stopping the turn drops this, and has to write down that it did.
    await served.far.result("session/prompt", {"text": "And then this."})


async def test_a_session_that_cannot_be_told_to_stop_is_ended_all_the_same_when_the_program_leaves(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    harness = served.harness
    await served.opened("mine")
    await thinking_with_a_task_waiting(served)
    disk_full(monkeypatch)

    with caplog.at_level(logging.CRITICAL), caplog.at_level(logging.ERROR, logger=LOGGER):
        leaving = served.far.ask("shutdown")
        code = await served.left()

    # The order to leave is done and answered, although the turn could not be told to stop.
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]
    assert list(harness.agents.live) == []
    # And the exit code says that a session could not be written to its end.
    assert code == 1
    # What is told is what went wrong first: the order to stop could not be written down.
    assert [record.getMessage() for record in caplog.records if record.name == LOGGER] == [
        "the agent of the session mine stopped for good: "
        "the session 'mine' cannot be written: [Errno 28] No space left on device"
    ]


async def test_a_session_that_cannot_be_told_to_stop_is_ended_all_the_same_when_it_is_left(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    harness = served.harness
    await served.opened("first")
    first = served.agent().session
    await thinking_with_a_task_waiting(served)
    disk_full(monkeypatch)

    with caplog.at_level(logging.CRITICAL), caplog.at_level(logging.ERROR, logger=LOGGER):
        opened = await served.far.result("session/open", {"session_id": "second"})
        await until(lambda: list(harness.agents.live) == ["second"])
        # There is room again by the time the second session is closed.
        monkeypatch.undo()
        served.far.hang_up()
        code = await served.left()

    assert opened["session_id"] == "second"
    assert first.closed
    assert code == 1
    assert [record.getMessage() for record in caplog.records if record.name == LOGGER] == [
        "the agent of the session first stopped for good: "
        "the session 'first' cannot be written: [Errno 28] No space left on device"
    ]


# Reading what a session holds


async def many(served: Served, count: int) -> None:
    """Open a stored session that holds this many things that a user said."""
    await keep(served, "long", *(said(f"Line {number}.") for number in range(count)))
    await served.opened("long")


async def test_what_a_session_holds_is_read_a_page_at_a_time(served: Served) -> None:
    await many(served, 9)
    held = as_stored(served.agent().session)

    first = await served.far.result("session/events", {"from": 0, "limit": 4})
    second = await served.far.result("session/events", {"from": first["next"], "limit": 4})
    third = await served.far.result("session/events", {"from": second["next"], "limit": 4})

    # Nine that were stored, and the one that says that the session was taken up.
    assert len(held) == 10
    assert (first["next"], second["next"], third["next"]) == (4, 8, None)
    assert first["events"] + second["events"] + third["events"] == held
    assert [len(page["events"]) for page in (first, second, third)] == [4, 4, 2]


async def test_a_page_that_ends_where_the_session_ends_says_that_nothing_follows(
    served: Served,
) -> None:
    await many(served, 3)

    exact = await served.far.result("session/events", {"from": 0, "limit": 4})
    beyond = await served.far.result("session/events", {"from": 4})
    far_beyond = await served.far.result("session/events", {"from": 4000})

    assert (len(exact["events"]), exact["next"]) == (4, None)
    assert beyond == far_beyond == {"events": [], "next": None}


async def test_a_page_has_five_hundred_events_at_most_whatever_is_asked_for(
    served: Served,
) -> None:
    await many(served, 520)

    unasked = await served.far.result("session/events", {"from": 0})
    greedy = await served.far.result("session/events", {"from": 0, "limit": 100_000})
    rest = await served.far.result("session/events", {"from": 500})

    assert (len(unasked["events"]), unasked["next"]) == (500, 500)
    assert greedy == unasked
    assert (len(rest["events"]), rest["next"]) == (21, None)


async def test_a_page_is_not_made_longer_than_a_line_should_be(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    await many(served, 6)
    held = as_stored(served.agent().session)
    one = len(encode_event(served.agent().session.events[0]))
    # Room for two events and a half.
    monkeypatch.setattr(server_module, "PAGE_BYTES", 2 * one + one // 2)

    first = await served.far.result("session/events", {"from": 0})
    second = await served.far.result("session/events", {"from": 2})

    assert (first["events"], first["next"]) == (held[:2], 2)
    assert (second["events"], second["next"]) == (held[2:4], 4)


async def test_an_event_that_is_longer_than_a_page_should_be_is_given_by_itself(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    await many(served, 3)
    held = as_stored(served.agent().session)
    monkeypatch.setattr(server_module, "PAGE_BYTES", 10)

    pages = [await served.far.result("session/events", {"from": start}) for start in range(4)]

    assert [page["events"] for page in pages] == [[event] for event in held]
    assert [page["next"] for page in pages] == [1, 2, 3, None]


@pytest.mark.parametrize(
    ("params", "wrong"),
    [
        ({}, "from: Field required"),
        ({"from": -1}, "from: Input should be greater than or equal to 0"),
        ({"from": "0"}, "from: Input should be a valid integer"),
        ({"from": 0, "limit": 0}, "limit: Input should be greater than or equal to 1"),
        ({"from": 0, "limit": 2.5}, "limit: Input should be a valid integer"),
    ],
)
async def test_a_page_that_is_asked_for_wrongly_is_refused(
    served: Served, params: dict[str, object], wrong: str
) -> None:
    await many(served, 3)

    code, message = await served.far.refusal("session/events", params)

    assert code == INVALID_PARAMS
    assert message.startswith(f"The params of session/events are not right: {wrong}")


# Without a session


@pytest.mark.parametrize(
    ("method", "params"),
    [
        ("session/events", {"from": 0}),
        ("session/prompt", {"text": "Hi"}),
        ("session/steer", {"text": "Hi"}),
        ("session/cancel", None),
        ("session/command", {"line": "/help"}),
    ],
)
async def test_what_is_asked_of_a_session_is_refused_while_none_is_open(
    served: Served, method: str, params: dict[str, object] | None
) -> None:
    await served.greeted()

    assert await served.far.refusal(method, params) == NO_SESSION_OPEN
