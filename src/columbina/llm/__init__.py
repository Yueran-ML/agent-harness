"""Model access: the shared vocabulary, the stream assembler, and the model service.

This module is a plugin. Loading it provides :data:`LLM`.
"""

from pydantic import BaseModel, Field

from columbina.kernel import Context
from columbina.llm.assembler import Assembler, MalformedStreamError
from columbina.llm.render import render_request, render_requests
from columbina.llm.runtime import LLM, Llm, LlmAdapter, LlmCall, LlmError
from columbina.llm.types import (
    NOTHING_ARRIVED,
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
    Picture,
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
    "NOTHING_ARRIVED",
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
    "Picture",
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
