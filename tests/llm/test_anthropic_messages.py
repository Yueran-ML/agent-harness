"""The adapter for Claude, through the Messages API of Anthropic."""

import json
from pathlib import Path
from typing import Any

import httpx
import pytest

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
from columbina.llm.adapters import anthropic_messages
from columbina.llm.adapters.anthropic_messages import AnthropicMessages, Config, build_body
from columbina.llm.credentials import keep
from support.vault import Memory

KEY = "sk-ant-test-0123456789"
GIVEN = "sk-ant-given-0123456789"
STORED = "sk-ant-stored-9876543210"
CONFIG = Config(provider="anthropic", base_url="https://api.anthropic.test", api_key_env="TEST_KEY")
MODEL = CallConfig(provider="anthropic", model="claude-test")
HELLO: tuple[Message, ...] = (UserMessage(content=(TextBlock(text="hello"),)),)
READ = ToolSchema(
    name="read",
    description="Read a file.",
    parameters={"type": "object", "properties": {"path": {"type": "string"}}},
)


def sse(*events: dict[str, Any] | str) -> bytes:
    """The body of a streamed reply: each event named and its data, as Anthropic sends it."""
    written: list[str] = []
    for event in events:
        if isinstance(event, str):
            written.append(f"data: {event}\n\n")
        else:
            written.append(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n")
    return "".join(written).encode()


def started(**usage: int) -> dict[str, Any]:
    counts = {"input_tokens": 10, "output_tokens": 1, **usage}
    return {
        "type": "message_start",
        "message": {"id": "msg_1", "role": "assistant", "content": [], "usage": counts},
    }


def block(index: int, content: dict[str, Any]) -> dict[str, Any]:
    return {"type": "content_block_start", "index": index, "content_block": content}


def delta(index: int, **fields: Any) -> dict[str, Any]:
    return {"type": "content_block_delta", "index": index, "delta": fields}


def stop(index: int) -> dict[str, Any]:
    return {"type": "content_block_stop", "index": index}


def ended(reason: str | None, **usage: int) -> dict[str, Any]:
    return {"type": "message_delta", "delta": {"stop_reason": reason}, "usage": usage}


MESSAGE_STOP = {"type": "message_stop"}
PING = {"type": "ping"}

TEXT_REPLY = sse(
    started(),
    block(0, {"type": "text", "text": ""}),
    PING,
    delta(0, type="text_delta", text="Hel"),
    delta(0, type="text_delta", text="lo"),
    stop(0),
    ended("end_turn", output_tokens=5),
    MESSAGE_STOP,
)


class Service:
    """Stands in for Anthropic: answers each request in turn, and keeps the requests."""

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


def streamed(body: bytes) -> httpx.Response:
    return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=body)


def refused(status: int, error: dict[str, Any] | str, **headers: str) -> httpx.Response:
    if isinstance(error, str):
        return httpx.Response(status, text=error, headers=headers)
    return httpx.Response(status, json={"type": "error", "error": error}, headers=headers)


@pytest.fixture(autouse=True)
def key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TEST_KEY", KEY)


def connect(
    ctx: Context, llm: Llm, service: Service, config: Config = CONFIG, *, secret: str | None = None
) -> AnthropicMessages:
    adapter = AnthropicMessages(config, transport=httpx.MockTransport(service), secret=secret)
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


OFFERED = (READ,)


def request(
    *messages: Message, config: CallConfig = MODEL, tools: tuple[ToolSchema, ...] = ()
) -> LlmRequest:
    return LlmRequest(config=config, messages=messages, tools=tools)


def call(call_id: str) -> AssistantMessage:
    return AssistantMessage(content=(ToolCallBlock(call_id=call_id, name="read", arguments="{}"),))


def result(call_id: str, text: str, *, is_error: bool = False) -> ToolMessage:
    return ToolMessage(call_id=call_id, content=(TextBlock(text=text),), is_error=is_error)


def said(text: str) -> UserMessage:
    return UserMessage(content=(TextBlock(text=text),))


# The request


async def test_a_request_is_a_streamed_message_with_its_version_and_key(
    ctx: Context, llm: Llm
) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    await ask(llm)

    sent = service.requests[0]
    assert sent.method == "POST"
    assert str(sent.url) == "https://api.anthropic.test/v1/messages"
    assert sent.headers["x-api-key"] == KEY
    assert sent.headers["anthropic-version"] == "2023-06-01"
    assert "authorization" not in sent.headers
    assert service.body == {
        "model": "claude-test",
        "max_tokens": 32000,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}],
        "stream": True,
        "thinking": {"type": "adaptive", "display": "summarized"},
        "cache_control": {"type": "ephemeral"},
    }


def test_the_instructions_go_into_system_and_the_tools_are_offered() -> None:
    instructions = SystemMessage(content=(TextBlock(text="Be brief."), TextBlock(text="Be kind.")))
    another = SystemMessage(content=(TextBlock(text="Answer in English."),))

    body = build_body(
        LlmRequest(config=MODEL, messages=(instructions, *HELLO, another), tools=(READ,)), CONFIG
    )

    assert body["system"] == "Be brief.\n\nBe kind.\n\nAnswer in English."
    assert body["messages"] == [{"role": "user", "content": [{"type": "text", "text": "hello"}]}]
    assert body["tools"] == [
        {
            "name": "read",
            "description": "Read a file.",
            "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}},
        }
    ]


