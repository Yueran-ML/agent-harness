"""The adapter for services that speak the OpenAI Chat Completions protocol."""

import asyncio
import json
import traceback
from collections.abc import AsyncIterator, Iterable
from datetime import UTC, datetime, timedelta
from email.utils import format_datetime
from pathlib import Path
from typing import Any

import httpx
import pytest
from pydantic import ValidationError

import columbina.llm
from columbina.kernel import Context
from columbina.llm import (
    LLM,
    AssistantMessage,
    BlockEnd,
    BlockStart,
    CallConfig,
    Finish,
    Llm,
    LlmCall,
    LlmError,
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
    ToolSchema,
    Usage,
    UsageChunk,
    UserMessage,
)
from columbina.llm.adapters import openai_chat
from columbina.llm.adapters.openai_chat import Config, OpenAIChat
from columbina.llm.credentials import keep
from support.vault import Locked, Memory

KEY = "sk-test-0123456789"
GIVEN = "sk-given-0123456789"
STORED = "sk-stored-9876543210"
CONFIG = Config(provider="deepseek", base_url="https://api.example.test", api_key_env="TEST_KEY")
MODEL = CallConfig(provider="deepseek", model="deepseek-test")
HELLO: tuple[Message, ...] = (UserMessage(content=(TextBlock(text="hello"),)),)
READ = ToolSchema(
    name="read",
    description="Read a file.",
    parameters={"type": "object", "properties": {"path": {"type": "string"}}},
)


def delta(**fields: Any) -> dict[str, Any]:
    """One streamed event that carries a delta."""
    return {"choices": [{"index": 0, "delta": fields, "finish_reason": None}]}


def finish(reason: str) -> dict[str, Any]:
    return {"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]}


def sse(*events: dict[str, Any] | str) -> bytes:
    """The body of a streamed reply: one ``data:`` line for each event."""
    lines = [event if isinstance(event, str) else json.dumps(event) for event in events]
    return "".join(f"data: {line}\n\n" for line in lines).encode()


TEXT_REPLY = sse(
    delta(role="assistant", content="Hel"), delta(content="lo"), finish("stop"), "[DONE]"
)


class Body(httpx.AsyncByteStream):
    """A response body in pieces. It can break off or hang, and it knows whether it was closed."""

    def __init__(
        self, pieces: Iterable[bytes], *, then: Exception | None = None, hang: bool = False
    ) -> None:
        self.pieces = list(pieces)
        self.then = then
        self.hang = hang
        self.closed = False

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for piece in self.pieces:
            yield piece
        if self.then is not None:
            raise self.then
        if self.hang:
            await asyncio.Event().wait()

    async def aclose(self) -> None:
        self.closed = True


class Service:
    """Stands in for the provider: answers each request in turn, and keeps the requests."""

    def __init__(self, *responses: httpx.Response | Exception) -> None:
        self.responses = list(responses)
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        response = self.responses.pop(0)
        if isinstance(response, Exception):
            raise response
        return response

    @property
    def body(self) -> dict[str, Any]:
        """The JSON body of the last request."""
        return json.loads(self.requests[-1].content)


def streamed(body: bytes | Body, **headers: str) -> httpx.Response:
    stream = body if isinstance(body, Body) else Body([body])
    return httpx.Response(
        200, headers={"content-type": "text/event-stream", **headers}, stream=stream
    )


def refused(status: int, error: dict[str, Any] | str, **headers: str) -> httpx.Response:
    if isinstance(error, str):
        return httpx.Response(status, text=error, headers=headers)
    return httpx.Response(status, json={"error": error}, headers=headers)


@pytest.fixture(autouse=True)
def key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TEST_KEY", KEY)


@pytest.fixture
async def unhurried(ctx: Context) -> Llm:
    """The model service for a request that waits for something before its reply begins.

    The other tests give a reply fifty milliseconds to begin. That is plenty for a reply
    that is written down in the test. A stored key is read in a thread before the request
    is made, from a keyring for some, and a socket takes a while to connect and to carry
    its first bytes. Where the machine is busy either takes longer than that, as a machine
    of the CI showed, and the test would fail for it.
    """
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 30})
    return ctx.use(LLM)


def connect(ctx: Context, llm: Llm, service: Service, config: Config = CONFIG) -> OpenAIChat:
    adapter = OpenAIChat(config, transport=httpx.MockTransport(service))
    llm.adapters.add(ctx, config.provider, adapter)
    ctx.effect(lambda: adapter.aclose)
    return adapter


async def ask(
    llm: Llm,
    messages: tuple[Message, ...] = HELLO,
    *,
    config: CallConfig = MODEL,
    tools: tuple[ToolSchema, ...] = (),
) -> LlmCall:
    call = llm.call(LlmRequest(config=config, messages=messages, tools=tools))
    async for _ in call:
        pass
    return call


# The request


