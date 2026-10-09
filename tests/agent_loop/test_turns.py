"""The loop: a task becomes a turn, and a turn is one or more requests to the model."""

import asyncio

from pydantic import BaseModel

from columbina.agent import AGENTS, STREAM, InboxItem, StreamFrame, enqueue
from columbina.llm import (
    BlockStart,
    CallConfig,
    Finish,
    LlmFailure,
    LlmRequest,
    SystemMessage,
    TextBlock,
    Usage,
)
from columbina.llm.adapters.scripted import Reply
from columbina.session import (
    TURN,
    USAGE,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    RequestHeaderEvent,
    StepEndEvent,
    StepStartEvent,
    SystemMessageEvent,
    TurnEndEvent,
    TurnStartEvent,
    TurnState,
    UserMessageEvent,
)
from columbina.tools import Tool, ToolRun
from support.loop import MODEL, SYSTEM, Harness, assistant, idle, types, user


async def test_a_task_becomes_a_turn_with_one_step(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!", usage=Usage(input_tokens=9, output_tokens=2)))

    agent.followup("Say hello.")
    await idle(agent)

    session = agent.session
    assert types(session) == [
        "agent/inbox/spliced",
        "turn/start",
        "system/message",
        "agent/inbox/spliced",
        "user/message",
        "step/start",
        "request/header",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    data = [event.data for event in session.events]
    assert data[1] == TurnStartEvent(turn=1)
    assert data[2] == SystemMessageEvent(message=SYSTEM)
    assert data[4] == UserMessageEvent(message=user("Say hello."))
    assert data[5] == StepStartEvent(turn=1, step=1)
    assert data[6] == RequestHeaderEvent(config=MODEL, tools=())
    assert data[7] == AssistantMessageEvent(
        message=assistant("Hello!"),
        usage=Usage(input_tokens=9, output_tokens=2),
        finish_reason="stop",
    )
    assert data[8] == StepEndEvent(turn=1, step=1)
    assert data[9] == TurnEndEvent(turn=1, reason="completed")
    assert session.messages == (SYSTEM, user("Say hello."), assistant("Hello!"))


async def test_the_request_is_what_the_log_says_at_that_moment(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"))

    agent.followup("Say hello.")
    await idle(agent)

    assert harness.model.requests == [
        LlmRequest(config=MODEL, messages=(SYSTEM, user("Say hello.")), tools=())
    ]


async def test_a_second_task_is_a_second_turn_that_sees_the_first(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"), Reply(text="Fine."))
    agent.followup("Say hello.")
    await idle(agent)
    before = len(agent.session)

    agent.followup("How are you?")
    await idle(agent)

    assert types(agent.session, before) == [
        "agent/inbox/spliced",
        "turn/start",
        "agent/inbox/spliced",
        "user/message",
        "step/start",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    assert harness.model.requests[1].messages == (
        SYSTEM,
        user("Say hello."),
        assistant("Hello!"),
        user("How are you?"),
    )
    assert TURN.of(agent.session) == TurnState(turn=2, step=1)


async def test_tasks_that_wait_are_taken_one_turn_at_a_time(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="one"), Reply(text="two"))

    agent.followup("First.")
    agent.followup("Second.")
    await idle(agent)

    turns = [event.data for event in agent.session.events if isinstance(event.data, TurnEndEvent)]
    assert turns == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]
    assert [request.messages[-1] for request in harness.model.requests] == [
        user("First."),
        user("Second."),
    ]


async def test_waking_an_agent_that_has_no_input_does_nothing(harness: Harness) -> None:
    agent = await harness.agent()

    agent.wake()
    await idle(agent)

    assert types(agent.session) == []
    assert harness.model.requests == []


async def test_an_agent_at_rest_is_idle_at_once(harness: Harness) -> None:
    agent = await harness.agent()

    await asyncio.wait_for(agent.when_idle(), timeout=1)

    assert types(agent.session) == []


async def test_injected_context_waits_for_a_task_and_is_read_before_it(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Noted."))

    agent.inject("The tests passed a minute ago.")
    await idle(agent)
    assert harness.model.requests == []

    agent.followup("Is the build green?")
    await idle(agent)

    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("The tests passed a minute ago."),
        user("Is the build green?"),
    )
    sources = [
        event.data.source
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert sources == ["inject", "user"]


async def test_a_steer_opens_a_turn_when_none_is_running(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="On it."))

    agent.steer("Look at the other file.")
    await idle(agent)

    assert harness.model.requests[0].messages == (SYSTEM, user("Look at the other file."))
    assert TURN.of(agent.session) == TurnState(turn=1, step=1)


async def test_every_chunk_of_a_reply_is_passed_on_as_it_arrives(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"))
    frames: list[StreamFrame] = []
    agent.ctx.on(STREAM, frames.append)

    agent.followup("Say hello.")
    await idle(agent)

    assert [frame.chunk.type for frame in frames] == [
        "block-start",
        "text-delta",
        "block-end",
        "finish",
    ]
    assert frames[0] == StreamFrame(
        agent=agent, turn=1, step=1, chunk=BlockStart(index=0, kind="text")
    )
    assert frames[-1].chunk == Finish(reason="stop")


async def test_without_instructions_there_is_no_system_message(harness: Harness) -> None:
    agent = await harness.agent(Reply(text="Hello!"), system_prompt="")

    agent.followup("Say hello.")
    await idle(agent)

    assert "system/message" not in types(agent.session)
    assert harness.model.requests[0].messages == (user("Say hello."),)


async def test_changed_instructions_take_the_place_of_the_old_ones(harness: Harness) -> None:
    first = await harness.agent(Reply(text="Hello!"), Reply(text="Hi."))
    first.followup("Say hello.")
    await idle(first)
    await first.close()

    second = await harness.agents.resume(
        harness.ctx, harness.spec(system_prompt="You are terse."), "s-1"
    )
    second.followup("Again.")
    await idle(second)

    terse = SystemMessage(content=(TextBlock(text="You are terse."),))
    assert harness.model.requests[1].messages == (
        terse,
        user("Say hello."),
        assistant("Hello!"),
        user("Again."),
    )
    replacement = next(
        event for event in second.session.events if event.data == SystemMessageEvent(message=terse)
    )
    assert replacement.replaces == (2,)


async def test_a_resumed_agent_goes_on_counting_turns(harness: Harness) -> None:
    first = await harness.agent(Reply(text="Hello!"), Reply(text="Hi."))
    first.followup("Say hello.")
    await idle(first)
    await first.close()

    second = await harness.agents.resume(harness.ctx, harness.spec(), "s-1")
    second.followup("Again.")
    await idle(second)

    assert TURN.of(second.session) == TurnState(turn=2, step=1)
    assert "system/message" not in types(second.session, len(first.session))


async def test_a_task_that_waited_through_a_restart_runs_when_the_agent_is_woken(
    harness: Harness,
) -> None:
    first = await harness.agent()
    enqueue(first.session, "next-turn", InboxItem(message=user("Do it.")))
    await first.close()

    harness.model.add(Reply(text="Done."))
    second = await harness.agents.resume(harness.ctx, harness.spec(), "s-1")
    await idle(second)
    assert harness.model.requests == []

    second.wake()
    await idle(second)

    assert harness.model.requests[0].messages[-1] == user("Do it.")
    assert TURN.of(second.session).turn == 1


async def test_a_call_that_fails_ends_the_turn_with_an_error(harness: Harness) -> None:
    failure = LlmFailure(message="the key is not valid", code="AUTH", status=401)
    agent = await harness.agent(Reply(text="I w", failure=failure), Reply(text="Hello!"))

    agent.followup("Say hello.")
    await idle(agent)

    assert types(agent.session)[-5:] == [
        "step/start",
        "request/header",
        "assistant/attempt",
        "step/end",
        "turn/end",
    ]
    assert agent.session.events[-3].data == AssistantAttemptEvent(
        message=assistant("I w"), usage=Usage(), failure=failure
    )
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="AUTH: the key is not valid"
    )
    assert agent.session.messages == (SYSTEM, user("Say hello."))

    agent.followup("Try again.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")
    assert agent.session.messages[-1] == assistant("Hello!")


async def test_a_model_that_is_not_there_ends_the_turn_with_an_error(harness: Harness) -> None:
    missing = CallConfig(provider="nowhere", model="none")
    agent = await harness.agent(model=missing)

    agent.followup("Say hello.")
    await idle(agent)

    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1,
        reason="error",
        detail="INVALID_REQUEST: no adapter is registered for the provider 'nowhere'",
    )


async def test_the_header_is_written_again_when_the_tools_change(harness: Harness) -> None:
    class NoArgs(BaseModel):
        pass

    async def now(args: NoArgs, run: ToolRun) -> str:
        return "noon"

    clock = Tool(name="clock", description="Tell the time.", parameters=NoArgs, execute=now)
    agent = await harness.agent(Reply(text="one"), Reply(text="two"), Reply(text="three"))
    agent.followup("First.")
    await idle(agent)

    harness.tools.add(harness.ctx, clock)
    agent.followup("Second.")
    await idle(agent)
    agent.followup("Third.")
    await idle(agent)

    headers = [
        event.data for event in agent.session.events if isinstance(event.data, RequestHeaderEvent)
    ]
    assert headers == [
        RequestHeaderEvent(config=MODEL, tools=()),
        RequestHeaderEvent(config=MODEL, tools=(clock.schema,)),
    ]
    assert [request.tools for request in harness.model.requests] == [
        (),
        (clock.schema,),
        (clock.schema,),
    ]


async def test_the_usage_of_a_session_adds_up(harness: Harness) -> None:
    agent = await harness.agent(
        Reply(text="one", usage=Usage(input_tokens=10, output_tokens=1)),
        Reply(text="two", usage=Usage(input_tokens=20, output_tokens=2, cache_read_tokens=10)),
    )

    agent.followup("First.")
    agent.followup("Second.")
    await idle(agent)

    total = USAGE.of(agent.session)
    assert total.calls == 2
    assert total.usage == Usage(input_tokens=30, output_tokens=3, cache_read_tokens=10)


async def test_the_model_of_an_agent_is_the_one_in_its_spec(harness: Harness) -> None:
    hot = CallConfig(provider="scripted", model="script", temperature=0.7, max_output_tokens=100)
    agent = await harness.agent(Reply(text="Hello!"), model=hot)

    agent.followup("Say hello.")
    await idle(agent)

    assert harness.model.requests[0].config == hot


async def test_closing_an_agent_at_rest_ends_it(harness: Harness) -> None:
    agent = await harness.agent()

    await asyncio.wait_for(agent.close(), timeout=2)

    assert agent.session.closed
    assert list(harness.ctx.use(AGENTS).live) == []
