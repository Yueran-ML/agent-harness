"""Folding a stream of chunks back into an assistant message."""

from dataclasses import dataclass, field

from pydantic import JsonValue

from tack.llm.types import (
    AssistantBlock,
    AssistantMessage,
    BlockEnd,
    BlockKind,
    BlockStart,
    Chunk,
    Finish,
    ReasoningBlock,
    ReasoningDelta,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    ToolCallDelta,
    Usage,
    UsageChunk,
)


class MalformedStreamError(Exception):
    """A stream of chunks does not follow the protocol."""


@dataclass(slots=True)
class _Partial:
    kind: BlockKind
    call_id: str | None
    name: str | None
    parts: list[str] = field(default_factory=list[str])
    ended: bool = False
    replay_state: dict[str, JsonValue] | None = None


class Assembler:
    """Collects the chunks of one reply.

    Push every chunk as it arrives. ``message()`` can be asked for at any
    time: before the finish it returns what has arrived so far, which is what
    an interrupted reply is recorded as.

    Attributes:
        usage: The most recent token counts, or zeros if none arrived.
        finish: The finish chunk, once it has arrived.
    """

    __slots__ = ("_blocks", "finish", "usage")

    def __init__(self) -> None:
        self._blocks: dict[int, _Partial] = {}
        self.usage = Usage()
        self.finish: Finish | None = None

    def push(self, chunk: Chunk) -> None:
        """Take in the next chunk.

        Raises:
            MalformedStreamError: The chunk cannot follow the ones before it.
        """
        if self.finish is not None:
            raise MalformedStreamError(f"a {chunk.type} chunk arrived after the finish")
        if isinstance(chunk, BlockStart):
            self._start(chunk)
        elif isinstance(chunk, TextDelta):
            self._open(chunk.index, "text").parts.append(chunk.text)
        elif isinstance(chunk, ReasoningDelta):
            self._open(chunk.index, "reasoning").parts.append(chunk.text)
        elif isinstance(chunk, ToolCallDelta):
            self._open(chunk.index, "tool-call").parts.append(chunk.arguments)
        elif isinstance(chunk, BlockEnd):
            block = self._open(chunk.index, None)
            block.ended = True
            block.replay_state = chunk.replay_state
        elif isinstance(chunk, UsageChunk):
            self.usage = chunk.usage
        else:
            self.finish = chunk

    def message(self) -> AssistantMessage:
        """Return the reply as it stands.

        Text and reasoning that are still arriving are included. A tool call
        is included only once it is complete, and never when the reply was cut
        off by the token limit: a truncated call is not safe to run.
        """
        truncated = self.finish is not None and self.finish.reason == "max-tokens"
        content: list[AssistantBlock] = []
        for _, block in sorted(self._blocks.items()):
            joined = "".join(block.parts)
            if block.kind == "text":
                if joined:
                    content.append(TextBlock(text=joined))
            elif block.kind == "reasoning":
                if joined or block.replay_state is not None:
                    content.append(ReasoningBlock(text=joined, replay_state=block.replay_state))
            elif block.ended and not truncated:
                assert block.call_id is not None
                assert block.name is not None
                content.append(
                    ToolCallBlock(call_id=block.call_id, name=block.name, arguments=joined)
                )
        return AssistantMessage(content=tuple(content))

    def _start(self, chunk: BlockStart) -> None:
        if chunk.index in self._blocks:
            raise MalformedStreamError(f"block {chunk.index} was started twice")
        if chunk.kind == "tool-call" and (chunk.call_id is None or chunk.name is None):
            raise MalformedStreamError(
                f"block {chunk.index}: a tool-call block needs a call id and a name"
            )
        self._blocks[chunk.index] = _Partial(chunk.kind, chunk.call_id, chunk.name)

    def _open(self, index: int, kind: BlockKind | None) -> _Partial:
        block = self._blocks.get(index)
        if block is None:
            raise MalformedStreamError(f"block {index} was never started")
        if block.ended:
            raise MalformedStreamError(f"block {index} has already ended")
        if kind is not None and block.kind != kind:
            raise MalformedStreamError(f"block {index} is a {block.kind} block, not a {kind} block")
        return block
