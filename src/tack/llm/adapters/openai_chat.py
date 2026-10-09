"""An adapter for services that speak the OpenAI Chat Completions protocol with streaming.

DeepSeek speaks it, and so do the ``/v1`` endpoint of Ollama, vLLM and many
others. This module is a plugin: loading it registers one adapter under the
provider name of its config. To talk to two services, load it twice.

The API key is read from an environment variable when a request is made. It
is never part of the config, and it is cut out of the text of every failure
that leaves the adapter, whether the service or the connection wrote that text.
"""

import math
import os
import time
import uuid
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass
from datetime import UTC
from email.utils import parsedate_to_datetime
from typing import Literal

import httpx
from pydantic import BaseModel, Field, JsonValue, ValidationError

from tack.kernel import Context
from tack.llm.runtime import LLM, LlmError
from tack.llm.types import (
    AssistantMessage,
    BlockEnd,
    BlockKind,
    BlockStart,
    Chunk,
    Finish,
    LlmFailure,
    LlmRequest,
    Message,
    ReasoningBlock,
    ReasoningDelta,
    SystemMessage,
    TextBlock,
    TextDelta,
    ToolCallDelta,
    ToolMessage,
    Usage,
    UsageChunk,
    UserMessage,
)

__all__ = ["Config", "OpenAIChat", "apply", "build_body", "inject", "name"]

name = "llm-openai-chat"
inject = (LLM,)

_FINISH_REASONS: dict[str, Literal["stop", "tool-calls", "max-tokens"]] = {
    "stop": "stop",
    "tool_calls": "tool-calls",
    "function_call": "tool-calls",
    "length": "max-tokens",
}
_STATUS_CODES = {
    400: "INVALID_REQUEST",
    401: "AUTH",
    402: "QUOTA",
    403: "AUTH",
    404: "INVALID_REQUEST",
    408: "TIMEOUT",
    422: "INVALID_REQUEST",
    429: "RATE_LIMIT",
}
_EFFORT = {"low": "low", "high": "high", "max": "high"}
_MESSAGE_CHARACTERS = 500
"""How much of the text of a failure is kept."""


class Config(BaseModel):
    """The config of one adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        base_url: The address of the service, up to but without
            ``/chat/completions``.
        api_key_env: The environment variable that holds the API key, or
            ``None`` for a service that needs none.
        reasoning_style: How the service is told how much to reason.
            ``deepseek`` sends ``thinking``, ``openai`` sends
            ``reasoning_effort``, and ``none`` sends nothing.
        include_usage: Whether to ask for token counts at the end of the
            stream. Switch it off for a service that rejects the option.
        connect_timeout_seconds: How long connecting may take. Silence after
            that is the business of the model service, not of the adapter.
        headers: Further headers for every request. Do not put a key here.
    """

    provider: str
    base_url: str
    api_key_env: str | None = None
    reasoning_style: Literal["deepseek", "openai", "none"] = "none"
    include_usage: bool = True
    connect_timeout_seconds: float = Field(default=10.0, gt=0)
    headers: dict[str, str] = Field(default_factory=dict[str, str])


class _Function(BaseModel):
    name: str | None = None
    arguments: str | None = None


class _ToolCall(BaseModel):
    index: int = 0
    id: str | None = None
    function: _Function = _Function()


class _Delta(BaseModel):
    content: str | None = None
    reasoning_content: str | None = None
    reasoning: str | None = None
    tool_calls: list[_ToolCall] | None = None


class _Choice(BaseModel):
    index: int = 0
    delta: _Delta = _Delta()
    finish_reason: str | None = None


class _CachedTokens(BaseModel):
    cached_tokens: int | None = None


class _Usage(BaseModel):
    prompt_tokens: int = 0
    completion_tokens: int = 0
    prompt_cache_hit_tokens: int | None = None
    prompt_tokens_details: _CachedTokens | None = None


class _Error(BaseModel):
    message: str = "the service reported an error"
    code: str | int | None = None


class _Event(BaseModel):
    choices: list[_Choice] = Field(default_factory=list[_Choice])
    usage: _Usage | None = None
    error: _Error | None = None


class _ErrorBody(BaseModel):
    error: _Error


def _joined(content: tuple[TextBlock, ...]) -> str:
    return "\n\n".join(block.text for block in content)