async def test_a_request_is_a_streamed_chat_completion(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    await ask(llm)

    request = service.requests[0]
    assert request.method == "POST"
    assert str(request.url) == "https://api.example.test/chat/completions"
    assert request.headers["authorization"] == f"Bearer {KEY}"
    assert request.headers["content-type"] == "application/json"
    assert service.body == {
        "model": "deepseek-test",
        "messages": [{"role": "user", "content": "hello"}],
        "stream": True,
        "stream_options": {"include_usage": True},
    }


def clients_made(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """Note what every HTTP client of the adapter is made with."""
    made: list[dict[str, Any]] = []
    real = httpx.AsyncClient

    def client(**given: Any) -> httpx.AsyncClient:
        made.append(given)
        return real(**given)

    monkeypatch.setattr(httpx, "AsyncClient", client)
    return made


async def test_the_requests_go_through_the_proxy_of_the_system(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    asked: list[str] = []

    def proxy_for(url: str, env: object) -> str:
        asked.append(url)
        return "http://127.0.0.1:7890"

    monkeypatch.setattr(openai_chat, "proxy_for", proxy_for)
    made = clients_made(monkeypatch)

    adapter = OpenAIChat(CONFIG)
    await adapter.aclose()

    assert asked == [CONFIG.base_url]
    assert [given["proxy"] for given in made] == ["http://127.0.0.1:7890"]


async def test_a_transport_of_a_test_goes_through_no_proxy(monkeypatch: pytest.MonkeyPatch) -> None:
    def proxy_for(url: str, env: object) -> str:
        raise AssertionError("the system is not asked for a proxy")

    monkeypatch.setattr(openai_chat, "proxy_for", proxy_for)
    made = clients_made(monkeypatch)

    adapter = OpenAIChat(CONFIG, transport=httpx.MockTransport(Service(streamed(TEXT_REPLY))))
    await adapter.aclose()

    assert [given["proxy"] for given in made] == [None]


async def test_a_key_that_is_given_is_sent_without_the_blanks_around_it() -> None:
    service = Service(listing("deepseek-flash"))
    adapter = OpenAIChat(CONFIG, transport=httpx.MockTransport(service), secret=f"  {GIVEN}\n")
    try:
        await adapter.models()
    finally:
        await adapter.aclose()

    assert service.requests[0].headers["authorization"] == f"Bearer {GIVEN}"


@pytest.mark.parametrize("given", ["", " \n"])
async def test_a_key_that_is_given_empty_is_looked_for_as_ever(given: str) -> None:
    service = Service(listing("deepseek-flash"))
    adapter = OpenAIChat(CONFIG, transport=httpx.MockTransport(service), secret=given)
    try:
        await adapter.models()
    finally:
        await adapter.aclose()

    assert service.requests[0].headers["authorization"] == f"Bearer {KEY}"


async def test_the_base_url_may_have_a_path(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    local = Config(provider="ollama", base_url="http://localhost:11434/v1/")
    connect(ctx, llm, service, local)

    await ask(llm, config=CallConfig(provider="ollama", model="qwen"))

    assert str(service.requests[0].url) == "http://localhost:11434/v1/chat/completions"
    assert "authorization" not in service.requests[0].headers


async def test_what_the_call_config_sets_is_sent_and_nothing_else(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)
    config = CallConfig(
        provider="deepseek",
        model="deepseek-test",
        context_window=64_000,
        max_output_tokens=1024,
        temperature=0.0,
        top_p=0.9,
        seed=7,
        options={"frequency_penalty": 0.5, "temperature": 0.2},
    )

    await ask(llm, config=config)

    assert service.body == {
        "model": "deepseek-test",
        "messages": [{"role": "user", "content": "hello"}],
        "stream": True,
        "stream_options": {"include_usage": True},
        "max_tokens": 1024,
        "temperature": 0.2,
        "top_p": 0.9,
        "seed": 7,
        "frequency_penalty": 0.5,
    }


async def test_the_limit_of_a_reply_is_sent_under_the_name_that_the_service_takes(
    ctx: Context, llm: Llm
) -> None:
    service = Service(streamed(TEXT_REPLY), streamed(TEXT_REPLY))
    connect(ctx, llm, service)
    newer = CONFIG.model_copy(
        update={"provider": "newer", "max_tokens_field": "max_completion_tokens"}
    )
    connect(ctx, llm, service, newer)
    limited = MODEL.model_copy(update={"max_output_tokens": 256})

    await ask(llm, config=limited)
    first = service.body
    await ask(llm, config=limited.model_copy(update={"provider": "newer"}))
    second = service.body

    assert CONFIG.max_tokens_field == "max_tokens"
    assert (first.get("max_tokens"), first.get("max_completion_tokens")) == (256, None)
    assert (second.get("max_tokens"), second.get("max_completion_tokens")) == (None, 256)


async def test_tools_are_offered_as_functions(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    await ask(llm, tools=(READ,))

    assert service.body["tools"] == [
        {
            "type": "function",
            "function": {
                "name": "read",
                "description": "Read a file.",
                "parameters": {"type": "object", "properties": {"path": {"type": "string"}}},
            },
        }
    ]


async def test_a_conversation_is_sent_in_the_shape_of_the_protocol(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)
    messages: tuple[Message, ...] = (
        SystemMessage(content=(TextBlock(text="You are helpful."),)),
        UserMessage(content=(TextBlock(text="Read a.txt."), TextBlock(text="Then b.txt."))),
        AssistantMessage(
            content=(
                ReasoningBlock(text="I need the file.", replay_state={"adapter": "deepseek"}),
                TextBlock(text="I will look."),
                ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
                ToolCallBlock(call_id="call-2", name="list", arguments=""),
            )
        ),
        ToolMessage(call_id="call-1", content=(TextBlock(text="contents"),)),
        ToolMessage(call_id="call-2", content=(TextBlock(text="no such tool"),), is_error=True),
        AssistantMessage(content=(ToolCallBlock(call_id="call-3", name="read", arguments="{}"),)),
    )

    await ask(llm, messages)

    assert service.body["messages"] == [
        {"role": "system", "content": "You are helpful."},
        {"role": "user", "content": "Read a.txt.\n\nThen b.txt."},
        {
            "role": "assistant",
            "content": "I will look.",
            "reasoning_content": "I need the file.",
            "tool_calls": [
                {
                    "id": "call-1",
                    "type": "function",
                    "function": {"name": "read", "arguments": '{"path": "a.txt"}'},
                },
                {
                    "id": "call-2",
                    "type": "function",
                    "function": {"name": "list", "arguments": "{}"},
                },
            ],
        },
        {"role": "tool", "tool_call_id": "call-1", "content": "contents"},
        {"role": "tool", "tool_call_id": "call-2", "content": "no such tool"},
        {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call-3",
                    "type": "function",
                    "function": {"name": "read", "arguments": "{}"},
                }
            ],
        },
    ]


async def test_reasoning_goes_back_only_to_the_adapter_it_came_from(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)
    messages: tuple[Message, ...] = (
        *HELLO,
        AssistantMessage(
            content=(
                ReasoningBlock(text="from elsewhere", replay_state={"adapter": "other"}),
                ReasoningBlock(text="shown only"),
                TextBlock(text="Hi."),
            )
        ),
        *HELLO,
    )

    await ask(llm, messages)

    assert service.body["messages"][1] == {"role": "assistant", "content": "Hi."}


@pytest.mark.parametrize(
    ("style", "level", "expected"),
    [
        ("deepseek", "off", {"thinking": {"type": "disabled"}}),
        ("deepseek", "low", {"thinking": {"type": "enabled"}, "reasoning_effort": "low"}),
        ("deepseek", "high", {"thinking": {"type": "enabled"}, "reasoning_effort": "high"}),
        ("deepseek", "max", {"thinking": {"type": "enabled"}, "reasoning_effort": "max"}),
        ("deepseek", None, {}),
        ("openai", "low", {"reasoning_effort": "low"}),
        ("openai", "high", {"reasoning_effort": "high"}),
        ("openai", "max", {"reasoning_effort": "high"}),
        ("openai", "off", {}),
        ("none", "high", {}),
    ],
)
async def test_how_much_to_reason_is_said_in_the_dialect_of_the_service(
    ctx: Context, llm: Llm, style: str, level: str | None, expected: dict[str, Any]
) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"reasoning_style": style}))
    config = CallConfig.model_validate(
        {"provider": "deepseek", "model": "deepseek-test", "reasoning": level}
    )

    await ask(llm, config=config)

    sent = {
        key: service.body[key] for key in ("thinking", "reasoning_effort") if key in service.body
    }
    assert sent == expected


@pytest.mark.parametrize("style", ["deepseek", "openai", "none"])
async def test_the_adapter_says_how_its_service_is_told_how_much_to_reason(style: str) -> None:
    adapter = OpenAIChat(CONFIG.model_copy(update={"reasoning_style": style}))
    try:
        assert adapter.reasoning_style == style
    finally:
        await adapter.aclose()


@pytest.mark.parametrize(
    ("style", "level", "options", "expected"),
    [
        (
            "deepseek",
            "off",
            {"thinking": {"type": "enabled"}, "reasoning_effort": "high"},
            {"thinking": {"type": "disabled"}},
        ),
        (
            "deepseek",
            "low",
            {"thinking": {"type": "disabled"}, "reasoning_effort": "max"},
            {"thinking": {"type": "enabled"}, "reasoning_effort": "low"},
        ),
        ("openai", "low", {"reasoning_effort": "high"}, {"reasoning_effort": "low"}),
        # Where the call asks for nothing, the options say what they say.
        (
            "deepseek",
            None,
            {"thinking": {"type": "disabled"}, "reasoning_effort": "low"},
            {"thinking": {"type": "disabled"}, "reasoning_effort": "low"},
        ),
    ],
)
async def test_how_much_to_reason_is_said_over_what_the_options_say(
    ctx: Context,
    llm: Llm,
    style: str,
    level: str | None,
    options: dict[str, Any],
    expected: dict[str, Any],
) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"reasoning_style": style}))
    config = CallConfig.model_validate(
        {"provider": "deepseek", "model": "deepseek-test", "reasoning": level, "options": options}
    )

    await ask(llm, config=config)

    sent = {
        key: service.body[key] for key in ("thinking", "reasoning_effort") if key in service.body
    }
    assert sent == expected


async def test_usage_is_not_asked_for_from_a_service_that_would_not_understand(
    ctx: Context, llm: Llm
) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"include_usage": False}))

    await ask(llm)

    assert "stream_options" not in service.body


