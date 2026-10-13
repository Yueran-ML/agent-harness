"""The loop: a task becomes a turn, and a turn is one or more requests to the model."""

import asyncio

from pydantic import BaseModel

import columbina.system_prompt
from columbina.agent import (
    AGENTS,
    INBOX,
    STREAM,
    Agent,
    ContextSource,
    InboxItem,
    StreamFrame,
    enqueue,
)
from columbina.kernel import Next
from columbina.llm import (
    BlockStart,
    CallConfig,
    Finish,
    LlmFailure,
    LlmRequest,
    SystemMessage,
    TextBlock,
    Usage,
    UserMessage,
)
from columbina.llm.adapters.scripted import Call, Reply
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
from columbina.system_prompt import SYSTEM_PROMPT, SYSTEM_PROMPT_ASSEMBLE, Draft, Section
from columbina.tools import Tool, ToolRun
from support.loop import MODEL, SYSTEM, Harness, assistant, context, idle, types, user


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


async def test_what_is_said_after_a_task_that_has_not_begun_yet_is_read_after_it(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="On it."))

    # Both are there before the turn has taken anything.
    agent.followup("Read a.txt.")
    agent.steer("And b.txt as well.")
    await idle(agent)

    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("Read a.txt."),
        user("And b.txt as well."),
    )
    said = [
        (event.data.source, event.data.message.content[0].text)
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert said == [("user", "Read a.txt."), ("steer", "And b.txt as well.")]
    # One turn read both.
    assert [
        event.data.turn for event in agent.session.events if isinstance(event.data, TurnEndEvent)
    ] == [1]


async def test_what_a_turn_begins_with_is_read_in_the_order_in_which_it_came(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))

    agent.inject("Before everything.")
    agent.followup("The first task.")
    agent.steer("Said after it.")
    agent.followup("The second task.")
    agent.steer("Said last.")
    await idle(agent)

    # What is said on the way is for the turn that reads it first. The second task opens
    # a turn of its own after that.
    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("Before everything."),
        user("The first task."),
        user("Said after it."),
        user("Said last."),
    )
    assert harness.model.requests[1].messages[-2:] == (assistant("One."), user("The second task."))


async def test_a_later_turn_begins_with_its_input_in_the_order_in_which_it_came_too(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    agent.steer("Said to open the first turn.")
    agent.followup("The first task.")
    await idle(agent)

    # What left the inbox for the first turn has nothing to say about the second one.
    agent.inject("Something that happened since.")
    agent.followup("The second task.")
    agent.steer("Said after it.")
    await idle(agent)

    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("Said to open the first turn."),
        user("The first task."),
    )
    assert harness.model.requests[1].messages[-4:] == (
        assistant("One."),
        user("Something that happened since."),
        user("The second task."),
        user("Said after it."),
    )


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
        turn=1, reason="error", detail="AUTH: the key is not valid", code="AUTH"
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
        code="INVALID_REQUEST",
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


# The system prompt service


async def test_what_plugins_add_to_the_instructions_follows_those_of_the_spec(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.system_prompt)
    harness.ctx.use(SYSTEM_PROMPT).sections.add(
        harness.ctx, "files", Section("Read a file before you write it.")
    )
    agent = await harness.agent(Reply(text="Hello!"))

    agent.followup("Say hello.")
    await idle(agent)

    assert harness.model.requests[0].messages[0] == SystemMessage(
        content=(TextBlock(text="You are helpful.\n\nRead a file before you write it."),)
    )


async def test_instructions_that_only_plugins_give_are_instructions_too(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.system_prompt)
    harness.ctx.use(SYSTEM_PROMPT).sections.add(harness.ctx, "files", Section("Read first."))
    agent = await harness.agent(Reply(text="Hello!"), system_prompt="")

    agent.followup("Say hello.")
    await idle(agent)

    assert harness.model.requests[0].messages[0] == SystemMessage(
        content=(TextBlock(text="Read first."),)
    )


async def test_instructions_that_change_between_turns_take_the_place_of_the_old_ones(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.system_prompt)
    sections = harness.ctx.use(SYSTEM_PROMPT).sections
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), Reply(text="Three."))
    agent.followup("First.")
    await idle(agent)

    added = sections.add(harness.ctx, "late", Section("Added later."))
    agent.followup("Second.")
    await idle(agent)
    await added.dispose()
    agent.followup("Third.")
    await idle(agent)

    longer = SystemMessage(content=(TextBlock(text="You are helpful.\n\nAdded later."),))
    assert [request.messages[0] for request in harness.model.requests] == [SYSTEM, longer, SYSTEM]
    assert types(agent.session).count("system/message") == 3
    assert [len(request.messages) for request in harness.model.requests] == [2, 4, 6]


