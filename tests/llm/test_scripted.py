"""The scripted adapter: a model whose replies are written down in advance."""

import asyncio

import pytest
from pydantic import ValidationError

from columbina.kernel import Context
from columbina.llm import (
    LLM,
    AssistantMessage,
    BlockEnd,
    BlockStart,
    CallConfig,
    Chunk,
    Finish,
    Llm,
    LlmCall,
    LlmFailure,
    LlmRequest,
    ReasoningBlock,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    Usage,
    UserMessage,
)
from columbina.llm.adapters import scripted
from columbina.llm.adapters.scripted import Call, Reply, Scripted


def request(text: str = "hello") -> LlmRequest:
    return LlmRequest(
        config=CallConfig(provider="scripted", model="script"),
        messages=(UserMessage(content=(TextBlock(text=text),)),),
    )


async def ask(llm: Llm, text: str = "hello") -> LlmCall:
    call = llm.call(request(text))
    async for _ in call:
        pass
    return call


def play(ctx: Context, llm: Llm, *steps: scripted.Step) -> Scripted:
    adapter = Scripted(steps)
    llm.adapters.add(ctx, "scripted", adapter)
    return adapter


async def test_the_replies_are_played_in_order(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm, Reply(text="first"), Reply(text="second"))

    first = await ask(llm, "one")
    second = await ask(llm, "two")

    assert first.message.text == "first"
    assert second.message.text == "second"
    assert first.finish == Finish(reason="stop")
    assert adapter.requests == [request("one"), request("two")]
    assert adapter.remaining == 0


async def test_a_reply_can_hold_reasoning_text_and_tool_calls(ctx: Context, llm: Llm) -> None:
    play(
        ctx,
        llm,
        Reply(
            reasoning="The file must be read first.",
            replay_state={"signature": "abc"},
            text="I will look.",
            calls=(Call(name="read", arguments={"path": "a.txt"}), Call(name="list")),
        ),
    )

    call = await ask(llm)

    assert call.message == AssistantMessage(
        content=(
            ReasoningBlock(text="The file must be read first.", replay_state={"signature": "abc"}),
            TextBlock(text="I will look."),
            ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
            ToolCallBlock(call_id="call-2", name="list", arguments="{}"),
        )
    )
    assert call.finish == Finish(reason="tool-calls")


async def test_call_ids_keep_counting_across_replies(ctx: Context, llm: Llm) -> None:
    play(
        ctx,
        llm,
        Reply(calls=(Call(name="read"),)),
        Reply(calls=(Call(name="read", call_id="mine"), Call(name="read"))),
    )

    first = await ask(llm)
    second = await ask(llm)

    assert [call.call_id for call in first.message.tool_calls] == ["call-1"]
    assert [call.call_id for call in second.message.tool_calls] == ["mine", "call-2"]


async def test_arguments_given_as_text_are_sent_as_they_are(ctx: Context, llm: Llm) -> None:
    play(ctx, llm, Reply(calls=(Call(name="read", arguments='{"path": '),)))

    call = await ask(llm)

    assert call.message.tool_calls[0].arguments == '{"path": '


async def test_arguments_keep_text_outside_ascii(ctx: Context, llm: Llm) -> None:
    play(ctx, llm, Reply(calls=(Call(name="write", arguments={"text": "你好"}),)))

    call = await ask(llm)

    assert call.message.tool_calls[0].arguments == '{"text": "你好"}'


async def test_a_reply_can_report_usage(ctx: Context, llm: Llm) -> None:
    usage = Usage(input_tokens=120, output_tokens=7, cache_read_tokens=900)
    play(ctx, llm, Reply(text="ok", usage=usage), Reply(text="ok"))

    with_usage = await ask(llm)
    without = await ask(llm)

    assert with_usage.usage == usage
    assert without.usage == Usage()


async def test_a_reply_can_be_cut_off_by_the_token_limit(ctx: Context, llm: Llm) -> None:
    play(ctx, llm, Reply(text="This sentence never", finish="max-tokens"))

    call = await ask(llm)

    assert call.finish == Finish(reason="max-tokens")
    assert call.message.text == "This sentence never"