# The reply


async def test_streamed_text_becomes_a_text_block(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(streamed(TEXT_REPLY)))

    call = await ask(llm)

    assert call.chunks == [
        BlockStart(index=0, kind="text"),
        TextDelta(index=0, text="Hel"),
        TextDelta(index=0, text="lo"),
        BlockEnd(index=0),
        Finish(reason="stop"),
    ]
    assert call.message == AssistantMessage(content=(TextBlock(text="Hello"),))


async def test_reasoning_comes_before_the_text_and_remembers_its_adapter(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        delta(role="assistant", content="", reasoning_content="Let me "),
        delta(reasoning_content="think."),
        delta(content="Four."),
        finish("stop"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.chunks == [
        BlockStart(index=0, kind="reasoning"),
        ReasoningDelta(index=0, text="Let me "),
        ReasoningDelta(index=0, text="think."),
        BlockEnd(index=0, replay_state={"adapter": "deepseek"}),
        BlockStart(index=1, kind="text"),
        TextDelta(index=1, text="Four."),
        BlockEnd(index=1),
        Finish(reason="stop"),
    ]
    assert call.message == AssistantMessage(
        content=(
            ReasoningBlock(text="Let me think.", replay_state={"adapter": "deepseek"}),
            TextBlock(text="Four."),
        )
    )


async def test_reasoning_is_also_read_from_the_field_that_other_services_use(
    ctx: Context, llm: Llm
) -> None:
    body = sse(delta(reasoning="Hm."), delta(content="Yes."), finish("stop"), "[DONE]")
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.message.content[0] == ReasoningBlock(
        text="Hm.", replay_state={"adapter": "deepseek"}
    )


async def test_tool_calls_arrive_in_fragments(ctx: Context, llm: Llm) -> None:
    def fragment(index: int, arguments: str, **first: str) -> dict[str, Any]:
        function = {"arguments": arguments} | ({"name": first["name"]} if first else {})
        call = {"index": index, "function": function} | (
            {"id": first["id"], "type": "function"} if first else {}
        )
        return delta(tool_calls=[call])

    body = sse(
        delta(role="assistant", content="I will look."),
        fragment(0, "", id="call_a", name="read"),
        fragment(0, '{"path": '),
        fragment(0, '"a.txt"}'),
        fragment(1, '{"path": "b.txt"}', id="call_b", name="read"),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.chunks == [
        BlockStart(index=0, kind="text"),
        TextDelta(index=0, text="I will look."),
        BlockEnd(index=0),
        BlockStart(index=1, kind="tool-call", call_id="call_a", name="read"),
        ToolCallDelta(index=1, arguments='{"path": '),
        ToolCallDelta(index=1, arguments='"a.txt"}'),
        BlockStart(index=2, kind="tool-call", call_id="call_b", name="read"),
        ToolCallDelta(index=2, arguments='{"path": "b.txt"}'),
        BlockEnd(index=1),
        BlockEnd(index=2),
        Finish(reason="tool-calls"),
    ]
    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call_a", name="read", arguments='{"path": "a.txt"}'),
        ToolCallBlock(call_id="call_b", name="read", arguments='{"path": "b.txt"}'),
    )


async def test_a_tool_call_without_an_id_gets_one(ctx: Context, llm: Llm) -> None:
    body = sse(
        delta(tool_calls=[{"index": 0, "function": {"name": "read", "arguments": "{}"}}]),
        delta(tool_calls=[{"index": 1, "function": {"name": "read", "arguments": "{}"}}]),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    first, second = call.message.tool_calls
    assert first.call_id.startswith("call_")
    assert len(first.call_id) > len("call_")
    assert first.call_id != second.call_id


def piece(index: int = 0, **fields: str) -> dict[str, Any]:
    """An event with one fragment of a tool call: any of ``id``, ``name`` and ``arguments``."""
    function = {key: fields[key] for key in ("name", "arguments") if key in fields}
    call: dict[str, Any] = {"index": index, "function": function}
    if "id" in fields:
        call["id"] = fields["id"]
    return delta(tool_calls=[call])


async def test_the_name_and_the_id_of_a_tool_call_may_arrive_in_pieces(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        piece(id="call_", name="re", arguments="{"),
        piece(id="abc", name="ad", arguments="}"),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.chunks == [
        BlockStart(index=0, kind="tool-call", call_id="call_abc", name="read"),
        ToolCallDelta(index=0, arguments="{}"),
        BlockEnd(index=0),
        Finish(reason="tool-calls"),
    ]


@pytest.mark.parametrize(
    "pieces",
    [
        [piece(id="call_a"), piece(name="read"), piece(arguments="{}"), piece(arguments="")],
        [piece(name="read"), piece(id="call_a"), piece(arguments="{}")],
        [piece(arguments=""), piece(id="call_a", name="read"), piece(arguments="{}")],
    ],
    ids=["id first", "name first", "both late"],
)
async def test_the_name_and_the_id_of_a_tool_call_may_arrive_one_after_the_other(
    ctx: Context, llm: Llm, pieces: list[dict[str, Any]]
) -> None:
    connect(ctx, llm, Service(streamed(sse(*pieces, finish("tool_calls"), "[DONE]"))))

    call = await ask(llm, tools=(READ,))

    assert call.failure is None
    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call_a", name="read", arguments="{}"),
    )


async def test_a_service_that_repeats_the_name_and_the_id_in_every_event_is_understood(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        piece(id="call_a", name="read", arguments='{"path": '),
        piece(id="call_a", name="read", arguments='"a.txt"}'),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call_a", name="read", arguments='{"path": "a.txt"}'),
    )


async def test_a_tool_call_is_passed_on_once_a_piece_of_it_comes_without_name_and_id(
    ctx: Context, llm: Llm
) -> None:
    body = Body(
        [
            sse(piece(id="call_a", name="read", arguments="")),
            sse(piece(arguments='{"path": ')),
            sse(piece(1, id="call_b", name="read", arguments="{}")),
            sse(delta(content="Both at once.")),
        ],
        hang=True,
    )
    connect(ctx, llm, Service(streamed(body)))
    call = llm.call(LlmRequest(config=MODEL, messages=HELLO, tools=(READ,)))

    async def read() -> None:
        async for _ in call:
            pass

    reader = asyncio.ensure_future(read())
    for _ in range(100):
        await asyncio.sleep(0)
        if len(call.chunks) == 4:
            break
    reader.cancel()
    await asyncio.gather(reader, return_exceptions=True)

    # The reply has not ended. The first call began when a piece of it came without a name
    # or an id. The second came whole, so more of its name may still follow: it waits.
    assert call.chunks == [
        BlockStart(index=0, kind="tool-call", call_id="call_a", name="read"),
        ToolCallDelta(index=0, arguments='{"path": '),
        BlockStart(index=1, kind="text"),
        TextDelta(index=1, text="Both at once."),
    ]


async def test_a_tool_call_that_came_whole_is_passed_on_when_the_reply_ends(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        piece(id="call_a", name="read", arguments='{"path": "a.txt"}'),
        delta(content="One moment."),
        piece(1, id="call_b", name="read", arguments='{"path": "b.txt"}'),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.chunks == [
        BlockStart(index=0, kind="text"),
        TextDelta(index=0, text="One moment."),
        BlockEnd(index=0),
        BlockStart(index=1, kind="tool-call", call_id="call_a", name="read"),
        ToolCallDelta(index=1, arguments='{"path": "a.txt"}'),
        BlockStart(index=2, kind="tool-call", call_id="call_b", name="read"),
        ToolCallDelta(index=2, arguments='{"path": "b.txt"}'),
        BlockEnd(index=1),
        BlockEnd(index=2),
        Finish(reason="tool-calls"),
    ]


@pytest.mark.parametrize(
    "between",
    [piece(1, id="call_b", name="read", arguments="{}"), delta(content="One moment.")],
    ids=["another call", "text"],
)
async def test_pieces_of_a_name_may_be_parted_by_something_else(
    ctx: Context, llm: Llm, between: dict[str, Any]
) -> None:
    body = sse(
        piece(id="call_", name="re", arguments="{"),
        between,
        piece(id="abc", name="ad", arguments="}"),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.failure is None
    assert call.message.tool_calls[0] == ToolCallBlock(
        call_id="call_abc", name="read", arguments="{}"
    )


async def test_a_name_that_comes_after_another_call_began_is_still_taken(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        piece(id="call_a", arguments="{"),
        piece(1, id="call_b", name="read", arguments="{}"),
        piece(name="read", arguments="}"),
        finish("tool_calls"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call_a", name="read", arguments="{}"),
        ToolCallBlock(call_id="call_b", name="read", arguments="{}"),
    )


async def test_a_tool_call_that_was_cut_off_before_it_began_is_left_out(
    ctx: Context, llm: Llm
) -> None:
    body = sse(
        delta(content="I will look."),
        piece(id="call_a", name="read", arguments='{"pa'),
        finish("length"),
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, tools=(READ,))

    assert call.finish == Finish(reason="max-tokens")
    assert call.message == AssistantMessage(content=(TextBlock(text="I will look."),))


async def test_a_tool_call_without_a_name_is_a_malformed_reply(ctx: Context, llm: Llm) -> None:
    body = sse(piece(id="call_a", arguments="{}"), finish("tool_calls"), "[DONE]")
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="a tool call began without a name", code="MALFORMED_RESPONSE"
    )


async def test_only_the_first_choice_of_a_reply_is_read(ctx: Context, llm: Llm) -> None:
    def choice(index: int, text: str | None, reason: str | None = None) -> dict[str, Any]:
        fields = {} if text is None else {"content": text}
        return {"index": index, "delta": fields, "finish_reason": reason}

    body = sse(
        {"choices": [choice(1, "B")]},
        {"choices": [choice(0, "A")]},
        {"choices": [choice(1, "b", "length"), choice(0, "a")]},
        {"choices": [choice(0, None, "stop")]},
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm, config=MODEL.model_copy(update={"options": {"n": 2}}))

    assert call.message.text == "Aa"
    assert call.finish == Finish(reason="stop")


async def test_a_reply_cut_off_by_the_token_limit_says_so(ctx: Context, llm: Llm) -> None:
    body = sse(delta(content="This sentence never"), finish("length"), "[DONE]")
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.finish == Finish(reason="max-tokens")
    assert call.message.text == "This sentence never"


async def test_a_reply_that_the_provider_withheld_is_a_failure(ctx: Context, llm: Llm) -> None:
    body = sse(delta(content="I can"), finish("content_filter"), "[DONE]")
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="the provider withheld the reply", code="CONTENT_FILTER"
    )


async def test_an_unknown_finish_reason_ends_the_reply_normally(ctx: Context, llm: Llm) -> None:
    body = sse(delta(content="Done."), finish("eos"), "[DONE]")
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.finish == Finish(reason="stop")


@pytest.mark.parametrize(
    ("reported", "expected"),
    [
        (
            {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "prompt_cache_hit_tokens": 900,
                "prompt_cache_miss_tokens": 100,
            },
            Usage(input_tokens=100, output_tokens=50, cache_read_tokens=900),
        ),
        (
            {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "prompt_tokens_details": {"cached_tokens": 400},
            },
            Usage(input_tokens=600, output_tokens=50, cache_read_tokens=400),
        ),
        (
            {"prompt_tokens": 12, "completion_tokens": 3},
            Usage(input_tokens=12, output_tokens=3),
        ),
    ],
)
async def test_the_token_counts_do_not_overlap(
    ctx: Context, llm: Llm, reported: dict[str, Any], expected: Usage
) -> None:
    body = sse(
        delta(content="Hi."),
        finish("stop"),
        {"choices": [], "usage": reported},
        "[DONE]",
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.usage == expected
    assert call.chunks[-2:] == [UsageChunk(usage=expected), Finish(reason="stop")]


async def test_the_stream_is_read_as_server_sent_events(ctx: Context, llm: Llm) -> None:
    body = (
        b": keep-alive\r\n\r\n"
        b"event: message\r\n"
        b'data:{"choices":[{"delta":{"content":"He"}}]}\r\n\r\n'
        b"id: 7\r\n"
        b'data: {"choices":[{"delta":\r\n'
        b'data: {"content":"llo"}}]}\r\n\r\n'
        b'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\r\n\r\n'
        b"data: [DONE]\r\n\r\n"
        b'data: {"choices":[{"delta":{"content":"ignored"}}]}\r\n\r\n'
    )
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.message.text == "Hello"


async def test_a_stream_that_ends_right_after_the_finish_reason_is_complete(
    ctx: Context, llm: Llm
) -> None:
    connect(ctx, llm, Service(streamed(sse(delta(content="Hi."), finish("stop")))))

    call = await ask(llm)

    assert call.finish == Finish(reason="stop")


async def test_a_last_event_without_a_blank_line_after_it_still_counts(
    ctx: Context, llm: Llm
) -> None:
    body = sse(delta(content="Hi.")) + b'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}'
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.finish == Finish(reason="stop")
    assert call.message.text == "Hi."


# Failures


async def test_a_service_without_a_key_can_refuse_too(ctx: Context, llm: Llm) -> None:
    local = Config(provider="ollama", base_url="http://localhost:11434/v1")
    connect(ctx, llm, Service(refused(500, {"message": "model failed to load"})), local)

    call = await ask(llm, config=CallConfig(provider="ollama", model="qwen"))

    assert call.failure == LlmFailure(message="model failed to load", code="SERVER", status=500)


async def test_a_stream_that_ends_without_a_finish_reason_is_a_failure(
    ctx: Context, llm: Llm
) -> None:
    connect(ctx, llm, Service(streamed(sse(delta(content="Hel")))))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"
    assert call.message.text == "Hel"


async def test_a_line_that_is_not_json_is_a_malformed_reply(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(streamed(sse(delta(content="Hel"), '{"choices": [{"delta"'))))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "MALFORMED_RESPONSE"
    assert call.failure.message.startswith("the service sent an event that cannot be read: ")
    assert call.message.text == "Hel"


async def test_an_event_of_the_wrong_shape_is_a_malformed_reply(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(streamed(sse({"choices": [{"delta": {"content": 5}}]}))))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "MALFORMED_RESPONSE"


async def test_an_error_inside_the_stream_is_a_failure(ctx: Context, llm: Llm) -> None:
    body = sse(delta(content="Hel"), {"error": {"message": "the engine is overloaded"}})
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="the engine is overloaded", code="SERVER")
    assert call.message.text == "Hel"


@pytest.mark.parametrize(
    ("status", "error", "code"),
    [
        (401, {"message": "Authentication Fails"}, "AUTH"),
        (403, {"message": "forbidden"}, "AUTH"),
        (402, {"message": "Insufficient Balance"}, "QUOTA"),
        (429, {"message": "Rate limit reached"}, "RATE_LIMIT"),
        (429, {"message": "You exceeded your quota", "code": "insufficient_quota"}, "QUOTA"),
        (400, {"message": "messages: field required"}, "INVALID_REQUEST"),
        (422, {"message": "Invalid Parameters"}, "INVALID_REQUEST"),
        (404, {"message": "Model Not Exist"}, "INVALID_REQUEST"),
        (
            400,
            {"message": "This model's maximum context length is 65536 tokens."},
            "CONTEXT_WINDOW_EXCEEDED",
        ),
        (
            400,
            {"message": "too long", "code": "context_length_exceeded"},
            "CONTEXT_WINDOW_EXCEEDED",
        ),
        (408, {"message": "Request Timeout"}, "TIMEOUT"),
        (500, {"message": "Server Error"}, "SERVER"),
        (503, {"message": "Server Overloaded"}, "SERVER"),
        (418, {"message": "I am a teapot"}, "HTTP_418"),
    ],
)
async def test_a_refusal_of_the_service_is_a_failure_with_a_stable_code(
    ctx: Context, llm: Llm, status: int, error: dict[str, Any], code: str
) -> None:
    connect(ctx, llm, Service(refused(status, error, **{"x-request-id": "req-7"})))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message=error["message"], code=code, status=status, request_id="req-7"
    )
    assert call.chunks == [call.finish]


async def test_a_refusal_that_is_no_json_keeps_its_text(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(refused(502, "<html>Bad Gateway</html>" + "x" * 600)))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "SERVER"
    assert call.failure.message.startswith("<html>Bad Gateway</html>")
    assert len(call.failure.message) == 500


async def test_a_refusal_without_a_body_is_named_by_its_status(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(httpx.Response(503)))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="HTTP 503 Service Unavailable", code="SERVER", status=503
    )


@pytest.mark.parametrize(
    ("header", "expected"),
    [
        ("2", 2000),
        ("0.5", 500),
        ("soon", None),
        ("-1", None),
        ("inf", None),
        ("nan", None),
        ("1e400", None),
        ("", None),
    ],
)
async def test_the_wait_that_the_service_asks_for_is_passed_on(
    ctx: Context, llm: Llm, header: str, expected: int | None
) -> None:
    response = refused(429, {"message": "slow down"}, **{"retry-after": header})
    connect(ctx, llm, Service(response))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="slow down", code="RATE_LIMIT", status=429, retry_after_ms=expected
    )


async def test_the_wait_may_be_given_as_a_point_in_time(ctx: Context, llm: Llm) -> None:
    now = datetime.now(UTC)
    dates = [
        format_datetime(now + timedelta(seconds=90), usegmt=True),
        # A date without a zone, which is how the oldest form of an HTTP date is read.
        format_datetime((now + timedelta(seconds=90)).replace(tzinfo=None)),
        format_datetime(now - timedelta(hours=1), usegmt=True),
    ]
    assert dates[0].endswith(" GMT")
    assert dates[1].endswith(" -0000")
    connect(
        ctx,
        llm,
        Service(*(refused(429, {"message": "slow down"}, **{"retry-after": d}) for d in dates)),
    )

    failures = [(await ask(llm)).failure for _ in dates]
    waits = [failure.retry_after_ms for failure in failures if failure is not None]

    assert len(waits) == 3
    assert waits[0] is not None
    assert 85_000 < waits[0] <= 90_000
    assert waits[1] is not None
    assert 85_000 < waits[1] <= 90_000
    assert waits[2] == 0


async def test_a_service_that_cannot_be_reached_is_a_transport_failure(
    ctx: Context, llm: Llm
) -> None:
    connect(ctx, llm, Service(httpx.ConnectError("All connection attempts failed")))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="ConnectError: All connection attempts failed", code="TRANSPORT"
    )


