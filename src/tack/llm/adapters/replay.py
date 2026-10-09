"""A model that gives the replies of a recorded session again.

This module is a plugin. It reads the session stored in a directory and
registers an adapter that answers each call with the next reply of that
session: what the model said, the token counts, and how the reply ended. A
recorded session is then both the input and the expected result of a test, and
no key is needed to run it.
"""

from collections.abc import AsyncIterator, Sequence
from pathlib import Path

from pydantic import BaseModel

from tack.kernel import Context
from tack.llm.adapters.scripted import Scripted
from tack.llm.runtime import LLM
from tack.llm.types import (
    AssistantMessage,
    BlockEnd,
    BlockStart,
    Chunk,
    Finish,
    LlmRequest,
    ReasoningBlock,
    ReasoningDelta,
    TextBlock,
    TextDelta,
    ToolCallDelta,
    Usage,
    UsageChunk,
)
from tack.session import (
    SESSIONS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    Sessions,
    read_session,
)

__all__ = ["Config", "Replay", "apply", "inject", "name", "replies_of"]

name = "llm-replay"
inject = (LLM, SESSIONS)


class Config(BaseModel):
    """The config of the replay adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        directory: The directory of the stored session to replay.
    """

    provider: str
    directory: Path


def _chunks(message: AssistantMessage, usage: Usage, finish: Finish) -> list[Chunk]:
    """Return a reply as the chunks that would assemble into it."""
    chunks: list[Chunk] = []
    for index, block in enumerate(message.content):
        if isinstance(block, ReasoningBlock):
            chunks += [
                BlockStart(index=index, kind="reasoning"),
                ReasoningDelta(index=index, text=block.text),
                BlockEnd(index=index, replay_state=block.replay_state),
            ]
        elif isinstance(block, TextBlock):
            chunks += [
                BlockStart(index=index, kind="text"),
                TextDelta(index=index, text=block.text),
                BlockEnd(index=index),
            ]
        else:
            chunks += [
                BlockStart(index=index, kind="tool-call", call_id=block.call_id, name=block.name),
                ToolCallDelta(index=index, arguments=block.arguments),
                BlockEnd(index=index),
            ]
    return [*chunks, UsageChunk(usage=usage), finish]


def replies_of(events: Sequence[SessionEvent[EventData]]) -> list[list[Chunk]]:
    """Return the replies that a session records, each as the chunks of one call.

    A call that failed is replayed as the same failure, after what had arrived
    of it.

    Raises:
        ValueError: The session holds a reply that was interrupted. When it
            was cut short depended on the moment, which a replay cannot know.
    """
    replies: list[list[Chunk]] = []
    for event in events:
        data = event.data
        if isinstance(data, AssistantAttemptEvent):
            finish = Finish(reason="error", failure=data.failure)
            replies.append(_chunks(data.message, data.usage, finish))
        elif isinstance(data, AssistantMessageEvent):
            if data.finish_reason is None:
                raise ValueError(
                    f"event {event.seq} is a reply that was interrupted, which cannot be replayed"
                )
            replies.append(_chunks(data.message, data.usage, Finish(reason=data.finish_reason)))
    return replies


class Replay:
    """An adapter that plays the replies of a stored session, in order.

    The session is read when the first call is made, so that every plugin has
    registered its event types by then.
    """

    __slots__ = ("_directory", "_script", "_sessions")

    def __init__(self, sessions: Sessions, directory: Path) -> None:
        self._sessions = sessions
        self._directory = directory
        self._script: Scripted | None = None

    @property
    def requests(self) -> list[LlmRequest]:
        """Every request received so far, in order."""
        return [] if self._script is None else self._script.requests

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Play the next reply of the session."""
        if self._script is None:
            stored = read_session(self._directory, self._sessions.kinds)
            self._script = Scripted(replies_of(stored.events))
        return self._script.stream(request)


def apply(ctx: Context, config: Config) -> None:
    """Register an adapter that replays the session of ``config``."""
    ctx.use(LLM).adapters.add(ctx, config.provider, Replay(ctx.use(SESSIONS), config.directory))
