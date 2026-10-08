"""The model service: one place where calls are made and failures are normalised."""

import asyncio
import logging
from collections.abc import AsyncIterator

import pytest
from pydantic import ValidationError

import tack.llm
from tack.kernel import Context, Plugin, RegistryConflictError
from tack.llm import (
    LLM,
    AssistantMessage,
    BlockEnd,
    BlockStart,
    CallConfig,
    Chunk,
    Finish,
    FinishReason,
    Llm,
    LlmError,
    LlmFailure,
    LlmRequest,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    ToolCallDelta,
    Usage,
    UsageChunk,
    UserMessage,
)

REQUEST = LlmRequest(
    config=CallConfig(provider="fake", model="tiny"),
    messages=(UserMessage(content=(TextBlock(text="hello"),)),),
)

HELLO: list[Chunk] = [
    BlockStart(index=0, kind="text"),
    TextDelta(index=0, text="Hel"),
    TextDelta(index=0, text="lo"),
    BlockEnd(index=0),
    UsageChunk(usage=Usage(input_tokens=3, output_tokens=2)),
    Finish(reason="stop"),
]

TOOL_CALL: list[Chunk] = [
    BlockStart(index=0, kind="tool-call", call_id="call-1", name="read"),
    ToolCallDelta(index=0, arguments='{"path": "a.txt"}'),
    BlockEnd(index=0),
]


class Fake:
    """An adapter that replays fixed chunks, then optionally raises or hangs."""

    def __init__(
        self,
        chunks: list[Chunk],
        *,
        then: BaseException | None = None,
        hang: bool = False,
        fail_on_close: bool = False,
    ) -> None:
        self.chunks = chunks
        self.then = then
        self.hang = hang
        self.fail_on_close = fail_on_close
        self.requests: list[LlmRequest] = []
        self.closed = False

    async def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        self.requests.append(request)
        try:
            for chunk in self.chunks:
                yield chunk
            if self.then is not None:
                raise self.then
            if self.hang:
                await asyncio.Event().wait()
        finally:
            self.closed = True
            if self.fail_on_close:
                raise RuntimeError("close failed")


async def drain(llm: Llm, request: LlmRequest = REQUEST) -> tuple[list[Chunk], Finish]:
    call = llm.call(request)
    seen = [chunk async for chunk in call]
    assert call.finish is not None
    return seen, call.finish


async def read_until(call: tack.llm.LlmCall, last: Chunk, arrived: asyncio.Event) -> None:
    async for chunk in call:
        if chunk == last:
            arrived.set()


# Streaming


async def test_a_call_streams_the_chunks_and_assembles_the_reply(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO)
    llm.adapters.add(ctx, "fake", adapter)

    call = llm.call(REQUEST)
    seen = [chunk async for chunk in call]

    assert seen == HELLO
    assert call.chunks == HELLO
    assert call.message == AssistantMessage(content=(TextBlock(text="Hello"),))
    assert call.usage == Usage(input_tokens=3, output_tokens=2)
    assert call.finish == Finish(reason="stop")
    assert call.failure is None
    assert adapter.requests == [REQUEST]
    assert adapter.closed


async def test_nothing_is_sent_until_the_call_is_iterated(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO)
    llm.adapters.add(ctx, "fake", adapter)

    call = llm.call(REQUEST)

    assert adapter.requests == []
    assert call.finish is None
    assert call.message == AssistantMessage(content=())


async def test_a_call_can_be_iterated_only_once(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO))
    call = llm.call(REQUEST)
    async for _ in call:
        pass

    with pytest.raises(RuntimeError, match="only once"):
        async for _ in call:
            pass


async def test_nothing_is_read_after_the_finish(ctx: Context, llm: Llm) -> None:
    adapter = Fake([*HELLO, TextDelta(index=0, text="late")])
    llm.adapters.add(ctx, "fake", adapter)

    seen, finish = await drain(llm)

    assert seen == HELLO
    assert finish == Finish(reason="stop")