async def test_instructions_that_stay_the_same_are_not_written_again(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.system_prompt)
    harness.ctx.use(SYSTEM_PROMPT).sections.add(harness.ctx, "files", Section("Read first."))
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))

    agent.followup("First.")
    await idle(agent)
    agent.followup("Second.")
    await idle(agent)

    assert types(agent.session).count("system/message") == 1


async def test_the_instructions_are_asked_for_once_in_a_turn(harness: Harness) -> None:
    asked: list[str] = []

    async def counting(draft: Draft, proceed: Next[Draft, str]) -> str:
        asked.append(draft.base)
        return await proceed(draft)

    class NoArgs(BaseModel):
        pass

    async def nothing(args: NoArgs, run: ToolRun) -> str:
        return "done"

    await harness.ctx.plugin(columbina.system_prompt)
    harness.ctx.on(SYSTEM_PROMPT_ASSEMBLE, counting)
    harness.tools.add(
        harness.ctx,
        Tool(name="nothing", description="Do nothing.", parameters=NoArgs, execute=nothing),
    )
    agent = await harness.agent(
        Reply(calls=(Call(name="nothing"),)),
        Reply(calls=(Call(name="nothing"),)),
        Reply(text="Done."),
    )

    agent.followup("Do nothing, twice.")
    await idle(agent)

    assert len(harness.model.requests) == 3
    assert asked == ["You are helpful."]


async def test_instructions_that_cannot_be_put_together_end_the_turn_and_take_the_input(
    harness: Harness,
) -> None:
    async def failing(draft: Draft, proceed: Next[Draft, str]) -> str:
        raise RuntimeError("no instructions today")

    await harness.ctx.plugin(columbina.system_prompt)
    harness.ctx.on(SYSTEM_PROMPT_ASSEMBLE, failing)
    agent = await harness.agent(Reply(text="Never said."))

    agent.followup("Say hello.")
    await idle(agent)

    ends = [event.data for event in agent.session.events if isinstance(event.data, TurnEndEvent)]
    assert [(end.reason, end.detail) for end in ends] == [
        ("error", "RuntimeError: no instructions today")
    ]
    assert harness.model.requests == []
    assert "user/message" in types(agent.session)
    assert not INBOX.of(agent.session).next_turn
    assert not INBOX.of(agent.session).next_step


# Text that cannot be written down

HALF = chr(0xD800)
GONE = chr(0xFFFD)


async def test_half_a_character_in_what_a_person_says_becomes_a_sign_that_can_be_written(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="Hello!"))

    # What is typed or pasted may hold half a character, and no log can hold one.
    agent.inject(f"The file is notes{HALF}.txt.")
    agent.steer(UserMessage(content=(TextBlock(text=f"Mind {HALF} this."),)))
    agent.followup(f"Say {HALF}hello.")
    await idle(agent)

    said = [
        event.data.message.content[0].text
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert said == [f"The file is notes{GONE}.txt.", f"Mind {GONE} this.", f"Say {GONE}hello."]
    assert [
        event.data.reason for event in agent.session.events if isinstance(event.data, TurnEndEvent)
    ] == ["completed"]


async def test_what_a_person_says_without_half_a_character_is_taken_as_it_is(
    harness: Harness,
) -> None:
    agent = await harness.agent(Reply(text="Hello!"))
    message = UserMessage(content=(TextBlock(text="Say hello."), TextBlock(text="And goodbye.")))

    agent.followup(message)
    await idle(agent)

    (said,) = [
        event.data.message
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert said == message


async def test_half_a_character_in_what_a_source_of_context_gives_becomes_a_sign(
    harness: Harness,
) -> None:
    async def facts(agent: Agent) -> str | None:
        # The working directory may be called anything that its disk takes.
        return f"You work in /home/user/notes{HALF}."

    harness.agents.context.add(harness.ctx, "facts", ContextSource(facts, order=10, kind="note"))
    agent = await harness.agent(Reply(text="Hello!"))

    agent.followup("Say hello.")
    await idle(agent)

    (request,) = harness.model.requests
    assert request.messages[1] == context("facts", f"You work in /home/user/notes{GONE}.")
    assert [
        event.data.reason for event in agent.session.events if isinstance(event.data, TurnEndEvent)
    ] == ["completed"]
