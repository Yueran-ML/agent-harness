"""The vocabulary is plain data that survives being written to a log and read back."""

import pytest
from pydantic import TypeAdapter, ValidationError

from tack.llm import (
    AssistantMessage,
    BlockEnd,
    BlockStart,
    CallConfig,
    Chunk,
    Finish,
    FinishReason,
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

REPLY = AssistantMessage(
    content=(
        ReasoningBlock(text="think", replay_state={"signature": "abc"}),
        TextBlock(text="I will "),
        TextBlock(text="look."),
        ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
        ToolCallBlock(call_id="call-2", name="read", arguments='{"path": "b.txt"}'),
    )
)

MESSAGES: list[Message] = [
    SystemMessage(content=(TextBlock(text="You are helpful."),)),
    UserMessage(content=(TextBlock(text="Read the files."),)),
    REPLY,
    ToolMessage(call_id="call-1", content=(TextBlock(text="contents"),)),
    ToolMessage(call_id="call-2", content=(TextBlock(text="no such file"),), is_error=True),
]

CHUNKS: list[Chunk] = [
    BlockStart(index=0, kind="reasoning"),
    ReasoningDelta(index=0, text="think"),
    BlockEnd(index=0, replay_state={"signature": "abc"}),
    BlockStart(index=1, kind="text"),
    TextDelta(index=1, text="hi"),
    BlockEnd(index=1),
    BlockStart(index=2, kind="tool-call", call_id="call-1", name="read"),
    ToolCallDelta(index=2, arguments="{}"),
    BlockEnd(index=2),
    UsageChunk(usage=Usage(input_tokens=3, output_tokens=5, cache_read_tokens=7)),
    Finish(reason="error", failure=LlmFailure(message="slow down", code="RATE_LIMIT", status=429)),
]


def test_an_assistant_message_exposes_its_text_and_its_tool_calls() -> None:
    assert REPLY.text == "I will look."
    assert [call.call_id for call in REPLY.tool_calls] == ["call-1", "call-2"]


@pytest.mark.parametrize("message", MESSAGES)
def test_a_message_survives_a_json_round_trip(message: Message) -> None:
    adapter: TypeAdapter[Message] = TypeAdapter(Message)

    assert adapter.validate_json(adapter.dump_json(message)) == message


@pytest.mark.parametrize("chunk", CHUNKS)
def test_a_chunk_survives_a_json_round_trip(chunk: Chunk) -> None:
    adapter: TypeAdapter[Chunk] = TypeAdapter(Chunk)

    assert adapter.validate_json(adapter.dump_json(chunk)) == chunk


@pytest.mark.parametrize("reason", ["stop", "tool-calls", "max-tokens"])
def test_a_finish_that_is_no_error_carries_no_failure(reason: FinishReason) -> None:
    assert Finish(reason=reason).failure is None
    with pytest.raises(ValidationError, match="carries a failure exactly when"):
        Finish(reason=reason, failure=LlmFailure(message="slow down", code="RATE_LIMIT"))


def test_a_finish_that_is_an_error_carries_a_failure() -> None:
    failure = LlmFailure(message="slow down", code="RATE_LIMIT")

    assert Finish(reason="error", failure=failure).failure == failure
    with pytest.raises(ValidationError, match="carries a failure exactly when"):
        Finish(reason="error")


def test_a_request_survives_a_json_round_trip() -> None:
    request = LlmRequest(
        config=CallConfig(
            provider="deepseek",
            model="deepseek-v4-flash",
            max_output_tokens=4096,
            temperature=0.0,
            reasoning="high",
            options={"thinking": {"type": "enabled"}},
        ),
        messages=tuple(MESSAGES),
        tools=(
            ToolSchema(
                name="read",
                description="Read a file.",
                parameters={"type": "object", "properties": {"path": {"type": "string"}}},
            ),
        ),
    )

    assert LlmRequest.model_validate_json(request.model_dump_json()) == request


def test_a_call_config_leaves_everything_but_the_model_to_the_provider() -> None:
    config = CallConfig(provider="deepseek", model="deepseek-v4-flash")

    assert config.model_dump(exclude_none=True) == {
        "provider": "deepseek",
        "model": "deepseek-v4-flash",
        "options": {},
    }


def test_the_vocabulary_is_immutable() -> None:
    block = TextBlock(text="fixed")

    with pytest.raises(ValidationError, match="frozen"):
        block.text = "changed"


def test_the_vocabulary_rejects_unknown_fields() -> None:
    with pytest.raises(ValidationError, match="colour"):
        TextBlock.model_validate({"type": "text", "text": "x", "colour": "blue"})
