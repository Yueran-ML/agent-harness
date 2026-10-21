"""An adapter for Claude: the Messages API of Anthropic, streamed.

This module is a plugin: loading it registers one adapter under the provider
name of its config. It speaks the protocol of Anthropic itself, and not the
layer of Anthropic that answers as OpenAI does, for what that layer leaves
out: the cache of the prompt, the thinking of the model, and the list of the
models.

The key is looked for, and kept out of every failure, as for every adapter
that asks a service over HTTP, see :mod:`columbina.llm.adapters.common`. It is
sent in the header ``x-api-key``, with the version of the API that the
requests are written for.

**Thinking.** The current models think by themselves, and say what they
thought only where they are asked to, as a summary. The adapter asks for the
summary. A block of thinking comes with a signature, and it is given back as
it came, with its signature, in the requests that follow: within a turn with
tools the service requires that. The reasoning of another adapter is not
sent, since the service could not read it.

**The cache.** Every request asks the service to cache the conversation up to
its last block, which it does once the prompt is long enough. A request that
begins as the one before it did reads that beginning from the cache, at a
fraction of the price. A change of how hard the model thinks starts the cache
over.

**How hard the model thinks.** The ``reasoning`` of a call is sent as the
effort of the request. Nothing is sent without one, and the default of the
model holds. Not every model lets its thinking be turned off, so ``off`` asks
for the least effort.

**A conversation of another service.** A conversation may begin with another
service and go on with Claude. What the service refuses is put in a shape
that it takes: the ids of calls hold letters, digits and ``_`` only, each id
is sent once, and blank text is left out. Where no tools are on offer, as in
a request for a summary, calls and their results are told of in text. So is
a result whose call is not in the reply before it, and a conversation that
begins with a reply of the model begins with a note of the user.

A ``temperature`` or ``top_p`` of a call is not sent: the current models take
only their defaults. The ``options`` of a call are put into the request last,
and an option that is ``None`` takes out what the adapter would send under its
name, such as ``thinking`` for a model that does not think by itself.
"""

import json
import os
import re
from collections import deque
from collections.abc import AsyncIterator, Iterator
from pathlib import Path
from typing import Any, Literal, cast

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
)
from columbina.net import proxy_for

__all__ = ["AnthropicMessages", "Config", "apply", "build_body", "inject", "name"]

name = "llm-anthropic-messages"
inject = (LLM,)

_FINISH_REASONS: dict[str, Literal["stop", "tool-calls", "max-tokens"]] = {
    "end_turn": "stop",
    "stop_sequence": "stop",
    "pause_turn": "stop",
    "tool_use": "tool-calls",
    "max_tokens": "max-tokens",
}
_STATUS_CODES = {
    400: "INVALID_REQUEST",
    401: "AUTH",
    402: "QUOTA",
    403: "AUTH",
    404: "INVALID_REQUEST",
    408: "TIMEOUT",
    413: "INVALID_REQUEST",
    422: "INVALID_REQUEST",
    429: "RATE_LIMIT",
}
_ERROR_TYPES = {
    "authentication_error": "AUTH",
    "permission_error": "AUTH",
    "billing_error": "QUOTA",
    "rate_limit_error": "RATE_LIMIT",
    "overloaded_error": "SERVER",
    "api_error": "SERVER",
    "timeout_error": "TIMEOUT",
    "invalid_request_error": "INVALID_REQUEST",
    "not_found_error": "INVALID_REQUEST",
    "request_too_large": "INVALID_REQUEST",
}
"""The kinds of error that Anthropic names, as the codes that the program goes by. An error
that comes in the middle of a stream has its kind and no status."""

_EFFORT = {"off": "low", "low": "low", "high": "high", "max": "max"}
_LIST_SECONDS = 30.0
"""How long the list of the models of a service may take to arrive."""
_LIST_PAGES = 10
"""How many pages of models are asked for at most."""
_TOO_LONG = "prompt is too long"
"""What Anthropic says of a conversation that does not fit into the window of the model."""
_NO_MONEY = "credit balance is too low"
"""What Anthropic says of an account that cannot pay for a request."""
_NOT_IN_AN_ID = re.compile(r"[^A-Za-z0-9_]")
"""What the id of a call may not hold, for Anthropic."""
_LEFT_OUT = "(The beginning of the conversation is left out here.)"
"""What a conversation that begins with a reply of the model begins with."""


