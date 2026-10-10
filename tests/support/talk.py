"""A conversation that a test writes by hand, with messages of sizes that it can count."""

from columbina.compaction.basic.region import Region, choose
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    Message,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    CORE_EVENTS,
    AssistantMessageEvent,
    Session,
    SystemMessageEvent,
    ToolResultEvent,
    TurnStartEvent,
    UserMessageEvent,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}
FAR = 10**9


def letters(message: Message) -> int:
    """A message is as large as its text is long, so that the tests can count."""
    return sum(len(block.text) for block in message.content if isinstance(block, TextBlock))


class Talk:
    """A conversation that a test writes by hand. Every message is made of so many letters."""

    def __init__(self, ctx: Context, *, system: bool = True) -> None:
        self.session = Session(ctx, "s-1", KINDS)
        self.session.append(TurnStartEvent(turn=1))
        self._calls = 0
        if system:
            message = SystemMessage(content=(TextBlock(text="s" * 50),))
            self.session.append(SystemMessageEvent(message=message))

    def says(self, size: int, source: str = "user") -> int:
        """Somebody says something in the role of the user. Returns where it stands in the log."""
        message = UserMessage(content=(TextBlock(text="u" * size),))
        return self.session.append(UserMessageEvent(message=message, source=source)).seq

    def replies(self, size: int) -> int:
        message = AssistantMessage(content=(TextBlock(text="a" * size),))
        event = AssistantMessageEvent(message=message, usage=Usage(), finish_reason="stop")
        return self.session.append(event).seq

    def calls(self, size: int, *results: int) -> list[int]:
        """The model calls a tool once for each result, and the results follow."""
        ids = [f"call-{self._calls + number}" for number in range(len(results))]
        self._calls += len(results)
        blocks = tuple(ToolCallBlock(call_id=one, name="read", arguments="{}") for one in ids)
        message = AssistantMessage(content=(TextBlock(text="a" * size), *blocks))
        asked = AssistantMessageEvent(message=message, usage=Usage(), finish_reason="tool-calls")
        seqs = [self.session.append(asked).seq]
        for one, result in zip(ids, results, strict=True):
            answer = ToolMessage(call_id=one, content=(TextBlock(text="r" * result),))
            seqs.append(self.session.append(ToolResultEvent(message=answer)).seq)
        return seqs

    def choose(self, *, retain: int, half: int = FAR, shed: int = 0) -> Region | None:
        """Choose what a summary is to take the place of, counting letters."""
        return choose(self.session, retain=retain, half=half, shed=shed, size=letters)
