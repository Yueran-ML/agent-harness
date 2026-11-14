"""The adapter for the Responses protocol of OpenAI."""

import json
import traceback
from typing import Any

import httpx
import pytest
from pydantic import JsonValue, ValidationError

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
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    Usage,
    UserMessage,
)
from columbina.llm.adapters import openai_responses
from columbina.llm.adapters.openai_responses import Config, OpenAIResponses, build_body
from columbina.llm.types import Picture

KEY = "sk-test-0123456789"
CONFIG = Config(provider="openai", base_url="https://api.example.test/v1", api_key_env="TEST_KEY")
MODEL = CallConfig(provider="openai", model="gpt-test")
HELLO: tuple[Message, ...] = (UserMessage(content=(TextBlock(text="hello"),)),)
READ = ToolSchema(
    name="read",
    description="Read a file.",
    parameters={"type": "object", "properties": {"path": {"type": "string"}}},
)
SEALED: dict[str, JsonValue] = {
    "adapter": "openai",
    "model": "gpt-test",
    "id": "rs_1",
    "encrypted_content": "gAAAA-sealed",
    "summary": ["First.", "Then."],
}
"""What the adapter keeps of the reasoning of a reply, to send it back."""


def event(kind: str, **fields: Any) -> dict[str, Any]:
    """One streamed event."""
    return {"type": kind, **fields}


def added(at: int, **item: Any) -> dict[str, Any]:
    return event("response.output_item.added", output_index=at, item=item)


def done(at: int, **item: Any) -> dict[str, Any]:
    return event("response.output_item.done", output_index=at, item=item)


def text(at: int, piece: str) -> dict[str, Any]:
    return event("response.output_text.delta", output_index=at, content_index=0, delta=piece)


def thought(at: int, part: int, piece: str) -> dict[str, Any]:
    return event(
        "response.reasoning_summary_text.delta", output_index=at, summary_index=part, delta=piece
    )


def arguments(at: int, piece: str) -> dict[str, Any]:
    return event("response.function_call_arguments.delta", output_index=at, delta=piece)


def completed(**response: Any) -> dict[str, Any]:
    return event("response.completed", response={"status": "completed", **response})


def incomplete(reason: str, **response: Any) -> dict[str, Any]:
    return event(
        "response.incomplete",
        response={"status": "incomplete", "incomplete_details": {"reason": reason}, **response},
    )


def sse(*events: dict[str, Any] | str) -> bytes:
    """The body of a streamed reply as the service writes it: the kind of an event, its data."""
    written: list[str] = []
    for one in events:
        if isinstance(one, str):
            written.append(f"data: {one}\n\n")
        else:
            written.append(f"event: {one['type']}\ndata: {json.dumps(one)}\n\n")
    return "".join(written).encode()


TEXT_REPLY = sse(
    event("response.created", response={"status": "in_progress"}),
    added(0, type="message", id="msg_1", role="assistant", content=[]),
    text(0, "Hel"),
    text(0, "lo"),
    done(0, type="message", id="msg_1", content=[{"type": "output_text", "text": "Hello"}]),
    completed(),
)


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


def streamed(body: bytes, **headers: str) -> httpx.Response:
    return httpx.Response(
        200, headers={"content-type": "text/event-stream", **headers}, content=body
    )


def refused(status: int, error: dict[str, Any] | str, **headers: str) -> httpx.Response:
    if isinstance(error, str):
        return httpx.Response(status, text=error, headers=headers)
    return httpx.Response(status, json={"error": error}, headers=headers)


@pytest.fixture(autouse=True)
def key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TEST_KEY", KEY)


def connect(ctx: Context, llm: Llm, service: Service, config: Config = CONFIG) -> OpenAIResponses:
    adapter = OpenAIResponses(config, transport=httpx.MockTransport(service))
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


async def replied(ctx: Context, llm: Llm, *events: dict[str, Any] | str) -> LlmCall:
    """Ask a service that answers with these events, and return the call."""
    connect(ctx, llm, Service(streamed(sse(*events))))
    return await ask(llm)


def body_of(
    messages: tuple[Message, ...] = HELLO,
    *,
    config: Config = CONFIG,
    call: CallConfig = MODEL,
    tools: tuple[ToolSchema, ...] = (),
) -> dict[str, Any]:
    return build_body(LlmRequest(config=call, messages=messages, tools=tools), config)