class Config(BaseModel):
    """The config of one adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        base_url: The address of the service, up to but without ``/v1``.
        api_key_env: The environment variable that holds the API key, or
            ``None`` if no variable does.
        api_key_file: The file of keys that the user stored, as
            ``columbina setup`` writes it, or ``None`` if there is none.
        api_key_required: Whether a request is made only with a key.
        version: The version of the API that the requests are written for.
        max_output_tokens: The limit on the length of a reply where a call
            names none. The thinking of the model counts toward it.
        thinking_display: Whether the service is asked for a summary of what
            the model thought, or for its signature alone.
        cache: How long the service is asked to keep the beginning of a
            conversation: five minutes, an hour, or not at all.
        connect_timeout_seconds: How long connecting may take. Silence after
            that is the business of the model service, not of the adapter.
        headers: Further headers for every request. Do not put a key here.
    """

    provider: str
    base_url: str = "https://api.anthropic.com"
    api_key_env: str | None = None
    api_key_file: Path | None = None
    api_key_required: bool = True
    version: str = "2023-06-01"
    max_output_tokens: int = Field(default=32_000, gt=0)
    thinking_display: Literal["summarized", "omitted"] = "summarized"
    cache: Literal["5m", "1h", "off"] = "5m"
    connect_timeout_seconds: float = Field(default=10.0, gt=0)
    headers: dict[str, str] = Field(default_factory=dict[str, str])


def _joined(content: tuple[TextBlock, ...]) -> str:
    return "\n\n".join(block.text for block in content)


def _no_constant(name: str) -> object:
    """Refuse ``NaN`` and ``Infinity``, which are no JSON and which the request cannot hold."""
    raise ValueError(name)


def _input(arguments: str) -> dict[str, JsonValue]:
    """Return the arguments of a tool call as the object that the service takes.

    Arguments that are no object of JSON, which the model may have written,
    are sent as an empty object: the call was answered as one that failed.
    """
    try:
        given: object = json.loads(arguments, parse_constant=_no_constant)
    except ValueError:
        return {}
    return cast("dict[str, JsonValue]", given) if isinstance(given, dict) else {}


def _text(text: str) -> dict[str, JsonValue] | None:
    """Return a block of text, or ``None`` for blank text, which the service refuses."""
    return {"type": "text", "text": text} if text.strip() else None