async def test_a_reply_can_fail_after_some_content(ctx: Context, llm: Llm) -> None:
    failure = LlmFailure(message="overloaded", code="SERVER", status=529)
    play(ctx, llm, Reply(text="partial", failure=failure))

    call = await ask(llm)

    assert call.finish == Finish(reason="error", failure=failure)
    assert call.message.text == "partial"


async def test_an_empty_reply_is_an_empty_response(ctx: Context, llm: Llm) -> None:
    play(ctx, llm, Reply())

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "EMPTY_RESPONSE"


async def test_a_step_can_be_an_exception(ctx: Context, llm: Llm) -> None:
    play(ctx, llm, RuntimeError("the connection dropped"))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="RuntimeError: the connection dropped", code="INTERNAL"
    )


async def test_a_step_can_be_raw_chunks(ctx: Context, llm: Llm) -> None:
    chunks: list[Chunk] = [
        BlockStart(index=3, kind="text"),
        TextDelta(index=3, text="a"),
        TextDelta(index=3, text="b"),
        BlockEnd(index=3),
        Finish(reason="stop"),
    ]
    play(ctx, llm, chunks)

    call = await ask(llm)

    assert call.chunks == chunks


async def test_raw_chunks_without_a_finish_are_a_stream_that_just_stops(
    ctx: Context, llm: Llm
) -> None:
    play(ctx, llm, (BlockStart(index=0, kind="text"), TextDelta(index=0, text="cut")))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"
    assert call.message.text == "cut"


async def test_a_step_can_be_computed_from_the_request(ctx: Context, llm: Llm) -> None:
    def echo(asked: LlmRequest) -> Reply:
        last = asked.messages[-1]
        assert isinstance(last, UserMessage)
        return Reply(text=f"you said {last.content[0].text}")

    play(ctx, llm, echo)

    call = await ask(llm, "ping")

    assert call.message.text == "you said ping"


async def test_a_script_that_runs_out_is_a_failure(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm, Reply(text="only"))
    await ask(llm)

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="the script has no reply left for call 2", code="SCRIPT_EXHAUSTED"
    )
    assert len(adapter.requests) == 2


async def test_steps_can_be_added_later(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm)
    adapter.add(Reply(text="late"), Reply(text="later"))

    assert adapter.remaining == 2
    assert (await ask(llm)).message.text == "late"
    assert adapter.remaining == 1


async def test_a_held_reply_waits_before_it_finishes(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm, Reply(text="thinking", hold=True), Reply(text="next"))
    call = llm.call(request())

    async def read() -> None:
        async for _ in call:
            pass

    reader = asyncio.ensure_future(read())
    await asyncio.wait_for(adapter.held(), timeout=1)
    assert call.message.text == "thinking"
    assert call.finish is None
    assert not reader.done()

    adapter.release()
    await asyncio.wait_for(reader, timeout=1)

    assert call.finish == Finish(reason="stop")
    assert (await ask(llm)).message.text == "next"


async def test_a_held_reply_can_be_cancelled(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm, Reply(text="thinking", hold=True), Reply(text="next", hold=True))
    first = llm.call(request())
    second = llm.call(request())

    async def read(call: LlmCall) -> None:
        async for _ in call:
            pass

    reader = asyncio.ensure_future(read(first))
    await asyncio.wait_for(adapter.held(), timeout=1)
    reader.cancel()
    with pytest.raises(asyncio.CancelledError):
        await reader
    assert first.finish is None

    reader = asyncio.ensure_future(read(second))
    await asyncio.wait_for(adapter.held(), timeout=1)
    assert second.message.text == "next"
    assert second.finish is None
    adapter.release()
    await asyncio.wait_for(reader, timeout=1)

    assert second.finish == Finish(reason="stop")