def test_empty_instructions_send_no_system() -> None:
    body = build_body(request(SystemMessage(content=(TextBlock(text=""),)), *HELLO), CONFIG)

    assert "system" not in body


@pytest.mark.parametrize(
    ("reasoning", "effort"), [("off", "low"), ("low", "low"), ("high", "high"), ("max", "max")]
)
def test_how_hard_the_model_thinks_is_its_effort(reasoning: Any, effort: str) -> None:
    config = MODEL.model_copy(update={"reasoning": reasoning})

    body = build_body(request(*HELLO, config=config), CONFIG)

    assert body["output_config"] == {"effort": effort}


def test_a_call_without_reasoning_asks_for_no_effort() -> None:
    assert "output_config" not in build_body(request(*HELLO), CONFIG)


def test_the_limit_of_a_call_goes_before_that_of_the_adapter() -> None:
    config = MODEL.model_copy(update={"max_output_tokens": 1000})

    assert build_body(request(*HELLO, config=config), CONFIG)["max_tokens"] == 1000
    assert (
        build_body(request(*HELLO), CONFIG.model_copy(update={"max_output_tokens": 64}))[
            "max_tokens"
        ]
        == 64
    )


@pytest.mark.parametrize(
    ("cache", "control"),
    [("5m", {"type": "ephemeral"}), ("1h", {"type": "ephemeral", "ttl": "1h"}), ("off", None)],
)
def test_the_cache_is_asked_for_as_the_config_says(cache: Any, control: Any) -> None:
    body = build_body(request(*HELLO), CONFIG.model_copy(update={"cache": cache}))

    assert body.get("cache_control") == control


def test_the_thinking_is_shown_as_the_config_says() -> None:
    body = build_body(request(*HELLO), CONFIG.model_copy(update={"thinking_display": "omitted"}))

    assert body["thinking"] == {"type": "adaptive", "display": "omitted"}


def test_options_go_in_last_and_one_that_is_none_takes_out_what_stands_under_its_name() -> None:
    config = MODEL.model_copy(
        update={
            "options": {"thinking": None, "metadata": {"user_id": "u-1"}, "max_tokens": 9},
            "temperature": 0.2,
            "top_p": 0.5,
            "seed": 3,
        }
    )

    body = build_body(request(*HELLO, config=config), CONFIG)

    assert "thinking" not in body
    assert body["metadata"] == {"user_id": "u-1"}
    assert body["max_tokens"] == 9
    # The current models take only their defaults of these, and a seed is no field of the API.
    assert not {"temperature", "top_p", "seed"} & set(body)


THOUGHT = ReasoningBlock(
    text="Look at the file first.", replay_state={"adapter": "anthropic", "signature": "sig-1"}
)


def test_a_reply_goes_back_in_its_order_with_its_thinking_and_the_results_of_its_tools() -> None:
    reply = AssistantMessage(
        content=(
            THOUGHT,
            TextBlock(text="I will read it."),
            ToolCallBlock(call_id="toolu_1", name="read", arguments='{"path": "a.txt"}'),
            ToolCallBlock(call_id="toolu_2", name="read", arguments='{"path": "b.txt"}'),
        )
    )
    results = (
        ToolMessage(call_id="toolu_1", content=(TextBlock(text="alpha"),)),
        ToolMessage(call_id="toolu_2", content=(TextBlock(text="no such file"),), is_error=True),
    )

    body = build_body(request(*HELLO, reply, *results, said("and then?"), tools=OFFERED), CONFIG)

    assert body["messages"] == [
        {"role": "user", "content": [{"type": "text", "text": "hello"}]},
        {
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "Look at the file first.", "signature": "sig-1"},
                {"type": "text", "text": "I will read it."},
                {"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a.txt"}},
                {"type": "tool_use", "id": "toolu_2", "name": "read", "input": {"path": "b.txt"}},
            ],
        },
        {
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "alpha"},
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_2",
                    "content": "no such file",
                    "is_error": True,
                },
                {"type": "text", "text": "and then?"},
            ],
        },
    ]


def test_the_results_of_tools_go_before_what_the_user_said_in_the_same_turn() -> None:
    reply = AssistantMessage(
        content=(ToolCallBlock(call_id="toolu_1", name="read", arguments="{}"),)
    )
    result = ToolMessage(call_id="toolu_1", content=())

    body: dict[str, Any] = build_body(
        request(*HELLO, reply, said("meanwhile"), result, tools=OFFERED), CONFIG
    )

    assert body["messages"][-1] == {
        "role": "user",
        "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1"},
            {"type": "text", "text": "meanwhile"},
        ],
    }


def test_what_one_side_says_in_a_row_is_one_message_and_nothing_empty_is_sent() -> None:
    nothing = AssistantMessage(content=(TextBlock(text=""),))

    body = build_body(request(said("one"), said(""), said("two"), nothing, said("three")), CONFIG)

    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {"type": "text", "text": "one"},
                {"type": "text", "text": "two"},
                {"type": "text", "text": "three"},
            ],
        }
    ]


