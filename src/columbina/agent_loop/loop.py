"""The loop that drives an agent: turns, steps and requests to the model."""

from __future__ import annotations

import asyncio
import logging
import re
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Literal, TypeVar, cast

from columbina import cancellation
from columbina.agent import (
    CONTEXT_PREFIX,
    CUT_OFF,
    INBOX,
    PRE_STEP,
    REQUEST,
    REQUEST_ERROR,
    STREAM,
    TURN_STOPPING,
    Agent,
    AgentBusyError,
    AgentSpec,
    CancelCause,
    ContextSource,
    Enter,
    ErrorDecision,
    GiveUp,
    InboxItem,
    Reject,
    RequestDraft,
    RequestFailure,
    Retry,
    StepDecision,
    StepInfo,
    StreamFrame,
    TurnStopping,
    arrivals,
    claim_for_step,
    claim_task,
    clear_inbox,
)
from columbina.kernel import Context, Event, FiberState, Registry
from columbina.llm import (
    NOTHING_ARRIVED,
    CallConfig,
    Llm,
    LlmCall,
    LlmFailure,
    LlmRequest,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    UserMessage,
)
from columbina.session import (
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
    ToolCallEvent,
    TurnEndEvent,
    TurnEndReason,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.system_prompt import SYSTEM_PROMPT, SYSTEM_PROMPT_ASSEMBLE
from columbina.text import whole
from columbina.tools import PendingCall, ToolOutcome, ToolRun, Tools

logger = logging.getLogger("columbina.agent_loop")

ResultT = TypeVar("ResultT")

_StepEnd = Literal["stop", "max-tokens", "tools", "concluded"]
"""How a step ended.

The model was done, its reply was cut off by the token limit, its tool calls
were carried out and it has to be asked again, or one of the tools asked for
the turn to end.
"""

_NOT_RUN = "The turn was cancelled before this call ran."
_CUT_SHORT = "The call was cancelled while it ran. It may have taken effect in part."

_CUT_OFF = "Your reply was cut off: it reached the limit on the length of one reply."
_PLAIN_NAME = re.compile("[A-Za-z0-9_.-]{1,64}")
"""What the name of a tool may look like where it is repeated to a model."""


def _cut_off(unfinished: tuple[str, ...]) -> str:
    """Return what a model is told when its reply was cut off by the limit on its length.

    Args:
        unfinished: The names of the tool calls that the reply was writing.
            None of them was carried out.
    """
    if not unfinished:
        return f"{_CUT_OFF} Go on from where it stopped, and do not repeat what is already written."
    # A name is whatever the model sent, and here it goes into a message. One that no tool
    # could have is not repeated.
    plain = dict.fromkeys(f"`{name}`" for name in unfinished if _PLAIN_NAME.fullmatch(name))
    if len(plain) > 1:
        calls = f"The calls of {', '.join(plain)} were not complete and were"
        again = "Make them again so that each fits"
    else:
        calls = f"The call of {next(iter(plain), 'a tool')} was not complete and was"
        again = "Make it again so that it fits"
    return (
        f"{_CUT_OFF} {calls} not carried out. {again} into one reply: what is long, such as "
        "a file, is written in several steps."
    )


def _last_header(
    state: RequestHeaderEvent | None, event: SessionEvent[EventData]
) -> RequestHeaderEvent | None:
    return event.data if isinstance(event.data, RequestHeaderEvent) else state


_HEADER: Projection[RequestHeaderEvent | None] = Projection("request-header", None, _last_header)

_CONTEXT = CONTEXT_PREFIX


def _last_context(
    state: Mapping[str, SessionEvent[EventData]], event: SessionEvent[EventData]
) -> Mapping[str, SessionEvent[EventData]]:
    data = event.data
    if isinstance(data, UserMessageEvent) and data.source.startswith(_CONTEXT):
        return {**state, data.source: event}
    return state


_GIVEN: Projection[Mapping[str, SessionEvent[EventData]]] = Projection(
    "context-given", {}, _last_context
)
"""The last message that each source of context gave, by the source of the message."""


def _framed(name: str, text: str) -> UserMessage:
    """Return the message that carries the text of a source of context.

    The text stands between two tags, so that the model can tell it from what
    the user says. Whatever in the text looks like the closing tag is taken
    apart, so that nothing in a file can end the frame early.
    """
    body = text.strip().replace("</context", "<\\/context")
    framed = f'<context source="{name}">\n{body}\n</context>'
    return UserMessage(content=(TextBlock(text=framed),))


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


async def _enter(info: StepInfo) -> StepDecision:
    return Enter()


async def _as_drafted(draft: RequestDraft) -> CallConfig:
    return draft.config


async def _give_up(failure: RequestFailure) -> ErrorDecision:
    return GiveUp()


@dataclass(frozen=True, slots=True)
class _Work:
    """A piece of work that is to be done between two turns.

    Attributes:
        job: Does the work.
        done: Gives whoever asked for it what came of it.
    """

    job: Callable[[], Awaitable[object]]
    done: asyncio.Future[object]


async def _did(job: Callable[[], Awaitable[ResultT]]) -> ResultT:
    return await job()


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
        self,
        ctx: Context,
        spec: AgentSpec,
        session: Session,
        llm: Llm,
        tools: Tools,
        context: Registry[ContextSource],
        max_parallel_tool_calls: int,
        continuations: int,
    ) -> None:
        super().__init__(ctx, spec, session)
        self._llm = llm
        self._tools = tools
        self._context = context
        self._max_parallel = max_parallel_tool_calls
        self._continuations = continuations
        # The tool calls that the last reply was writing when it was cut off.
        self._unfinished: tuple[str, ...] = ()
        self._wake = asyncio.Event()
        self._idle = asyncio.Event()
        self._idle.set()
        self._closing = False
        self._halted = False
        self._failure: Exception | None = None
        self._cause: CancelCause | None = None
        # How often messages of the session had been replaced when the context was last seen to.
        self._replaced = 0
        self._work: _Work | None = None
        # The turn that is running, or the work between turns that is being done.
        self._turn_task: asyncio.Task[object] | None = None
        self._driver = asyncio.ensure_future(self._drive())
        ctx.effect(lambda: self._stop)

    # The interface

    def wake(self) -> None:
        """Tell the agent that input may be waiting for it."""
        if self._closing or self._failure is not None or self._driver.done():
            return
        self._halted = False
        self._idle.clear()
        self._wake.set()

    def cancel(self, cause: CancelCause = "user", *, keep_inbox: bool = False) -> None:
        """Stop the turn that is running, if any. This only asks: wait with :meth:`when_idle`.

        Until the agent is woken again it starts no further turn, even if
        tasks wait. Of several causes the first one is recorded.

        Raises:
            SessionStoreError: The input that waited was dropped, and that
                could not be written down. The turn is stopped all the same.
        """
        try:
            if not keep_inbox and not self.session.closed:
                clear_inbox(self.session)
        finally:
            # A log that takes nothing more fails the line above. Whoever stops a turn
            # then may have no other way left to stop it, so it is stopped whatever happens.
            self._halted = True
            self._wake.clear()
            self._drop_work()
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

    @property
    def at_rest(self) -> bool:
        """Whether the agent is at rest at this moment."""
        return self._idle.is_set()

    def run_maintenance(self, job: Callable[[], Awaitable[ResultT]]) -> Awaitable[ResultT]:
        """Do a piece of work on the conversation between two turns.

        Raises:
            AgentBusyError: The agent is not at rest, or it has ended.
        """
        if self._ending() or self._failure is not None or self._driver.done():
            raise AgentBusyError(f"the agent {self.id!r} has ended")
        if not self._idle.is_set():
            raise AgentBusyError(f"the agent {self.id!r} is not at rest")
        done: asyncio.Future[object] = asyncio.get_running_loop().create_future()
        self._work = _Work(job, done)
        # The driver does it, as it runs the turns: one thing at a time writes the conversation.
        self._idle.clear()
        self._wake.set()
        return cast("Awaitable[ResultT]", done)

    # The driver

    def _ending(self) -> bool:
        """Tell whether the agent is being ended.

        An agent begins to end when its fiber begins to unload. What was added
        to the agent after it was made is undone first, which may take a while,
        and only then is the loop stopped. Nothing new begins in between: a
        turn that runs goes on until the loop is stopped, no turn follows it,
        and input that waits is kept for whoever takes the session up again.
        """
        return self.ctx.fiber.state is FiberState.UNLOADING

    async def _stop(self) -> None:
        """End the agent: stop the turn that is running, then the driver."""
        self._closing = True
        self.cancel("disposed", keep_inbox=True)
        self._wake.set()
        await asyncio.gather(self._driver, return_exceptions=True)

    async def _drive(self) -> None:
        try:
            while True:
                await self._wake.wait()
                self._wake.clear()
                if self._closing:
                    return
                work, self._work = self._work, None
                if work is not None:
                    await self._do(work)
                # A wake that a cancellation has taken back since is not acted on.
                if not self._halted:
                    try:
                        await self._run_turns()
                    except Exception as error:
                        logger.exception("the agent %r has stopped", self.id)
                        self._failure = error
                        return
                if not self._wake.is_set():
                    self._idle.set()
        finally:
            # However the driver ends, nobody is left waiting for an agent that is gone.
            self._idle.set()
            self._drop_work()

    def _drop_work(self) -> None:
        """Give up the work that was taken and has not begun, and tell whoever waits for it."""
        work, self._work = self._work, None
        if work is not None:
            work.done.cancel()

    async def _do(self, work: _Work) -> None:
        """Do a piece of work in a task of its own, so that it can be stopped like a turn."""
        self._cause = None
        task = self._turn_task = asyncio.ensure_future(_did(work.job))
        before = cancellation.requests()
        try:
            result = await task
        except asyncio.CancelledError:
            # Either the driver is being cancelled, or only the work was stopped.
            if cancellation.requested_since(before):
                raise
        except Exception as error:
            # Whoever asked may have stopped waiting. The work was done for the agent all the same.
            if not work.done.done():
                work.done.set_exception(error)
        else:
            if not work.done.done():
                work.done.set_result(result)
        finally:
            self._turn_task = None
            # However it ended, nobody is left waiting for it.
            work.done.cancel()

    async def _run_turns(self) -> None:
        """Run a turn for the input that waits, and then one for every further task."""
        first = True
        while not self._ending() and not self._halted:
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
        detail: str | None
        cancelled_by_others = False
        try:
            reason, detail = await self._steps(turn)
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

    async def _steps(self, turn: int) -> tuple[TurnEndReason, str | None]:
        """Run the steps of a turn and return why the turn ends."""
        session = self.session
        step = 0
        taken_up = 0
        while True:
            opening: Exception | None = None
            if step == 0:
                try:
                    await self._reconcile_system_message()
                    self._write_context(await self._ask_for_context())
                except Exception as error:
                    # The input is taken all the same. Left in the inbox, it would begin
                    # another turn at once, and that one would fail the same way.
                    opening = error
            for item in self._claim(first_step=step == 0):
                session.append(UserMessageEvent(message=item.message, source=item.source))
            if opening is not None:
                raise opening
            decision = cast(
                "object",
                await self._heard(
                    PRE_STEP, self.ctx.waterfall(PRE_STEP, StepInfo(self, turn, step + 1), _enter)
                ),
            )
            if isinstance(decision, Reject):
                return "blocked", decision.reason
            if not isinstance(decision, Enter):
                # No answer is no permission.
                return "blocked", "a listener of agent/pre-step gave no decision"
            step += 1
            session.append(StepStartEvent(turn=turn, step=step))
            try:
                ended = await self._step(turn, step)
            finally:
                session.append(StepEndEvent(turn=turn, step=step))
            if ended == "tools":
                continue
            if ended == "concluded":
                # A tool asked for the turn to end. Input that waits stays for the next turn.
                return "completed", None
            if INBOX.of(session).next_step:
                continue
            if ended == "max-tokens" and taken_up < self._continuations:
                # The model is told what happened and goes on, a few times in a turn at
                # most. What it does next is a step like any other.
                taken_up += 1
                note = _framed(CUT_OFF.removeprefix(_CONTEXT), _cut_off(self._unfinished))
                session.append(UserMessageEvent(message=note, source=CUT_OFF))
                continue
            # The turn would end here, as its last reply did. A listener that steers the
            # agent keeps it going.
            reason: TurnEndReason = "max-tokens" if ended == "max-tokens" else "completed"
            stopping = TurnStopping(self, turn, reason)
            await self._heard(TURN_STOPPING, self.ctx.serial(TURN_STOPPING, stopping))
            if not INBOX.of(session).next_step:
                return reason, None

    def _claim(self, *, first_step: bool) -> tuple[InboxItem, ...]:
        """Take from the inbox what a step begins with.

        A turn that is being cancelled takes nothing more. What waits at that
        moment was kept on purpose, or was given after the cancellation, and
        either way it is for a later turn. The cancellation may be asked for
        by a listener of an event that is written here, so it is looked for
        before each of the two moves.
        """
        if cancellation.requests():
            return ()
        came = arrivals(self.session).next_step
        claimed = claim_for_step(self.session)
        if first_step and not cancellation.requests():
            # The task stands where it came: after what waited for it, and before what
            # was said once it was given. A turn may be slow to begin, and what a person
            # says meanwhile is said about the task.
            first = arrivals(self.session).next_turn[:1]
            earlier = sum(place < first[0] for place in came) if first else 0
            claimed = (*claimed[:earlier], *claim_task(self.session), *claimed[earlier:])
        return claimed

    async def _heard(self, event: Event, answer: Awaitable[ResultT]) -> ResultT:
        """Wait for the listeners of ``event`` and return their answer.

        A listener that raises a cancellation of its own has failed, like one
        that raises anything else. Without this, the turn would be recorded as
        cancelled although nobody cancelled it.
        """
        return await cancellation.guard(answer, what=f"a listener of {event.name}")

    async def _reconcile_system_message(self) -> None:
        """Make the first message the model sees the instructions of this turn.

        They are the instructions of the spec, with what plugins add to them
        if the system prompt service is there. They are asked for once, when
        a turn begins: the beginning of a conversation that stays the same is
        what a provider can keep from one request to the next.
        """
        session = self.session
        prompt = self.spec.system_prompt
        assembling = self.ctx.get(SYSTEM_PROMPT)
        if assembling is not None:
            prompt = await self._heard(SYSTEM_PROMPT_ASSEMBLE, assembling.render(self.ctx, prompt))
        if not prompt:
            return
        wanted = SystemMessage(content=(TextBlock(text=prompt),))
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

    async def _restore_context(self) -> None:
        """Give the model back the context that was taken from what it sees.

        Whatever makes a conversation shorter puts a summary in the place of
        its beginning, and context that stood there is gone with it. That may
        happen in the middle of a turn, and the model is about to be asked.
        So the sources are asked again, as when a turn begins. The ones that
        give word of something that happened are not: such word is given
        once. Nothing is asked if no message was replaced since the context
        was last seen to, or if the loop was the one that replaced it.
        """
        if self.session.replaced != self._replaced:
            self._write_context(await self._ask_for_context(notes=False))

    async def _ask_for_context(self, *, notes: bool = True) -> list[tuple[str, ContextSource, str]]:
        """Ask every source of context for its text, and return the texts that were given.

        All are asked before anything is written. If one fails, the turn has
        no context at all, and it ends before the model is asked.

        Args:
            notes: Whether the sources that give word of something that
                happened are asked as well.
        """
        ordered = sorted(self._context.items(), key=lambda item: (item[1].order, item[0]))
        given: list[tuple[str, ContextSource, str]] = []
        for name, source in ordered:
            if source.kind == "note" and not notes:
                continue
            what = f"the source of context {name!r}"
            text = cast("object", await cancellation.guard(source.render(self), what=what))
            if text is None:
                continue
            if not isinstance(text, str):
                raise TypeError(f"{what} gave {text!r}, which is no text")
            given.append((name, source, text))
        return given

    def _write_context(self, given: list[tuple[str, ContextSource, str]]) -> None:
        """Write what the sources of context gave, as far as it is news to the model.

        Text that a source gave before is news again if the model no longer
        sees it.
        """
        session = self.session
        for name, source, text in given:
            # A source tells what it found, such as the name of a directory, as it found it.
            framed = _framed(name, whole(text))
            written = UserMessageEvent(message=framed, source=_CONTEXT + name)
            last = _GIVEN.of(session).get(written.source)
            seen = last is not None and last.seq in session.surface
            if source.kind == "note":
                session.append(written)
            elif last is None or last.data != written or not seen:
                if source.kind == "replace" and last is not None and seen:
                    # The old text is replaced where it stands, as the model still sees it there.
                    session.append(written, replaces=[last.seq])
                else:
                    session.append(written)
        # What the loop itself has just replaced takes no context from the model.
        self._replaced = session.replaced

    async def _step(self, turn: int, step: int) -> _StepEnd:
        """Ask the model, carry out the tool calls of its reply, and return how the step ended."""
        session = self.session
        attempt = 0
        while True:
            attempt += 1
            call = await self._request(turn, step, attempt)
            finish = call.finish
            assert finish is not None
            if finish.failure is None:
                break
            session.append(
                AssistantAttemptEvent(
                    message=call.message, usage=call.usage, failure=finish.failure
                )
            )
            failed = RequestFailure(self, turn, step, attempt, finish.failure)
            decision = cast(
                "object",
                await self._heard(
                    REQUEST_ERROR, self.ctx.waterfall(REQUEST_ERROR, failed, _give_up)
                ),
            )
            if not isinstance(decision, Retry):
                # Only an explicit answer makes the request again.
                raise _RequestError(finish.failure)
        assert finish.reason != "error"
        message = call.message
        if finish.reason == "max-tokens":
            self._unfinished = call.unfinished
            if not message.content:
                # Nothing arrived that can be kept, and a message that says nothing is put
                # before no model. What the request cost is written down all the same.
                session.append(
                    AssistantAttemptEvent(
                        message=message, usage=call.usage, failure=NOTHING_ARRIVED
                    )
                )
                return "max-tokens"
        session.append(
            AssistantMessageEvent(message=message, usage=call.usage, finish_reason=finish.reason)
        )
        if finish.reason == "max-tokens":
            return "max-tokens"
        if not message.tool_calls:
            return "stop"
        concluded = await self._run_tools(message.tool_calls)
        return "concluded" if concluded else "tools"

    async def _request(self, turn: int, step: int, attempt: int) -> LlmCall:
        """Ask the model once and return the call when its reply is over.

        The request is derived from the log. Context that was taken from
        what the model sees is given back first. If the reader is cancelled,
        what had arrived of the reply is written down before the cancellation
        goes on.
        """
        session = self.session
        await self._restore_context()
        draft = RequestDraft(self, turn, step, attempt, self.spec.model)
        config = await self._heard(REQUEST, self.ctx.waterfall(REQUEST, draft, _as_drafted))
        schemas = self._tools.schemas(self.spec.tools)
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
                # The tool calls it holds are not carried out, but each gets its result.
                message = call.message
                session.append(
                    AssistantMessageEvent(message=message, usage=call.usage, finish_reason=None)
                )
                for asked in message.tool_calls:
                    session.append(self._not_run(asked).to_event())
                raise
        return call

    # Tool calls

    def _begin(self, call: ToolCallBlock) -> PendingCall:
        """Write a tool call down and take it up."""
        session = self.session
        session.append(
            ToolCallEvent(call_id=call.call_id, name=call.name, arguments=call.arguments)
        )
        run = ToolRun(call.call_id, self.cwd, session)
        return self._tools.begin(self.ctx, call, run, self.spec.tools)

    def _not_run(self, call: ToolCallBlock) -> ToolOutcome:
        """Write down a tool call that will not be carried out, and return its result."""
        pending = self._begin(call)
        return pending.outcome or pending.fail("ABORTED_BEFORE_DISPATCH", _NOT_RUN)

    async def _run_tools(self, calls: tuple[ToolCallBlock, ...]) -> bool:
        """Carry out the tool calls of a reply. Return whether a tool asked for the turn to end.

        The calls are taken up in the order of the model, and the checks of
        each are asked before the next one is looked at. Calls that are safe
        beside each other then overlap, up to a limit. A call that is not
        waits until nothing else runs, and nothing starts until it is done.

        The checks of a call are asked when it can run, and it runs as soon
        as they have let it through. So what they went by has no time to
        change in between: no other call is waited for after the checks.

        The results are written in the order of the model, each as soon as
        all before it are in. If the turn is cancelled, the tools that run
        are stopped, and every call still gets a result.
        """
        session = self.session
        pendings: list[PendingCall | None] = [None] * len(calls)
        outcomes: dict[int, ToolOutcome] = {}
        running: dict[asyncio.Task[ToolOutcome], int] = {}
        started: set[int] = set()
        written = 0

        def write() -> None:
            nonlocal written
            while written in outcomes:
                session.append(outcomes[written].to_event())
                written += 1

        def finished(task: asyncio.Task[ToolOutcome]) -> None:
            index = running.pop(task)
            if not task.cancelled():
                outcomes[index] = task.result()
                write()

        async def until_running(count: int) -> None:
            while len(running) > count:
                await asyncio.wait(running, return_when=asyncio.FIRST_COMPLETED)

        try:
            for index, call in enumerate(calls):
                pending = pendings[index] = self._begin(call)
                if pending.outcome is None:
                    room = self._max_parallel - 1 if pending.concurrency_safe else 0
                    await until_running(room)
                    await session.flush()
                    await pending.authorize()
                if pending.outcome is not None:
                    outcomes[index] = pending.outcome
                    write()
                elif pending.concurrency_safe:
                    started.add(index)
                    task = asyncio.ensure_future(pending.execute())
                    running[task] = index
                    task.add_done_callback(finished)
                else:
                    started.add(index)
                    outcomes[index] = await pending.execute()
                    write()
            await until_running(0)
        except asyncio.CancelledError:
            stopping = list(running)
            for task in stopping:
                task.cancel()
            try:
                if stopping:
                    await asyncio.wait(stopping)
            finally:
                # Whether or not the tools have come to rest, every call gets its result
                # now: a second cancellation must not leave a call without one.
                for index, call in enumerate(calls):
                    if index in outcomes:
                        continue
                    pending = pendings[index]
                    if pending is None:
                        write()
                        outcomes[index] = self._not_run(call)
                    elif index in started:
                        outcomes[index] = pending.outcome or pending.fail(
                            "TOOL_CANCELLED", _CUT_SHORT
                        )
                    else:
                        outcomes[index] = pending.outcome or pending.fail(
                            "ABORTED_BEFORE_DISPATCH", _NOT_RUN
                        )
                write()
            raise
        return any(outcome.concludes_turn for outcome in outcomes.values())