async def test_the_adapter_can_be_loaded_as_a_plugin(ctx: Context, llm: Llm) -> None:
    fiber = await ctx.plugin(
        scripted,
        {
            "provider": "canned",
            "replies": [
                {"calls": [{"name": "read", "arguments": {"path": "a.txt"}}]},
                {"text": "done"},
            ],
        },
    )
    asked = LlmRequest(
        config=CallConfig(provider="canned", model="script"),
        messages=(UserMessage(content=(TextBlock(text="go"),)),),
    )

    first = llm.call(asked)
    async for _ in first:
        pass
    second = llm.call(asked)
    async for _ in second:
        pass

    assert first.message.tool_calls == (
        ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
    )
    assert second.message.text == "done"

    await fiber.dispose()

    assert list(ctx.use(LLM).adapters) == []


async def test_the_plugin_answers_for_the_provider_scripted_by_default(
    ctx: Context, llm: Llm
) -> None:
    await ctx.plugin(scripted, {"replies": [{"text": "hi"}]})

    assert list(llm.adapters) == ["scripted"]
    assert (await ask(llm)).message.text == "hi"


def asked_with(*said: str) -> LlmRequest:
    """A request in which the user said these things, each answered but the last."""
    messages: list[UserMessage | AssistantMessage] = []
    for text in said:
        messages.append(UserMessage(content=(TextBlock(text=text),)))
        messages.append(AssistantMessage(content=(TextBlock(text="I see."),)))
    return LlmRequest(
        config=CallConfig(provider="scripted", model="script"), messages=tuple(messages[:-1])
    )


async def answered(llm: Llm, asked: LlmRequest) -> LlmCall:
    call = llm.call(asked)
    async for _ in call:
        pass
    return call


async def test_a_reply_that_is_meant_for_a_request_answers_it_whenever_it_comes(
    ctx: Context, llm: Llm
) -> None:
    adapter = play(
        ctx,
        llm,
        Reply(text="for the second", when="task B"),
        Reply(text="for anyone"),
        Reply(text="for the first", when="task A"),
    )

    # Two agents ask at once, and which of them asks first is not known.
    first = await ask(llm, "Here is task A for you.")
    other = await ask(llm, "Something else.")
    second = await ask(llm, "And task B.")

    assert [call.message.text for call in (first, other, second)] == [
        "for the first",
        "for anyone",
        "for the second",
    ]
    assert adapter.remaining == 0


async def test_replies_that_are_meant_for_the_same_request_are_played_in_order_once_each(
    ctx: Context, llm: Llm
) -> None:
    adapter = play(
        ctx,
        llm,
        Reply(text="one", when="task A"),
        Reply(text="two", when="task A"),
        Reply(text="for anyone"),
    )

    said = [(await ask(llm, "task A")).message.text for _ in range(3)]

    # When none of its own is left, a request is answered as any other is.
    assert said == ["one", "two", "for anyone"]
    assert adapter.remaining == 0


async def test_a_reply_that_is_meant_for_a_request_answers_no_other(ctx: Context, llm: Llm) -> None:
    adapter = play(ctx, llm, Reply(text="for the first", when="task A"))

    call = await ask(llm, "task B")

    assert call.failure == LlmFailure(
        message="the script has no reply left for call 1", code="SCRIPT_EXHAUSTED"
    )
    # It is still there for the request that it is meant for.
    assert adapter.remaining == 1
    assert (await ask(llm, "task A")).message.text == "for the first"


async def test_a_reply_is_meant_for_a_request_by_what_the_user_said_last(
    ctx: Context, llm: Llm
) -> None:
    play(
        ctx,
        llm,
        Reply(text="about the first", when="task A"),
        Reply(text="about the second", when="task B"),
        Reply(text="about neither"),
    )

    later = await answered(llm, asked_with("Here is task A.", "Now task B."))
    # What an assistant or a tool said after it does not make it less the last thing the user said.
    told = asked_with("No task.")
    told = told.model_copy(
        update={
            "messages": (
                *told.messages,
                AssistantMessage(content=(TextBlock(text="I looked at task A."),)),
            )
        }
    )
    neither = await answered(llm, told)
    earlier = await answered(llm, asked_with("Here is task A."))

    assert [call.message.text for call in (later, earlier, neither)] == [
        "about the second",
        "about the first",
        "about neither",
    ]


