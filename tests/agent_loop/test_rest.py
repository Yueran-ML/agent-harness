"""Whether an agent is at rest, told at once and without waiting for it."""

import asyncio
import logging

import pytest

from columbina.llm.adapters.scripted import Reply
from columbina.session import SessionStoreError
from support.loop import Harness, idle


async def test_an_agent_is_at_rest_until_it_is_given_something_to_do(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Done."))

    before = agent.at_rest
    agent.followup("Go.")
    # No turn has begun yet. One is about to, and that is enough.
    given = agent.at_rest
    await idle(agent)

    assert (before, given, agent.at_rest) == (True, False, True)


async def test_an_agent_is_not_at_rest_while_a_turn_runs(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="One.", hold=True))
    agent.followup("Go.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    during = agent.at_rest
    harness.model.release()
    await idle(agent)

    assert (during, agent.at_rest) == (False, True)


async def test_an_agent_is_not_at_rest_between_two_turns_that_follow_each_other(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="One."), Reply(text="Two.", hold=True))
    agent.followup("First.")
    agent.followup("Second.")
    # The first turn is over, and the second one is held.
    await asyncio.wait_for(harness.model.held(), timeout=5)

    between = agent.at_rest
    harness.model.release()
    await idle(agent)

    assert (between, agent.at_rest) == (False, True)


async def test_context_that_only_waits_for_other_input_leaves_an_agent_at_rest(
    harness: Harness,
) -> None:
    agent = await harness.agent()

    agent.inject("The branch is main.")

    assert agent.at_rest


async def test_an_agent_is_not_at_rest_while_it_does_work_between_turns(harness: Harness) -> None:
    agent = await harness.agent()
    begun, let_go = asyncio.Event(), asyncio.Event()

    async def work() -> str:
        begun.set()
        await let_go.wait()
        return "done"

    waiting = asyncio.ensure_future(agent.run_maintenance(work))
    # From the moment the work is taken, although it has not begun.
    taken = agent.at_rest
    await asyncio.wait_for(begun.wait(), timeout=5)
    during = agent.at_rest
    let_go.set()
    await asyncio.wait_for(waiting, timeout=5)
    await idle(agent)

    assert (taken, during, agent.at_rest) == (False, False, True)


async def test_an_agent_whose_turn_is_stopped_comes_to_rest(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="One.", hold=True))
    agent.followup("Go.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.cancel("user")
    # Stopping only asks. The turn has not ended yet.
    asked = agent.at_rest
    await idle(agent)

    assert (asked, agent.at_rest) == (False, True)


async def test_an_agent_that_has_stopped_for_good_is_at_rest_and_says_why_to_whoever_waits(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    agent = await harness.agent(Reply(text="One.", hold=True))
    agent.followup("Go.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    monkeypatch.setattr("columbina.session.store.encode_event", full)

    harness.model.release()
    with (
        caplog.at_level(logging.CRITICAL, logger="columbina.agent_loop"),
        pytest.raises(SessionStoreError, match="No space left on device"),
    ):
        await idle(agent)
    monkeypatch.undo()

    # Nothing runs and nothing will. Only waiting for it tells that it has stopped.
    assert agent.at_rest
    await asyncio.wait_for(agent.close(), timeout=5)
