"""The points at which a plugin can watch a turn or change its course."""

import asyncio

import pytest
from pydantic import BaseModel

from support.loop import MODEL, Harness, assistant, idle, types, user
from tack.agent import (
    PRE_STEP,
    REQUEST,
    REQUEST_ERROR,
    TURN_STOPPING,
    Enter,
    ErrorDecision,
    GiveUp,
    Reject,
    RequestDraft,
    RequestFailure,
    Retry,
    StepDecision,
    StepInfo,
    TurnStopping,
)
from tack.kernel import Next
from tack.llm import CallConfig, LlmFailure
from tack.llm.adapters.scripted import Call, Reply
from tack.session import (
    AssistantAttemptEvent,
    RequestHeaderEvent,
    TurnEndEvent,
    UserMessageEvent,
)
from tack.tools import Tool, ToolRun

BUSY = LlmFailure(message="overloaded", code="SERVER", status=529)


class NoArgs(BaseModel):
    pass


async def noop(args: NoArgs, run: ToolRun) -> str:
    return "ok"


NOOP = Tool(name="noop", description="Do nothing.", parameters=NoArgs, execute=noop)


# Before a step


async def test_a_listener_is_asked_before_every_step(harness: Harness) -> None:
    harness.tools.add(harness.ctx, NOOP)
    agent = await harness.agent(Reply(calls=(Call(name="noop"),)), Reply(text="Done."))
    asked: list[StepInfo] = []

    async def watch(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        asked.append(info)
        return await proceed(info)

    agent.ctx.on(PRE_STEP, watch)

    agent.followup("Do it.")
    await idle(agent)

    assert asked == [StepInfo(agent=agent, turn=1, step=1), StepInfo(agent=agent, turn=1, step=2)]
    assert Enter() == Enter()


async def test_a_listener_can_keep_a_turn_from_asking_the_model(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="never"))

    async def spent(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        return Reject(reason="the token budget is spent")

    agent.ctx.on(PRE_STEP, spent)

    agent.followup("Do it.")
    await idle(agent)

    assert types(agent.session, 1) == [
        "turn/start",
        "system/message",
        "agent/inbox/spliced",
        "user/message",
        "turn/end",
    ]
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="blocked", detail="the token budget is spent"
    )
    assert harness.model.requests == []
    assert agent.session.messages[-1] == user("Do it.")


async def test_a_turn_can_be_stopped_between_two_steps(harness: Harness) -> None:
    harness.tools.add(harness.ctx, NOOP)
    agent = await harness.agent(Reply(calls=(Call(name="noop"),)), Reply(text="never"))

    async def one_step(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        if info.step > 1:
            return Reject(reason="one step is enough")
        return await proceed(info)

    agent.ctx.on(PRE_STEP, one_step)

    agent.followup("Do it.")
    await idle(agent)

    assert types(agent.session)[-3:] == ["tool/result", "step/end", "turn/end"]
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="blocked", detail="one step is enough"
    )
    assert len(harness.model.requests) == 1


# Before a request


