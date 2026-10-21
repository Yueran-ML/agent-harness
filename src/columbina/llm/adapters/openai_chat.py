"""An adapter for services that speak the OpenAI Chat Completions protocol with streaming.

DeepSeek speaks it, and so do the ``/v1`` endpoint of Ollama, vLLM and many
others. This module is a plugin: loading it registers one adapter under the
provider name of its config. To talk to two services, load it twice.

The API key is looked for when a request is made: first in an environment
variable, then among the keys that the user stored, under the name of the
provider. The file of keys says which there are, and a key lies in the
keyring of the system or in that file, see :mod:`columbina.llm.credentials`.
A stored key is one that was typed in for an address, and it is used only
for requests to the place that this address leads to: the address of an
adapter may come from a variable of the environment, and a key does not
follow it there. A stored key is in the memory of this process and nowhere
else: a command that an agent runs does not find it in its environment.

The key is never part of the config, and it is cut out of the text of every
failure that leaves the adapter, whether the service or the connection wrote
that text. It is looked for as it is and in the spellings in which a text
commonly repeats it: as JSON writes it, as an address writes it, as a page
of HTML writes it. A service that shows a part of the key, with the rest
blotted out, is shown as it wrote that. The error that was caught is not
passed on with the failure.

Besides replies, a service is asked for the list of its models, with the key
and in the same care for it. That is how ``columbina doctor`` sees that a
service answers, and how ``columbina setup --check`` sees that it takes a key
that was typed and is not stored: such a key is handed to the adapter itself.

Where no variable of the environment names a proxy, the requests go through
the proxy of the system, see :mod:`columbina.net`.

Blanks and line breaks around the key are not sent. A key with anything in it
that a header cannot carry is refused before a request is made. The library
that sends the request would refuse it too, and would say so with the key in
its words, written in a form that is not found when the key is looked for.

One thing is not in the hands of the adapter. The HTTP library has a logger of
its own, ``httpx``, which at the level INFO writes the status line of every
response as the service sent it.
"""

import os
import uuid
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

import httpx
from pydantic import BaseModel, Field, JsonValue, ValidationError

