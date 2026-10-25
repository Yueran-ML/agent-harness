"""The vocabulary shared by the agent loop and every model adapter.

Everything here is plain data. A message is what a model sees, a chunk is one
piece of a streamed reply, and a failure is what went wrong with a call.
"""

from typing import Annotated, Literal, Self

from pydantic import Field, JsonValue, model_validator

from columbina.data import Data


class TextBlock(Data):
    """A piece of text."""

    type: Literal["text"] = "text"
    text: str


class ReasoningBlock(Data):
    """Reasoning that a model produced before its answer.

    ``replay_state`` belongs to the adapter that produced the block. It is sent
    back unchanged with the next request, and only to that adapter.
    """

    type: Literal["reasoning"] = "reasoning"
    text: str
    replay_state: dict[str, JsonValue] | None = None


class ToolCallBlock(Data):
    """A request by the model to call a tool.

    ``arguments`` is the JSON text exactly as the model produced it. It is not
    parsed or repaired on the way.
    """

    type: Literal["tool-call"] = "tool-call"
    call_id: str
    name: str
    arguments: str


AssistantBlock = Annotated[TextBlock | ReasoningBlock | ToolCallBlock, Field(discriminator="type")]


class SystemMessage(Data):
    """The instructions that open a conversation."""

    role: Literal["system"] = "system"
    content: tuple[TextBlock, ...]


class UserMessage(Data):
    """Input from the person, or context that is supplied on their behalf."""

    role: Literal["user"] = "user"
    content: tuple[TextBlock, ...]


class AssistantMessage(Data):
    """A reply of the model."""

    role: Literal["assistant"] = "assistant"
    content: tuple[AssistantBlock, ...]

    @property
    def text(self) -> str:
        """The text of the reply, without reasoning and tool calls."""
        return "".join(block.text for block in self.content if isinstance(block, TextBlock))

    @property
    def tool_calls(self) -> tuple[ToolCallBlock, ...]:
        """The tool calls of the reply, in the order the model made them."""
        return tuple(block for block in self.content if isinstance(block, ToolCallBlock))


class ToolMessage(Data):
    """The result of one tool call, as the model sees it."""

    role: Literal["tool"] = "tool"
    call_id: str
    content: tuple[TextBlock, ...]
    is_error: bool = False


Message = Annotated[
    SystemMessage | UserMessage | AssistantMessage | ToolMessage, Field(discriminator="role")
]


class Usage(Data):
    """The token counts of one call. The four counts do not overlap.

    ``input_tokens`` counts only the input that was not read from a cache.
    """

    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_tokens: int = 0
    cache_write_tokens: int = 0

    def __add__(self, other: "Usage") -> "Usage":
        return Usage(
            input_tokens=self.input_tokens + other.input_tokens,
            output_tokens=self.output_tokens + other.output_tokens,
            cache_read_tokens=self.cache_read_tokens + other.cache_read_tokens,
            cache_write_tokens=self.cache_write_tokens + other.cache_write_tokens,
        )


class LlmFailure(Data):
    """Why a call to a model failed.

    Attributes:
        message: A description for people.
        code: A stable code for programs, such as ``RATE_LIMIT``.
        status: The HTTP status, when the failure came with one.
        retry_after_ms: How long the provider asked the caller to wait.
        request_id: The provider's identifier of the request.
    """

    message: str
    code: str
    status: int | None = None
    retry_after_ms: int | None = None
    request_id: str | None = None


NOTHING_ARRIVED = LlmFailure(
    message="the reply was cut off before it held anything", code="CUT_OFF"
)
"""What is written down of a reply that the token limit cut off before it held anything.

Such a reply is no failure of the call: it ends with ``max-tokens`` like any other that
was cut off. But it leaves no message that could be kept, so whoever keeps a log writes it
down as an attempt, with this in the place of a failure.
"""

FinishReason = Literal["stop", "tool-calls", "max-tokens", "error"]
BlockKind = Literal["text", "reasoning", "tool-call"]


class BlockStart(Data):
    """A new content block begins. A tool call carries its id and name here.

    A block of reasoning may carry its ``replay_state`` from the beginning, for
    an adapter that can send the reasoning back in part: what had arrived of it
    when the reply was cut short is then sent back with the next request. What
    the end of the block carries takes its place.
    """

    type: Literal["block-start"] = "block-start"
    index: int
    kind: BlockKind
    call_id: str | None = None
    name: str | None = None
    replay_state: dict[str, JsonValue] | None = None


class TextDelta(Data):
    """More text for a text block."""

    type: Literal["text-delta"] = "text-delta"
    index: int
    text: str


class ReasoningDelta(Data):
    """More text for a reasoning block."""

    type: Literal["reasoning-delta"] = "reasoning-delta"
    index: int
    text: str


class ToolCallDelta(Data):
    """More of the JSON text of a tool call's arguments."""

    type: Literal["tool-call-delta"] = "tool-call-delta"
    index: int
    arguments: str


class BlockEnd(Data):
    """A content block is complete."""

    type: Literal["block-end"] = "block-end"
    index: int
    replay_state: dict[str, JsonValue] | None = None


class UsageChunk(Data):
    """The token counts so far. A later one replaces an earlier one."""

    type: Literal["usage"] = "usage"
    usage: Usage


class Finish(Data):
    """The reply is over. ``failure`` is set exactly when the reason is ``error``."""

    type: Literal["finish"] = "finish"
    reason: FinishReason
    failure: LlmFailure | None = None

    @model_validator(mode="after")
    def _failure_goes_with_error(self) -> Self:
        if (self.reason == "error") != (self.failure is not None):
            raise ValueError("a finish carries a failure exactly when its reason is 'error'")
        return self


Chunk = Annotated[
    BlockStart | TextDelta | ReasoningDelta | ToolCallDelta | BlockEnd | UsageChunk | Finish,
    Field(discriminator="type"),
]


class ToolSchema(Data):
    """What a model is told about one tool.

    ``parameters`` is a JSON Schema object describing the arguments.
    """

    name: str
    description: str
    parameters: dict[str, JsonValue]


class CallConfig(Data):
    """Everything about one call that is not the conversation.

    A field left as ``None`` is not sent, and the provider's default applies.
    The whole config is written to the session log with each request, so a
    run can be reproduced from the log.

    Attributes:
        provider: The name under which the adapter is registered.
        model: The model identifier, in the provider's own spelling.
        context_window: The size of the model's context window, in tokens.
        max_output_tokens: The limit on the length of the reply.
        temperature: The sampling temperature.
        top_p: The nucleus sampling threshold.
        seed: The sampling seed, for providers that honour one.
        reasoning: How much the model should reason before it answers.
        options: Further settings that only one adapter understands.
    """

    provider: str
    model: str
    context_window: int | None = None
    max_output_tokens: int | None = None
    temperature: float | None = None
    top_p: float | None = None
    seed: int | None = None
    reasoning: Literal["off", "low", "high", "max"] | None = None
    options: dict[str, JsonValue] = Field(default_factory=dict[str, JsonValue])


class LlmRequest(Data):
    """One call to a model: the config, the conversation, and the tools on offer."""

    config: CallConfig
    messages: tuple[Message, ...]
    tools: tuple[ToolSchema, ...] = ()
