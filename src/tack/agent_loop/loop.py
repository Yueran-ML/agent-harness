"""The loop that drives an agent: turns, steps and requests to the model."""

from __future__ import annotations

import asyncio
import logging
from typing import Literal

from tack import cancellation
from tack.agent import (
    INBOX,
    STREAM,
    Agent,
    AgentSpec,
    CancelCause,
    StreamFrame,
    claim_for_step,
    claim_for_turn,
    clear_inbox,
)
from tack.kernel import Context
from tack.llm import Llm, LlmFailure, LlmRequest, SystemMessage, TextBlock
from tack.session import (
    TURN,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    Projection,
    RequestHeaderEvent,
    Session,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    SystemMessageEvent,
    TurnEndEvent,
    TurnEndReason,
    TurnStartEvent,
    UserMessageEvent,
)
from tack.tools import Tools

logger = logging.getLogger("tack.agent_loop")

_StepEnd = Literal["stop", "max-tokens"]
"""How a step ended: the model was done, or its reply was cut off by the token limit."""


def _last_header(
    state: RequestHeaderEvent | None, event: SessionEvent[EventData]
) -> RequestHeaderEvent | None:
    return event.data if isinstance(event.data, RequestHeaderEvent) else state


_HEADER: Projection[RequestHeaderEvent | None] = Projection("request-header", None, _last_header)


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


class _RequestError(Exception):
    """A call to the model failed for good."""

    def __init__(self, failure: LlmFailure) -> None:
        super().__init__(f"{failure.code}: {failure.message}")
        self.failure = failure