async def test_a_service_that_is_too_slow_to_connect_is_a_timeout(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(httpx.ConnectTimeout("")))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="ConnectTimeout", code="TIMEOUT")


async def test_a_connection_that_breaks_in_the_middle_keeps_what_arrived(
    ctx: Context, llm: Llm
) -> None:
    body = Body([sse(delta(content="Hel"))], then=httpx.ReadError("connection reset"))
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="ReadError: connection reset", code="TRANSPORT")
    assert call.message.text == "Hel"
    assert body.closed


async def test_without_the_key_nothing_is_sent(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="the environment variable TEST_KEY is not set", code="AUTH"
    )
    assert service.requests == []


@pytest.mark.parametrize(
    "found",
    [f"{KEY}\n", f"{KEY}\r\n", f"  {KEY}", f"{KEY}\t ", f"\n{KEY}\n"],
    ids=["a line break after", "a line break of Windows", "blanks before", "a tab after", "both"],
)
async def test_blanks_and_line_breaks_around_the_key_are_not_sent(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch, found: str
) -> None:
    monkeypatch.setenv("TEST_KEY", found)
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    call = await ask(llm)

    assert call.failure is None
    assert service.requests[0].headers["authorization"] == f"Bearer {KEY}"


async def test_a_variable_that_holds_only_blanks_holds_no_key(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_KEY", " \n")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="the environment variable TEST_KEY is not set", code="AUTH"
    )
    assert service.requests == []


