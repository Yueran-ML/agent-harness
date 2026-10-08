"""Session events are plain data that survives being written to a log and read back."""

from typing import ClassVar

import pytest

from tack.llm import (
    AssistantMessage,
    CallConfig,
    LlmFailure,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    Usage,
    UserMessage,
)
from tack.session import (
    CORE_EVENTS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EndSeedEvent,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    SystemMessageEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)

REPLY = AssistantMessage(
    content=(
        TextBlock(text="I will look."),
        ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
    )
)

SAMPLES: list[EventData] = [
    TurnStartEvent(turn=1),
    TurnEndEvent(turn=1, reason="completed"),
    TurnEndEvent(turn=2, reason="aborted", detail="user"),
    StepStartEvent(turn=1, step=1),
    StepEndEvent(turn=1, step=1),
    SystemMessageEvent(message=SystemMessage(content=(TextBlock(text="You are helpful."),))),
    UserMessageEvent(message=UserMessage(content=(TextBlock(text="Read a.txt."),))),
    UserMessageEvent(message=UserMessage(content=(TextBlock(text="Stop."),)), source="steer"),
    AssistantMessageEvent(
        message=REPLY, usage=Usage(input_tokens=30, output_tokens=12), finish_reason="tool-calls"
    ),
    AssistantMessageEvent(
        message=AssistantMessage(content=(TextBlock(text="I wi"),)),
        usage=Usage(),
        finish_reason=None,
    ),
    AssistantAttemptEvent(
        message=AssistantMessage(content=()),
        usage=Usage(),
        failure=LlmFailure(message="slow down", code="RATE_LIMIT", status=429),
    ),
    ToolCallEvent(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
    ToolResultEvent(message=ToolMessage(call_id="call-1", content=(TextBlock(text="hello"),))),
    ToolResultEvent(
        message=ToolMessage(
            call_id="call-2", content=(TextBlock(text="no such file"),), is_error=True
        ),
        error=ToolError(code="TOOL_ERROR", reason="no such file"),
        meta={"duration_ms": 12},
    ),
    RequestHeaderEvent(
        config=CallConfig(provider="deepseek", model="deepseek-v4-flash", temperature=0.0),
        tools=(ToolSchema(name="read", description="Read a file.", parameters={"type": "object"}),),
    ),
    EndSeedEvent(inherited=True),
]


@pytest.mark.parametrize("data", SAMPLES, ids=lambda data: type(data).__name__)
def test_an_event_survives_a_json_round_trip(data: EventData) -> None:
    assert type(data).model_validate_json(data.model_dump_json()) == data


def test_the_samples_cover_every_core_event() -> None:
    assert {type(data) for data in SAMPLES} == set(CORE_EVENTS)


def test_the_core_events_have_these_names() -> None:
    assert [kind.event_type for kind in CORE_EVENTS] == [
        "turn/start",
        "turn/end",
        "step/start",
        "step/end",
        "system/message",
        "user/message",
        "assistant/message",
        "assistant/attempt",
        "tool/call",
        "tool/result",
        "request/header",
        "session/end-seed",
    ]


def test_no_core_event_may_be_skipped_by_a_reader() -> None:
    assert [kind.event_type for kind in CORE_EVENTS if kind.ignorable] == []


def test_a_reply_without_a_finish_reason_was_interrupted() -> None:
    finished, interrupted = (data for data in SAMPLES if isinstance(data, AssistantMessageEvent))

    assert not finished.interrupted
    assert interrupted.interrupted


def test_a_session_event_is_known_by_the_type_of_its_data() -> None:
    event = SessionEvent(seq=3, time=1_791_234_567_890, data=TurnStartEvent(turn=1))

    assert event.type == "turn/start"


def test_an_event_class_must_name_its_type() -> None:
    with pytest.raises(TypeError, match="Nameless does not name its event type"):
        type("Nameless", (EventData,), {})


def test_an_event_class_may_be_ignorable() -> None:
    class Hint(EventData):
        event_type: ClassVar[str] = "test/hint"
        ignorable: ClassVar[bool] = True

        text: str

    assert Hint.ignorable
    assert Hint(text="x").model_dump() == {"text": "x"}