class _Turns:
    """The messages of a request, put together message by message.

    The instructions are not among them: they go into ``system``. The service
    takes turns of the two sides one after the other, so what one side says in
    a row is one message. The results of tools come first in a message of the
    user: the service refuses them after other text. How a conversation of
    another service is put in a shape that the service takes is said in the
    description of this module.
    """

    __slots__ = ("_adapter", "_calls", "_sent", "_tools", "_turns")

    def __init__(self, adapter: str, *, tools: bool) -> None:
        self._adapter = adapter
        self._tools = tools
        self._turns: list[dict[str, Any]] = []
        self._sent: set[str] = set()
        # The ids that the calls of the last reply went under, by the ids that they have.
        self._calls: dict[str, deque[str]] = {}

    def add(self, message: Message) -> None:
        """Take in the next message of the conversation."""
        if isinstance(message, SystemMessage):
            return
        if isinstance(message, AssistantMessage):
            if not self._turns or self._turns[-1]["role"] != "assistant":
                # The calls of the reply before are answered by now.
                self._calls = {}
            self._put("assistant", self._reply(message))
        elif isinstance(message, ToolMessage):
            self._put("user", [self._result(message)])
        else:
            block = _text(_joined(message.content))
            self._put("user", [] if block is None else [block])

    def turns(self) -> list[JsonValue]:
        """Return the messages in the shape of the protocol."""
        turns = self._turns
        for turn in turns:
            if turn["role"] == "user":
                turn["content"].sort(key=_after_the_results)
        if turns and turns[0]["role"] == "assistant":
            turns.insert(0, {"role": "user", "content": [{"type": "text", "text": _LEFT_OUT}]})
        return cast("list[JsonValue]", turns)

    def _put(self, role: str, blocks: list[JsonValue]) -> None:
        if not blocks:
            return
        if self._turns and self._turns[-1]["role"] == role:
            self._turns[-1]["content"].extend(blocks)
        else:
            self._turns.append({"role": role, "content": blocks})

    def _reply(self, message: AssistantMessage) -> list[JsonValue]:
        """Return the blocks of a reply of the model, in their order.

        A block of thinking goes back with its signature, and only to the
        adapter that marked it. One whose signature never arrived, in a reply
        that was cut off, is left out: the service would refuse it.
        """
        blocks: list[JsonValue] = []
        for block in message.content:
            if isinstance(block, ReasoningBlock):
                state = block.replay_state
                if state is None or state.get("adapter") != self._adapter:
                    continue
                redacted, signature = state.get("redacted"), state.get("signature")
                if isinstance(redacted, str):
                    blocks.append({"type": "redacted_thinking", "data": redacted})
                elif isinstance(signature, str) and signature:
                    blocks.append(
                        {"type": "thinking", "thinking": block.text, "signature": signature}
                    )
            elif isinstance(block, TextBlock):
                said = _text(block.text)
                if said is not None:
                    blocks.append(said)
            elif self._tools:
                blocks.append(
                    {
                        "type": "tool_use",
                        "id": self._sent_as(block.call_id),
                        "name": block.name,
                        "input": _input(block.arguments),
                    }
                )
            else:
                blocks.append(
                    {"type": "text", "text": f"[A call of {block.name}: {block.arguments}]"}
                )
        return blocks

    def _sent_as(self, call_id: str) -> str:
        """Return the id that a call goes under: one that the service takes, and has not had."""
        base = _NOT_IN_AN_ID.sub("_", call_id)
        sent, number = base, 1
        while sent in self._sent:
            number += 1
            sent = f"{base}_{number}"
        self._sent.add(sent)
        self._calls.setdefault(call_id, deque()).append(sent)
        return sent

    def _result(self, message: ToolMessage) -> JsonValue:
        """Return the result of a call: under the id of its call, or as text where it has none."""
        text = _joined(message.content)
        waiting = self._calls.get(message.call_id)
        if not waiting:
            head = (
                "[The result of a call, which failed:]"
                if message.is_error
                else "[The result of a call:]"
            )
            return {"type": "text", "text": f"{head}\n{text}"}
        result: dict[str, JsonValue] = {"type": "tool_result", "tool_use_id": waiting.popleft()}
        if text.strip():
            result["content"] = text
        if message.is_error:
            result["is_error"] = True
        return result


def _conversation(request: LlmRequest, adapter: str) -> list[JsonValue]:
    """Return the messages of a request in the shape of the protocol."""
    turns = _Turns(adapter, tools=bool(request.tools))
    for message in request.messages:
        turns.add(message)
    return turns.turns()


def _after_the_results(block: dict[str, Any]) -> bool:
    """Whether a block of a message of the user goes after the results of tools."""
    return block["type"] != "tool_result"