# The request


async def test_a_request_is_a_streamed_response_that_is_not_stored(ctx: Context, llm: Llm) -> None:
    service = Service(streamed(TEXT_REPLY))
    connect(ctx, llm, service)

    await ask(llm)

    request = service.requests[0]
    assert request.method == "POST"
    assert str(request.url) == "https://api.example.test/v1/responses"
    assert request.headers["authorization"] == f"Bearer {KEY}"
    assert service.body == {
        "model": "gpt-test",
        "input": [{"role": "user", "content": "hello"}],
        "stream": True,
        "store": False,
        "include": ["reasoning.encrypted_content"],
        "reasoning": {"summary": "auto"},
    }


def test_a_tool_is_sent_flat_and_as_its_schema_is() -> None:
    assert body_of(tools=(READ,))["tools"] == [
        {
            "type": "function",
            "name": "read",
            "description": "Read a file.",
            "parameters": {"type": "object", "properties": {"path": {"type": "string"}}},
            "strict": False,
        }
    ]
    assert "tools" not in body_of()


def test_a_conversation_goes_as_the_items_of_its_replies_in_their_order() -> None:
    messages: tuple[Message, ...] = (
        SystemMessage(content=(TextBlock(text="Be brief."), TextBlock(text="Be kind."))),
        *HELLO,
        AssistantMessage(
            content=(
                ReasoningBlock(text="First.\n\nThen.", replay_state=SEALED),
                TextBlock(text="I look."),
                TextBlock(text=""),
                ToolCallBlock(call_id="call_1", name="read", arguments='{"path": "a"}'),
                ToolCallBlock(call_id="call_2", name="read", arguments=""),
            )
        ),
        ToolMessage(call_id="call_1", content=(TextBlock(text="one"), TextBlock(text="two"))),
        ToolMessage(call_id="call_2", content=(TextBlock(text="no such file"),), is_error=True),
    )

    assert body_of(messages)["input"] == [
        {"role": "system", "content": "Be brief.\n\nBe kind."},
        {"role": "user", "content": "hello"},
        {
            "type": "reasoning",
            "id": "rs_1",
            "summary": [
                {"type": "summary_text", "text": "First."},
                {"type": "summary_text", "text": "Then."},
            ],
            "encrypted_content": "gAAAA-sealed",
        },
        {"role": "assistant", "content": "I look."},
        {
            "type": "function_call",
            "call_id": "call_1",
            "name": "read",
            "arguments": '{"path": "a"}',
        },
        {"type": "function_call", "call_id": "call_2", "name": "read", "arguments": "{}"},
        {"type": "function_call_output", "call_id": "call_1", "output": "one\n\ntwo"},
        {"type": "function_call_output", "call_id": "call_2", "output": "no such file"},
    ]


@pytest.mark.parametrize(
    "state",
    [
        None,
        {**SEALED, "adapter": "deepseek"},
        {**SEALED, "model": "gpt-other"},
        {**SEALED, "encrypted_content": None},
        {**SEALED, "encrypted_content": ""},
        {**SEALED, "id": None},
    ],
    ids=[
        "none kept",
        "of another adapter",
        "of another model",
        "not sealed",
        "sealed empty",
        "without its id",
    ],
)
def test_reasoning_goes_back_only_to_whom_it_came_from_and_as_the_service_reads_it(
    state: dict[str, Any] | None,
) -> None:
    reply = AssistantMessage(
        content=(ReasoningBlock(text="Hm.", replay_state=state), TextBlock(text="Hello"))
    )

    assert body_of((*HELLO, reply))["input"] == [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "Hello"},
    ]


def test_reasoning_that_nothing_follows_does_not_go_back() -> None:
    cut_short = AssistantMessage(
        content=(TextBlock(text="Hello"), ReasoningBlock(text="Hm.", replay_state=SEALED))
    )
    summary_unread = AssistantMessage(
        content=(
            ReasoningBlock(text="Hm.", replay_state={**SEALED, "summary": "First."}),
            TextBlock(text="Again"),
        )
    )

    assert body_of((*HELLO, cut_short, summary_unread))["input"] == [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "Hello"},
        {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "gAAAA-sealed"},
        {"role": "assistant", "content": "Again"},
    ]