@pytest.mark.parametrize(
    "reasoning",
    [
        pytest.param(ReasoningBlock(text="mine?"), id="unmarked"),
        pytest.param(
            ReasoningBlock(text="deep", replay_state={"adapter": "deepseek"}), id="of another"
        ),
        pytest.param(
            ReasoningBlock(
                text="theirs", replay_state={"adapter": "claude-elsewhere", "signature": "sig-9"}
            ),
            id="of another with its signature",
        ),
        pytest.param(
            ReasoningBlock(text="cut", replay_state={"adapter": "anthropic", "signature": ""}),
            id="without its signature",
        ),
        pytest.param(
            ReasoningBlock(text="odd", replay_state={"adapter": "anthropic", "signature": 7}),
            id="with no signature that is text",
        ),
    ],
)
def test_reasoning_that_the_service_cannot_read_is_not_sent(reasoning: ReasoningBlock) -> None:
    reply = AssistantMessage(content=(reasoning, TextBlock(text="done")))

    body: dict[str, Any] = build_body(request(*HELLO, reply), CONFIG)

    assert body["messages"][1] == {
        "role": "assistant",
        "content": [{"type": "text", "text": "done"}],
    }


def test_thinking_that_the_service_kept_back_goes_back_as_it_came() -> None:
    hidden = ReasoningBlock(text="", replay_state={"adapter": "anthropic", "redacted": "blob"})
    reply = AssistantMessage(content=(hidden, TextBlock(text="done")))

    body: dict[str, Any] = build_body(request(*HELLO, reply), CONFIG)

    assert body["messages"][1]["content"][0] == {"type": "redacted_thinking", "data": "blob"}


# A request cannot hold NaN, which is no JSON, though Python reads it.
@pytest.mark.parametrize(
    "arguments", ["", "not json", "[1, 2]", '"text"', '{"x": NaN}', '{"x": -Infinity}']
)
def test_arguments_that_are_no_object_go_as_an_empty_one(arguments: str) -> None:
    reply = AssistantMessage(
        content=(ToolCallBlock(call_id="toolu_1", name="read", arguments=arguments),)
    )

    body: dict[str, Any] = build_body(request(*HELLO, reply, tools=OFFERED), CONFIG)

    assert body["messages"][1]["content"][0]["input"] == {}


# A conversation of another service


def test_ids_that_the_service_does_not_take_go_as_ones_that_it_takes_and_once_each() -> None:
    # Moonshot names its calls so, and names calls of different replies alike. Zhipu puts a
    # hyphen in its ids, which an id of a call may not hold either.
    body: dict[str, Any] = build_body(
        request(
            *HELLO,
            call("functions.read:0"),
            result("functions.read:0", "one"),
            call("functions.read:0"),
            result("functions.read:0", "two"),
            call("call_-9"),
            result("call_-9", "three"),
            tools=OFFERED,
        ),
        CONFIG,
    )

    sent = [
        block.get("id") or block.get("tool_use_id")
        for turn in body["messages"][1:]
        for block in turn["content"]
    ]
    assert sent == [
        "functions_read_0",
        "functions_read_0",
        "functions_read_0_2",
        "functions_read_0_2",
        "call__9",
        "call__9",
    ]


def test_calls_of_one_turn_that_have_one_id_are_answered_in_their_order() -> None:
    first = AssistantMessage(
        content=(TextBlock(text="One."), ToolCallBlock(call_id="x", name="read", arguments="{}"))
    )

    body: dict[str, Any] = build_body(
        request(*HELLO, first, call("x"), result("x", "a"), result("x", "b"), tools=OFFERED),
        CONFIG,
    )

    assert [block.get("id") for block in body["messages"][1]["content"]] == [None, "x", "x_2"]
    assert body["messages"][2]["content"] == [
        {"type": "tool_result", "tool_use_id": "x", "content": "a"},
        {"type": "tool_result", "tool_use_id": "x_2", "content": "b"},
    ]


def test_blank_text_is_not_sent() -> None:
    # Some models write a blank line before their calls.
    reply = AssistantMessage(
        content=(
            TextBlock(text="\n\n"),
            ToolCallBlock(call_id="toolu_1", name="read", arguments="{}"),
        )
    )

    body: dict[str, Any] = build_body(
        request(*HELLO, reply, result("toolu_1", "  "), said(" \n"), tools=OFFERED), CONFIG
    )

    assert body["messages"][1:] == [
        {
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "toolu_1", "name": "read", "input": {}}],
        },
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1"}]},
    ]


def test_without_tools_on_offer_calls_and_their_results_are_told_of_in_text() -> None:
    # As in a request for a summary: the service refuses calls where it is offered no tools.
    reply = AssistantMessage(
        content=(
            TextBlock(text="I will read them."),
            ToolCallBlock(call_id="toolu_1", name="read", arguments='{"path": "a.txt"}'),
            ToolCallBlock(call_id="toolu_2", name="read", arguments='{"path": "b.txt"}'),
        )
    )

    body: dict[str, Any] = build_body(
        request(
            *HELLO,
            reply,
            result("toolu_1", "alpha"),
            result("toolu_2", "no such file", is_error=True),
            said("Sum it up."),
        ),
        CONFIG,
    )

    assert body["messages"][1:] == [
        {
            "role": "assistant",
            "content": [
                {"type": "text", "text": "I will read them."},
                {"type": "text", "text": '[A call of read: {"path": "a.txt"}]'},
                {"type": "text", "text": '[A call of read: {"path": "b.txt"}]'},
            ],
        },
        {
            "role": "user",
            "content": [
                {"type": "text", "text": "[The result of a call:]\nalpha"},
                {"type": "text", "text": "[The result of a call, which failed:]\nno such file"},
                {"type": "text", "text": "Sum it up."},
            ],
        },
    ]
    assert "tools" not in body


