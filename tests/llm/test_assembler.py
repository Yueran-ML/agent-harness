"""The assembler folds a chunk stream back into an assistant message."""

from typing import cast

import pytest

from tack.llm import (
    Assembler,
    AssistantMessage,
    BlockEnd,
    BlockStart,
    Chunk,
    Finish,
    LlmFailure,
    MalformedStreamError,
    ReasoningBlock,
    ReasoningDelta,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    ToolCallDelta,
    Usage,
    UsageChunk,
)


def assemble(chunks: list[Chunk]) -> Assembler:
    assembler = Assembler()
    for chunk in chunks:
        assembler.push(chunk)
    return assembler


def text(index: int, *parts: str) -> list[Chunk]:
    return [
        BlockStart(index=index, kind="text"),
        *(TextDelta(index=index, text=part) for part in parts),
        BlockEnd(index=index),
    ]


def tool_call(index: int, call_id: str, name: str, *parts: str) -> list[Chunk]:
    return [
        BlockStart(index=index, kind="tool-call", call_id=call_id, name=name),
        *(ToolCallDelta(index=index, arguments=part) for part in parts),
        BlockEnd(index=index),
    ]


def test_text_deltas_are_joined() -> None:
    assembler = assemble([*text(0, "Hel", "lo"), Finish(reason="stop")])

    assert assembler.message() == AssistantMessage(content=(TextBlock(text="Hello"),))
    assert assembler.finish == Finish(reason="stop")


def test_blocks_keep_the_order_of_their_indices() -> None:
    chunks: list[Chunk] = [
        BlockStart(index=0, kind="reasoning"),
        ReasoningDelta(index=0, text="think"),
        BlockEnd(index=0, replay_state={"signature": "abc"}),
        *text(1, "answer"),
        *tool_call(2, "call-1", "read", '{"path":', ' "a.txt"}'),
        Finish(reason="tool-calls"),
    ]

    assert assemble(chunks).message() == AssistantMessage(
        content=(
            ReasoningBlock(text="think", replay_state={"signature": "abc"}),
            TextBlock(text="answer"),
            ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
        )
    )


def test_blocks_are_ordered_by_index_in_whatever_order_they_begin() -> None:
    chunks: list[Chunk] = [*text(1, "second"), *text(0, "first"), Finish(reason="stop")]

    assert assemble(chunks).message() == AssistantMessage(
        content=(TextBlock(text="first"), TextBlock(text="second"))
    )


def test_something_that_is_no_chunk_is_rejected_and_changes_nothing() -> None:
    assembler = assemble(text(0, "kept"))

    with pytest.raises(MalformedStreamError, match="a str object is not a chunk"):
        assembler.push(cast("Chunk", "not a chunk"))

    assert assembler.finish is None
    assert assembler.message() == AssistantMessage(content=(TextBlock(text="kept"),))


def test_interleaved_blocks_are_kept_apart() -> None:
    chunks: list[Chunk] = [
        BlockStart(index=0, kind="tool-call", call_id="a", name="read"),
        BlockStart(index=1, kind="tool-call", call_id="b", name="grep"),
        ToolCallDelta(index=1, arguments='{"q":'),
        ToolCallDelta(index=0, arguments='{"p":1}'),
        ToolCallDelta(index=1, arguments="2}"),
        BlockEnd(index=0),
        BlockEnd(index=1),
        Finish(reason="tool-calls"),
    ]

    assert assemble(chunks).message().content == (
        ToolCallBlock(call_id="a", name="read", arguments='{"p":1}'),
        ToolCallBlock(call_id="b", name="grep", arguments='{"q":2}'),
    )


def test_the_last_usage_wins() -> None:
    assembler = assemble(
        [
            UsageChunk(usage=Usage(input_tokens=10)),
            *text(0, "hi"),
            UsageChunk(usage=Usage(input_tokens=10, output_tokens=2, cache_read_tokens=90)),
            Finish(reason="stop"),
        ]
    )

    assert assembler.usage == Usage(input_tokens=10, output_tokens=2, cache_read_tokens=90)


def test_usage_defaults_to_zero() -> None:
    assert assemble([*text(0, "hi"), Finish(reason="stop")]).usage == Usage()


def test_a_reply_cut_off_by_the_token_limit_loses_its_tool_calls() -> None:
    chunks: list[Chunk] = [
        *text(0, "let me look"),
        *tool_call(1, "call-1", "read", '{"path": "a.txt"}'),
        Finish(reason="max-tokens"),
    ]

    assert assemble(chunks).message().content == (TextBlock(text="let me look"),)


def test_an_unfinished_tool_call_is_dropped_and_unfinished_text_is_kept() -> None:
    assembler = assemble(
        [
            BlockStart(index=0, kind="text"),
            TextDelta(index=0, text="partial"),
            BlockStart(index=1, kind="tool-call", call_id="call-1", name="read"),
            ToolCallDelta(index=1, arguments='{"path":'),
        ]
    )

    assert assembler.finish is None
    assert assembler.message().content == (TextBlock(text="partial"),)


def test_empty_text_and_reasoning_blocks_are_left_out() -> None:
    chunks: list[Chunk] = [
        BlockStart(index=0, kind="reasoning"),
        BlockEnd(index=0),
        *text(1, ""),
        *text(2, "kept"),
        Finish(reason="stop"),
    ]

    assert assemble(chunks).message().content == (TextBlock(text="kept"),)


def test_an_empty_reasoning_block_that_carries_replay_state_is_kept() -> None:
    chunks: list[Chunk] = [
        BlockStart(index=0, kind="reasoning"),
        BlockEnd(index=0, replay_state={"signature": "abc"}),
        Finish(reason="stop"),
    ]

    assert assemble(chunks).message().content == (
        ReasoningBlock(text="", replay_state={"signature": "abc"}),
    )


def test_a_failure_is_kept_with_the_finish() -> None:
    failure = LlmFailure(message="too many requests", code="RATE_LIMIT", status=429)

    assembler = assemble([Finish(reason="error", failure=failure)])

    assert assembler.finish == Finish(reason="error", failure=failure)
    assert assembler.message().content == ()


@pytest.mark.parametrize(
    ("chunks", "complaint"),
    [
        ([TextDelta(index=0, text="x")], "block 0 was never started"),
        ([BlockStart(index=0, kind="text"), BlockStart(index=0, kind="text")], "started twice"),
        ([BlockStart(index=0, kind="text"), ToolCallDelta(index=0, arguments="{}")], "is a text"),
        ([BlockStart(index=0, kind="tool-call")], "needs a call id and a name"),
        ([*text(0, "x"), TextDelta(index=0, text="y")], "already ended"),
        ([Finish(reason="stop"), *text(0, "late")], "after the finish"),
    ],
)
def test_a_stream_that_breaks_the_protocol_is_rejected(chunks: list[Chunk], complaint: str) -> None:
    with pytest.raises(MalformedStreamError, match=complaint):
        assemble(chunks)