def build_body(request: LlmRequest, config: Config) -> dict[str, JsonValue]:
    """Return the body of a request to the Messages API.

    A field of the call that is ``None`` is not sent, and the default of the
    service applies; the limit on the length of the reply is the adapter's then,
    since the service needs one. The ``options`` of the call are put in last,
    and an option that is ``None`` takes out what stands under its name.
    """
    call = request.config
    body: dict[str, JsonValue] = {
        "model": call.model,
        "max_tokens": call.max_output_tokens or config.max_output_tokens,
        "messages": _conversation(request, config.provider),
        "stream": True,
        "thinking": {"type": "adaptive", "display": config.thinking_display},
    }
    system = "\n\n".join(
        _joined(message.content)
        for message in request.messages
        if isinstance(message, SystemMessage)
    )
    if system:
        body["system"] = system
    if request.tools:
        body["tools"] = [
            {"name": tool.name, "description": tool.description, "input_schema": tool.parameters}
            for tool in request.tools
        ]
    if config.cache != "off":
        control: dict[str, JsonValue] = {"type": "ephemeral"}
        if config.cache == "1h":
            control["ttl"] = "1h"
        body["cache_control"] = control
    if call.reasoning is not None:
        body["output_config"] = {"effort": _EFFORT[call.reasoning]}
    body.update(call.options)
    return {field: value for field, value in body.items() if value is not None}


class _Usage(BaseModel):
    input_tokens: int | None = None
    output_tokens: int | None = None
    cache_read_input_tokens: int | None = None
    cache_creation_input_tokens: int | None = None


class _Error(BaseModel):
    type: str = "error"
    message: str = "the service reported an error"


class _ErrorBody(BaseModel):
    error: _Error


class _Listed(BaseModel):
    id: str


class _Listing(BaseModel):
    data: list[_Listed]
    has_more: bool = False
    last_id: str | None = None


def _malformed(problem: str) -> LlmError:
    return LlmError(
        LlmFailure(
            message=f"the service sent an event that cannot be read: {problem}",
            code="MALFORMED_RESPONSE",
        )
    )