def test_a_result_whose_call_is_not_in_the_reply_before_it_is_told_of_in_text() -> None:
    both = AssistantMessage(
        content=(
            ToolCallBlock(call_id="toolu_1", name="read", arguments="{}"),
            ToolCallBlock(call_id="toolu_2", name="read", arguments="{}"),
        )
    )
    done = AssistantMessage(content=(TextBlock(text="Done."),))

    # The second call was answered only after another reply.
    body: dict[str, Any] = build_body(
        request(
            *HELLO,
            both,
            result("toolu_1", "first"),
            done,
            result("toolu_2", "late"),
            tools=OFFERED,
        ),
        CONFIG,
    )

    assert body["messages"][-1] == {
        "role": "user",
        "content": [{"type": "text", "text": "[The result of a call:]\nlate"}],
    }


def test_a_conversation_that_begins_with_a_reply_begins_with_a_note_of_the_user() -> None:
    # As a request for a summary of the newer half of a conversation may.
    instructions = SystemMessage(content=(TextBlock(text="Be brief."),))
    reply = AssistantMessage(content=(TextBlock(text="Where was I?"),))

    body: dict[str, Any] = build_body(request(instructions, reply, said("Sum it up.")), CONFIG)

    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {"type": "text", "text": "(The beginning of the conversation is left out here.)"}
            ],
        },
        {"role": "assistant", "content": [{"type": "text", "text": "Where was I?"}]},
        {"role": "user", "content": [{"type": "text", "text": "Sum it up."}]},
    ]


# The reply


