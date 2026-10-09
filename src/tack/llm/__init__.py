"""Model access: the shared vocabulary, the stream assembler, and the model service.

This module is a plugin. Loading it provides :data:`LLM`.
"""

from pydantic import BaseModel, Field

from tack.kernel import Context
from tack.llm.assembler import Assembler, MalformedStreamError
from tack.llm.render import render_request, render_requests
from tack.llm.runtime import LLM, Llm, LlmAdapter, LlmCall, LlmError
from tack.llm.types import (
    AssistantBlock,
    AssistantMessage,
    BlockEnd,
    BlockKind,
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

__all__ = [
    "LLM",
    "Assembler",
    "AssistantBlock",
    "AssistantMessage",
    "BlockEnd",
    "BlockKind",
    "BlockStart",
    "CallConfig",
    "Chunk",
    "Config",
    "Finish",
    "FinishReason",
    "Llm",
    "LlmAdapter",
    "LlmCall",
    "LlmError",
    "LlmFailure",
    "LlmRequest",
    "MalformedStreamError",
    "Message",
    "ReasoningBlock",
    "ReasoningDelta",
    "SystemMessage",
    "TextBlock",
    "TextDelta",
    "ToolCallBlock",
    "ToolCallDelta",
    "ToolMessage",
    "ToolSchema",
    "Usage",
    "UsageChunk",
    "UserMessage",
    "apply",
    "name",
    "render_request",
    "render_requests",
]

name = "llm"


class Config(BaseModel):
    """The config of the model service.

    Attributes:
        idle_timeout_seconds: How long a stream may stay silent between two
            chunks before the call fails with ``TIMEOUT``.
    """

    idle_timeout_seconds: float = Field(default=300.0, gt=0)


def apply(ctx: Context, config: Config) -> None:
    """Provide the model service."""
    ctx.provide(LLM, Llm(config.idle_timeout_seconds))