class _Translator:
    """Turns the events of one streamed reply into chunks.

    The blocks keep the places that the service gives them. A block of a
    kind that the adapter does not know, such as one of a tool of the
    service, is passed over with everything that belongs to it.
    """

    __slots__ = ("_adapter", "_kinds", "_reason", "_redacted", "_signatures", "_usage")

    def __init__(self, adapter: str) -> None:
        self._adapter = adapter
        self._kinds: dict[int, BlockKind] = {}
        self._signatures: dict[int, str] = {}
        self._redacted: dict[int, str] = {}
        self._reason: str | None = None
        self._usage = Usage()

    def feed(self, data: str) -> Iterator[Chunk]:
        """Take in the data of one event."""
        try:
            event: object = json.loads(data)
        except ValueError as error:
            raise _malformed(str(error)) from None
        if not isinstance(event, dict):
            raise _malformed("it is no object")
        given = cast("dict[str, Any]", event)
        kind = given.get("type")
        if kind == "message_start":
            message = given.get("message")
            self._count(
                cast("dict[str, Any]", message).get("usage") if isinstance(message, dict) else None
            )
        elif kind == "content_block_start":
            yield from self._start(self._index(given), given.get("content_block"))
        elif kind == "content_block_delta":
            yield from self._delta(self._index(given), given.get("delta"))
        elif kind == "content_block_stop":
            yield from self._stop(self._index(given))
        elif kind == "message_delta":
            delta = given.get("delta")
            if isinstance(delta, dict):
                reason = cast("dict[str, Any]", delta).get("stop_reason")
                if isinstance(reason, str):
                    self._reason = reason
            self._count(given.get("usage"))
        elif kind == "error":
            raise LlmError(_stream_failure(given.get("error")))

    def finish(self) -> Iterator[Chunk]:
        """End the reply. Without a reason to stop nothing is said: the stream just stopped."""
        reason = self._reason
        if reason is None:
            return
        yield UsageChunk(usage=self._usage)
        if reason == "refusal":
            failure = LlmFailure(message="the model declined to answer", code="CONTENT_FILTER")
            yield Finish(reason="error", failure=failure)
        elif reason == "model_context_window_exceeded":
            failure = LlmFailure(
                message="the reply did not fit into the window of the model",
                code="CONTEXT_WINDOW_EXCEEDED",
            )
            yield Finish(reason="error", failure=failure)
        else:
            yield Finish(reason=_FINISH_REASONS.get(reason, "stop"))

    @staticmethod
    def _index(given: dict[str, Any]) -> int:
        index = given.get("index")
        if isinstance(index, bool) or not isinstance(index, int) or index < 0:
            raise _malformed("a block has no place")
        return index

    def _count(self, raw: object) -> None:
        """Take in the token counts of an event. A count that it does not give stays as it was."""
        if not isinstance(raw, dict):
            return
        try:
            told = _Usage.model_validate(raw)
        except ValidationError:
            raise _malformed("its token counts are no numbers") from None
        usage = self._usage
        self._usage = Usage(
            input_tokens=usage.input_tokens if told.input_tokens is None else told.input_tokens,
            output_tokens=usage.output_tokens if told.output_tokens is None else told.output_tokens,
            cache_read_tokens=usage.cache_read_tokens
            if told.cache_read_input_tokens is None
            else told.cache_read_input_tokens,
            cache_write_tokens=usage.cache_write_tokens
            if told.cache_creation_input_tokens is None
            else told.cache_creation_input_tokens,
        )

    def _start(self, index: int, raw: object) -> Iterator[Chunk]:
        if not isinstance(raw, dict):
            raise _malformed(f"block {index} begins without its content")
        block = cast("dict[str, Any]", raw)
        kind = block.get("type")
        if kind == "text":
            self._kinds[index] = "text"
            yield BlockStart(index=index, kind="text")
            if block.get("text"):
                yield TextDelta(index=index, text=str(block["text"]))
        elif kind in ("thinking", "redacted_thinking"):
            self._kinds[index] = "reasoning"
            yield BlockStart(index=index, kind="reasoning")
            if kind == "redacted_thinking":
                self._redacted[index] = str(block.get("data") or "")
            elif block.get("thinking"):
                yield ReasoningDelta(index=index, text=str(block["thinking"]))
            if block.get("signature"):
                self._signatures[index] = str(block["signature"])
        elif kind == "tool_use":
            call_id, called = block.get("id"), block.get("name")
            if (
                not isinstance(call_id, str)
                or not call_id
                or not isinstance(called, str)
                or not called
            ):
                raise _malformed(f"the tool call of block {index} has no id or no name")
            self._kinds[index] = "tool-call"
            yield BlockStart(index=index, kind="tool-call", call_id=call_id, name=called)
            # The service begins with an empty object and sends the arguments in pieces. A
            # service in between may send them whole here.
            given = block.get("input")
            if isinstance(given, dict) and given:
                yield ToolCallDelta(index=index, arguments=json.dumps(given))

    def _delta(self, index: int, raw: object) -> Iterator[Chunk]:
        if not isinstance(raw, dict):
            return
        kind = self._kinds.get(index)
        delta = cast("dict[str, Any]", raw)
        what = delta.get("type")
        if what == "text_delta" and kind == "text" and delta.get("text"):
            yield TextDelta(index=index, text=str(delta["text"]))
        elif what == "thinking_delta" and kind == "reasoning" and delta.get("thinking"):
            yield ReasoningDelta(index=index, text=str(delta["thinking"]))
        elif what == "signature_delta" and kind == "reasoning":
            # The service sends the signature whole, in one delta, which goes over one that the
            # block began with.
            signature = delta.get("signature")
            if isinstance(signature, str) and signature:
                self._signatures[index] = signature
        elif what == "input_json_delta" and kind == "tool-call" and delta.get("partial_json"):
            yield ToolCallDelta(index=index, arguments=str(delta["partial_json"]))

    def _stop(self, index: int) -> Iterator[Chunk]:
        kind = self._kinds.get(index)
        if kind is None:
            return
        state: dict[str, JsonValue] | None = None
        if kind == "reasoning":
            state = {"adapter": self._adapter}
            if index in self._redacted:
                state["redacted"] = self._redacted[index]
            else:
                state["signature"] = self._signatures.get(index, "")
        yield BlockEnd(index=index, replay_state=state)


