"""A scripted model behind a service that is told how much to reason, and what a request said."""

from collections.abc import AsyncIterator

from columbina.llm import Chunk, LlmRequest, SystemMessage, UserMessage
from columbina.llm.adapters.scripted import Scripted

__all__ = ["Thinking", "instructions_now", "system_text"]


class Thinking:
    """The scripted model, behind a service that is told how much to reason as DeepSeek is.

    Or as another style says, where a test names one.
    """

    def __init__(self, scripted: Scripted, style: str = "deepseek") -> None:
        self._scripted = scripted
        self.reasoning_style = style

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        return self._scripted.stream(request)


def system_text(request: LlmRequest) -> str:
    """Return the instructions that open the conversation of a request."""
    first = request.messages[0]
    assert isinstance(first, SystemMessage)
    return "\n".join(block.text for block in first.content)


_INSTEAD = '<context source="system-prompt">'


def instructions_now(request: LlmRequest) -> str:
    """Return the instructions that hold at the end of the conversation of a request.

    They are those it opens with, or the last that were said to take their place.
    """
    for message in reversed(request.messages):
        if isinstance(message, UserMessage):
            text = "\n".join(block.text for block in message.content)
            if text.startswith(_INSTEAD):
                return text
    return system_text(request)