@pytest.mark.parametrize(
    "found",
    [
        "sk-test\n0123456789",
        "sk-test 0123456789",
        chr(0x201C) + KEY + chr(0x201D),
        "sk-test-" + chr(0xE9) + "-0123456789",
        KEY + chr(0x200B),
        KEY + chr(0x7F),
    ],
    ids=[
        "a line break in the middle",
        "a blank in the middle",
        "the quotation marks of a word processor",
        "a letter with an accent",
        "a mark that cannot be seen",
        "a control character",
    ],
)
async def test_a_key_that_cannot_be_sent_is_refused_and_is_not_shown(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, found: str
) -> None:
    monkeypatch.setenv("TEST_KEY", found)
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service)

    call = await ask(unhurried)

    # The words are the same for every such key, so nothing of the key is in them.
    assert call.failure == LlmFailure(
        message="the API key in the environment variable TEST_KEY cannot be sent: there is a "
        "character in it that no key has, such as a blank, a line break or a quotation mark of "
        "a word processor. Set it again",
        code="AUTH",
    )
    assert service.requests == []


# The keys that a user stored


PLACE = "https://api.example.test"
"""Where the address of the adapter of these tests leads."""


def with_stored(tmp_path: Path, **keys: str) -> Config:
    """The config of an adapter that looks into a file of keys, with these keys in it."""
    file = tmp_path / "credentials.json"
    for provider, stored in keys.items():
        keep(file, provider, stored, PLACE)
    return CONFIG.model_copy(update={"api_key_file": file})


def by_hand(file: Path, key: str, place: str = PLACE) -> None:
    """Write a file of keys by hand, with a key for the provider of these tests."""
    held = {"version": 1, "keys": {"deepseek": {"key": key, "sent_to": place}}}
    file.write_text(json.dumps(held), encoding="utf-8")
    file.chmod(0o600)


def sent_as_key(service: Service) -> list[str | None]:
    return [request.headers.get("authorization") for request in service.requests]