def _wire(message: Message, adapter: str) -> dict[str, JsonValue]:
    """Return one message in the shape of the protocol."""
    if isinstance(message, SystemMessage):
        return {"role": "system", "content": _joined(message.content)}
    if isinstance(message, UserMessage):
        return {"role": "user", "content": _joined(message.content)}
    if isinstance(message, ToolMessage):
        return {
            "role": "tool",
            "tool_call_id": message.call_id,
            "content": _joined(message.content),
        }
    return _wire_assistant(message, adapter)


def _wire_assistant(message: AssistantMessage, adapter: str) -> dict[str, JsonValue]:
    wire: dict[str, JsonValue] = {"role": "assistant", "content": message.text or None}
    # Reasoning goes back only to the adapter that it came from, which marked it.
    reasoning = "".join(
        block.text
        for block in message.content
        if isinstance(block, ReasoningBlock)
        and block.replay_state is not None
        and block.replay_state.get("adapter") == adapter
    )
    if reasoning:
        wire["reasoning_content"] = reasoning
    if message.tool_calls:
        wire["tool_calls"] = [
            {
                "id": call.call_id,
                "type": "function",
                "function": {"name": call.name, "arguments": call.arguments or "{}"},
            }
            for call in message.tool_calls
        ]
    return wire


def build_body(request: LlmRequest, config: Config) -> dict[str, JsonValue]:
    """Return the JSON body of the request to the service.

    A setting that the call config leaves open is not sent, so that the
    default of the service applies. The ``options`` of the call config are put
    in last and as they are.
    """
    call = request.config
    body: dict[str, JsonValue] = {
        "model": call.model,
        "messages": [_wire(message, config.provider) for message in request.messages],
        "stream": True,
    }
    if config.include_usage:
        body["stream_options"] = {"include_usage": True}
    if request.tools:
        body["tools"] = [
            {
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                },
            }
            for tool in request.tools
        ]
    if call.max_output_tokens is not None:
        body["max_tokens"] = call.max_output_tokens
    if call.temperature is not None:
        body["temperature"] = call.temperature
    if call.top_p is not None:
        body["top_p"] = call.top_p
    if call.seed is not None:
        body["seed"] = call.seed
    if call.reasoning is not None:
        if config.reasoning_style == "deepseek":
            enabled = "disabled" if call.reasoning == "off" else "enabled"
            body["thinking"] = {"type": enabled}
        elif config.reasoning_style == "openai" and call.reasoning != "off":
            body["reasoning_effort"] = _EFFORT[call.reasoning]
    body.update(call.options)
    return body


async def _events(lines: AsyncIterator[str]) -> AsyncIterator[str]:
    """Yield the data of each server-sent event of a stream of lines."""
    data: list[str] = []
    async for line in lines:
        if not line:
            if data:
                yield "\n".join(data)
                data = []
            continue
        field, _, value = line.partition(":")
        if field == "data":
            data.append(value.removeprefix(" "))
    if data:
        yield "\n".join(data)


@dataclass(slots=True)
class _Call:
    """A tool call of a streamed reply.

    Attributes:
        id: What has arrived of its id.
        name: What has arrived of its name.
        waiting: The arguments that arrived before its block began.
        block: The index of its block, or -1 as long as the block has not begun.
    """

    id: str = ""
    name: str = ""
    waiting: str = ""
    block: int = -1


def _grown(so_far: str, piece: str | None) -> str:
    """Return a name or an id with one more piece of it.

    Services differ: one sends it whole and once, one in pieces, and one whole
    again with every event.
    """
    if not piece or piece == so_far:
        return so_far
    return so_far + piece


