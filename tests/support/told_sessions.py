"""A session in which a little of everything is said, for the pages that tell one."""

from columbina.compaction import SUMMARY_SOURCE
from columbina.export import RESULT_MOST
from columbina.llm import (
    AssistantMessage,
    CallConfig,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)


def said(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


def answered(*blocks: TextBlock | ToolCallBlock) -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=AssistantMessage(content=blocks), usage=Usage(), finish_reason="stop"
    )


def result(call_id: str, text: str, code: str | None = None) -> ToolResultEvent:
    return ToolResultEvent(
        message=ToolMessage(call_id=call_id, content=(TextBlock(text=text),)),
        error=None if code is None else ToolError(code=code, reason=text),
    )


HAPPENED: tuple[EventData, ...] = (
    RequestHeaderEvent(config=CallConfig(provider="deepseek", model="deepseek-flash"), tools=()),
    TurnStartEvent(turn=1),
    said('<context source="project">\nBe brief.\n</context>', "context/project"),
    said("Fix <b>calc.py</b> & say why."),
    answered(
        TextBlock(text="I will read it."),
        ToolCallBlock(call_id="c1", name="read", arguments='{"path": "calc.py"}'),
    ),
    result("c1", "1\tdef mean(xs):"),
    answered(ToolCallBlock(call_id="c2", name="shell", arguments="not json ```")),
    result("c2", "x" * (RESULT_MOST + 5), "TOOL_ERROR"),
    said("Keep it small.", "steer"),
    answered(TextBlock(text="Done: mean() divides by len(xs)."), TextBlock(text="  ")),
    TurnEndEvent(turn=1, reason="completed"),
    said("Earlier, calc.py was fixed.", SUMMARY_SOURCE),
    TurnStartEvent(turn=2),
    said("And the tests?"),
    TurnEndEvent(turn=2, reason="error", detail="AUTH: no key"),
    TurnStartEvent(turn=3),
    TurnEndEvent(turn=3, reason="aborted"),
)