async def test_text_arrives_with_the_token_counts(ctx: Context, llm: Llm) -> None:
    reply = sse(
        started(input_tokens=12, cache_read_input_tokens=300, cache_creation_input_tokens=40),
        block(0, {"type": "text", "text": "He"}),
        delta(0, type="text_delta", text="llo"),
        delta(0, type="text_delta", text=""),
        stop(0),
        ended("end_turn", output_tokens=7),
        MESSAGE_STOP,
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.chunks == [
        BlockStart(index=0, kind="text"),
        TextDelta(index=0, text="He"),
        TextDelta(index=0, text="llo"),
        BlockEnd(index=0),
        UsageChunk(
            usage=Usage(
                input_tokens=12, output_tokens=7, cache_read_tokens=300, cache_write_tokens=40
            )
        ),
        Finish(reason="stop"),
    ]
    assert call.message == AssistantMessage(content=(TextBlock(text="Hello"),))


async def test_thinking_arrives_with_its_signature_and_goes_back_with_it(
    ctx: Context, llm: Llm
) -> None:
    reply = sse(
        started(),
        block(0, {"type": "thinking", "thinking": "", "signature": ""}),
        delta(0, type="thinking_delta", thinking="Think "),
        delta(0, type="thinking_delta", thinking=""),
        delta(0, type="thinking_delta", thinking="first."),
        delta(0, type="signature_delta", signature="sig-2"),
        delta(0, type="signature_delta"),
        stop(0),
        block(1, {"type": "text", "text": ""}),
        delta(1, type="text_delta", text="Done."),
        stop(1),
        ended("end_turn", output_tokens=9),
        MESSAGE_STOP,
    )
    service = Service(streamed(reply), streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    first = await ask(llm)
    await ask(llm, (*HELLO, first.message, said("more")))

    assert first.chunks[:7] == [
        BlockStart(index=0, kind="reasoning"),
        ReasoningDelta(index=0, text="Think "),
        ReasoningDelta(index=0, text="first."),
        BlockEnd(index=0, replay_state={"adapter": "anthropic", "signature": "sig-2"}),
        BlockStart(index=1, kind="text"),
        TextDelta(index=1, text="Done."),
        BlockEnd(index=1),
    ]
    assert service.body["messages"][1]["content"] == [
        {"type": "thinking", "thinking": "Think first.", "signature": "sig-2"},
        {"type": "text", "text": "Done."},
    ]


async def test_thinking_that_is_shown_only_by_its_signature_goes_back(
    ctx: Context, llm: Llm
) -> None:
    reply = sse(
        started(),
        block(0, {"type": "thinking", "thinking": "", "signature": "sig-whole"}),
        stop(0),
        block(1, {"type": "redacted_thinking", "data": "kept-back"}),
        stop(1),
        block(2, {"type": "text", "text": "Hi."}),
        stop(2),
        ended("end_turn"),
    )
    service = Service(streamed(reply), streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    first = await ask(llm)
    await ask(llm, (*HELLO, first.message, said("more")))

    assert service.body["messages"][1]["content"] == [
        {"type": "thinking", "thinking": "", "signature": "sig-whole"},
        {"type": "redacted_thinking", "data": "kept-back"},
        {"type": "text", "text": "Hi."},
    ]


async def test_thinking_that_arrives_whole_at_its_start_goes_back_whole(
    ctx: Context, llm: Llm
) -> None:
    # As a service in between may send it.
    reply = sse(
        started(),
        block(0, {"type": "thinking", "thinking": "All at once.", "signature": "sig-3"}),
        stop(0),
        ended("end_turn"),
    )
    service = Service(streamed(reply), streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    first = await ask(llm)
    await ask(llm, (*HELLO, first.message, said("more")))

    assert first.chunks[:2] == [
        BlockStart(index=0, kind="reasoning"),
        ReasoningDelta(index=0, text="All at once."),
    ]
    assert service.body["messages"][1]["content"] == [
        {"type": "thinking", "thinking": "All at once.", "signature": "sig-3"},
    ]


async def test_a_signature_that_a_block_begins_with_gives_way_to_the_one_of_a_delta(
    ctx: Context, llm: Llm
) -> None:
    # As a service in between may send it: at the beginning of the block, and again at its end.
    reply = sse(
        started(),
        block(0, {"type": "thinking", "thinking": "", "signature": "sig-start"}),
        delta(0, type="signature_delta", signature="sig-final"),
        stop(0),
        block(1, {"type": "text", "text": "Hi."}),
        stop(1),
        ended("end_turn"),
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.chunks[1] == BlockEnd(
        index=0, replay_state={"adapter": "anthropic", "signature": "sig-final"}
    )


async def test_thinking_is_marked_with_the_name_of_the_service_and_goes_back_to_it(
    ctx: Context, llm: Llm
) -> None:
    reply = sse(
        started(),
        block(0, {"type": "thinking", "thinking": "Hm.", "signature": "sig-4"}),
        stop(0),
        ended("end_turn"),
    )
    service = Service(streamed(reply), streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"provider": "claude"}))
    claude = CallConfig(provider="claude", model="claude-test")

    first = await ask(llm, config=claude)
    await ask(llm, (*HELLO, first.message, said("more")), config=claude)

    assert BlockEnd(index=0, replay_state={"adapter": "claude", "signature": "sig-4"}) in (
        first.chunks
    )
    assert service.body["messages"][1]["content"] == [
        {"type": "thinking", "thinking": "Hm.", "signature": "sig-4"},
    ]


async def test_the_arguments_of_a_call_that_arrive_whole_at_its_start_are_kept(
    ctx: Context, llm: Llm
) -> None:
    # As a service in between may send them.
    reply = sse(
        started(),
        block(0, {"type": "tool_use", "id": "toolu_5", "name": "read", "input": {"path": "a"}}),
        stop(0),
        ended("tool_use"),
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm, tools=(READ,))

    assert call.message == AssistantMessage(
        content=(ToolCallBlock(call_id="toolu_5", name="read", arguments='{"path": "a"}'),)
    )


async def test_a_tool_call_arrives_in_pieces(ctx: Context, llm: Llm) -> None:
    reply = sse(
        started(),
        block(0, {"type": "tool_use", "id": "toolu_9", "name": "read", "input": {}}),
        delta(0, type="input_json_delta", partial_json=""),
        delta(0, type="input_json_delta", partial_json='{"path": '),
        delta(0, type="input_json_delta", partial_json='"a.txt"}'),
        stop(0),
        ended("tool_use", output_tokens=20),
        MESSAGE_STOP,
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm, tools=(READ,))

    assert call.chunks[:4] == [
        BlockStart(index=0, kind="tool-call", call_id="toolu_9", name="read"),
        ToolCallDelta(index=0, arguments='{"path": '),
        ToolCallDelta(index=0, arguments='"a.txt"}'),
        BlockEnd(index=0),
    ]
    assert call.finish == Finish(reason="tool-calls")
    assert call.message.tool_calls == (
        ToolCallBlock(call_id="toolu_9", name="read", arguments='{"path": "a.txt"}'),
    )


@pytest.mark.parametrize(
    ("reason", "finish"),
    [
        ("end_turn", Finish(reason="stop")),
        ("stop_sequence", Finish(reason="stop")),
        ("pause_turn", Finish(reason="stop")),
        ("something new", Finish(reason="stop")),
        ("max_tokens", Finish(reason="max-tokens")),
        (
            "refusal",
            Finish(
                reason="error",
                failure=LlmFailure(message="the model declined to answer", code="CONTENT_FILTER"),
            ),
        ),
        (
            "model_context_window_exceeded",
            Finish(
                reason="error",
                failure=LlmFailure(
                    message="the reply did not fit into the window of the model",
                    code="CONTEXT_WINDOW_EXCEEDED",
                ),
            ),
        ),
    ],
)
async def test_why_the_reply_stopped_is_how_it_finishes(
    ctx: Context, llm: Llm, reason: str, finish: Finish
) -> None:
    reply = sse(
        started(),
        block(0, {"type": "text", "text": "partly"}),
        stop(0),
        ended(reason, output_tokens=3),
        MESSAGE_STOP,
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.finish == finish


async def test_what_the_adapter_does_not_know_is_passed_over(ctx: Context, llm: Llm) -> None:
    reply = sse(
        started(),
        {"type": "something_new"},
        block(0, {"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search"}),
        delta(0, type="input_json_delta", partial_json='{"query": "x"}'),
        stop(0),
        block(1, {"type": "text", "text": "A"}),
        delta(1, type="citations_delta", citation={}),
        delta(1, type="thinking_delta", thinking="out of place"),
        delta(1, type="text_delta", text=""),
        delta(1, type="text_delta", text="B"),
        {"type": "content_block_delta", "index": 1, "delta": "no object"},
        delta(5, type="text_delta", text="nowhere"),
        stop(1),
        block(2, {"type": "tool_use", "id": "toolu_3", "name": "read"}),
        delta(2, type="text_delta", text="no text here"),
        delta(2, type="signature_delta", signature="no signature here"),
        stop(2),
        ended("tool_use"),
        MESSAGE_STOP,
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm, tools=(READ,))

    assert call.message == AssistantMessage(
        content=(TextBlock(text="AB"), ToolCallBlock(call_id="toolu_3", name="read", arguments=""))
    )


async def test_counts_that_the_end_of_a_reply_gives_go_over_those_of_its_beginning(
    ctx: Context, llm: Llm
) -> None:
    reply = sse(
        started(input_tokens=5, cache_read_input_tokens=1),
        block(0, {"type": "text", "text": "x"}),
        stop(0),
        {
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 4, "input_tokens": 6, "cache_creation_input_tokens": 2},
        },
        {"type": "message_delta", "delta": "no object", "usage": "no object"},
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.usage == Usage(
        input_tokens=6, output_tokens=4, cache_read_tokens=1, cache_write_tokens=2
    )


async def test_a_reply_that_stops_without_saying_why_is_cut_off(ctx: Context, llm: Llm) -> None:
    # An end whose reason is null says no more than an end that gives none.
    reply = sse(started(), block(0, {"type": "text", "text": "Hal"}), stop(0), ended(None))
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"
    assert UsageChunk(usage=Usage(input_tokens=10, output_tokens=1)) not in call.chunks


@pytest.mark.parametrize(
    ("error", "failure"),
    [
        (
            {"type": "overloaded_error", "message": "Overloaded"},
            LlmFailure(message="Overloaded", code="SERVER"),
        ),
        (
            {"type": "rate_limit_error", "message": "Slow down"},
            LlmFailure(message="Slow down", code="RATE_LIMIT"),
        ),
        (
            {"type": "invalid_request_error", "message": "prompt is too long: 1000001 tokens"},
            LlmFailure(
                message="prompt is too long: 1000001 tokens", code="CONTEXT_WINDOW_EXCEEDED"
            ),
        ),
        (
            {"type": "invalid_request_error", "message": "messages: field required"},
            LlmFailure(message="messages: field required", code="INVALID_REQUEST"),
        ),
        (
            {"type": "not_found_error", "message": "model: claude-none"},
            LlmFailure(message="model: claude-none", code="INVALID_REQUEST"),
        ),
        (
            {"type": "request_too_large", "message": "too large"},
            LlmFailure(message="too large", code="INVALID_REQUEST"),
        ),
        (
            {"type": "brand_new_error", "message": "What is this"},
            LlmFailure(message="What is this", code="SERVER"),
        ),
        ("no object", LlmFailure(message="the service reported an error", code="SERVER")),
    ],
)
async def test_an_error_in_the_middle_of_a_stream_is_a_failure(
    ctx: Context, llm: Llm, error: object, failure: LlmFailure
) -> None:
    reply = sse(
        started(), block(0, {"type": "text", "text": "Hal"}), {"type": "error", "error": error}
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.failure == failure


@pytest.mark.parametrize(
    ("data", "problem"),
    [
        ("not json", "Expecting value"),
        ("[1, 2]", "it is no object"),
        (json.dumps({"type": "content_block_stop"}), "a block has no place"),
        (json.dumps({"type": "content_block_stop", "index": True}), "a block has no place"),
        (json.dumps({"type": "content_block_stop", "index": -1}), "a block has no place"),
        (
            json.dumps({"type": "content_block_start", "index": 0}),
            "block 0 begins without its content",
        ),
        (
            json.dumps(block(0, {"type": "tool_use", "name": "read"})),
            "the tool call of block 0 has no id or no name",
        ),
        (
            json.dumps(block(0, {"type": "tool_use", "id": "", "name": "read"})),
            "the tool call of block 0 has no id or no name",
        ),
        (
            json.dumps(block(0, {"type": "tool_use", "id": "toolu_1", "name": ""})),
            "the tool call of block 0 has no id or no name",
        ),
        (
            json.dumps({"type": "message_start", "message": {"usage": {"input_tokens": "many"}}}),
            "its token counts are no numbers",
        ),
    ],
)
async def test_an_event_that_cannot_be_read_is_a_malformed_reply(
    ctx: Context, llm: Llm, data: str, problem: str
) -> None:
    connect(ctx, llm, Service(streamed(sse(data))))

    call = await ask(llm)

    assert call.failure is not None
    assert call.failure.code == "MALFORMED_RESPONSE"
    assert call.failure.message.startswith("the service sent an event that cannot be read: ")
    assert problem in call.failure.message


async def test_a_message_start_without_its_message_counts_nothing(ctx: Context, llm: Llm) -> None:
    reply = sse(
        {"type": "message_start", "message": "no object"},
        block(0, {"type": "text", "text": "a"}),
        stop(0),
        ended("end_turn"),
    )
    connect(ctx, llm, Service(streamed(reply)))

    call = await ask(llm)

    assert call.usage == Usage()


# What went wrong


@pytest.mark.parametrize(
    ("status", "error", "code"),
    [
        (401, {"type": "authentication_error", "message": "invalid x-api-key"}, "AUTH"),
        (403, {"type": "permission_error", "message": "not allowed"}, "AUTH"),
        (402, {"type": "billing_error", "message": "pay first"}, "QUOTA"),
        (
            400,
            {
                "type": "invalid_request_error",
                "message": "Your credit balance is too low to access the Anthropic API.",
            },
            "QUOTA",
        ),
        (
            400,
            {"type": "invalid_request_error", "message": "Prompt is too long: 1M"},
            "CONTEXT_WINDOW_EXCEEDED",
        ),
        (
            400,
            {"type": "invalid_request_error", "message": "messages: field required"},
            "INVALID_REQUEST",
        ),
        (404, {"type": "not_found_error", "message": "model: claude-none"}, "INVALID_REQUEST"),
        (413, {"type": "request_too_large", "message": "too large"}, "INVALID_REQUEST"),
        (429, {"type": "rate_limit_error", "message": "Rate limited"}, "RATE_LIMIT"),
        (500, {"type": "api_error", "message": "Internal"}, "SERVER"),
        (529, {"type": "overloaded_error", "message": "Overloaded"}, "SERVER"),
        (504, {"type": "timeout_error", "message": "took too long"}, "TIMEOUT"),
        (500, {"type": "something", "message": "Internal"}, "SERVER"),
        (418, {"type": "something", "message": "I am a teapot"}, "HTTP_418"),
    ],
)
async def test_a_refusal_of_the_service_is_a_failure_with_a_stable_code(
    ctx: Context, llm: Llm, status: int, error: dict[str, Any], code: str
) -> None:
    answer = refused(status, error, **{"request-id": "req_7", "retry-after": "2"})
    connect(ctx, llm, Service(answer))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message=error["message"], code=code, status=status, retry_after_ms=2000, request_id="req_7"
    )
    assert call.chunks == [call.finish]


async def test_a_refusal_that_is_no_json_keeps_its_text(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(refused(502, "<html>Bad Gateway</html>")))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="<html>Bad Gateway</html>", code="SERVER", status=502)


@pytest.mark.parametrize(
    ("status", "code"),
    [
        (400, "INVALID_REQUEST"),
        (401, "AUTH"),
        (402, "QUOTA"),
        (403, "AUTH"),
        (404, "INVALID_REQUEST"),
        (408, "TIMEOUT"),
        (413, "INVALID_REQUEST"),
        (422, "INVALID_REQUEST"),
        (429, "RATE_LIMIT"),
    ],
)
async def test_a_refusal_that_names_no_kind_is_named_by_its_status(
    ctx: Context, llm: Llm, status: int, code: str
) -> None:
    # As a proxy in between may answer.
    connect(ctx, llm, Service(refused(status, "<html>No</html>")))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="<html>No</html>", code=code, status=status)


async def test_a_refusal_without_a_body_is_named_by_its_status(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(httpx.Response(529)))

    call = await ask(llm)

    assert call.failure == LlmFailure(message="HTTP 529 ", code="SERVER", status=529)


async def test_the_key_is_cut_out_of_what_the_service_and_the_connection_say(
    ctx: Context, llm: Llm
) -> None:
    echo = refused(401, {"type": "authentication_error", "message": f"no good: {KEY}"})
    connect(ctx, llm, Service(echo, httpx.ConnectError(f"refused with {KEY}")))

    first = await ask(llm)
    second = await ask(llm)

    assert first.failure is not None
    assert first.failure.message == "no good: [key]"
    assert second.failure == LlmFailure(
        message="ConnectError: refused with [key]", code="TRANSPORT"
    )


async def test_a_failure_in_the_middle_of_a_reply_holds_neither_the_key_nor_what_was_caught(
    ctx: Context, llm: Llm
) -> None:
    reply = sse(started(), {"type": "error", "error": {"type": "api_error", "message": f"{KEY}?"}})
    adapter = connect(ctx, llm, Service(streamed(reply)))

    with pytest.raises(LlmError) as raised:
        async for _ in adapter.stream(request(*HELLO)):
            pass

    assert raised.value.failure == LlmFailure(message="[key]?", code="SERVER")
    assert raised.value.__context__ is None


async def test_without_a_key_nothing_is_asked(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service()
    connect(ctx, llm, service)

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message="the environment variable TEST_KEY is not set", code="AUTH"
    )
    assert service.requests == []


async def test_a_service_that_asks_for_no_key_is_sent_none(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"api_key_required": False}))

    call = await ask(llm)

    assert call.failure is None
    assert "x-api-key" not in service.requests[0].headers


