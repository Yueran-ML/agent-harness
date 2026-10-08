"""The model service: adapters are registered here, and every call goes through here."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import AsyncGenerator, AsyncIterator
from typing import Protocol, Self

from tack import cancellation
from tack.kernel import Registry, ServiceKey
from tack.llm.assembler import Assembler, MalformedStreamError
from tack.llm.types import (
    AssistantMessage,
    Chunk,
    Finish,
    LlmFailure,
    LlmRequest,
    Usage,
)

logger = logging.getLogger("tack.llm")


class LlmError(Exception):
    """Raised by an adapter to report that a call failed."""

    def __init__(self, failure: LlmFailure) -> None:
        super().__init__(failure.message)
        self.failure = failure


class LlmAdapter(Protocol):
    """Talks to one provider.

    One call to ``stream`` is one attempt. An adapter does not retry. It
    reports a failure either by raising :class:`LlmError` or by ending the
    stream with an error finish.
    """

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Yield the chunks of the reply to ``request``, ending with a finish."""
        ...


def _failed(code: str, message: str) -> Finish:
    return Finish(reason="error", failure=LlmFailure(message=message, code=code))


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


class LlmCall:
    """One call to a model.

    Iterate the call to receive the chunks as they arrive. Whatever the adapter
    does, the stream ends with exactly one finish chunk, and that finish agrees
    with what arrived:

    * A failure of any kind is a finish whose reason is ``error``.
    * Any other finish comes with a reply that has content.
    * The reason ``tool-calls`` means that the reply holds tool calls, and the
      reason ``stop`` means that it holds none, whatever the provider said.

    The adapter's stream is closed before the finish is delivered. It is also
    closed when the reader is cancelled, and when the call is closed early,
    for which the call is an asynchronous context manager::

        async with llm.call(request) as call:
            async for chunk in call:
                ...

    A failure to close is logged and changes nothing else. A cancellation of
    the reader always gets through. One that arrives while the stream is
    closing cuts the closing short, as it does with any awaited cleanup, and
    :meth:`aclose` then tries again. An adapter whose cleanup must not be cut
    short has to shield it.

    While and after iterating, the attributes describe what has been delivered
    so far. They stay readable after a cancellation or an early close, in
    which case there is no finish.

    Attributes:
        chunks: Every chunk delivered so far.
    """

    __slots__ = (
        "_adapter",
        "_assembler",
        "_finish",
        "_idle_timeout",
        "_request",
        "_source",
        "_stream",
        "chunks",
    )

    def __init__(
        self, adapter: LlmAdapter | None, request: LlmRequest, idle_timeout: float
    ) -> None:
        self._adapter = adapter
        self._request = request
        self._idle_timeout = idle_timeout
        self._assembler = Assembler()
        self._finish: Finish | None = None
        self._source: AsyncIterator[Chunk] | None = None
        self._stream: AsyncGenerator[Chunk] | None = None
        self.chunks: list[Chunk] = []

    @property
    def message(self) -> AssistantMessage:
        """The reply as assembled from the chunks so far."""
        return self._assembler.message()

    @property
    def usage(self) -> Usage:
        """The most recent token counts."""
        return self._assembler.usage

    @property
    def finish(self) -> Finish | None:
        """The finish chunk, or ``None`` if it has not been delivered."""
        return self._finish

    @property
    def failure(self) -> LlmFailure | None:
        """Why the call failed, if it did."""
        return None if self._finish is None else self._finish.failure

    def __aiter__(self) -> AsyncIterator[Chunk]:
        if self._stream is not None:
            raise RuntimeError("a call can be iterated only once")
        self._stream = self._run()
        return self._stream

    async def aclose(self) -> None:
        """Stop reading and close the adapter's stream.

        Closing a second time does nothing, unless closing the adapter's
        stream was cut short by a cancellation before: then it is tried again.
        """
        if self._stream is not None:
            await self._stream.aclose()
        await self._close_source()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        await self.aclose()

    async def _run(self) -> AsyncGenerator[Chunk]:
        opened = self._open()
        if isinstance(opened, Finish):
            ending = opened
        else:
            self._source = opened
            try:
                while True:
                    chunk = await self._next(opened)
                    if isinstance(chunk, Finish):
                        break
                    self.chunks.append(chunk)
                    yield chunk
            finally:
                await self._close_source()
            ending = chunk
        # Only now does the finish count: a reader that was cancelled while the
        # stream was closing has not seen it, and neither has the reply.
        finish = self._settle(ending)
        self._finish = finish
        self.chunks.append(finish)
        yield finish

    def _open(self) -> AsyncIterator[Chunk] | Finish:
        """Ask the adapter for its stream. A finish is returned if there is none to read."""
        if self._adapter is None:
            provider = self._request.config.provider
            return _failed(
                "INVALID_REQUEST", f"no adapter is registered for the provider {provider!r}"
            )
        try:
            return aiter(self._adapter.stream(self._request))
        except Exception as error:
            return self._failure(error, timed_out=False)

    async def _next(self, source: AsyncIterator[Chunk]) -> Chunk:
        """Read one chunk. A finish is returned when the stream is over, however it ended."""
        before = cancellation.requests()
        watchdog = asyncio.timeout(self._idle_timeout)
        try:
            async with watchdog:
                chunk = await anext(source)
        except asyncio.CancelledError:
            if cancellation.requested_since(before):
                raise
            return _failed("INTERNAL", "the adapter's stream was cancelled from inside")
        except Exception as error:
            if cancellation.replaced(error, before):
                logger.error(
                    "an adapter failed while its stream was being cancelled", exc_info=error
                )
                raise asyncio.CancelledError from error
            return self._failure(error, timed_out=watchdog.expired())
        if isinstance(chunk, Finish):
            return chunk
        try:
            self._assembler.push(chunk)
        except MalformedStreamError as error:
            return _failed("MALFORMED_RESPONSE", str(error))
        return chunk

    async def _close_source(self) -> None:
        """Close the adapter's stream, unless that has been done.

        A failure to close is logged, not raised. Only a cancellation of the
        current task gets through. If it cut the closing short, the stream
        counts as open, and the next call tries again.
        """
        source = self._source
        if source is None:
            return
        close = getattr(source, "aclose", None)
        if close is not None:
            before = cancellation.requests()
            try:
                await close()
            except asyncio.CancelledError:
                if cancellation.requested_since(before):
                    raise
                logger.error("an adapter cancelled the closing of its stream from inside")
            except Exception as error:
                logger.error("an adapter failed to close its stream", exc_info=error)
                if cancellation.replaced(error, before):
                    self._source = None
                    raise asyncio.CancelledError from error
        self._source = None

    def _failure(self, error: Exception, *, timed_out: bool) -> Finish:
        """Turn what reading a chunk raised into an error finish."""
        if timed_out:
            if not isinstance(error, TimeoutError):
                logger.error(
                    "an adapter failed while its stream was being cancelled", exc_info=error
                )
            return _failed("TIMEOUT", f"no chunk arrived for {self._idle_timeout:g} seconds")
        if isinstance(error, StopAsyncIteration):
            return _failed("STREAM_CLOSED", "the stream ended without a finish")
        if isinstance(error, LlmError):
            return Finish(reason="error", failure=error.failure)
        if isinstance(error, TimeoutError):
            return _failed("TIMEOUT", _describe(error))
        return _failed("INTERNAL", _describe(error))

    def _settle(self, finish: Finish) -> Finish:
        """Take in the finish that ended the stream and make it agree with what arrived."""
        self._assembler.push(finish)
        if finish.reason == "error":
            return finish
        message = self._assembler.message()
        if not message.content:
            return _failed(
                "EMPTY_RESPONSE", f"the reply ended ({finish.reason}) without any content"
            )
        if finish.reason == "max-tokens":
            return finish
        return Finish(reason="tool-calls" if message.tool_calls else "stop")


class Llm:
    """The model service.

    Attributes:
        adapters: The adapters, by the name of the provider each one answers for.
    """

    __slots__ = ("_idle_timeout", "adapters")

    def __init__(self, idle_timeout_seconds: float) -> None:
        self.adapters: Registry[LlmAdapter] = Registry("model provider")
        self._idle_timeout = idle_timeout_seconds

    def call(self, request: LlmRequest) -> LlmCall:
        """Prepare a call. Nothing is sent until the returned call is iterated."""
        adapter = self.adapters.get(request.config.provider)
        return LlmCall(adapter, request, self._idle_timeout)


LLM = ServiceKey[Llm]("llm")
"""The model service."""
