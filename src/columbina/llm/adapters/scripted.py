"""A model whose replies are written down in advance, for tests and offline runs.

This module is a plugin. Loading it registers a :class:`Scripted` adapter that
plays the replies given in its config.
"""

import asyncio
import itertools
import json
from collections import deque
from collections.abc import AsyncIterator, Callable, Iterable, Iterator
from typing import Literal, cast

from pydantic import BaseModel, Field, JsonValue

from columbina.data import Data
from columbina.kernel import Context
from columbina.llm.runtime import LLM, LlmError
from columbina.llm.types import (
    BlockEnd,
    BlockStart,
    Chunk,
    Finish,
    LlmFailure,
    LlmRequest,
    ReasoningDelta,
    TextDelta,
    ToolCallDelta,
    Usage,
    UsageChunk,
    UserMessage,
)


class Call(Data):
    """A tool call in a scripted reply.

    Attributes:
        name: The tool to call.
        arguments: The arguments. A mapping is written out as JSON. Text is
            sent as it is, which is how malformed arguments are scripted.
        call_id: The id of the call. By default the calls that one adapter
            sends are numbered ``call-1``, ``call-2`` and so on.
    """

    name: str
    arguments: dict[str, JsonValue] | str = Field(default_factory=dict[str, JsonValue])
    call_id: str | None = None


class Reply(Data):
    """One scripted reply.

    The content is sent in a fixed order: the reasoning, the text, then the
    tool calls.

    Attributes:
        text: The text of the reply.
        reasoning: The reasoning that precedes the text.
        replay_state: The adapter state attached to the reasoning.
        calls: The tool calls.
        usage: The token counts to report, if any.
        finish: The finish reason. By default it is ``tool-calls`` for a reply
            with tool calls and ``stop`` for any other.
        failure: When set, the reply fails with this once its content is sent,
            and ``finish`` is not used.
        hold: Whether the reply pauses once its content is sent, until
            :meth:`Scripted.release` lets it go on.
        when: A text that the user has to have said last in a request for
            this reply to answer it. Such a reply waits in the script for
            its request, whichever request comes before it. Replies with the
            same text are used once each, in the order in which they stand.
            Where two agents ask at once, this is how each gets its own.
        after: A text that the user has to have said last in some request,
            of any agent, before this reply is sent. Until such a request
            has come, the reply waits. Where two agents work at once, this
            is how the one is kept from ending before the other has asked.
    """

    text: str = ""
    reasoning: str = ""
    replay_state: dict[str, JsonValue] | None = None
    calls: tuple[Call, ...] = ()
    usage: Usage | None = None
    finish: Literal["stop", "tool-calls", "max-tokens"] | None = None
    failure: LlmFailure | None = None
    hold: bool = False
    when: str | None = Field(default=None, min_length=1)
    after: str | None = Field(default=None, min_length=1)


Fixed = Reply | list[Chunk] | tuple[Chunk, ...] | Exception
"""A step that does not depend on the request: a reply, raw chunks, or an error to raise."""

Step = Fixed | Callable[[LlmRequest], Fixed]
"""One step of a script: a fixed step, or a function that computes one from the request."""


def _resolve(step: Step, request: LlmRequest) -> Fixed:
    """Return the fixed step that ``step`` stands for when answering ``request``."""
    if isinstance(step, (Reply, Exception)):
        return step
    if isinstance(step, (list, tuple)):
        return cast("list[Chunk] | tuple[Chunk, ...]", step)
    return step(request)


def _said_last(request: LlmRequest) -> str | None:
    """Return what the user said last in a request, or ``None`` if they said nothing."""
    for message in reversed(request.messages):
        if isinstance(message, UserMessage):
            return "".join(block.text for block in message.content)
    return None


def _meant_for(step: Step, said: str | None) -> bool | None:
    """Tell whether a step is meant for a request in which the user said this last.

    Returns:
        ``None`` for a step that is meant for no request in particular.
    """
    if not isinstance(step, Reply) or step.when is None:
        return None
    return said is not None and step.when in said