async def test_a_stored_key_is_sent_where_the_variable_holds_none(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    Memory().install()
    monkeypatch.delenv("TEST_KEY")
    file = tmp_path / "credentials.json"
    keep(file, "anthropic", STORED, "https://api.anthropic.test")
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"api_key_file": file}))

    call = await ask(llm)

    assert call.failure is None
    assert service.requests[0].headers["x-api-key"] == STORED


async def test_further_headers_go_with_every_request(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service, CONFIG.model_copy(update={"headers": {"anthropic-beta": "x-1"}}))

    await ask(llm)

    assert service.requests[0].headers["anthropic-beta"] == "x-1"


async def test_the_version_of_the_api_is_the_one_that_the_config_names(
    ctx: Context, llm: Llm
) -> None:
    service = Service(streamed(TEXT_REPLY), listing("claude-a"))
    adapter = connect(ctx, llm, service, CONFIG.model_copy(update={"version": "2099-01-01"}))

    await ask(llm)
    await adapter.models()

    assert [sent.headers["anthropic-version"] for sent in service.requests] == [
        "2099-01-01",
        "2099-01-01",
    ]


# The list of models


def listing(*ids: str, more: str | None = None) -> httpx.Response:
    page: dict[str, Any] = {"data": [{"type": "model", "id": each} for each in ids]}
    if more is not None:
        page.update(has_more=True, last_id=more)
    return httpx.Response(200, json=page)