def test_the_pictures_of_a_result_go_in_the_result() -> None:
    png = Picture(media_type="image/png", data="AAAA")
    jpeg = Picture(media_type="image/jpeg", data="BBBB")
    shown = ToolMessage(
        call_id="call_1", content=(TextBlock(text="a chart"),), pictures=(png, jpeg)
    )
    wordless = ToolMessage(call_id="call_2", content=(), pictures=(png,))
    seeing = CONFIG.model_copy(update={"pictures": True})

    assert body_of((shown, wordless), config=seeing)["input"] == [
        {
            "type": "function_call_output",
            "call_id": "call_1",
            "output": [
                {"type": "input_text", "text": "a chart"},
                {"type": "input_image", "image_url": "data:image/png;base64,AAAA"},
                {"type": "input_image", "image_url": "data:image/jpeg;base64,BBBB"},
            ],
        },
        {
            "type": "function_call_output",
            "call_id": "call_2",
            "output": [{"type": "input_image", "image_url": "data:image/png;base64,AAAA"}],
        },
    ]
    # A model that takes no pictures is told that there was one.
    assert body_of((shown,))["input"] == [
        {
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "a chart\n(A picture was given back here, and this model takes no pictures.)",
        }
    ]


def test_what_a_call_sets_is_sent_and_its_options_last() -> None:
    call = MODEL.model_copy(
        update={
            "max_output_tokens": 512,
            "temperature": 0.5,
            "top_p": 0.9,
            "seed": 7,
            "options": {"temperature": 0.1, "prompt_cache_key": "k"},
        }
    )

    body = body_of(call=call)

    assert body["max_output_tokens"] == 512
    assert body["temperature"] == 0.1
    assert body["top_p"] == 0.9
    assert body["prompt_cache_key"] == "k"
    # The protocol has no seed.
    assert "seed" not in body


@pytest.mark.parametrize(
    ("asked", "sent"),
    [
        ("low", {"summary": "auto", "effort": "low"}),
        ("high", {"summary": "auto", "effort": "high"}),
        ("max", {"summary": "auto", "effort": "xhigh"}),
        ("off", {"summary": "auto"}),
        (None, {"summary": "auto"}),
    ],
)
def test_the_effort_that_a_call_asks_for_is_sent_beside_the_summary(
    asked: str | None, sent: dict[str, str]
) -> None:
    assert body_of(call=MODEL.model_copy(update={"reasoning": asked}))["reasoning"] == sent


def test_the_effort_is_written_over_what_the_options_say_of_it() -> None:
    call = MODEL.model_copy(
        update={"reasoning": "high", "options": {"reasoning": {"effort": "low", "mode": "pro"}}}
    )
    assert body_of(call=call)["reasoning"] == {"effort": "high", "mode": "pro"}
    # Options that say something else of the reasoning than an object are not kept beside it.
    call = MODEL.model_copy(update={"reasoning": "low", "options": {"reasoning": None}})
    assert body_of(call=call)["reasoning"] == {"effort": "low"}


def test_a_service_that_is_told_nothing_of_reasoning_is_sent_nothing_of_it() -> None:
    quiet = CONFIG.model_copy(update={"reasoning_style": "none", "summary": None})

    assert "reasoning" not in body_of(
        config=quiet, call=MODEL.model_copy(update={"reasoning": "high"})
    )
    assert OpenAIResponses(quiet).reasoning_style == "none"
    assert OpenAIResponses(CONFIG).reasoning_style == "openai"


# The reply


async def test_a_text_arrives_in_pieces(ctx: Context, llm: Llm) -> None:
    connect(ctx, llm, Service(streamed(TEXT_REPLY)))

    call = await ask(llm)

    assert call.failure is None
    assert call.finish == Finish(reason="stop")
    assert call.message.text == "Hello"


async def test_the_tokens_are_counted_without_those_of_the_cache_twice(
    ctx: Context, llm: Llm
) -> None:
    usage = {
        "input_tokens": 1000,
        "input_tokens_details": {"cached_tokens": 600, "cache_write_tokens": 150},
        "output_tokens": 40,
        "output_tokens_details": {"reasoning_tokens": 30},
        "total_tokens": 1040,
    }

    call = await replied(ctx, llm, text(0, "Hi"), completed(usage=usage))

    assert call.usage == Usage(
        input_tokens=250, output_tokens=40, cache_read_tokens=600, cache_write_tokens=150
    )