class LoopAgent(Agent):
    """An agent that is driven by the standard loop.

    A driver task waits for input. For every task that waits it runs a turn,
    in a task of its own, so that cancelling a turn stops that turn and
    nothing else. Everything a turn does is written to the session log as it
    happens, and every request to the model is derived from the log.
    """

    def __init__(
        self, ctx: Context, spec: AgentSpec, session: Session, llm: Llm, tools: Tools
    ) -> None:
        super().__init__(ctx, spec, session)
        self._llm = llm
        self._tools = tools
        self._wake = asyncio.Event()
        self._idle = asyncio.Event()
        self._idle.set()
        self._closing = False
        self._halted = False
        self._failure: Exception | None = None
        self._cause: CancelCause | None = None
        self._turn_task: asyncio.Task[None] | None = None
        self._driver = asyncio.ensure_future(self._drive())
        ctx.effect(lambda: self._stop)

    # The interface

    def wake(self) -> None:
        """Tell the agent that input may be waiting for it."""
        if self._closing or self._failure is not None:
            return
        self._idle.clear()
        self._wake.set()

    def cancel(self, cause: CancelCause = "user", *, keep_inbox: bool = False) -> None:
        """Stop the turn that is running, if any. This only asks: wait with :meth:`when_idle`.

        Until the agent is woken again it starts no further turn, even if
        tasks wait. Of several causes the first one is recorded.
        """
        if not keep_inbox and not self.session.closed:
            clear_inbox(self.session)
        self._halted = True
        self._wake.clear()
        task = self._turn_task
        if task is None:
            self._idle.set()
        elif self._cause is None:
            self._cause = cause
            task.cancel()

    async def when_idle(self) -> None:
        """Wait until the agent is at rest.

        Raises:
            Exception: The agent has stopped for good, because a turn could
                not be written down. This is the error that stopped it.
        """
        await self._idle.wait()
        if self._failure is not None:
            raise self._failure

    # The driver

    async def _stop(self) -> None:
        """End the agent: stop the turn that is running, then the driver."""
        self._closing = True
        self.cancel("disposed", keep_inbox=True)
        self._wake.set()
        await asyncio.gather(self._driver, return_exceptions=True)

    async def _drive(self) -> None:
        while True:
            await self._wake.wait()
            self._wake.clear()
            if self._closing:
                return
            try:
                await self._run_turns()
            except Exception as error:
                logger.exception("the agent %r has stopped", self.id)
                self._failure = error
                self._idle.set()
                return
            if not self._wake.is_set():
                self._idle.set()

    async def _run_turns(self) -> None:
        """Run a turn for the input that waits, and then one for every further task."""
        self._halted = False
        first = True
        while not self._closing and not self._halted:
            inbox = INBOX.of(self.session)
            if not (inbox.next_turn or (first and inbox.next_step)):
                return
            first = False
            self._cause = None
            task = self._turn_task = asyncio.ensure_future(self._turn())
            before = cancellation.requests()
            try:
                await task
            except asyncio.CancelledError:
                # Either the driver is being cancelled, or only the turn ended cancelled.
                if cancellation.requested_since(before):
                    raise
            finally:
                self._turn_task = None

    # A turn

    async def _turn(self) -> None:
        session = self.session
        turn = TURN.of(session).turn + 1
        session.append(TurnStartEvent(turn=turn))
        reason: TurnEndReason
        detail: str | None = None
        cancelled_by_others = False
        try:
            reason = await self._steps(turn)
        except asyncio.CancelledError:
            reason, detail = "aborted", self._cause or "disposed"
            # A cancellation that this agent asked for ends here. Any other one goes on.
            others = cancellation.current_task().uncancel() if self._cause else 1
            cancelled_by_others = others > 0
        except _RequestError as failed:
            reason, detail = "error", str(failed)
        except Exception as error:
            logger.exception("turn %d of the agent %r failed", turn, self.id)
            reason, detail = "error", _describe(error)
        session.append(TurnEndEvent(turn=turn, reason=reason, detail=detail))
        await session.flush()
        if cancelled_by_others:
            raise asyncio.CancelledError

    async def _steps(self, turn: int) -> TurnEndReason:
        """Run the steps of a turn and return why the turn ends."""
        session = self.session
        step = 0
        cut_off = False
        while True:
            self._reconcile_system_message()
            claimed = claim_for_turn(session) if step == 0 else claim_for_step(session)
            for item in claimed:
                session.append(UserMessageEvent(message=item.message, source=item.source))
            step += 1
            session.append(StepStartEvent(turn=turn, step=step))
            try:
                ended = await self._request(turn, step)
            finally:
                session.append(StepEndEvent(turn=turn, step=step))
            cut_off = cut_off or ended == "max-tokens"
            if not INBOX.of(session).next_step:
                return "max-tokens" if cut_off else "completed"

    def _reconcile_system_message(self) -> None:
        """Make the first message the model sees the instructions of the spec."""
        session = self.session
        if not self.spec.system_prompt:
            return
        wanted = SystemMessage(content=(TextBlock(text=self.spec.system_prompt),))
        if not session.surface:
            session.append(SystemMessageEvent(message=wanted))
            return
        first = session.events[session.surface[0]]
        if not isinstance(first.data, SystemMessageEvent):
            logger.warning(
                "the session %r has history without instructions, so none are added", session.id
            )
        elif first.data.message != wanted:
            session.append(SystemMessageEvent(message=wanted), replaces=[first.seq])

    async def _request(self, turn: int, step: int) -> _StepEnd:
        """Ask the model once, write the reply down, and return how the step ended."""
        session = self.session
        config = self.spec.model
        schemas = self._tools.schemas()
        header = _HEADER.of(session)
        if header is None or header.config != config or header.tools != schemas:
            session.append(RequestHeaderEvent(config=config, tools=schemas))
        request = LlmRequest(config=config, messages=session.messages, tools=schemas)
        await session.flush()
        async with self._llm.call(request) as call:
            try:
                async for chunk in call:
                    self.ctx.emit(STREAM, StreamFrame(self, turn, step, chunk))
            except asyncio.CancelledError:
                # The reply was cut short: what had arrived is kept, marked as interrupted.
                session.append(
                    AssistantMessageEvent(
                        message=call.message, usage=call.usage, finish_reason=None
                    )
                )
                raise
        finish = call.finish
        assert finish is not None
        if finish.failure is not None:
            session.append(
                AssistantAttemptEvent(
                    message=call.message, usage=call.usage, failure=finish.failure
                )
            )
            raise _RequestError(finish.failure)
        assert finish.reason != "error"
        session.append(
            AssistantMessageEvent(
                message=call.message, usage=call.usage, finish_reason=finish.reason
            )
        )
        return "max-tokens" if finish.reason == "max-tokens" else "stop"