async def test_a_request_in_which_the_user_said_nothing_is_answered_by_a_reply_for_anyone(
    ctx: Context, llm: Llm
) -> None:
    adapter = play(ctx, llm, Reply(text="for the first", when="task A"), Reply(text="for anyone"))
    nobody = LlmRequest(config=CallConfig(provider="scripted", model="script"), messages=())

    call = await answered(llm, nobody)

    assert call.message.text == "for anyone"
    assert adapter.remaining == 1


def test_a_reply_cannot_be_meant_for_a_request_by_a_text_of_no_length() -> None:
    # A text of no length is in every text, so it would tell no request from another.
    with pytest.raises(ValidationError):
        Reply(text="for whom", when="")


async def test_the_plugin_reads_which_request_a_reply_is_meant_for(ctx: Context, llm: Llm) -> None:
    await ctx.plugin(
        scripted,
        {"replies": [{"text": "for the first", "when": "task A"}, {"text": "for anyone"}]},
    )

    assert (await ask(llm, "task B")).message.text == "for anyone"
    assert (await ask(llm, "task A")).message.text == "for the first"


async def test_a_reply_that_waits_for_another_request_is_sent_once_that_one_has_come() -> None:
    # The adapter is asked directly: the model service of these tests gives up on a stream
    # that stays silent for longer than a moment.
    adapter = Scripted(
        [
            Reply(text="done with A", when="task A", after="task B"),
            Reply(text="done with B", when="task B"),
        ]
    )
    sent: list[Chunk] = []

    async def read() -> None:
        async for chunk in adapter.stream(request("Here is task A.")):
            sent.append(chunk)  # noqa: PERF401 - each chunk is kept as it comes, while the test looks

    reader = asyncio.ensure_future(read())
    # Nothing of the reply is sent while no request of task B has come.
    done, _ = await asyncio.wait({reader}, timeout=0.05)
    assert not done
    assert sent == []

    other = [chunk async for chunk in adapter.stream(request("And task B."))]
    await asyncio.wait_for(reader, timeout=1)

    assert sent == [
        BlockStart(index=0, kind="text"),
        TextDelta(index=0, text="done with A"),
        BlockEnd(index=0),
        Finish(reason="stop"),
    ]
    assert TextDelta(index=0, text="done with B") in other


async def test_a_reply_waits_for_no_request_that_has_come_already(ctx: Context, llm: Llm) -> None:
    play(
        ctx,
        llm,
        Reply(text="its own", after="task B"),
        Reply(text="an earlier one", after="task B"),
    )

    # The request that it answers is one of those that have come.
    own = await asyncio.wait_for(ask(llm, "Here is task B."), timeout=1)
    earlier = await asyncio.wait_for(ask(llm, "Something else."), timeout=1)

    assert (own.message.text, earlier.message.text) == ("its own", "an earlier one")


def test_a_reply_cannot_wait_for_a_text_of_no_length() -> None:
    # A text of no length is in every text, so the reply would wait for nothing.
    with pytest.raises(ValidationError):
        Reply(text="for whom", after="")


async def test_a_reply_is_meant_for_a_request_by_what_the_user_said_and_not_by_context(
    ctx: Context, llm: Llm
) -> None:
    play(ctx, llm, Reply(text="for the task", when="task A"), Reply(text="for anyone"))
    asked = asked_with("Here is task A.")
    noted = asked.model_copy(
        update={
            "messages": (
                *asked.messages,
                UserMessage(
                    content=(
                        TextBlock(text='<context source="steps-left">\nTwo left.\n</context>'),
                    )
                ),
            )
        }
    )

    call = await answered(llm, noted)

    assert call.message.text == "for the task"