async def test_a_stored_key_is_sent_where_the_variable_holds_none(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service(streamed(TEXT_REPLY))
    connect(
        ctx, unhurried, service, with_stored(tmp_path, another="sk-of-another", deepseek=STORED)
    )

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {STORED}"]


async def test_the_variable_goes_before_the_stored_key(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, with_stored(tmp_path, deepseek=STORED))

    await ask(unhurried)

    assert sent_as_key(service) == [f"Bearer {KEY}"]


async def test_where_the_variable_holds_a_key_the_file_is_not_looked_into(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    config = with_stored(tmp_path)
    assert config.api_key_file is not None
    config.api_key_file.write_text("not what a file of keys holds", encoding="utf-8")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {KEY}"]


async def test_a_key_that_lies_in_a_keyring_is_sent_as_one_from_the_file_is(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    vault = Memory().install()
    config = with_stored(tmp_path, another="sk-of-another", deepseek=STORED)
    assert config.api_key_file is not None
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {STORED}"]
    assert STORED.encode() not in config.api_key_file.read_bytes()
    assert sorted(provider for _, provider in vault.held) == ["another", "deepseek"]


async def test_a_key_that_its_keyring_does_not_give_is_a_failure_that_says_what_to_do(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    vault = Memory().install()
    config = with_stored(tmp_path, deepseek=STORED)
    vault.locked = True
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure == LlmFailure(
        message="the key of deepseek is kept in the keyring of this system, and the keyring "
        "could not be asked for it: Failed to unlock the collection. Put the key into the "
        "variable of the service, or enter it again with 'columbina setup'",
        code="AUTH",
    )
    assert service.requests == []


async def test_where_the_variable_holds_a_key_no_keyring_is_asked(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    Memory().install()
    config = with_stored(tmp_path, deepseek=STORED)
    vault = Locked().install()
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {KEY}"]
    assert vault.asked == []


async def test_a_stored_key_needs_no_variable_to_be_named(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    config = with_stored(tmp_path, deepseek=STORED).model_copy(update={"api_key_env": None})
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    await ask(unhurried)

    assert sent_as_key(service) == [f"Bearer {STORED}"]


async def test_a_key_that_is_stored_while_the_program_runs_is_found_at_the_next_request(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path)
    assert config.api_key_file is not None
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    first = await ask(unhurried)
    keep(config.api_key_file, "deepseek", STORED, PLACE)
    second = await ask(unhurried)

    assert first.failure == LlmFailure(
        message="there is no API key for deepseek: the environment variable TEST_KEY is not "
        "set, and no key is stored. Enter one with 'columbina setup', or set the variable",
        code="AUTH",
    )
    assert second.failure is None
    assert sent_as_key(service) == [f"Bearer {STORED}"]


async def test_where_only_a_file_is_named_the_words_about_a_missing_key_name_no_variable(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    config = with_stored(tmp_path, other=STORED).model_copy(update={"api_key_env": None})
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure == LlmFailure(
        message="no API key is stored for deepseek. Enter one with 'columbina setup'",
        code="AUTH",
    )
    assert service.requests == []


async def test_a_service_that_asks_for_no_key_is_sent_one_only_if_one_is_found(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path).model_copy(update={"api_key_required": False})
    assert config.api_key_file is not None
    service = Service(streamed(TEXT_REPLY), streamed(TEXT_REPLY), streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    without = await ask(unhurried)
    keep(config.api_key_file, "deepseek", STORED, PLACE)
    await ask(unhurried)
    monkeypatch.setenv("TEST_KEY", KEY)
    await ask(unhurried)

    assert without.failure is None
    assert sent_as_key(service) == [None, f"Bearer {STORED}", f"Bearer {KEY}"]


async def test_where_no_key_is_looked_for_none_is_sent(ctx: Context, llm: Llm) -> None:
    # Neither a variable nor a file: there is nowhere to find a key, so none is asked for.
    config = Config(provider="deepseek", base_url="https://api.example.test")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, config)

    call = await ask(llm)

    assert config.api_key_required
    assert call.failure is None
    assert sent_as_key(service) == [None]


async def test_what_is_around_a_stored_key_is_not_sent(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path)
    assert config.api_key_file is not None
    # Written by hand, and not by the command that stores a key.
    by_hand(config.api_key_file, f" {STORED}\n")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    await ask(unhurried)

    assert sent_as_key(service) == [f"Bearer {STORED}"]


@pytest.mark.parametrize("stored", ["sk-stored 9876543210", ""], ids=["a blank", "nothing"])
async def test_a_stored_key_that_cannot_be_sent_is_not_sent(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path, stored: str
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path)
    assert config.api_key_file is not None
    by_hand(config.api_key_file, stored)
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    cannot_be_sent = (
        "the API key that is stored for deepseek cannot be sent: there is a character in it "
        "that no key has, such as a blank, a line break or a quotation mark of a word "
        "processor. Enter it again with 'columbina setup'"
    )
    none = (
        "there is no API key for deepseek: the environment variable TEST_KEY is not set, and "
        "no key is stored. Enter one with 'columbina setup', or set the variable"
    )
    assert call.failure == LlmFailure(message=cannot_be_sent if stored else none, code="AUTH")
    assert service.requests == []


async def test_a_file_of_keys_that_cannot_be_used_is_a_failure_that_says_why(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path)
    assert config.api_key_file is not None
    config.api_key_file.write_text(f"deepseek = {STORED}", encoding="utf-8")
    config.api_key_file.chmod(0o600)
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure == LlmFailure(
        message=f"the file of stored keys, {config.api_key_file}, is not what 'columbina "
        "setup' writes, so it is not used. Delete it, and enter the keys again with "
        "'columbina setup'",
        code="AUTH",
    )
    assert service.requests == []


async def test_a_stored_key_never_shows_up_in_a_failure(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    echo = refused(401, {"message": f"Incorrect API key provided: {STORED}"})
    connect(ctx, unhurried, Service(echo), with_stored(tmp_path, deepseek=STORED))

    call = await ask(unhurried)

    assert call.failure is not None
    assert call.failure.message == "Incorrect API key provided: [key]"


@pytest.mark.parametrize(
    "address",
    [
        "http://other-host.example.test/v1",
        "http://api.example.test",
        "https://api.example.test:8443/v1",
        "https://api.example.test.other-host.example/v1",
    ],
    ids=["another machine", "without encryption", "another port", "a name that begins alike"],
)
async def test_a_stored_key_is_not_sent_where_it_was_not_stored_for(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path, address: str
) -> None:
    # The address of an adapter may come from a variable of the environment. A key that
    # was typed in for one place does not follow the name of its service to another one.
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path, deepseek=STORED).model_copy(update={"base_url": address})
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is not None
    assert call.failure.code == "AUTH"
    assert call.failure.message.startswith(
        "the key that is stored for deepseek was stored for https://api.example.test, and the "
        "requests for deepseek now go to "
    )
    assert "It is not sent there." in call.failure.message
    assert STORED not in call.failure.message
    assert service.requests == []


async def test_a_stored_key_is_sent_wherever_the_same_place_is_written_otherwise(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    config = with_stored(tmp_path, deepseek=STORED).model_copy(
        update={"base_url": "HTTPS://API.Example.Test:443/v2/"}
    )
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {STORED}"]


async def test_a_key_of_the_variable_is_sent_wherever_the_address_leads(
    ctx: Context, unhurried: Llm, tmp_path: Path
) -> None:
    # The variable and the address come from the same hands. Only a stored key has a place.
    config = with_stored(tmp_path, deepseek=STORED).model_copy(
        update={"base_url": "http://other-host.example.test/v1"}
    )
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, unhurried, service, config)

    call = await ask(unhurried)

    assert call.failure is None
    assert sent_as_key(service) == [f"Bearer {KEY}"]


async def test_the_key_never_shows_up_in_a_failure(ctx: Context, llm: Llm) -> None:
    echo = refused(401, {"message": f"Incorrect API key provided: {KEY}"})
    connect(ctx, llm, Service(echo))

    call = await ask(llm)

    assert call.failure is not None
    assert KEY not in call.failure.message
    assert call.failure.message == "Incorrect API key provided: [key]"


async def test_the_key_is_cut_out_of_an_error_inside_the_stream(ctx: Context, llm: Llm) -> None:
    body = sse(delta(content="Hel"), {"error": {"message": f"Invalid API key: {KEY}"}})
    connect(ctx, llm, Service(streamed(body)))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="Invalid API key: [key]", code="SERVER")


async def test_the_key_is_cut_out_of_a_failure_to_reach_the_service(ctx: Context, llm: Llm) -> None:
    broken = Body([sse(delta(content="Hel"))], then=httpx.ReadError(f"lost after Bearer {KEY}"))
    connect(
        ctx,
        llm,
        Service(httpx.ConnectError(f"cannot send Bearer {KEY}"), streamed(broken)),
    )

    first = await ask(llm)
    second = await ask(llm)

    assert first.failure == LlmFailure(
        message="ConnectError: cannot send Bearer [key]", code="TRANSPORT"
    )
    assert second.failure == LlmFailure(
        message="ReadError: lost after Bearer [key]", code="TRANSPORT"
    )


async def test_the_key_is_cut_out_before_a_long_text_is_cut_short(ctx: Context, llm: Llm) -> None:
    # The key lies across the place where the text ends.
    connect(ctx, llm, Service(refused(401, "x" * 490 + KEY + " is not valid")))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.message == "x" * 490 + "[key] is n"


async def test_a_long_error_inside_the_stream_is_cut_short(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(streamed(sse({"error": {"message": "y" * 800}}))))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="y" * 500, code="SERVER")


async def test_the_error_that_held_the_key_does_not_travel_with_the_failure(
    ctx: Context, llm: Llm
) -> None:
    adapter = connect(ctx, llm, Service(httpx.ConnectError(f"cannot send Bearer {KEY}")))

    with pytest.raises(LlmError) as raised:
        async for _ in adapter.stream(LlmRequest(config=MODEL, messages=HELLO)):
            pass

    assert KEY not in "".join(traceback.format_exception(raised.value))
    # Not only out of what is printed: the error that was caught is not held on to at all.
    assert raised.value.__cause__ is None
    assert raised.value.__context__ is None


async def test_a_refusal_that_held_the_key_does_not_travel_with_the_failure(
    ctx: Context, llm: Llm
) -> None:
    adapter = connect(ctx, llm, Service(refused(401, f"bad {KEY}")))

    with pytest.raises(LlmError) as raised:
        async for _ in adapter.stream(LlmRequest(config=MODEL, messages=HELLO)):
            pass

    assert raised.value.failure.message == "bad [key]"
    assert raised.value.__cause__ is None
    assert raised.value.__context__ is None


ODD_KEY = "sk-a" + chr(92) + 'b/c+d"e&f<g'
"""A key with signs in it that a text which repeats it may spell otherwise."""


def spelled_as_json(key: str) -> str:
    return json.dumps(key)[1:-1]


@pytest.mark.parametrize(
    ("body", "cleaned"),
    [
        # A gateway that answers in a shape of its own. Its body is passed on as it came,
        # and JSON writes a backslash and a quotation mark with a backslash before them.
        (
            json.dumps({"detail": f"Invalid key {ODD_KEY}"}),
            json.dumps({"detail": "Invalid key [key]"}),
        ),
        # Some writers of JSON put a backslash before a slash as well.
        (
            '{"detail": "Invalid key '
            + spelled_as_json(ODD_KEY).replace("/", chr(92) + "/")
            + '"}',
            '{"detail": "Invalid key [key]"}',
        ),
        # A key that is repeated as a part of an address.
        ("see /errors?key=sk-a%5Cb%2Fc%2Bd%22e%26f%3Cg for more", "see /errors?key=[key] for more"),
        # A key that is repeated in a page of HTML.
        (
            "<p>Invalid key sk-a" + chr(92) + "b/c+d&quot;e&amp;f&lt;g</p>",
            "<p>Invalid key [key]</p>",
        ),
        # And as it is.
        (f"Invalid key {ODD_KEY}", "Invalid key [key]"),
    ],
    ids=["as JSON writes it", "with the slash escaped", "as an address", "as HTML", "as it is"],
)
async def test_a_key_is_cut_out_in_the_spellings_in_which_a_text_repeats_it(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch, body: str, cleaned: str
) -> None:
    monkeypatch.setenv("TEST_KEY", ODD_KEY)
    service = Service(refused(401, body))
    connect(ctx, llm, service)

    call = await ask(llm)

    assert service.requests[0].headers["authorization"] == f"Bearer {ODD_KEY}"
    assert call.failure is not None
    assert call.failure.message == cleaned
    assert call.failure.code == "AUTH"


async def test_a_key_in_the_message_of_an_error_is_cut_out_whatever_signs_it_has(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_KEY", ODD_KEY)
    connect(ctx, llm, Service(refused(401, {"message": f"Incorrect API key provided: {ODD_KEY}"})))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.message == "Incorrect API key provided: [key]"


async def test_a_spelling_that_holds_another_one_is_cut_out_whole(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The key as an address writes it begins with the key as it is, where its first signs
    # are plain. Cut out in the wrong order, the rest of the longer spelling would stay.
    key = "sk-plain%"
    monkeypatch.setenv("TEST_KEY", key)
    connect(ctx, llm, Service(refused(401, f"no such key: sk-plain%25, sent as {key}")))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.message == "no such key: [key], sent as [key]"


async def test_the_key_is_cut_out_of_the_id_of_a_request(ctx: Context, llm: Llm) -> None:
    echo = refused(401, f"bad {KEY}", **{"x-request-id": f"req-{KEY}-" + "7" * 300})
    connect(ctx, llm, Service(echo))

    call = await ask(llm)

    cleaned = LlmFailure(
        message="bad [key]", code="AUTH", status=401, request_id="req-[key]-" + "7" * 190
    )
    assert call.failure == cleaned
    assert KEY not in cleaned.model_dump_json()


async def test_a_body_that_cannot_be_decoded_is_a_transport_failure(ctx: Context, llm: Llm) -> None:
    garbled = httpx.Response(
        200,
        headers={"content-type": "text/event-stream", "content-encoding": "gzip"},
        stream=Body([b"this is not gzip"]),
    )
    connect(ctx, llm, Service(garbled))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "TRANSPORT"
    assert call.failure.message.startswith("DecodingError")


# The list of models


def listing(*ids: str) -> httpx.Response:
    return httpx.Response(
        200, json={"object": "list", "data": [{"id": id, "object": "model"} for id in ids]}
    )


async def listed(adapter: OpenAIChat) -> LlmFailure:
    """Ask for the list of models where that fails, and return the failure."""
    with pytest.raises(LlmError) as raised:
        await adapter.models()
    assert raised.value.__cause__ is None
    assert raised.value.__context__ is None
    return raised.value.failure


async def test_the_list_of_models_is_asked_for_with_the_key(ctx: Context, llm: Llm) -> None:
    service = Service(listing("deepseek-test", "deepseek-other"))
    adapter = connect(ctx, llm, service)

    ids = await adapter.models()

    assert ids == ["deepseek-test", "deepseek-other"]
    request = service.requests[0]
    assert request.method == "GET"
    assert str(request.url) == "https://api.example.test/models"
    assert request.headers["authorization"] == f"Bearer {KEY}"


async def test_the_list_may_take_half_a_minute_to_arrive(ctx: Context, llm: Llm) -> None:
    service = Service(listing())
    adapter = connect(ctx, llm, service)

    assert await adapter.models() == []

    timeout = service.requests[0].extensions["timeout"]
    assert timeout["read"] == 30.0
    assert timeout["connect"] == CONFIG.connect_timeout_seconds


async def test_without_the_key_the_list_is_not_asked_for(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service()
    adapter = connect(ctx, llm, service)

    with pytest.raises(LlmError) as raised:
        await adapter.models()

    assert raised.value.failure == LlmFailure(
        message="the environment variable TEST_KEY is not set", code="AUTH"
    )
    assert service.requests == []


async def test_a_refusal_of_the_list_says_why_without_the_key(ctx: Context, llm: Llm) -> None:
    echo = refused(401, {"message": f"Incorrect API key provided: {KEY}"})
    adapter = connect(ctx, llm, Service(echo))

    failure = await listed(adapter)

    assert failure == LlmFailure(
        message="Incorrect API key provided: [key]", code="AUTH", status=401
    )


async def test_a_service_without_a_list_of_models_refuses_with_its_status(
    ctx: Context, llm: Llm
) -> None:
    adapter = connect(ctx, llm, Service(refused(404, "404 page not found")))

    failure = await listed(adapter)

    assert failure == LlmFailure(message="404 page not found", code="INVALID_REQUEST", status=404)


@pytest.mark.parametrize(
    ("answer", "problem"),
    [
        (httpx.Response(200, text="<html>Welcome</html>"), "Invalid JSON"),
        (httpx.Response(200, json={"models": ["deepseek-test"]}), "Field required"),
        (httpx.Response(200, json={"data": [{"name": "deepseek-test"}]}), "Field required"),
    ],
)
async def test_a_list_that_cannot_be_read_is_a_malformed_reply(
    ctx: Context, llm: Llm, answer: httpx.Response, problem: str
) -> None:
    adapter = connect(ctx, llm, Service(answer))

    failure = await listed(adapter)

    assert failure.code == "MALFORMED_RESPONSE"
    assert failure.message.startswith(
        f"the service sent a list of models that cannot be read: {problem}"
    )


@pytest.mark.parametrize(
    ("error", "failure"),
    [
        (
            httpx.ConnectError(f"cannot send Bearer {KEY}"),
            LlmFailure(message="ConnectError: cannot send Bearer [key]", code="TRANSPORT"),
        ),
        (httpx.ReadTimeout(""), LlmFailure(message="ReadTimeout", code="TIMEOUT")),
    ],
    ids=["unreached", "too-slow"],
)
async def test_a_list_that_does_not_come_is_a_failure_without_the_key(
    ctx: Context, llm: Llm, error: Exception, failure: LlmFailure
) -> None:
    adapter = connect(ctx, llm, Service(error))

    assert await listed(adapter) == failure


# Stopping


async def test_cancelling_the_reader_closes_the_response(ctx: Context, llm: Llm) -> None:
    body = Body([sse(delta(content="Hel"))], hang=True)
    connect(ctx, llm, Service(streamed(body)))
    call = llm.call(LlmRequest(config=MODEL, messages=HELLO))
    arrived = asyncio.Event()

    async def read() -> None:
        async for chunk in call:
            if isinstance(chunk, TextDelta):
                arrived.set()

    reader = asyncio.ensure_future(read())
    await asyncio.wait_for(arrived.wait(), timeout=5)
    reader.cancel()
    with pytest.raises(asyncio.CancelledError):
        await reader

    assert body.closed
    assert call.message.text == "Hel"
    assert call.finish is None


# The plugin, over a real socket


async def serve(reply: bytes, received: list[bytes]) -> asyncio.Server:
    """Start a server on a free local port that answers one request with ``reply`` and hangs up."""

    async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        head = await reader.readuntil(b"\r\n\r\n")
        length = int(head.lower().split(b"content-length:")[1].split(b"\r\n")[0])
        received.append(head + await reader.readexactly(length))
        writer.write(reply)
        await writer.drain()
        writer.close()

    return await asyncio.start_server(handle, "127.0.0.1", 0)


def http(body: bytes) -> bytes:
    return b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n" + body


async def test_the_plugin_talks_to_a_service_over_a_socket(ctx: Context, unhurried: Llm) -> None:
    received: list[bytes] = []
    server = await serve(http(TEXT_REPLY), received)
    port = server.sockets[0].getsockname()[1]
    fiber = await ctx.plugin(
        openai_chat,
        {"provider": "local", "base_url": f"http://127.0.0.1:{port}/v1", "api_key_env": "TEST_KEY"},
    )

    call = await ask(unhurried, config=CallConfig(provider="local", model="tiny"))
    await fiber.dispose()
    server.close()
    await server.wait_closed()

    assert call.message.text == "Hello"
    assert call.finish == Finish(reason="stop")
    assert received[0].startswith(b"POST /v1/chat/completions HTTP/1.1\r\n")
    assert f"Bearer {KEY}".encode() in received[0]
    assert b'"model":"tiny"' in received[0]
    assert list(ctx.use(LLM).adapters) == []


async def test_a_key_with_a_line_break_after_it_reaches_a_service_without_it(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Over a socket the header is checked by the library that writes it. It refuses a line
    # break, and it names the header that it refuses, with the key in it.
    monkeypatch.setenv("TEST_KEY", f"{KEY}\n")
    received: list[bytes] = []
    server = await serve(http(TEXT_REPLY), received)
    port = server.sockets[0].getsockname()[1]
    await ctx.plugin(
        openai_chat,
        {"provider": "local", "base_url": f"http://127.0.0.1:{port}/v1", "api_key_env": "TEST_KEY"},
    )

    call = await ask(unhurried, config=CallConfig(provider="local", model="tiny"))
    server.close()
    await server.wait_closed()

    assert call.failure is None
    assert call.message.text == "Hello"
    assert f"Bearer {KEY}\r\n".encode() in received[0]


async def test_a_service_that_hangs_up_in_the_middle_is_a_failure(
    ctx: Context, unhurried: Llm
) -> None:
    received: list[bytes] = []
    server = await serve(http(sse(delta(content="Hel"))), received)
    port = server.sockets[0].getsockname()[1]
    await ctx.plugin(openai_chat, {"provider": "local", "base_url": f"http://127.0.0.1:{port}"})

    call = await ask(unhurried, config=CallConfig(provider="local", model="tiny"))
    server.close()
    await server.wait_closed()

    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"
    assert call.message.text == "Hel"


async def test_a_port_where_nothing_listens_is_a_transport_failure(
    ctx: Context, unhurried: Llm
) -> None:
    server = await serve(b"", [])
    port = server.sockets[0].getsockname()[1]
    server.close()
    await server.wait_closed()
    await ctx.plugin(openai_chat, {"provider": "local", "base_url": f"http://127.0.0.1:{port}"})

    call = await ask(unhurried, config=CallConfig(provider="local", model="tiny"))

    # Windows takes two seconds to say that nothing listens, and then says so like the others.
    assert call.failure is not None
    assert call.failure.code == "TRANSPORT"


async def test_the_plugin_takes_the_file_of_keys_by_its_path(
    ctx: Context, unhurried: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("TEST_KEY")
    file = tmp_path / "credentials.json"
    received: list[bytes] = []
    server = await serve(http(TEXT_REPLY), received)
    port = server.sockets[0].getsockname()[1]
    keep(file, "local", STORED, f"http://127.0.0.1:{port}")
    await ctx.plugin(
        openai_chat,
        {
            "provider": "local",
            "base_url": f"http://127.0.0.1:{port}/v1",
            "api_key_env": "TEST_KEY",
            "api_key_file": str(file),
        },
    )

    call = await ask(unhurried, config=CallConfig(provider="local", model="tiny"))
    server.close()
    await server.wait_closed()

    assert call.message.text == "Hello"
    assert f"Bearer {STORED}\r\n".encode() in received[0]


@pytest.mark.parametrize(
    "more",
    [{"max_tokens_field": "max_output_tokens"}, {"api_key_required": "perhaps"}],
    ids=["a name that no service takes", "neither yes nor no"],
)
def test_a_config_that_makes_no_sense_is_refused(more: dict[str, object]) -> None:
    with pytest.raises(ValidationError):
        Config.model_validate({"provider": "local", "base_url": "http://localhost/v1", **more})


def test_the_module_is_a_plugin_that_needs_the_model_service() -> None:
    assert openai_chat.name == "llm-openai-chat"
    assert openai_chat.inject == (LLM,)
    assert openai_chat.Config is Config
