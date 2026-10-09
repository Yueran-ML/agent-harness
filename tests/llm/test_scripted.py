"""The scripted adapter: a model whose replies are written down in advance."""

import asyncio

import pytest

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