async def test_tokens_without_details_are_all_new(ctx: Context, llm: Llm) -> None:
    call = await replied(
        ctx, llm, text(0, "Hi"), completed(usage={"input_tokens": 9, "output_tokens": 2})
    )

    assert call.usage == Usage(input_tokens=9, output_tokens=2)


async def test_the_summary_of_the_reasoning_is_shown_and_what_is_sealed_is_kept(
    ctx: Context, llm: Llm
) -> None:
    call = await replied(
        ctx,
        llm,
        added(0, type="reasoning", id="rs_1", summary=[]),
        thought(0, 0, "Fir"),
        thought(0, 0, "st."),
        thought(0, 1, "Then."),
        done(
            0,
            type="reasoning",
            id="rs_1",
            summary=[
                {"type": "summary_text", "text": "First."},
                {"type": "summary_text", "text": "Then."},
            ],
            encrypted_content="gAAAA-sealed",
        ),
        added(1, type="message", id="msg_1", content=[]),
        text(1, "Hello"),
        done(1, type="message", id="msg_1", content=[{"type": "output_text", "text": "Hello"}]),
        completed(),
    )

    assert call.message.content == (
        ReasoningBlock(text="First.\n\nThen.", replay_state=SEALED),
        TextBlock(text="Hello"),
    )
    starts = [chunk for chunk in call.chunks if isinstance(chunk, BlockStart)]
    # Reasoning that is cut short goes back to nobody: nothing is kept of it as it begins.
    assert [(start.kind, start.replay_state) for start in starts] == [
        ("reasoning", None),
        ("text", None),
    ]


async def test_reasoning_that_the_service_shows_nothing_of_is_kept_all_the_same(
    ctx: Context, llm: Llm
) -> None:
    call = await replied(
        ctx,
        llm,
        added(0, type="reasoning", id="rs_1", summary=[]),
        done(0, type="reasoning", id="rs_1", summary=[], encrypted_content="gAAAA-sealed"),
        text(1, "Hello"),
        completed(),
    )

    assert call.message.content == (
        ReasoningBlock(text="", replay_state={**SEALED, "summary": []}),
        TextBlock(text="Hello"),
    )


async def test_a_tool_call_arrives_with_its_arguments_in_pieces(ctx: Context, llm: Llm) -> None:
    call = await replied(
        ctx,
        llm,
        added(0, type="function_call", id="fc_1", call_id="call_1", name="read", arguments=""),
        arguments(0, '{"path"'),
        arguments(0, ': "a"}'),
        event("response.function_call_arguments.done", output_index=0, arguments='{"path": "a"}'),
        done(
            0,
            type="function_call",
            id="fc_1",
            call_id="call_1",
            name="read",
            arguments='{"path": "a"}',
        ),
        added(1, type="function_call", id="fc_2", call_id="call_2", name="read", arguments=""),
        arguments(1, "{}"),
        done(1, type="function_call", id="fc_2", call_id="call_2", name="read", arguments="{}"),
        completed(),
    )

    assert call.finish == Finish(reason="tool-calls")
    assert call.message.tool_calls == (
        ToolCallBlock(call_id="call_1", name="read", arguments='{"path": "a"}'),
        ToolCallBlock(call_id="call_2", name="read", arguments="{}"),
    )


async def test_what_did_not_arrive_in_pieces_is_taken_whole_at_its_end(
    ctx: Context, llm: Llm
) -> None:
    call = await replied(
        ctx,
        llm,
        done(
            0,
            type="reasoning",
            id="rs_1",
            summary=[{"type": "summary_text", "text": "First."}, {"type": "summary_text"}],
            encrypted_content="gAAAA-sealed",
        ),
        done(
            1,
            type="message",
            content=[
                {"type": "output_text", "text": "I "},
                {"type": "refusal", "refusal": "look."},
            ],
        ),
        # Its name is not said as it begins, so it begins at its end.
        added(2, type="function_call", call_id="call_1", arguments=""),
        arguments(2, "lost"),
        done(2, type="function_call", call_id="call_1", name="read", arguments='{"path": "a"}'),
        done(3, type="function_call", name="read", arguments=""),
        done(4, type="message", content=[]),
        done(5, type="web_search_call", id="ws_1"),
        completed(),
    )

    reasoning, said, first, second = call.message.content
    assert reasoning == ReasoningBlock(
        text="First.", replay_state={**SEALED, "summary": ["First."]}
    )
    assert said == TextBlock(text="I look.")
    assert first == ToolCallBlock(call_id="call_1", name="read", arguments='{"path": "a"}')
    # A call without an id is given one, and one without arguments has none.
    assert isinstance(second, ToolCallBlock)
    assert second.call_id.startswith("call_")
    assert (second.name, second.arguments) == ("read", "")


