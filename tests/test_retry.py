"""The retry plugin: a request that failed for a passing reason is made again."""

import asyncio
import random

import pytest
from pydantic import ValidationError

import columbina.retry
from columbina.llm import LlmFailure
from columbina.llm.adapters.scripted import Reply
from columbina.retry import Config, RetryEvent, delay_for
from columbina.session import (
    SESSION_EVENT,
    AssistantAttemptEvent,
    Committed,
    Session,
    TurnEndEvent,
    UnknownEvent,
)
from support.loop import Harness, assistant, idle, types

BUSY = LlmFailure(message="overloaded", code="SERVER", status=529)
QUICK = {"base_delay_seconds": 0.001, "max_delay_seconds": 0.004, "jitter": 0}


def retries(session: Session) -> list[RetryEvent]:
    return [event.data for event in session.events if isinstance(event.data, RetryEvent)]


async def test_a_request_that_failed_for_a_passing_reason_is_made_again(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.retry, QUICK)
    agent = await harness.agent(Reply(text="I w", failure=BUSY), Reply(text="Hello!"))

    agent.followup("Say hello.")
    await idle(agent)

    assert types(agent.session, 5) == [
        "step/start",
        "request/header",
        "assistant/attempt",
        "llm/retry",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    assert retries(agent.session) == [
        RetryEvent(turn=1, step=1, attempt=1, code="SERVER", delay_ms=1)
    ]
    assert agent.session.messages[-1] == assistant("Hello!")
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_after_so_many_attempts_the_failure_stands(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.retry, {**QUICK, "max_attempts": 3})
    agent = await harness.agent(*[Reply(failure=BUSY)] * 3, Reply(text="never"))

    agent.followup("Say hello.")
    await idle(agent)

    attempts = [e for e in agent.session.events if isinstance(e.data, AssistantAttemptEvent)]
    assert len(attempts) == 3
    assert [(retry.attempt, retry.delay_ms) for retry in retries(agent.session)] == [(1, 1), (2, 2)]
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="SERVER: overloaded"
    )
    assert harness.model.remaining == 1


@pytest.mark.parametrize("code", ["AUTH", "INVALID_REQUEST", "CONTEXT_WINDOW_EXCEEDED", "INTERNAL"])
async def test_a_failure_that_will_not_pass_is_not_tried_again(harness: Harness, code: str) -> None:
    await harness.ctx.plugin(columbina.retry, QUICK)
    agent = await harness.agent(
        Reply(failure=LlmFailure(message="no", code=code)), Reply(text="never")
    )

    agent.followup("Say hello.")
    await idle(agent)

    assert retries(agent.session) == []
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail=f"{code}: no"
    )


@pytest.mark.parametrize(
    "code", ["EMPTY_RESPONSE", "RATE_LIMIT", "SERVER", "TIMEOUT", "TRANSPORT", "STREAM_CLOSED"]
)
async def test_these_failures_pass(harness: Harness, code: str) -> None:
    await harness.ctx.plugin(columbina.retry, QUICK)
    agent = await harness.agent(
        Reply(failure=LlmFailure(message="wait", code=code)), Reply(text="Hello!")
    )

    agent.followup("Say hello.")
    await idle(agent)

    assert [retry.code for retry in retries(agent.session)] == [code]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_which_failures_are_tried_again_can_be_configured(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.retry, {**QUICK, "codes": ["FLAKY"]})
    agent = await harness.agent(
        Reply(failure=LlmFailure(message="hm", code="FLAKY")), Reply(failure=BUSY)
    )

    agent.followup("Say hello.")
    await idle(agent)

    assert [retry.code for retry in retries(agent.session)] == ["FLAKY"]
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="SERVER: overloaded"
    )


async def test_cancelling_during_the_wait_ends_the_turn(harness: Harness) -> None:
    await harness.ctx.plugin(
        columbina.retry, {"base_delay_seconds": 30, "max_delay_seconds": 60, "jitter": 0}
    )
    agent = await harness.agent(Reply(failure=BUSY), Reply(text="never"))
    waiting = asyncio.Event()

    def watch(committed: Committed) -> None:
        if isinstance(committed.event.data, RetryEvent):
            waiting.set()

    agent.ctx.on(SESSION_EVENT, watch)
    agent.followup("Say hello.")
    await asyncio.wait_for(waiting.wait(), timeout=5)

    agent.cancel()
    await idle(agent)

    assert retries(agent.session) == [
        RetryEvent(turn=1, step=1, attempt=1, code="SERVER", delay_ms=30_000)
    ]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert len(harness.model.requests) == 1


async def test_a_session_with_retries_can_be_read_without_the_plugin(harness: Harness) -> None:
    plugin = await harness.ctx.plugin(columbina.retry, QUICK)
    agent = await harness.agent(Reply(failure=BUSY), Reply(text="Hello!"))
    agent.followup("Say hello.")
    await idle(agent)
    await agent.close()
    await plugin.dispose()

    stored = harness.sessions.read(harness.cwd, "s-1")

    unknown = [event for event in stored.events if isinstance(event.data, UnknownEvent)]
    assert [event.type for event in unknown] == ["llm/retry"]


# The wait


def wait(attempt: int, failure: LlmFailure = BUSY, **changes: object) -> float:
    config = Config.model_validate({"jitter": 0, **changes})
    return delay_for(failure, attempt, config, random.Random(0))


def test_the_wait_doubles_with_every_attempt_up_to_a_limit() -> None:
    assert [wait(attempt) for attempt in range(1, 8)] == [0.5, 1.0, 2.0, 4.0, 8.0, 10.0, 10.0]


def test_the_wait_follows_the_config() -> None:
    assert [wait(attempt, base_delay_seconds=1, max_delay_seconds=3) for attempt in (1, 2, 3)] == [
        1.0,
        2.0,
        3.0,
    ]


def test_a_wait_that_the_provider_asks_for_is_kept_if_it_is_within_the_limit() -> None:
    asked = LlmFailure(message="slow down", code="RATE_LIMIT", retry_after_ms=2500)
    too_long = LlmFailure(message="slow down", code="RATE_LIMIT", retry_after_ms=60_000)

    assert wait(1, asked) == 2.5
    assert wait(4, asked) == 2.5
    assert wait(1, too_long) == 0.5
    assert wait(1, asked, jitter=0.5) == 2.5


def test_the_wait_is_spread_a_little() -> None:
    config = Config(jitter=0.1)
    randomness = random.Random(7)

    waits = [delay_for(BUSY, 3, config, randomness) for _ in range(200)]

    assert all(1.8 <= value <= 2.2 for value in waits)
    assert min(waits) < 1.9 < 2.1 < max(waits)


def test_the_same_seed_gives_the_same_waits() -> None:
    config = Config()

    one, other = random.Random(3), random.Random(3)

    first = [delay_for(BUSY, 2, config, one) for _ in range(3)]
    second = [delay_for(BUSY, 2, config, other) for _ in range(3)]

    assert first == second
    assert len(set(first)) == 3


@pytest.mark.parametrize(
    "bad",
    [
        {"max_attempts": 0},
        {"base_delay_seconds": -1},
        {"max_delay_seconds": -1},
        {"jitter": 1.5},
        {"jitter": -0.1},
    ],
)
async def test_a_config_that_makes_no_sense_is_refused(
    harness: Harness, bad: dict[str, float]
) -> None:
    with pytest.raises(ValidationError):
        await harness.ctx.plugin(columbina.retry, bad)