async def test_an_adapter_may_return_an_iterator_that_cannot_be_closed(
    ctx: Context, llm: Llm
) -> None:
    class Plain:
        def __init__(self) -> None:
            self.left = list(HELLO)

        def __aiter__(self) -> "Plain":
            return self

        async def __anext__(self) -> Chunk:
            if not self.left:
                raise StopAsyncIteration
            return self.left.pop(0)

    class Adapter:
        def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
            return Plain()

    llm.adapters.add(ctx, "fake", Adapter())

    seen, _ = await drain(llm)

    assert seen == HELLO


# Failures


class Refusing:
    """An adapter that fails before it returns a stream."""

    def __init__(self, error: Exception) -> None:
        self.error = error

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        raise self.error


async def test_an_adapter_may_refuse_a_request_before_it_streams(ctx: Context, llm: Llm) -> None:
    failure = LlmFailure(message="this model takes no tools", code="INVALID_REQUEST")
    llm.adapters.add(ctx, "fake", Refusing(LlmError(failure)))

    seen, finish = await drain(llm)

    assert seen == [Finish(reason="error", failure=failure)]
    assert finish.failure == failure


async def test_an_adapter_that_cannot_start_is_an_internal_failure(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Refusing(RuntimeError("no client")))

    _, finish = await drain(llm)

    assert finish.failure == LlmFailure(message="RuntimeError: no client", code="INTERNAL")


async def test_an_unknown_provider_is_a_failure(llm: Llm) -> None:
    seen, finish = await drain(llm)

    assert finish.failure == LlmFailure(
        message="no adapter is registered for the provider 'fake'", code="INVALID_REQUEST"
    )
    assert seen == [finish]


async def test_a_failure_raised_by_the_adapter_ends_the_stream(ctx: Context, llm: Llm) -> None:
    failure = LlmFailure(message="slow down", code="RATE_LIMIT", status=429, retry_after_ms=1500)
    llm.adapters.add(ctx, "fake", Fake(HELLO[:2], then=LlmError(failure)))

    call = llm.call(REQUEST)
    seen = [chunk async for chunk in call]

    assert seen == [*HELLO[:2], Finish(reason="error", failure=failure)]
    assert call.failure == failure
    assert call.message == AssistantMessage(content=(TextBlock(text="Hel"),))


async def test_an_error_finish_from_the_adapter_is_passed_on(ctx: Context, llm: Llm) -> None:
    finish = Finish(reason="error", failure=LlmFailure(message="overloaded", code="SERVER_ERROR"))
    llm.adapters.add(ctx, "fake", Fake([finish]))

    seen, seen_finish = await drain(llm)

    assert seen == [finish]
    assert seen_finish == finish


async def test_an_unexpected_exception_is_an_internal_failure(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([], then=RuntimeError("adapter bug")))

    _, finish = await drain(llm)

    assert finish.failure == LlmFailure(message="RuntimeError: adapter bug", code="INTERNAL")


async def test_an_exception_without_text_is_described_by_its_type(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([], then=KeyError()))

    _, finish = await drain(llm)

    assert finish.failure == LlmFailure(message="KeyError", code="INTERNAL")


async def test_a_timeout_inside_the_adapter_is_a_timeout(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([], then=TimeoutError("the connection timed out")))

    _, finish = await drain(llm)

    assert finish.failure == LlmFailure(
        message="TimeoutError: the connection timed out", code="TIMEOUT"
    )


async def test_an_adapter_that_cancels_its_own_stream_is_a_failure(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO[:2], then=asyncio.CancelledError()))

    call = llm.call(REQUEST)
    seen = [chunk async for chunk in call]

    assert call.failure == LlmFailure(
        message="the adapter's stream was cancelled from inside", code="INTERNAL"
    )
    assert seen[:2] == HELLO[:2]


async def test_a_stream_that_just_stops_is_a_failure(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO[:4]))

    seen, finish = await drain(llm)

    assert finish.failure == LlmFailure(
        message="the stream ended without a finish", code="STREAM_CLOSED"
    )
    assert seen == [*HELLO[:4], finish]