async def test_reasoning_and_a_refusal_begin_with_their_first_piece(ctx: Context, llm: Llm) -> None:
    call = await replied(
        ctx,
        llm,
        event("response.reasoning_text.delta", output_index=0, content_index=0, delta="Hm."),
        thought(0, 0, ""),
        event("response.refusal.delta", output_index=1, content_index=0, delta="I will "),
        event("response.refusal.delta", output_index=1, content_index=0, delta="not."),
        text(1, ""),
        event("response.output_text.delta", output_index=1, delta={"not": "a text"}),
        arguments(7, "of no call"),
        event("response.audio.delta", delta="AAAA"),
        completed(),
    )

    assert call.message.content == (ReasoningBlock(text="Hm."), TextBlock(text="I will not."))


@pytest.mark.parametrize(
    ("reason", "finish"),
    [("max_output_tokens", "max-tokens"), ("steered", "stop")],
)
async def test_a_reply_that_is_cut_short_keeps_its_text_and_not_half_a_call(
    ctx: Context, llm: Llm, reason: str, finish: str
) -> None:
    call = await replied(
        ctx,
        llm,
        text(0, "Hel"),
        added(1, type="function_call", call_id="call_1", name="read", arguments=""),
        arguments(1, '{"pa'),
        incomplete(reason, usage={"input_tokens": 5, "output_tokens": 3}),
    )

    assert call.failure is None
    assert call.message.text == "Hel"
    assert call.usage == Usage(input_tokens=5, output_tokens=3)
    ended = [chunk.index for chunk in call.chunks if isinstance(chunk, BlockEnd)]
    assert ended == [0]
    if finish == "max-tokens":
        assert call.finish == Finish(reason="max-tokens")


async def test_a_reply_that_is_withheld_is_a_failure(ctx: Context, llm: Llm) -> None:
    call = await replied(ctx, llm, text(0, "Hel"), incomplete("content_filter"))

    assert call.failure == LlmFailure(
        message="the provider withheld the reply", code="CONTENT_FILTER"
    )


@pytest.mark.parametrize(
    ("code", "ours"),
    [
        ("server_error", "SERVER"),
        ("server_is_overloaded", "SERVER"),
        ("rate_limit_exceeded", "RATE_LIMIT"),
        ("context_length_exceeded", "CONTEXT_WINDOW_EXCEEDED"),
        ("insufficient_quota", "QUOTA"),
        ("invalid_prompt", "INVALID_REQUEST"),
        (None, "INVALID_REQUEST"),
    ],
)
async def test_a_reply_that_fails_under_way_says_why(
    ctx: Context, llm: Llm, code: str | None, ours: str
) -> None:
    failed = event(
        "response.failed",
        response={"status": "failed", "error": {"code": code, "message": "It went wrong."}},
    )

    call = await replied(ctx, llm, text(0, "Hel"), failed)

    assert call.failure == LlmFailure(message="It went wrong.", code=ours)


async def test_an_error_in_the_stream_is_a_failure(ctx: Context, llm: Llm) -> None:
    said = await replied(
        ctx, llm, event("error", code="rate_limit_exceeded", message="Slow down.", param=None)
    )
    assert said.failure == LlmFailure(message="Slow down.", code="RATE_LIMIT")


async def test_a_failure_that_says_nothing_is_one_all_the_same(ctx: Context, llm: Llm) -> None:
    unsaid = await replied(ctx, llm, event("error"))
    assert unsaid.failure == LlmFailure(
        message="the service reported an error", code="INVALID_REQUEST"
    )


async def test_a_reply_that_fails_without_an_error_is_one_too(ctx: Context, llm: Llm) -> None:
    call = await replied(ctx, llm, event("response.failed", response={"status": "failed"}))
    assert call.failure == LlmFailure(
        message="the service reported an error", code="INVALID_REQUEST"
    )