from columbina.kernel import Context
from columbina.llm.adapters.common import events, failure_of, retry_after_ms, secret_of
from columbina.llm.credentials import trimmed
from columbina.llm.runtime import LLM, LlmError
from columbina.llm.types import (
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
from columbina.net import proxy_for

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
_LIST_SECONDS = 30.0
"""How long the list of the models of a service may take to arrive."""


class Config(BaseModel):
    """The config of one adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        base_url: The address of the service, up to but without
            ``/chat/completions``.
        api_key_env: The environment variable that holds the API key, or
            ``None`` if no variable does.
        api_key_file: The file of keys that the user stored, as
            ``columbina setup`` writes it, or ``None`` if there is none. The
            key that it knows of for ``provider`` is used when the variable
            holds none.
        api_key_required: Whether a request is made only with a key. A
            service on the machine of the user asks for none, and is sent
            one only if one is found. Where neither a variable nor a file is
            named there is no key to look for, and none is sent.
        reasoning_style: How the service is told how much to reason.
            ``deepseek`` sends ``thinking``, and with it ``reasoning_effort``
            where the thinking is on. ``openai`` sends ``reasoning_effort``,
            which it takes as ``low`` or ``high``, and ``none`` sends nothing.
            What a call asks for is sent over what its options say of either.
        include_usage: Whether to ask for token counts at the end of the
            stream. Switch it off for a service that rejects the option.
        max_tokens_field: The name under which the limit on the length of a
            reply is sent. The models of OpenAI that reason take it only as
            ``max_completion_tokens``.
        connect_timeout_seconds: How long connecting may take. Silence after
            that is the business of the model service, not of the adapter.
        headers: Further headers for every request. Do not put a key here.
    """

    provider: str
    base_url: str
    api_key_env: str | None = None
    api_key_file: Path | None = None
    api_key_required: bool = True
    reasoning_style: Literal["deepseek", "openai", "none"] = "none"
    include_usage: bool = True
    max_tokens_field: Literal["max_tokens", "max_completion_tokens"] = "max_tokens"
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


class _Listed(BaseModel):
    id: str


class _Listing(BaseModel):
    data: list[_Listed]


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
        body[config.max_tokens_field] = call.max_output_tokens
    if call.temperature is not None:
        body["temperature"] = call.temperature
    if call.top_p is not None:
        body["top_p"] = call.top_p
    if call.seed is not None:
        body["seed"] = call.seed
    body.update(call.options)
    # What the call asks of the reasoning is written last, over what the options say of it.
    if call.reasoning is not None:
        if config.reasoning_style == "deepseek":
            if call.reasoning == "off":
                body["thinking"] = {"type": "disabled"}
                body.pop("reasoning_effort", None)
            else:
                body["thinking"] = {"type": "enabled"}
                body["reasoning_effort"] = call.reasoning
        elif config.reasoning_style == "openai" and call.reasoning != "off":
            body["reasoning_effort"] = _EFFORT[call.reasoning]
    return body


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
        fragment of the call comes without either, or when the reply ends.
        Arguments that arrive before then wait. Nothing else shows that a name
        is complete: fragments of another call, or text, may come in between.
        """
        if fragment.index not in self._calls:
            yield from self._close()
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

    def _begin_waiting(self) -> Iterator[Chunk]:
        """Begin the block of every tool call that has none yet."""
        for _, call in sorted(self._calls.items()):
            if call.block < 0:
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
        retry_after_ms=retry_after_ms(response),
        request_id=response.headers.get("x-request-id"),
    )


def _listed(content: bytes) -> list[str]:
    """Return the ids of the models in a list that a service sent.

    Raises:
        LlmError: What it sent is no list of models.
    """
    try:
        listing = _Listing.model_validate_json(content)
    except ValidationError as error:
        problem = error.errors()[0]["msg"]
        raise LlmError(
            LlmFailure(
                message=f"the service sent a list of models that cannot be read: {problem}",
                code="MALFORMED_RESPONSE",
            )
        ) from None
    return [model.id for model in listing.data]


class OpenAIChat:
    """Talks to one service that speaks the Chat Completions protocol.

    Args:
        config: Where the service is and how to talk to it.
        transport: The transport of the HTTP client, for tests. A request that
            goes to it goes through no proxy.
        secret: A key to send instead of the one that would be looked for: one
            that is checked before it is stored. It is held by this object and
            nowhere else.
    """

    __slots__ = ("_client", "_config", "_given")

    def __init__(
        self,
        config: Config,
        *,
        transport: httpx.AsyncBaseTransport | None = None,
        secret: str | None = None,
    ) -> None:
        self._config = config
        self._given = trimmed(secret or "") or None
        self._client = httpx.AsyncClient(
            base_url=config.base_url,
            headers=config.headers,
            timeout=httpx.Timeout(config.connect_timeout_seconds, read=None),
            transport=transport,
            proxy=None if transport is not None else proxy_for(config.base_url, os.environ),
        )

    @property
    def reasoning_style(self) -> Literal["deepseek", "openai", "none"]:
        """How the service is told how much to reason."""
        return self._config.reasoning_style

    async def aclose(self) -> None:
        """Close the connections to the service."""
        await self._client.aclose()

    async def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Yield the chunks of the reply to ``request``.

        Raises:
            LlmError: The key is missing, the service could not be reached or
                refused, or its reply cannot be read.
        """
        secret = await secret_of(self._config, self._given)
        headers = {} if secret is None else {"Authorization": f"Bearer {secret}"}
        body = build_body(request, self._config)
        translator = _Translator(self._config.provider)
        failure: LlmFailure | None = None
        try:
            async with self._client.stream(
                "POST", "chat/completions", json=body, headers=headers
            ) as response:
                if response.status_code != 200:
                    await response.aread()
                    raise LlmError(_refusal(response))
                async for data in events(response.aiter_lines()):
                    if data == "[DONE]":
                        break
                    for chunk in translator.feed(data):
                        yield chunk
            for chunk in translator.finish():
                yield chunk
        except (httpx.RequestError, LlmError) as error:
            failure = failure_of(error, secret)
        if failure is not None:
            # Raised out here, so that the error that was caught is not held on to. Its text
            # may hold the key, and so may that of its cause.
            raise LlmError(failure)

    async def models(self) -> list[str]:
        """Return the ids of the models that the service lists.

        The list is asked for with the key, as a reply is. So an answer says
        that the service is there, and that it takes the key if it is one
        that shows its list to nobody else.

        Raises:
            LlmError: The key is missing, the service could not be reached or
                refused, or what it answered is no list of models.
        """
        secret = await secret_of(self._config, self._given)
        headers = {} if secret is None else {"Authorization": f"Bearer {secret}"}
        timeout = httpx.Timeout(self._config.connect_timeout_seconds, read=_LIST_SECONDS)
        try:
            response = await self._client.get("models", headers=headers, timeout=timeout)
            if response.status_code != 200:
                raise LlmError(_refusal(response))
            return _listed(response.content)
        except (httpx.RequestError, LlmError) as error:
            failure = failure_of(error, secret)
        # Raised out here, as a failure of a reply is.
        raise LlmError(failure)


def apply(ctx: Context, config: Config) -> None:
    """Register an adapter for the service of ``config``."""
    adapter = OpenAIChat(config)
    ctx.effect(lambda: adapter.aclose)
    ctx.use(LLM).adapters.add(ctx, config.provider, adapter)