class _Translator:
    """Turns the events of one streamed reply into chunks."""

    __slots__ = ("_adapter", "_calls", "_next", "_open", "_reason", "_usage")

    def __init__(self, adapter: str) -> None:
        self._adapter = adapter
        self._next = 0
        self._open: tuple[int, BlockKind] | None = None
        self._calls: dict[int, _Call] = {}
        self._reason: str | None = None
        self._usage: Usage | None = None

    def feed(self, data: str) -> Iterator[Chunk]:
        """Take in the data of one event."""
        try:
            event = _Event.model_validate_json(data)
        except ValidationError as error:
            problem = error.errors()[0]["msg"]
            raise LlmError(
                LlmFailure(
                    message=f"the service sent an event that cannot be read: {problem}",
                    code="MALFORMED_RESPONSE",
                )
            ) from error
        if event.error is not None:
            raise LlmError(LlmFailure(message=event.error.message, code="SERVER"))
        if event.usage is not None:
            self._usage = _usage(event.usage)
        for choice in event.choices:
            if choice.index != 0:
                # A request may ask for several replies. Only the first one is read.
                continue
            delta = choice.delta
            reasoning = delta.reasoning_content or delta.reasoning
            if reasoning:
                yield from self._text("reasoning", reasoning)
            if delta.content:
                yield from self._text("text", delta.content)
            for call in delta.tool_calls or ():
                yield from self._tool(call)
            if choice.finish_reason:
                self._reason = choice.finish_reason

    def finish(self) -> Iterator[Chunk]:
        """End the reply. Without a finish reason nothing is said: the stream just stopped."""
        yield from self._close()
        reason = self._reason
        if reason is None:
            return
        if reason != "length":
            yield from self._begin_waiting()
            for block in sorted(call.block for call in self._calls.values()):
                yield BlockEnd(index=block)
        if self._usage is not None:
            yield UsageChunk(usage=self._usage)
        if reason == "content_filter":
            failure = LlmFailure(message="the provider withheld the reply", code="CONTENT_FILTER")
            yield Finish(reason="error", failure=failure)
        else:
            yield Finish(reason=_FINISH_REASONS.get(reason, "stop"))

    def _text(self, kind: Literal["reasoning", "text"], piece: str) -> Iterator[Chunk]:
        if self._open is None or self._open[1] != kind:
            yield from self._close()
            yield from self._begin_waiting()
            self._open = (self._next, kind)
            self._next += 1
            yield BlockStart(index=self._open[0], kind=kind)
        index = self._open[0]
        if kind == "reasoning":
            yield ReasoningDelta(index=index, text=piece)
        else:
            yield TextDelta(index=index, text=piece)

    def _tool(self, fragment: _ToolCall) -> Iterator[Chunk]:
        """Take in one fragment of a tool call.

        The block of a call begins once its name and its id are known in full,
        because a block says both when it begins. That is the case when a
        fragment comes without either, when something else begins, or when the
        reply ends. Arguments that arrive before then wait.
        """
        if fragment.index not in self._calls:
            yield from self._close()
        yield from self._begin_waiting(but=fragment.index)
        call = self._calls.setdefault(fragment.index, _Call())
        function = fragment.function
        if call.block < 0:
            if fragment.id or function.name or not call.name:
                call.id = _grown(call.id, fragment.id)
                call.name = _grown(call.name, function.name)
                call.waiting += function.arguments or ""
                return
            yield from self._begin(call)
        if function.arguments:
            yield ToolCallDelta(index=call.block, arguments=function.arguments)

    def _begin(self, call: _Call) -> Iterator[Chunk]:
        """Begin the block of a tool call and pass on the arguments that waited."""
        if not call.name:
            raise LlmError(
                LlmFailure(message="a tool call began without a name", code="MALFORMED_RESPONSE")
            )
        call.block = self._next
        self._next += 1
        yield BlockStart(
            index=call.block,
            kind="tool-call",
            call_id=call.id or f"call_{uuid.uuid4().hex[:12]}",
            name=call.name,
        )
        if call.waiting:
            yield ToolCallDelta(index=call.block, arguments=call.waiting)
            call.waiting = ""

    def _begin_waiting(self, but: int | None = None) -> Iterator[Chunk]:
        """Begin the block of every tool call that has none yet, except the call ``but``."""
        for index, call in sorted(self._calls.items()):
            if call.block < 0 and index != but:
                yield from self._begin(call)

    def _close(self) -> Iterator[Chunk]:
        """End the block of text or reasoning that is open, if any."""
        if self._open is None:
            return
        index, kind = self._open
        self._open = None
        state: dict[str, JsonValue] | None = None
        if kind == "reasoning":
            state = {"adapter": self._adapter}
        yield BlockEnd(index=index, replay_state=state)


def _usage(raw: _Usage) -> Usage:
    cached = raw.prompt_cache_hit_tokens
    if cached is None and raw.prompt_tokens_details is not None:
        cached = raw.prompt_tokens_details.cached_tokens
    cached = cached or 0
    return Usage(
        input_tokens=max(raw.prompt_tokens - cached, 0),
        output_tokens=raw.completion_tokens,
        cache_read_tokens=cached,
    )


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