async def test_an_event_that_cannot_be_read_is_a_failure(ctx: Context, llm: Llm) -> None:
    call = await replied(ctx, llm, text(0, "Hel"), '{"type": "response.completed", "response": 3}')

    assert call.failure is not None
    assert call.failure.code == "MALFORMED_RESPONSE"
    assert call.failure.message.startswith("the service sent an event that cannot be read: ")


async def test_a_tool_call_that_ends_without_a_name_is_a_failure(ctx: Context, llm: Llm) -> None:
    call = await replied(ctx, llm, done(0, type="function_call", call_id="call_1", arguments="{}"))

    assert call.failure == LlmFailure(
        message="a tool call ended without a name", code="MALFORMED_RESPONSE"
    )


async def test_a_stream_that_stops_before_the_service_ends_the_reply_was_closed(
    ctx: Context, llm: Llm
) -> None:
    call = await replied(ctx, llm, text(0, "Hel"))

    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"
    assert call.message.text == "Hel"


async def test_nothing_is_read_after_the_end_of_the_reply(ctx: Context, llm: Llm) -> None:
    after = await replied(ctx, llm, text(0, "Hello"), completed(), "not an event")
    assert (after.failure, after.message.text) == (None, "Hello")


async def test_a_stream_may_end_as_those_of_chat_completions_do(ctx: Context, llm: Llm) -> None:
    call = await replied(ctx, llm, text(0, "Hel"), "[DONE]", "not an event")
    assert call.failure is not None
    assert call.failure.code == "STREAM_CLOSED"


# Refusals


@pytest.mark.parametrize(
    ("status", "error", "code"),
    [
        (
            400,
            {"message": "Bad.", "type": "invalid_request_error", "code": None},
            "INVALID_REQUEST",
        ),
        (401, {"message": "No such key.", "code": "invalid_api_key"}, "AUTH"),
        (403, {"message": "Not for you."}, "AUTH"),
        (429, {"message": "Slow down.", "code": "rate_limit_exceeded"}, "RATE_LIMIT"),
        (429, {"message": "Pay first.", "code": "insufficient_quota"}, "QUOTA"),
        (429, {"message": "Pay first.", "code": "credit_balance_exhausted"}, "QUOTA"),
        (
            400,
            {"message": "Too long.", "code": "context_length_exceeded"},
            "CONTEXT_WINDOW_EXCEEDED",
        ),
        (400, {"message": "Over the maximum context length."}, "CONTEXT_WINDOW_EXCEEDED"),
        (500, {"message": "Oops."}, "SERVER"),
        (503, {"message": "Busy.", "code": "server_is_overloaded"}, "SERVER"),
        (418, {"message": "A teapot."}, "HTTP_418"),
    ],
)
async def test_a_refusal_says_what_the_service_said_and_what_kind_it_is(
    ctx: Context, llm: Llm, status: int, error: dict[str, Any], code: str
) -> None:
    connect(ctx, llm, Service(refused(status, error, **{"x-request-id": "req_1"})))

    call = await ask(llm)

    assert call.failure == LlmFailure(
        message=error["message"], code=code, status=status, request_id="req_1"
    )


async def test_a_refusal_that_is_no_json_is_shown_as_it_is(ctx: Context, llm: Llm) -> None:
    connect(
        ctx,
        llm,
        Service(refused(502, "Bad Gateway", **{"retry-after": "2"}), refused(404, "")),
    )

    first = await ask(llm)
    second = await ask(llm)

    assert first.failure == LlmFailure(
        message="Bad Gateway", code="SERVER", status=502, retry_after_ms=2000
    )
    assert second.failure == LlmFailure(
        message="HTTP 404 Not Found", code="INVALID_REQUEST", status=404
    )


async def test_the_key_is_cut_out_of_a_failure_and_the_error_is_not_held(
    ctx: Context, llm: Llm
) -> None:
    adapter = connect(ctx, llm, Service(httpx.ConnectError(f"no route with {KEY}")))

    with pytest.raises(LlmError) as raised:
        async for _ in adapter.stream(LlmRequest(config=MODEL, messages=HELLO)):
            pass

    assert KEY not in "".join(traceback.format_exception(raised.value))
    assert raised.value.failure.code == "TRANSPORT"
    assert raised.value.__cause__ is None