async def test_the_models_are_listed_page_by_page_with_the_version_and_the_key(
    ctx: Context, llm: Llm
) -> None:
    service = Service(listing("claude-a", "claude-b", more="claude-b"), listing("claude-c"))
    adapter = connect(ctx, llm, service)

    assert await adapter.models() == ["claude-a", "claude-b", "claude-c"]

    first, second = service.requests
    assert str(first.url) == "https://api.anthropic.test/v1/models?limit=1000"
    assert str(second.url) == "https://api.anthropic.test/v1/models?limit=1000&after_id=claude-b"
    assert first.headers["anthropic-version"] == "2023-06-01"
    assert first.headers["x-api-key"] == KEY
    timeout = first.extensions["timeout"]
    assert (timeout["read"], timeout["connect"]) == (30.0, CONFIG.connect_timeout_seconds)


async def test_a_page_that_says_more_without_saying_from_where_is_the_last(
    ctx: Context, llm: Llm
) -> None:
    page = httpx.Response(200, json={"data": [{"id": "claude-a"}], "has_more": True})
    service = Service(page)
    adapter = connect(ctx, llm, service)

    assert await adapter.models() == ["claude-a"]
    assert len(service.requests) == 1


async def test_no_more_than_ten_pages_are_asked_for(ctx: Context, llm: Llm) -> None:
    service = Service(*(listing(f"claude-{n}", more=f"claude-{n}") for n in range(11)))
    adapter = connect(ctx, llm, service)

    assert len(await adapter.models()) == 10
    assert len(service.requests) == 10