def _content(reply: Reply, numbers: Iterator[int]) -> Iterator[Chunk]:
    """Yield the chunks of the content of ``reply``, without usage and finish."""
    index = itertools.count()
    if reply.reasoning or reply.replay_state is not None:
        block = next(index)
        yield BlockStart(index=block, kind="reasoning")
        yield ReasoningDelta(index=block, text=reply.reasoning)
        yield BlockEnd(index=block, replay_state=reply.replay_state)
    if reply.text:
        block = next(index)
        yield BlockStart(index=block, kind="text")
        yield TextDelta(index=block, text=reply.text)
        yield BlockEnd(index=block)
    for call in reply.calls:
        block = next(index)
        call_id = call.call_id or f"call-{next(numbers)}"
        arguments = call.arguments
        if not isinstance(arguments, str):
            arguments = json.dumps(arguments, ensure_ascii=False)
        yield BlockStart(index=block, kind="tool-call", call_id=call_id, name=call.name)
        yield ToolCallDelta(index=block, arguments=arguments)
        yield BlockEnd(index=block)


class Scripted:
    """An adapter that plays a script, one step for each call.

    A request is answered by the first reply that is meant for it, see
    :attr:`Reply.when`, and otherwise by the first step that is meant for no
    request in particular. A script that has no step for a request answers
    with the failure ``SCRIPT_EXHAUSTED``.

    Attributes:
        requests: Every request received so far, in order.
    """

    __slots__ = ("_asked", "_held", "_numbers", "_released", "_script", "requests")

    def __init__(self, script: Iterable[Step] = ()) -> None:
        self._script = deque(script)
        self._numbers = itertools.count(1)
        self._held = asyncio.Event()
        self._released = asyncio.Event()
        # Told of every request, for the replies that wait for one: see Reply.after.
        self._asked = asyncio.Condition()
        self.requests: list[LlmRequest] = []

    @property
    def remaining(self) -> int:
        """How many steps have not been played."""
        return len(self._script)

    def add(self, *steps: Step) -> None:
        """Append steps to the script."""
        self._script.extend(steps)

    async def held(self) -> None:
        """Wait until a reply is paused at its hold."""
        await self._held.wait()

    def release(self) -> None:
        """Let the paused reply go on.

        Called while no reply is paused, it lets the next held reply pass
        without pausing.
        """
        self._released.set()

    async def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        """Play the next step of the script."""
        self.requests.append(request)
        async with self._asked:
            self._asked.notify_all()
        step = self._taken(request)
        if step is None:
            raise LlmError(
                LlmFailure(
                    message=f"the script has no reply left for call {len(self.requests)}",
                    code="SCRIPT_EXHAUSTED",
                )
            )
        fixed = _resolve(step, request)
        if isinstance(fixed, Exception):
            raise fixed
        if not isinstance(fixed, Reply):
            for chunk in fixed:
                yield chunk
            return
        if fixed.after is not None:
            await self._after(fixed.after)
        for chunk in _content(fixed, self._numbers):
            yield chunk
        if fixed.hold:
            await self._hold()
        if fixed.usage is not None:
            yield UsageChunk(usage=fixed.usage)
        if fixed.failure is not None:
            yield Finish(reason="error", failure=fixed.failure)
        else:
            yield Finish(reason=fixed.finish or ("tool-calls" if fixed.calls else "stop"))

    def _taken(self, request: LlmRequest) -> Step | None:
        """Take the step that answers a request out of the script, if there is one."""
        said = _said_last(request)
        for wanted in (True, None):
            for at, step in enumerate(self._script):
                if _meant_for(step, said) is wanted:
                    del self._script[at]
                    return step
        return None

    async def _after(self, text: str) -> None:
        """Wait until the user said this last in some request, this one or another."""
        async with self._asked:
            await self._asked.wait_for(
                lambda: any(text in (_said_last(one) or "") for one in self.requests)
            )

    async def _hold(self) -> None:
        self._held.set()
        try:
            await self._released.wait()
        finally:
            self._held.clear()
            self._released.clear()


name = "llm-scripted"
inject = (LLM,)


class Config(BaseModel):
    """The config of the scripted adapter.

    Attributes:
        provider: The provider name under which the adapter is registered.
        replies: The script.
    """

    provider: str = "scripted"
    replies: tuple[Reply, ...] = ()


def apply(ctx: Context, config: Config) -> None:
    """Register an adapter that plays ``config.replies``."""
    ctx.use(LLM).adapters.add(ctx, config.provider, Scripted(config.replies))
