"""The events of a session log.

A session event is an envelope around typed data. The envelope says where the
event stands in the log and when it was appended. The data says what happened,
and its class is the type of the event.
"""

from dataclasses import dataclass
from typing import Any, ClassVar, Generic, Literal, TypeVar

from pydantic import JsonValue

from columbina.data import Data
from columbina.llm.types import (
    AssistantMessage,
    CallConfig,
    LlmFailure,
    Message,
    SystemMessage,
    ToolMessage,
    ToolSchema,
    Usage,
    UserMessage,
)


class EventData(Data):
    """Base of what a session event carries. Each subclass is one event type.

    An event in a log is never changed. The fields of the event are frozen,
    but Python does not stop a change to a list or a mapping that a field
    holds, so declare collections as tuples and leave JSON values alone. A
    change would make the session disagree with its file.

    Attributes:
        event_type: The name of the type, such as ``tool/call``. A log file
            records it with every event of the type.
        ignorable: Whether a reader that does not know the type may skip the
            event. A session with an unknown event that is not ignorable
            cannot be loaded.
    """

    event_type: ClassVar[str]
    ignorable: ClassVar[bool] = False

    @classmethod
    def __pydantic_init_subclass__(cls, **kwargs: Any) -> None:
        super().__pydantic_init_subclass__(**kwargs)
        if not isinstance(getattr(cls, "event_type", None), str):
            raise TypeError(f"{cls.__name__} does not name its event type")

    def for_model(self) -> Message | None:
        """Return the message that this event adds to what the model sees, if any."""
        return None


DataT_co = TypeVar("DataT_co", bound=EventData, covariant=True)


@dataclass(frozen=True, slots=True)
class SessionEvent(Generic[DataT_co]):
    """An event as the log holds it.

    Attributes:
        seq: The position of the event, which is the length of the log when
            the event was appended.
        time: When the event was appended, in milliseconds since the epoch.
        data: What happened.
        on_surface: Whether the event carries a message that the model sees.
        replaces: The events whose messages this one takes the place of, as
            the model sees it. They stay in the log.
    """

    seq: int
    time: int
    data: DataT_co
    on_surface: bool = False
    replaces: tuple[int, ...] = ()

    @property
    def type(self) -> str:
        """The name of the event type."""
        return self.data.event_type


TurnEndReason = Literal[
    "completed", "aborted", "blocked", "error", "max-tokens", "interrupted", "forked"
]
"""Why a turn ended. ``interrupted`` and ``forked`` are only written when a log is repaired."""


class TurnStartEvent(EventData):
    """A turn begins."""

    event_type: ClassVar[str] = "turn/start"

    turn: int


class TurnEndEvent(EventData):
    """A turn is over. ``detail`` says more about the reason, such as who cancelled."""

    event_type: ClassVar[str] = "turn/end"

    turn: int
    reason: TurnEndReason
    detail: str | None = None


class StepStartEvent(EventData):
    """A step begins: one request to the model and the tool calls it leads to."""

    event_type: ClassVar[str] = "step/start"

    turn: int
    step: int


class StepEndEvent(EventData):
    """A step is over."""

    event_type: ClassVar[str] = "step/end"

    turn: int
    step: int


class SystemMessageEvent(EventData):
    """The system prompt as it was rendered."""

    event_type: ClassVar[str] = "system/message"

    message: SystemMessage

    def for_model(self) -> Message:
        """Return the system message."""
        return self.message


class UserMessageEvent(EventData):
    """Input for the model in the role of the user.

    ``source`` says where the input came from. ``user`` is what the person
    typed, or for a subagent what the agent wrote that handed it its task.
    Other sources mark input that was supplied on their behalf.
    """

    event_type: ClassVar[str] = "user/message"

    message: UserMessage
    source: str = "user"

    def for_model(self) -> Message:
        """Return the user message."""
        return self.message


class AssistantMessageEvent(EventData):
    """A reply of the model.

    ``finish_reason`` is ``None`` for a reply that was interrupted, in which
    case ``message`` holds what had arrived.
    """

    event_type: ClassVar[str] = "assistant/message"

    message: AssistantMessage
    usage: Usage
    finish_reason: Literal["stop", "tool-calls", "max-tokens"] | None

    @property
    def interrupted(self) -> bool:
        """Whether the reply was cut short by a cancellation."""
        return self.finish_reason is None

    def for_model(self) -> Message | None:
        """Return the reply, unless it has no content."""
        return self.message if self.message.content else None


class AssistantAttemptEvent(EventData):
    """A call to the model that failed. ``message`` holds what had arrived."""

    event_type: ClassVar[str] = "assistant/attempt"

    message: AssistantMessage
    usage: Usage
    failure: LlmFailure


class AsideEvent(EventData):
    """A request to a model that is no part of the conversation.

    A summary of the conversation is asked for in this way. What the model
    answers is not shown to it as something it said, so there is no message
    here. But the request cost what any request costs, and that is what this
    event is for.

    Attributes:
        purpose: What the request was made for, in a word.
        usage: What it cost.
        failure: Why it failed, if it did.
    """

    event_type: ClassVar[str] = "request/aside"
    ignorable: ClassVar[bool] = True

    purpose: str
    usage: Usage
    failure: LlmFailure | None = None


class ToolCallEvent(EventData):
    """A tool call is about to be dispatched, with the arguments as the model wrote them."""

    event_type: ClassVar[str] = "tool/call"

    call_id: str
    name: str
    arguments: str


class ToolError(Data):
    """Why a tool call did not succeed, for programs.

    Attributes:
        code: A stable code, such as ``UNKNOWN_TOOL``.
        reason: A description for people.
    """

    code: str
    reason: str


class ToolResultEvent(EventData):
    """The result of a tool call.

    Attributes:
        message: The result as the model sees it.
        error: Set when the call did not succeed.
        meta: Facts about the call that the model does not see.
    """

    event_type: ClassVar[str] = "tool/result"

    message: ToolMessage
    error: ToolError | None = None
    meta: dict[str, JsonValue] | None = None

    def for_model(self) -> Message:
        """Return the result as the model sees it."""
        return self.message


class RequestHeaderEvent(EventData):
    """The call config and the tool schemas that the following requests use."""

    event_type: ClassVar[str] = "request/header"

    config: CallConfig
    tools: tuple[ToolSchema, ...]


class EndSeedEvent(EventData):
    """The boundary between history that was taken over and work of this lifetime.

    ``inherited`` is true when the history comes from another session.
    """

    event_type: ClassVar[str] = "session/end-seed"

    inherited: bool


CORE_EVENTS: tuple[type[EventData], ...] = (
    TurnStartEvent,
    TurnEndEvent,
    StepStartEvent,
    StepEndEvent,
    SystemMessageEvent,
    UserMessageEvent,
    AssistantMessageEvent,
    AssistantAttemptEvent,
    AsideEvent,
    ToolCallEvent,
    ToolResultEvent,
    RequestHeaderEvent,
    EndSeedEvent,
)
"""The event types that every session knows."""