def _code_of(kind: str, message: str, status: int | None) -> str:
    """Return the code of a failure from the kind of error, its words and its status."""
    lowered = message.lower()
    if _TOO_LONG in lowered:
        return "CONTEXT_WINDOW_EXCEEDED"
    if _NO_MONEY in lowered:
        return "QUOTA"
    if kind in _ERROR_TYPES:
        return _ERROR_TYPES[kind]
    if status is None:
        return "SERVER"
    return _STATUS_CODES.get(status) or ("SERVER" if status >= 500 else f"HTTP_{status}")


def _stream_failure(raw: object) -> LlmFailure:
    """Turn an error that came in the middle of a stream into a failure."""
    try:
        error = _Error.model_validate(raw)
    except ValidationError:
        error = _Error()
    return LlmFailure(message=error.message, code=_code_of(error.type, error.message, None))


def _refusal(response: httpx.Response) -> LlmFailure:
    """Turn a response that is not a stream into a failure."""
    status = response.status_code
    text = response.text
    message = text or f"HTTP {status} {response.reason_phrase}"
    kind = ""
    try:
        error = _ErrorBody.model_validate_json(text).error
    except ValidationError:
        pass
    else:
        message, kind = error.message, error.type
    return LlmFailure(
        message=message,
        code=_code_of(kind, message, status),
        status=status,
        retry_after_ms=retry_after_ms(response),
        request_id=response.headers.get("request-id"),
    )


def _listed(content: bytes) -> _Listing:
    """Return a page of the list of models that the service sent.

    Raises:
        LlmError: What it sent is no list of models.
    """
    try:
        return _Listing.model_validate_json(content)
    except ValidationError as error:
        problem = error.errors()[0]["msg"]
        raise LlmError(
            LlmFailure(
                message=f"the service sent a list of models that cannot be read: {problem}",
                code="MALFORMED_RESPONSE",
            )
        ) from None


class AnthropicMessages:
    """Talks to Claude through the Messages API of Anthropic.

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
    def reasoning_style(self) -> Literal["anthropic"]:
        """How the service is told how much to reason."""
        return "anthropic"

    async def aclose(self) -> None:
        """Close the connections to the service."""
        await self._client.aclose()

    def _headers(self, secret: str | None) -> dict[str, str]:
        headers = {"anthropic-version": self._config.version}
        if secret is not None:
            headers["x-api-key"] = secret
        return headers

    async def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Yield the chunks of the reply to ``request``.

        Raises:
            LlmError: The key is missing, the service could not be reached or
                refused, or its reply cannot be read.
        """
        secret = await secret_of(self._config, self._given)
        body = build_body(request, self._config)
        translator = _Translator(self._config.provider)
        failure: LlmFailure | None = None
        try:
            async with self._client.stream(
                "POST", "v1/messages", json=body, headers=self._headers(secret)
            ) as response:
                if response.status_code != 200:
                    await response.aread()
                    raise LlmError(_refusal(response))
                async for data in events(response.aiter_lines()):
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
        """Return the ids of the models that the service lists, page by page.

        Raises:
            LlmError: The key is missing, the service could not be reached or
                refused, or what it answered is no list of models.
        """
        secret = await secret_of(self._config, self._given)
        timeout = httpx.Timeout(self._config.connect_timeout_seconds, read=_LIST_SECONDS)
        found: list[str] = []
        params: dict[str, str | int] = {"limit": 1000}
        try:
            for _ in range(_LIST_PAGES):
                response = await self._client.get(
                    "v1/models", params=params, headers=self._headers(secret), timeout=timeout
                )
                if response.status_code != 200:
                    raise LlmError(_refusal(response))
                page = _listed(response.content)
                found.extend(model.id for model in page.data)
                if not page.has_more or page.last_id is None:
                    break
                params = {"limit": 1000, "after_id": page.last_id}
            return found
        except (httpx.RequestError, LlmError) as error:
            failure = failure_of(error, secret)
        # Raised out here, as a failure of a reply is.
        raise LlmError(failure)


def apply(ctx: Context, config: Config) -> None:
    """Register an adapter for the service of ``config``."""
    adapter = AnthropicMessages(config)
    ctx.effect(lambda: adapter.aclose)
    ctx.use(LLM).adapters.add(ctx, config.provider, adapter)