@pytest.mark.parametrize("reason", ["stop", "tool-calls", "max-tokens"])
async def test_a_reply_without_content_is_a_failure(
    ctx: Context, llm: Llm, reason: FinishReason
) -> None:
    llm.adapters.add(ctx, "fake", Fake([Finish(reason=reason)]))

    seen, finish = await drain(llm)

    assert finish.failure == LlmFailure(
        message=f"the reply ended ({reason}) without any content", code="EMPTY_RESPONSE"
    )
    assert seen == [finish]


async def test_a_stream_that_breaks_the_protocol_is_a_failure(ctx: Context, llm: Llm) -> None:
    adapter = Fake([TextDelta(index=0, text="orphan"), Finish(reason="stop")])
    llm.adapters.add(ctx, "fake", adapter)

    seen, finish = await drain(llm)

    assert finish.failure == LlmFailure(
        message="block 0 was never started", code="MALFORMED_RESPONSE"
    )
    assert seen == [finish]
    assert adapter.closed


async def test_a_stream_that_goes_quiet_times_out(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO[:2], hang=True)
    llm.adapters.add(ctx, "fake", adapter)

    call = llm.call(REQUEST)
    seen = [chunk async for chunk in call]

    assert call.failure == LlmFailure(message="no chunk arrived for 0.05 seconds", code="TIMEOUT")
    assert seen[:2] == HELLO[:2]
    assert call.message == AssistantMessage(content=(TextBlock(text="Hel"),))
    assert adapter.closed


async def test_a_stream_that_goes_quiet_and_fails_to_close_still_times_out(
    ctx: Context, llm: Llm
) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO[:2], hang=True, fail_on_close=True))

    _, finish = await drain(llm)

    assert finish.failure == LlmFailure(message="no chunk arrived for 0.05 seconds", code="TIMEOUT")


# The finish agrees with what arrived


async def test_a_reply_with_tool_calls_finishes_as_tool_calls(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([*TOOL_CALL, Finish(reason="stop")]))

    call = llm.call(REQUEST)
    seen = [chunk async for chunk in call]

    assert call.finish == Finish(reason="tool-calls")
    assert seen == [*TOOL_CALL, Finish(reason="tool-calls")]
    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
    )


async def test_a_reply_without_tool_calls_finishes_as_stop(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([*HELLO[:4], Finish(reason="tool-calls")]))

    _, finish = await drain(llm)

    assert finish == Finish(reason="stop")


async def test_a_reply_cut_off_by_the_token_limit_keeps_its_reason(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake([*HELLO[:3], Finish(reason="max-tokens")]))

    call = llm.call(REQUEST)
    async for _ in call:
        pass

    assert call.finish == Finish(reason="max-tokens")
    assert call.message == AssistantMessage(content=(TextBlock(text="Hello"),))


# Closing


async def test_the_stream_is_closed_before_the_finish_is_delivered(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO)
    llm.adapters.add(ctx, "fake", adapter)

    closed_at_finish = [
        adapter.closed async for chunk in llm.call(REQUEST) if isinstance(chunk, Finish)
    ]

    assert closed_at_finish == [True]


async def test_leaving_a_call_early_closes_the_stream(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO)
    llm.adapters.add(ctx, "fake", adapter)

    async with llm.call(REQUEST) as call:
        async for chunk in call:
            if chunk == HELLO[1]:
                break

    assert adapter.closed
    assert call.finish is None
    assert call.chunks == HELLO[:2]
    assert call.message == AssistantMessage(content=(TextBlock(text="Hel"),))


async def test_closing_a_call_that_was_never_iterated_does_nothing(ctx: Context, llm: Llm) -> None:
    adapter = Fake(HELLO)
    llm.adapters.add(ctx, "fake", adapter)

    async with llm.call(REQUEST):
        pass

    assert adapter.requests == []