def _retry_after_ms(response: httpx.Response) -> int | None:
    """Return how long the service asks to wait: it names seconds, or a point in time."""
    value = response.headers.get("retry-after", "")
    try:
        seconds = float(value)
    except ValueError:
        try:
            moment = parsedate_to_datetime(value)
        except ValueError:
            return None
        if moment.tzinfo is None:
            # A date of HTTP is in universal time also where it does not say so.
            moment = moment.replace(tzinfo=UTC)
        return max(round((moment.timestamp() - time.time()) * 1000), 0)
    return round(seconds * 1000) if math.isfinite(seconds) and seconds >= 0 else None


def _refusal(response: httpx.Response) -> LlmFailure:
    """Turn a response that is not a stream into a failure."""
    status = response.status_code
    code = _STATUS_CODES.get(status) or ("SERVER" if status >= 500 else f"HTTP_{status}")
    text = response.text
    message = text or f"HTTP {status} {response.reason_phrase}"
    try:
        error = _ErrorBody.model_validate_json(text).error
    except ValidationError:
        pass
    else:
        message = error.message
        lowered = message.lower()
        if error.code == "insufficient_quota":
            code = "QUOTA"
        elif status == 400 and (
            error.code == "context_length_exceeded" or "context length" in lowered
        ):
            code = "CONTEXT_WINDOW_EXCEEDED"
    return LlmFailure(
        message=message,
        code=code,
        status=status,
        retry_after_ms=_retry_after_ms(response),
        request_id=response.headers.get("x-request-id"),
    )


def _failure(error: httpx.RequestError | LlmError, secret: str | None) -> LlmFailure:
    """Return the failure that an error stands for, fit to be shown and to be stored.

    Every failure of the adapter passes through here. The key is cut out of
    its text first, and then the text is cut to a length, so that no part of
    the key is left at the end of it.
    """
    if isinstance(error, LlmError):
        failure = error.failure
    else:
        code = "TIMEOUT" if isinstance(error, httpx.TimeoutException) else "TRANSPORT"
        failure = LlmFailure(message=_describe(error), code=code)
    message = failure.message
    if secret:
        message = message.replace(secret, "[key]")
    return failure.model_copy(update={"message": message[:_MESSAGE_CHARACTERS]})


class OpenAIChat:
    """Talks to one service that speaks the Chat Completions protocol.

    Args:
        config: Where the service is and how to talk to it.
        transport: The transport of the HTTP client, for tests.
    """

    __slots__ = ("_client", "_config")

    def __init__(
        self, config: Config, *, transport: httpx.AsyncBaseTransport | None = None
    ) -> None:
        self._config = config
        self._client = httpx.AsyncClient(
            base_url=config.base_url,
            headers=config.headers,
            timeout=httpx.Timeout(config.connect_timeout_seconds, read=None),
            transport=transport,
        )

    async def aclose(self) -> None:
        """Close the connections to the service."""
        await self._client.aclose()

    async def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Yield the chunks of the reply to ``request``.

        Raises:
            LlmError: The key is missing, the service could not be reached or
                refused, or its reply cannot be read.
        """
        secret = self._secret()
        headers = {} if secret is None else {"Authorization": f"Bearer {secret}"}
        body = build_body(request, self._config)
        translator = _Translator(self._config.provider)
        try:
            async with self._client.stream(
                "POST", "chat/completions", json=body, headers=headers
            ) as response:
                if response.status_code != 200:
                    await response.aread()
                    raise LlmError(_refusal(response))
                async for data in _events(response.aiter_lines()):
                    if data == "[DONE]":
                        break
                    for chunk in translator.feed(data):
                        yield chunk
            for chunk in translator.finish():
                yield chunk
        except (httpx.RequestError, LlmError) as error:
            # The error itself is left behind: its text may hold the key, and so may its cause.
            raise LlmError(_failure(error, secret)) from None

    def _secret(self) -> str | None:
        variable = self._config.api_key_env
        if variable is None:
            return None
        secret = os.environ.get(variable)
        if not secret:
            raise LlmError(
                LlmFailure(message=f"the environment variable {variable} is not set", code="AUTH")
            )
        return secret


def apply(ctx: Context, config: Config) -> None:
    """Register an adapter for the service of ``config``."""
    adapter = OpenAIChat(config)
    ctx.effect(lambda: adapter.aclose)
    ctx.use(LLM).adapters.add(ctx, config.provider, adapter)
