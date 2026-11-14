"""An adapter for the Responses protocol of OpenAI, with streaming.

The newer models of OpenAI call tools only here: asked through Chat
Completions to reason and to call a tool in one reply, they refuse. This
module is a plugin as :mod:`columbina.llm.adapters.openai_chat` is: loading
it registers one adapter under the provider name of its config.

Nothing is kept at the service. Each request carries the whole conversation
and asks that the reply be not stored, so that what the service knows of a
conversation is what this request says of it.

A model that reasons gives back, with each reply, what it thought in a form
that only the service reads. That goes back to the service with the next
request, in its place before the text or the call that it led to: without
it the model would think the same again. It goes back only to the adapter
and the model that it came from, and only where something follows it in its
reply, since the service takes it in no other way. What is shown of the
reasoning is the summary that the service writes of it.

The key is looked for, sent and kept out of failures as
:mod:`columbina.llm.adapters.openai_chat` does it, see there.
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
    ToolCallBlock,
    ToolCallDelta,
    ToolMessage,
    Usage,
    UsageChunk,
    UserMessage,
)
from columbina.net import proxy_for

__all__ = ["Config", "OpenAIResponses", "apply", "build_body", "inject", "name"]

name = "llm-openai-responses"
inject = (LLM,)

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
_NO_MONEY = frozenset(
    {"insufficient_quota", "credit_balance_exhausted", "billing_hard_limit_reached"}
)
"""What the service calls it when the account cannot pay: to ask again is of no use."""
_IN_A_STREAM = {
    "server_error": "SERVER",
    "server_is_overloaded": "SERVER",
    "rate_limit_exceeded": "RATE_LIMIT",
    "context_length_exceeded": "CONTEXT_WINDOW_EXCEEDED",
}
"""The code of a failure that a stream reports, by what the service calls it."""
_EFFORT = {"low": "low", "high": "high", "max": "xhigh"}
_LIST_SECONDS = 30.0
"""How long the list of the models of the service may take to arrive."""
_NO_PICTURES = "(A picture was given back here, and this model takes no pictures.)"


class Config(BaseModel):
    """The config of one adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        base_url: The address of the service, up to but without
            ``/responses``.
        api_key_env: The environment variable that holds the API key, or
            ``None`` if no variable does.
        api_key_file: The file of keys that the user stored, as
            ``columbina setup`` writes it, or ``None`` if there is none.
        api_key_required: Whether a request is made only with a key.
        reasoning_style: How the service is told how much to reason.
            ``openai`` sends the effort that a call asks for, ``low`` or
            ``high``, and for ``max`` the most that every model that reasons
            takes. ``none`` sends nothing of it.
        summary: How long a summary of its reasoning the model is asked to
            write, which is what is shown of the reasoning. ``None`` asks for
            none: a model that does not reason refuses the question.
        connect_timeout_seconds: How long connecting may take.
        headers: Further headers for every request. Do not put a key here.
        pictures: Whether the model takes pictures. The pictures that a tool
            gave back go to the model in the result of its call. Where the
            model takes none, it is told so in their place.
    """

    provider: str
    base_url: str
    api_key_env: str | None = None
    api_key_file: Path | None = None
    api_key_required: bool = True
    reasoning_style: Literal["openai", "none"] = "openai"
    summary: Literal["auto", "concise", "detailed"] | None = "auto"
    connect_timeout_seconds: float = Field(default=10.0, gt=0)
    headers: dict[str, str] = Field(default_factory=dict[str, str])
    pictures: bool = False


class _Summary(BaseModel):
    text: str = ""


class _Part(BaseModel):
    text: str | None = None
    refusal: str | None = None


class _Item(BaseModel):
    type: str = ""
    id: str | None = None
    call_id: str | None = None
    name: str | None = None
    arguments: str | None = None
    encrypted_content: str | None = None
    summary: list[_Summary] = Field(default_factory=list[_Summary])
    content: list[_Part] | None = None


class _Cached(BaseModel):
    cached_tokens: int = 0
    cache_write_tokens: int = 0


class _Usage(BaseModel):
    input_tokens: int = 0
    output_tokens: int = 0
    input_tokens_details: _Cached | None = None


class _Error(BaseModel):
    message: str = "the service reported an error"
    code: str | int | None = None


class _Incomplete(BaseModel):
    reason: str | None = None


class _Response(BaseModel):
    usage: _Usage | None = None
    error: _Error | None = None
    incomplete_details: _Incomplete | None = None


class _Event(BaseModel):
    type: str = ""
    output_index: int = 0
    summary_index: int = 0
    delta: JsonValue = None
    item: _Item | None = None
    response: _Response | None = None
    message: str | None = None
    code: str | int | None = None


class _ErrorBody(BaseModel):
    error: _Error


class _Listed(BaseModel):
    id: str


class _Listing(BaseModel):
    data: list[_Listed]


def _joined(content: tuple[TextBlock, ...]) -> str:
    return "\n\n".join(block.text for block in content)


def _thought(block: ReasoningBlock, config: Config, model: str) -> dict[str, JsonValue] | None:
    """Return what the service gave of its reasoning, to go back to it, or ``None``.

    It goes back to the adapter and to the model that it came from, and only
    if the service gave the form of it that it reads again.
    """
    state = block.replay_state
    if state is None or state.get("adapter") != config.provider or state.get("model") != model:
        return None
    sealed, of = state.get("encrypted_content"), state.get("id")
    if not isinstance(sealed, str) or not sealed or not isinstance(of, str):
        return None
    said = state.get("summary")
    summary: list[JsonValue] = [
        {"type": "summary_text", "text": text}
        for text in (said if isinstance(said, list) else [])
        if isinstance(text, str)
    ]
    return {"type": "reasoning", "id": of, "summary": summary, "encrypted_content": sealed}


def _wire_assistant(
    message: AssistantMessage, config: Config, model: str
) -> list[dict[str, JsonValue]]:
    """Return a reply as the items that the service gave it in, in their order."""
    items: list[dict[str, JsonValue]] = []
    for block in message.content:
        if isinstance(block, ReasoningBlock):
            thought = _thought(block, config, model)
            if thought is not None:
                items.append(thought)
        elif isinstance(block, ToolCallBlock):
            items.append(
                {
                    "type": "function_call",
                    "call_id": block.call_id,
                    "name": block.name,
                    "arguments": block.arguments or "{}",
                }
            )
        elif block.text:
            items.append({"role": "assistant", "content": block.text})
    # The service takes its reasoning back only before what it led to. A reply that was cut
    # short in its reasoning has nothing after it.
    while items and items[-1].get("type") == "reasoning":
        items.pop()
    return items


def _wire_result(message: ToolMessage, config: Config) -> dict[str, JsonValue]:
    """Return the result of a tool call in the shape of the protocol, with its pictures."""
    text = _joined(message.content)
    output: JsonValue = text
    if message.pictures and config.pictures:
        shown: list[JsonValue] = [{"type": "input_text", "text": text}] if text else []
        shown.extend(
            {
                "type": "input_image",
                "image_url": f"data:{picture.media_type};base64,{picture.data}",
            }
            for picture in message.pictures
        )
        output = shown
    elif message.pictures:
        output = f"{text}\n{_NO_PICTURES}"
    return {"type": "function_call_output", "call_id": message.call_id, "output": output}


def _wired(messages: tuple[Message, ...], config: Config, model: str) -> list[JsonValue]:
    """Return the conversation as the items of the protocol."""
    items: list[JsonValue] = []
    for message in messages:
        if isinstance(message, SystemMessage):
            items.append({"role": "system", "content": _joined(message.content)})
        elif isinstance(message, UserMessage):
            items.append({"role": "user", "content": _joined(message.content)})
        elif isinstance(message, ToolMessage):
            items.append(_wire_result(message, config))
        else:
            items.extend(_wire_assistant(message, config, model))
    return items


def build_body(request: LlmRequest, config: Config) -> dict[str, JsonValue]:
    """Return the JSON body of the request to the service.

    A setting that the call config leaves open is not sent, so that the
    default of the service applies. The ``options`` of the call config are put
    in last and as they are, but for the effort that the call asks for, which
    is written over what they say of it.
    """
    call = request.config
    body: dict[str, JsonValue] = {
        "model": call.model,
        "input": _wired(request.messages, config, call.model),
        "stream": True,
        # The conversation is kept here and sent whole. What the model thought comes back
        # sealed, to be sent again, only where it is asked for by name.
        "store": False,
        "include": ["reasoning.encrypted_content"],
    }
    if request.tools:
        body["tools"] = [
            {
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                # The schema of a tool is sent as it is. Left open, the service would first
                # try to hold the model to a stricter reading of it.
                "strict": False,
            }
            for tool in request.tools
        ]
    if call.max_output_tokens is not None:
        body["max_output_tokens"] = call.max_output_tokens
    if call.temperature is not None:
        body["temperature"] = call.temperature
    if call.top_p is not None:
        body["top_p"] = call.top_p
    if config.summary is not None:
        body["reasoning"] = {"summary": config.summary}
    body.update(call.options)
    if config.reasoning_style == "openai" and call.reasoning in _EFFORT:
        said = body.get("reasoning")
        reasoning: dict[str, JsonValue] = dict(said) if isinstance(said, dict) else {}
        reasoning["effort"] = _EFFORT[call.reasoning]
        body["reasoning"] = reasoning
    return body


@dataclass(slots=True)
class _Open:
    """A block of the reply that has begun.

    Attributes:
        index: The index of the block.
        kind: What it is a block of.
        arrived: Whether anything of it has arrived in pieces.
        part: The part of a summary that the last piece of reasoning was of.
    """

    index: int
    kind: BlockKind
    arrived: bool = False
    part: int = 0


class _Translator:
    """Turns the events of one streamed reply into chunks."""

    __slots__ = ("_adapter", "_calls", "_ended", "_model", "_next", "_open")

    def __init__(self, adapter: str, model: str) -> None:
        self._adapter = adapter
        self._model = model
        self._next = 0
        self._open: dict[int, _Open] = {}
        self._calls = False
        self._ended = False

    @property
    def ended(self) -> bool:
        """Whether the service said that the reply is over."""
        return self._ended

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
        kind = event.type
        if kind == "error":
            raise LlmError(
                _failed(_Error(message=event.message or _Error().message, code=event.code))
            )
        if kind == "response.output_item.added" and event.item is not None:
            yield from self._begin(event.output_index, event.item)
        elif kind in ("response.reasoning_summary_text.delta", "response.reasoning_text.delta"):
            yield from self._thinks(event)
        elif kind in ("response.output_text.delta", "response.refusal.delta"):
            if isinstance(event.delta, str) and event.delta:
                yield from self._says(event.output_index, event.delta)
        elif kind == "response.function_call_arguments.delta":
            block = self._open.get(event.output_index)
            if block is not None and isinstance(event.delta, str) and event.delta:
                block.arrived = True
                yield ToolCallDelta(index=block.index, arguments=event.delta)
        elif kind == "response.output_item.done" and event.item is not None:
            yield from self._end(event.output_index, event.item)
        elif kind in ("response.completed", "response.incomplete", "response.failed"):
            yield from self._over(kind, event.response or _Response())

    def _begin(self, at: int, item: _Item) -> Iterator[Chunk]:
        """Begin the block of a call or of reasoning. That of a text begins with its text."""
        if item.type == "function_call" and item.name:
            yield from self._call(at, item)
        elif item.type == "reasoning":
            yield from self._block(at, "reasoning")

    def _block(self, at: int, kind: Literal["reasoning", "text"]) -> Iterator[Chunk]:
        block = _Open(self._next, kind)
        self._next += 1
        self._open[at] = block
        # Reasoning that is cut short goes back to nobody: what the service reads again of it
        # comes at its end.
        yield BlockStart(index=block.index, kind=kind)

    def _call(self, at: int, item: _Item) -> Iterator[Chunk]:
        block = _Open(self._next, "tool-call")
        self._next += 1
        self._open[at] = block
        self._calls = True
        yield BlockStart(
            index=block.index,
            kind="tool-call",
            call_id=item.call_id or f"call_{uuid.uuid4().hex[:12]}",
            name=item.name,
        )

    def _thinks(self, event: _Event) -> Iterator[Chunk]:
        if not isinstance(event.delta, str) or not event.delta:
            return
        block = self._open.get(event.output_index)
        if block is None:
            yield from self._block(event.output_index, "reasoning")
            block = self._open[event.output_index]
        piece = event.delta
        if block.arrived and event.summary_index != block.part:
            # A summary comes in parts, each a paragraph of its own.
            piece = f"\n\n{piece}"
        block.arrived, block.part = True, event.summary_index
        yield ReasoningDelta(index=block.index, text=piece)

    def _says(self, at: int, piece: str) -> Iterator[Chunk]:
        block = self._open.get(at)
        if block is None:
            yield from self._block(at, "text")
            block = self._open[at]
        block.arrived = True
        yield TextDelta(index=block.index, text=piece)

    def _end(self, at: int, item: _Item) -> Iterator[Chunk]:
        """End the block of an item, with what of it did not arrive in pieces."""
        block = self._open.get(at)
        if item.type == "function_call":
            if block is None:
                if not item.name:
                    raise LlmError(
                        LlmFailure(
                            message="a tool call ended without a name", code="MALFORMED_RESPONSE"
                        )
                    )
                yield from self._call(at, item)
                block = self._open[at]
            if not block.arrived and item.arguments:
                yield ToolCallDelta(index=block.index, arguments=item.arguments)
            del self._open[at]
            yield BlockEnd(index=block.index)
        elif item.type == "reasoning":
            if block is None:
                yield from self._block(at, "reasoning")
                block = self._open[at]
            parts = [part.text for part in item.summary if part.text]
            if not block.arrived and parts:
                yield ReasoningDelta(index=block.index, text="\n\n".join(parts))
            del self._open[at]
            state: dict[str, JsonValue] = {
                "adapter": self._adapter,
                "model": self._model,
                "id": item.id,
                "encrypted_content": item.encrypted_content,
                "summary": [*parts],
            }
            yield BlockEnd(index=block.index, replay_state=state)
        elif item.type == "message":
            whole = "".join(part.text or part.refusal or "" for part in item.content or ())
            if block is None and whole:
                yield from self._says(at, whole)
                block = self._open[at]
            if block is not None:
                del self._open[at]
                yield BlockEnd(index=block.index)

    def _over(self, kind: str, response: _Response) -> Iterator[Chunk]:
        """End the reply as the service said that it ended."""
        if kind == "response.failed":
            raise LlmError(_failed(response.error or _Error()))
        self._ended = True
        # What of a text or of the reasoning arrived is kept. A call that was cut short is
        # not: half of its arguments are none.
        for at, block in sorted(self._open.items()):
            if block.kind != "tool-call":
                del self._open[at]
                yield BlockEnd(index=block.index)
        if response.usage is not None:
            yield UsageChunk(usage=_usage(response.usage))
        cut = response.incomplete_details.reason if response.incomplete_details else None
        if kind == "response.incomplete" and cut == "content_filter":
            failure = LlmFailure(message="the provider withheld the reply", code="CONTENT_FILTER")
            yield Finish(reason="error", failure=failure)
        elif kind == "response.incomplete" and cut == "max_output_tokens":
            yield Finish(reason="max-tokens")
        else:
            yield Finish(reason="tool-calls" if self._calls else "stop")


def _failed(error: _Error) -> LlmFailure:
    """Return the failure that a stream reported."""
    code = "QUOTA" if error.code in _NO_MONEY else _IN_A_STREAM.get(str(error.code))
    return LlmFailure(message=error.message, code=code or "INVALID_REQUEST")


def _usage(raw: _Usage) -> Usage:
    details = raw.input_tokens_details or _Cached()
    read, written = details.cached_tokens, details.cache_write_tokens
    return Usage(
        input_tokens=max(raw.input_tokens - read - written, 0),
        output_tokens=raw.output_tokens,
        cache_read_tokens=read,
        cache_write_tokens=written,
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
        if error.code in _NO_MONEY:
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
    """Return the ids of the models in a list that the service sent.

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


class OpenAIResponses:
    """Talks to one service that speaks the Responses protocol.

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
    def reasoning_style(self) -> Literal["openai", "none"]:
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
        translator = _Translator(self._config.provider, request.config.model)
        failure: LlmFailure | None = None
        try:
            async with self._client.stream(
                "POST", "responses", json=body, headers=headers
            ) as response:
                if response.status_code != 200:
                    await response.aread()
                    raise LlmError(_refusal(response))
                async for data in events(response.aiter_lines()):
                    # The service says in an event that a reply is over. What follows that,
                    # if anything does, is not of the reply.
                    if translator.ended or data == "[DONE]":
                        break
                    for chunk in translator.feed(data):
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
        that the service is there and that it takes the key.

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
    adapter = OpenAIResponses(config)
    ctx.effect(lambda: adapter.aclose)
    ctx.use(LLM).adapters.add(ctx, config.provider, adapter)