async def test_cancelling_the_reader_closes_the_stream_and_keeps_what_arrived(
    ctx: Context,
) -> None:
    await ctx.plugin(tack.llm, {"idle_timeout_seconds": 30})
    llm = ctx.use(LLM)
    adapter = Fake(HELLO[:2], hang=True)
    llm.adapters.add(ctx, "fake", adapter)
    call = llm.call(REQUEST)
    arrived = asyncio.Event()

    reader = asyncio.ensure_future(read_until(call, HELLO[1], arrived))
    await asyncio.wait_for(arrived.wait(), timeout=1)
    reader.cancel()
    with pytest.raises(asyncio.CancelledError):
        await reader

    assert adapter.closed
    assert call.finish is None
    assert call.message == AssistantMessage(content=(TextBlock(text="Hel"),))
    assert call.chunks == HELLO[:2]


async def test_an_adapter_that_fails_to_close_does_not_spoil_the_reply(
    ctx: Context, llm: Llm, caplog: pytest.LogCaptureFixture
) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO, fail_on_close=True))

    with caplog.at_level(logging.ERROR, logger="tack.llm"):
        seen, finish = await drain(llm)

    assert seen == HELLO
    assert finish == Finish(reason="stop")
    assert "failed to close its stream" in caplog.text
    assert "close failed" in caplog.text


async def test_an_adapter_that_fails_to_close_does_not_swallow_a_cancellation(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    await ctx.plugin(tack.llm, {"idle_timeout_seconds": 30})
    llm = ctx.use(LLM)
    llm.adapters.add(ctx, "fake", Fake(HELLO[:2], hang=True, fail_on_close=True))
    call = llm.call(REQUEST)
    arrived = asyncio.Event()

    reader = asyncio.ensure_future(read_until(call, HELLO[1], arrived))
    await asyncio.wait_for(arrived.wait(), timeout=1)
    reader.cancel()
    with caplog.at_level(logging.ERROR, logger="tack.llm"), pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(reader, timeout=1)

    assert call.finish is None
    assert "failed while its stream was being cancelled" in caplog.text
    assert "close failed" in caplog.text


async def test_a_failure_stays_a_failure_in_a_task_that_survived_a_cancellation(
    ctx: Context, llm: Llm
) -> None:
    error = RuntimeError("adapter bug")
    error.__context__ = ValueError("an earlier error that was no cancellation")
    llm.adapters.add(ctx, "fake", Fake([], then=error))
    waiting = asyncio.Event()

    async def read() -> LlmFailure | None:
        try:
            waiting.set()
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            pass
        _, finish = await drain(llm)
        return finish.failure

    reader = asyncio.ensure_future(read())
    await asyncio.wait_for(waiting.wait(), timeout=1)
    reader.cancel()

    failure = await asyncio.wait_for(reader, timeout=1)

    assert failure == LlmFailure(message="RuntimeError: adapter bug", code="INTERNAL")


# The service


async def test_a_provider_can_be_registered_only_once(ctx: Context, llm: Llm) -> None:
    llm.adapters.add(ctx, "fake", Fake(HELLO))

    with pytest.raises(
        RegistryConflictError, match="the model provider 'fake' is already registered"
    ):
        llm.adapters.add(ctx, "fake", Fake(HELLO))

    assert list(llm.adapters) == ["fake"]


async def test_an_adapter_is_unregistered_with_the_plugin_that_registered_it(
    ctx: Context, llm: Llm
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(LLM).adapters.add(inner, "fake", Fake(HELLO))

    fiber = await ctx.plugin(Plugin("fake-model", apply, inject=(LLM,)))
    assert list(llm.adapters) == ["fake"]

    await fiber.dispose()

    assert list(llm.adapters) == []


@pytest.mark.parametrize("seconds", [0, -1])
async def test_the_idle_timeout_must_be_positive(ctx: Context, seconds: float) -> None:
    fiber = ctx.plugin(tack.llm, {"idle_timeout_seconds": seconds})

    with pytest.raises(ValidationError, match="idle_timeout_seconds"):
        await fiber

    assert ctx.get(LLM) is None