async def test_a_service_that_asks_for_no_key_is_sent_none(
    ctx: Context, llm: Llm, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_KEY")
    service = Service(streamed(TEXT_REPLY), httpx.Response(200, json={"data": []}))
    adapter = connect(ctx, llm, service, CONFIG.model_copy(update={"api_key_required": False}))

    call = await ask(llm)
    ids = await adapter.models()

    assert call.message.text == "Hello"
    assert ids == []
    assert all("authorization" not in request.headers for request in service.requests)


# The list of models


async def test_the_list_of_models_is_asked_for_with_the_key(ctx: Context, llm: Llm) -> None:
    listing = httpx.Response(200, json={"object": "list", "data": [{"id": "a"}, {"id": "b"}]})
    service = Service(listing)
    adapter = connect(ctx, llm, service)

    ids = await adapter.models()

    assert ids == ["a", "b"]
    request = service.requests[0]
    assert request.method == "GET"
    assert str(request.url) == "https://api.example.test/v1/models"
    assert request.headers["authorization"] == f"Bearer {KEY}"


async def listed(adapter: OpenAIResponses) -> LlmFailure:
    """Ask for the list of models where that fails, and return the failure."""
    with pytest.raises(LlmError) as raised:
        await adapter.models()
    assert raised.value.__cause__ is None
    assert raised.value.__context__ is None
    return raised.value.failure


async def test_a_list_of_models_that_fails_says_why(ctx: Context, llm: Llm) -> None:
    service = Service(
        refused(401, {"message": f"No such key: {KEY}", "code": "invalid_api_key"}),
        httpx.Response(200, json={"models": []}),
        httpx.ConnectError("no route"),
    )
    adapter = connect(ctx, llm, service)

    refusal, unread, unreached = await listed(adapter), await listed(adapter), await listed(adapter)

    assert (refusal.code, refusal.status) == ("AUTH", 401)
    assert KEY not in refusal.message
    assert unread.code == "MALFORMED_RESPONSE"
    assert unread.message.startswith("the service sent a list of models that cannot be read: ")
    assert unreached.code == "TRANSPORT"


# The adapter


async def test_the_requests_go_through_the_proxy_of_the_system(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    asked: list[str] = []
    made: list[dict[str, Any]] = []
    real = httpx.AsyncClient

    def proxy_for(url: str, env: object) -> str:
        asked.append(url)
        return "http://127.0.0.1:7890"

    def client(**given: Any) -> httpx.AsyncClient:
        made.append(given)
        return real(**given)

    monkeypatch.setattr(openai_responses, "proxy_for", proxy_for)
    monkeypatch.setattr(httpx, "AsyncClient", client)

    await OpenAIResponses(CONFIG).aclose()
    await OpenAIResponses(CONFIG, transport=httpx.MockTransport(Service())).aclose()

    assert asked == [CONFIG.base_url]
    assert [given["proxy"] for given in made] == ["http://127.0.0.1:7890", None]


async def test_a_key_that_is_given_is_sent_instead_of_the_one_that_is_looked_for() -> None:
    service = Service(httpx.Response(200, json={"data": [{"id": "a"}]}))
    adapter = OpenAIResponses(
        CONFIG, transport=httpx.MockTransport(service), secret="  sk-given-0123456789\n"
    )

    await adapter.models()
    await adapter.aclose()

    assert service.requests[0].headers["authorization"] == "Bearer sk-given-0123456789"


async def test_the_plugin_registers_an_adapter_under_the_name_of_its_provider(
    ctx: Context, llm: Llm
) -> None:
    await ctx.plugin(openai_responses, {"provider": "openai", "base_url": "http://127.0.0.1:9/v1"})

    assert isinstance(llm.adapters["openai"], OpenAIResponses)


@pytest.mark.parametrize(
    "more",
    [{"summary": "long"}, {"reasoning_style": "deepseek"}, {"connect_timeout_seconds": 0}],
    ids=["no length of a summary", "the habit of another service", "no time to connect"],
)
def test_a_config_that_makes_no_sense_is_refused(more: dict[str, object]) -> None:
    with pytest.raises(ValidationError):
        Config.model_validate({"provider": "openai", "base_url": "http://localhost/v1", **more})


def test_the_module_is_a_plugin_that_needs_the_model_service() -> None:
    assert openai_responses.name == "llm-openai-responses"
    assert openai_responses.inject == (LLM,)
    assert openai_responses.Config is Config