async def test_a_listener_can_choose_the_model_of_a_request(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"))
    careful = CallConfig(provider="scripted", model="script", temperature=0.0)
    drafts: list[RequestDraft] = []

    async def choose(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        drafts.append(draft)
        return careful

    agent.ctx.on(REQUEST, choose)

    agent.followup("Say hello.")
    await idle(agent)

    assert drafts == [RequestDraft(agent=agent, turn=1, step=1, attempt=1, config=MODEL)]
    assert harness.model.requests[0].config == careful
    headers = [e.data for e in agent.session.events if isinstance(e.data, RequestHeaderEvent)]
    assert headers == [RequestHeaderEvent(config=careful, tools=())]


# When a request fails


async def test_a_failed_request_can_be_tried_again_in_the_same_step(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="I w", failure=BUSY), Reply(text="Hello!"))
    failures: list[RequestFailure] = []
    attempts: list[int] = []

    async def again(failed: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]) -> Retry:
        failures.append(failed)
        return Retry()

    async def count(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        attempts.append(draft.attempt)
        return await proceed(draft)

    agent.ctx.on(REQUEST_ERROR, again)
    agent.ctx.on(REQUEST, count)

    agent.followup("Say hello.")
    await idle(agent)

    assert types(agent.session, 5) == [
        "step/start",
        "request/header",
        "assistant/attempt",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    assert failures == [RequestFailure(agent=agent, turn=1, step=1, attempt=1, failure=BUSY)]
    assert attempts == [1, 2]
    assert harness.model.requests[0] == harness.model.requests[1]
    inputs = [e for e in agent.session.events if isinstance(e.data, UserMessageEvent)]
    assert len(inputs) == 1
    assert agent.session.messages[-1] == assistant("Hello!")
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_a_listener_decides_when_to_stop_trying(harness: Harness) -> None:
    agent = await harness.agent(
        Reply(failure=BUSY), Reply(failure=BUSY), Reply(failure=BUSY), Reply(text="never")
    )

    async def thrice(
        failed: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        return Retry() if failed.attempt < 3 else await proceed(failed)

    agent.ctx.on(REQUEST_ERROR, thrice)

    agent.followup("Say hello.")
    await idle(agent)

    attempts = [e for e in agent.session.events if isinstance(e.data, AssistantAttemptEvent)]
    assert len(attempts) == 3
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="SERVER: overloaded"
    )
    assert harness.model.remaining == 1
    assert GiveUp() == GiveUp()


# When a turn is about to end


async def test_listeners_are_told_that_a_turn_is_about_to_end(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"))
    stopping: list[TurnStopping] = []

    async def watch(event: TurnStopping) -> None:
        stopping.append(event)

    agent.ctx.on(TURN_STOPPING, watch)

    agent.followup("Say hello.")
    await idle(agent)

    assert stopping == [TurnStopping(agent=agent, turn=1, reason="completed")]


async def test_a_listener_can_keep_a_turn_going_by_steering(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Done."), Reply(text="Now the tests pass."))
    told: list[str] = []

    async def check(event: TurnStopping) -> None:
        told.append(event.reason)
        if len(told) == 1:
            event.agent.steer("The tests still fail.", source="hook")

    agent.ctx.on(TURN_STOPPING, check)

    agent.followup("Fix the bug.")
    await idle(agent)

    assert told == ["completed", "completed"]
    assert harness.model.requests[1].messages[-2:] == (
        assistant("Done."),
        user("The tests still fail."),
    )
    ends = [e.data for e in agent.session.events if isinstance(e.data, TurnEndEvent)]
    assert ends == [TurnEndEvent(turn=1, reason="completed")]


async def test_a_turn_that_does_not_end_by_itself_does_not_ask_whether_it_may(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(failure=BUSY), Reply(text="thinking", hold=True))
    stopping: list[TurnStopping] = []

    async def watch(event: TurnStopping) -> None:
        stopping.append(event)

    agent.ctx.on(TURN_STOPPING, watch)

    agent.followup("First.")
    await idle(agent)
    agent.followup("Second.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.cancel()
    await idle(agent)

    assert stopping == []
    reasons = [e.data.reason for e in agent.session.events if isinstance(e.data, TurnEndEvent)]
    assert reasons == ["error", "aborted"]


async def test_a_turn_that_was_cut_off_says_so_when_it_is_about_to_end(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="This sentence never", finish="max-tokens"))
    stopping: list[TurnStopping] = []

    async def watch(event: TurnStopping) -> None:
        stopping.append(event)

    agent.ctx.on(TURN_STOPPING, watch)

    agent.followup("Write.")
    await idle(agent)

    assert [event.reason for event in stopping] == ["max-tokens"]


# Listeners that fail


@pytest.mark.parametrize(
    ("point", "first_reply"),
    [
        ("pre-step", None),
        ("request", None),
        ("request-error", Reply(failure=BUSY)),
        ("turn-stopping", Reply(text="Hello!")),
    ],
)
async def test_a_listener_that_fails_ends_the_turn_with_an_error(
    harness: Harness, point: str, first_reply: Reply | None
) -> None:
    agent = await harness.agent(*([] if first_reply is None else [first_reply]), Reply(text="Ok."))

    async def step(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        raise RuntimeError("listener bug")

    async def request(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        raise RuntimeError("listener bug")

    async def failed(
        failure: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        raise RuntimeError("listener bug")

    async def stopping(event: TurnStopping) -> None:
        raise RuntimeError("listener bug")

    if point == "pre-step":
        handle = agent.ctx.on(PRE_STEP, step)
    elif point == "request":
        handle = agent.ctx.on(REQUEST, request)
    elif point == "request-error":
        handle = agent.ctx.on(REQUEST_ERROR, failed)
    else:
        handle = agent.ctx.on(TURN_STOPPING, stopping)

    agent.followup("Say hello.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="RuntimeError: listener bug"
    )
    assert types(agent.session).count("step/start") == types(agent.session).count("step/end")

    await handle.dispose()
    agent.followup("Again.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")
    assert agent.session.messages[-1] == assistant("Ok.")


@pytest.mark.parametrize(
    ("point", "first_reply"),
    [
        ("pre-step", None),
        ("request", None),
        ("request-error", Reply(failure=BUSY)),
        ("turn-stopping", Reply(text="Hello!")),
    ],
)
async def test_a_listener_that_raises_a_cancellation_of_its_own_ends_the_turn_with_an_error(
    harness: Harness, point: str, first_reply: Reply | None
) -> None:
    agent = await harness.agent(*([] if first_reply is None else [first_reply]), Reply(text="Ok."))

    async def step(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        raise asyncio.CancelledError

    async def request(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        raise asyncio.CancelledError

    async def failed(
        failure: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        raise asyncio.CancelledError

    async def stopping(event: TurnStopping) -> None:
        raise asyncio.CancelledError

    if point == "pre-step":
        handle = agent.ctx.on(PRE_STEP, step)
    elif point == "request":
        handle = agent.ctx.on(REQUEST, request)
    elif point == "request-error":
        handle = agent.ctx.on(REQUEST_ERROR, failed)
    else:
        handle = agent.ctx.on(TURN_STOPPING, stopping)

    agent.followup("Say hello.")
    await idle(agent)

    # Nobody cancelled the turn and nothing was disposed: it is the listener that is at fault.
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1,
        reason="error",
        detail=f"CancelledFromInside: a listener of agent/{point} "
        "ended with a cancellation that nobody asked for",
    )
    assert types(agent.session).count("step/start") == types(agent.session).count("step/end")

    await handle.dispose()
    agent.followup("Again.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")
    assert agent.session.messages[-1] == assistant("Ok.")


async def test_a_turn_can_be_cancelled_while_a_listener_decides(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="never"))
    asked = asyncio.Event()

    async def ponder(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        asked.set()
        await asyncio.Event().wait()
        return await proceed(info)

    agent.ctx.on(PRE_STEP, ponder)
    agent.followup("Do it.")
    await asyncio.wait_for(asked.wait(), timeout=5)

    agent.cancel()
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert "step/start" not in types(agent.session)
    assert harness.model.requests == []


# Listeners that give no answer


async def test_a_listener_that_gives_no_answer_before_a_step_keeps_the_step_from_beginning(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="never"))

    async def forgetful(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        await proceed(info)
        return None  # type: ignore[return-value]

    agent.ctx.on(PRE_STEP, forgetful)
    agent.followup("Do it.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="blocked", detail="a listener of agent/pre-step gave no decision"
    )
    assert harness.model.requests == []


async def test_a_listener_that_gives_no_answer_to_a_failure_lets_the_failure_stand(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(failure=BUSY), Reply(text="never"))

    async def forgetful(
        failed: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        return None  # type: ignore[return-value]

    agent.ctx.on(REQUEST_ERROR, forgetful)
    agent.followup("Say hello.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="SERVER: overloaded"
    )
    assert len(harness.model.requests) == 1