async def test_a_key_that_is_given_lists_the_models(ctx: Context, llm: Llm) -> None:
    service = Service(listing("claude-a"))
    adapter = connect(ctx, llm, service, secret=f"  {GIVEN}\n")

    assert await adapter.models() == ["claude-a"]
    assert service.requests[0].headers["x-api-key"] == GIVEN


async def test_a_refusal_of_the_list_says_why_without_the_key(ctx: Context, llm: Llm) -> None:
    echo = refused(401, {"type": "authentication_error", "message": f"bad key {KEY}"})
    adapter = connect(ctx, llm, Service(echo))

    with pytest.raises(LlmError) as raised:
        await adapter.models()

    assert raised.value.failure == LlmFailure(message="bad key [key]", code="AUTH", status=401)
    assert raised.value.__context__ is None


@pytest.mark.parametrize(
    ("answer", "problem"),
    [
        (httpx.Response(200, text="<html>Welcome</html>"), "Invalid JSON"),
        (httpx.Response(200, json={"models": ["claude-a"]}), "Field required"),
    ],
)
async def test_a_list_that_cannot_be_read_is_a_malformed_reply(
    ctx: Context, llm: Llm, answer: httpx.Response, problem: str
) -> None:
    adapter = connect(ctx, llm, Service(answer))

    with pytest.raises(LlmError) as raised:
        await adapter.models()

    failure = raised.value.failure
    assert failure.code == "MALFORMED_RESPONSE"
    assert failure.message.startswith("the service sent a list of models that cannot be read: ")
    assert problem in failure.message


async def test_a_list_that_cannot_be_reached_is_a_failure_of_the_connection(
    ctx: Context, llm: Llm
) -> None:
    adapter = connect(ctx, llm, Service(httpx.ConnectTimeout("slow")))

    with pytest.raises(LlmError) as raised:
        await adapter.models()

    assert raised.value.failure == LlmFailure(message="ConnectTimeout: slow", code="TIMEOUT")


# The plugin


async def test_the_plugin_registers_an_adapter_that_thinks_in_the_manner_of_anthropic(
    ctx: Context,
) -> None:
    await ctx.plugin(columbina.llm, {})
    await ctx.plugin(anthropic_messages, {"provider": "claude", "base_url": "https://x.test"})

    adapter = ctx.use(LLM).adapters.get("claude")

    assert isinstance(adapter, AnthropicMessages)
    assert adapter.reasoning_style == "anthropic"
    assert anthropic_messages.name == "llm-anthropic-messages"
    assert anthropic_messages.inject == (LLM,)


def clients_made(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, Any]]:
    """Note what every HTTP client of the adapter is made with."""
    made: list[dict[str, Any]] = []
    real = httpx.AsyncClient

    def client(**given: Any) -> httpx.AsyncClient:
        made.append(given)
        return real(**given)

    monkeypatch.setattr(httpx, "AsyncClient", client)
    return made


def test_the_requests_go_through_the_proxy_of_the_system(monkeypatch: pytest.MonkeyPatch) -> None:
    asked: list[str] = []

    def proxy_for(url: str, env: Any) -> str:
        asked.append(url)
        return "http://proxy.test:8080"

    monkeypatch.setattr(anthropic_messages, "proxy_for", proxy_for)
    made = clients_made(monkeypatch)

    AnthropicMessages(CONFIG)
    AnthropicMessages(CONFIG, transport=httpx.MockTransport(Service()))

    assert asked == ["https://api.anthropic.test"]
    assert [client["proxy"] for client in made] == ["http://proxy.test:8080", None]
