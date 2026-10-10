"""Work that is done on a conversation between two turns, when the agent is at rest."""

import asyncio
from collections.abc import Awaitable, Callable

import pytest

from columbina.agent import Agent, AgentBusyError
from columbina.llm import LlmRequest
from columbina.llm.adapters.scripted import Reply
from columbina.session import TurnEndEvent
from support.loop import Harness, assistant, idle


class Work:
    """A piece of work that begins, waits to be let go, and ends. It tells in what order."""

    def __init__(self, told: list[str] | None = None, result: object = "done") -> None:
        self.told = [] if told is None else told
        self.result = result
        self.begun = asyncio.Event()
        self.let_go = asyncio.Event()

    async def __call__(self) -> object:
        self.told.append("the work begins")
        self.begun.set()
        await self.let_go.wait()
        self.told.append("the work ends")
        if isinstance(self.result, Exception):
            raise self.result
        return self.result


async def soon(waited: Awaitable[object]) -> object:
    """Wait for something that is to come at once. A test that waits for ever says nothing."""
    return await asyncio.wait_for(waited, timeout=5)


async def at_once() -> str:
    return "done"


def telling(told: list[str], reply: Reply) -> Callable[[LlmRequest], Reply]:
    """A step of a script that writes down when the model is asked."""

    def asked(request: LlmRequest) -> Reply:
        told.append("the model is asked")
        return reply

    return asked


async def begun(work: Work, doing: Awaitable[object]) -> asyncio.Future[object]:
    """Start waiting for a piece of work, and return when the work has begun."""
    waiting = asyncio.ensure_future(doing)
    await asyncio.wait_for(work.begun.wait(), timeout=5)
    return waiting


async def test_work_is_done_when_the_agent_is_at_rest_and_what_it_gives_comes_back(
    harness: Harness,
) -> None:
    agent = await harness.agent()

    assert await soon(agent.run_maintenance(at_once)) == "done"

    await idle(agent)
    assert harness.model.requests == []
    # No turn was run for it, and nothing of it is in the log but what the work wrote.
    assert not any(isinstance(event.data, TurnEndEvent) for event in agent.session.events)


async def test_an_agent_in_a_turn_refuses_at_once(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Go.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    work = Work()

    # Not when it is waited for: where it is asked.
    with pytest.raises(AgentBusyError, match="is not at rest"):
        agent.run_maintenance(work)

    harness.model.release()
    await idle(agent)
    assert work.told == []
    assert agent.session.messages[-1] == assistant("thinking")


async def test_an_agent_whose_input_waits_for_its_turn_refuses(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="One."))
    agent.followup("Go.")

    with pytest.raises(AgentBusyError, match="is not at rest"):
        agent.run_maintenance(at_once)

    await idle(agent)
    assert agent.session.messages[-1] == assistant("One.")


async def test_one_piece_of_work_is_done_at_a_time(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = await begun(work, agent.run_maintenance(work))

    with pytest.raises(AgentBusyError, match="is not at rest"):
        agent.run_maintenance(at_once)

    work.let_go.set()
    assert await soon(waiting) == "done"
    # When it is over, the next can begin.
    assert await soon(agent.run_maintenance(at_once)) == "done"


async def test_input_that_comes_meanwhile_waits_until_the_work_is_over(harness: Harness) -> None:
    told: list[str] = []
    agent = await harness.agent(telling(told, Reply(text="One.")))
    work = Work(told)
    waiting = await begun(work, agent.run_maintenance(work))

    agent.followup("Go.")
    await asyncio.sleep(0.05)
    still_working = list(told)
    work.let_go.set()

    assert await soon(waiting) == "done"
    await idle(agent)
    assert still_working == ["the work begins"]
    assert told == ["the work begins", "the work ends", "the model is asked"]
    assert agent.session.messages[-1] == assistant("One.")


async def test_the_agent_is_not_at_rest_while_the_work_goes_on(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = await begun(work, agent.run_maintenance(work))
    resting = asyncio.ensure_future(agent.when_idle())

    await asyncio.sleep(0.05)
    assert not resting.done()

    work.let_go.set()
    await soon(waiting)
    await asyncio.wait_for(resting, timeout=5)


async def test_what_the_work_raises_goes_to_whoever_asked_and_the_agent_goes_on(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="One."))
    work = Work(result=RuntimeError("the broom broke"))
    work.let_go.set()

    with pytest.raises(RuntimeError, match="the broom broke"):
        await soon(agent.run_maintenance(work))

    await idle(agent)
    agent.followup("Go.")
    await idle(agent)
    assert agent.session.messages[-1] == assistant("One.")


async def test_cancelling_the_agent_stops_the_work(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="One."))
    work = Work()
    waiting = await begun(work, agent.run_maintenance(work))

    agent.cancel()

    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    await idle(agent)
    assert work.told == ["the work begins"]
    # The agent is as usable as after a turn that was stopped.
    agent.followup("Go.")
    await idle(agent)
    assert agent.session.messages[-1] == assistant("One.")
    assert await soon(agent.run_maintenance(at_once)) == "done"


async def test_work_that_is_cancelled_before_it_began_never_begins(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = agent.run_maintenance(work)

    # The agent has taken it and has not got to it yet.
    agent.cancel()

    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    await idle(agent)
    await asyncio.sleep(0.05)
    assert work.told == []
    assert await soon(agent.run_maintenance(at_once)) == "done"


async def test_closing_the_agent_stops_the_work(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = await begun(work, agent.run_maintenance(work))

    await agent.close()

    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    assert work.told == ["the work begins"]


async def test_an_agent_that_has_ended_takes_no_work(harness: Harness) -> None:
    agent = await harness.agent()
    await agent.close()

    with pytest.raises(AgentBusyError, match="has ended"):
        agent.run_maintenance(at_once)


async def test_work_that_was_taken_as_the_agent_ended_is_not_left_waiting(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = agent.run_maintenance(work)

    await agent.close()

    # It may have begun before the end came, or not. It does not end, and nobody waits on.
    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    assert "the work ends" not in work.told


async def test_work_is_not_left_waiting_if_the_driver_of_the_agent_is_stopped(
    harness: Harness,
) -> None:
    # A program that ends stops every task, this one too.
    agent = await harness.agent()
    work = Work()
    # The driver has begun and waits for something to do.
    await asyncio.sleep(0)
    waiting = agent.run_maintenance(work)
    driver: asyncio.Task[None] = getattr(agent, "_driver")  # noqa: B009

    driver.cancel()

    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    assert work.told == []
    await idle(agent)


async def test_whoever_asked_may_stop_waiting_and_the_work_goes_on(harness: Harness) -> None:
    agent = await harness.agent()
    work = Work()
    waiting = await begun(work, agent.run_maintenance(work))

    waiting.cancel()
    with pytest.raises(asyncio.CancelledError):
        await soon(waiting)
    work.let_go.set()

    await idle(agent)
    assert work.told == ["the work begins", "the work ends"]
    assert await soon(agent.run_maintenance(at_once)) == "done"


async def test_any_agent_can_be_asked(harness: Harness) -> None:
    agent: Agent = await harness.agent()

    assert await soon(agent.run_maintenance(at_once)) == "done"
