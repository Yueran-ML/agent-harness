"""Whatever is asked of the loop, and whenever it is cancelled, the log holds together.

Each example builds a real program around a scripted model: session log, tool
pipeline, agent service and loop. It then hands the agent a random series of
tasks, steers, injected context and cancellations, some of them asked for from
inside a listener while the turn is running, and checks what is left behind.
"""

import asyncio
import logging
import tempfile
from collections.abc import Callable, Collection, Sequence
from itertools import pairwise
from pathlib import Path

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from pydantic import BaseModel

import columbina.agent
import columbina.agent_loop
import columbina.llm
import columbina.session
import columbina.tools
from columbina.agent import AGENTS, INBOX, STREAM, Agent, AgentSpec, StreamFrame
from columbina.kernel import Context
from columbina.llm import (
    LLM,
    AssistantMessage,
    LlmFailure,
    Message,
    SystemMessage,
    ToolMessage,
    UserMessage,
)
from columbina.llm.adapters.scripted import Call, Reply, Scripted
from columbina.session import (
    SESSION_EVENT,
    SESSIONS,
    TURN,
    AssistantMessageEvent,
    Committed,
    EventData,
    Session,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
    closing_events,
)
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun
from support.loop import MODEL


class Args(BaseModel):
    text: str = ""


class Bench:
    """The tools of one example. They write down any overlap that the loop must not allow."""

    def __init__(self) -> None:
        self.shared = 0
        self.alone = 0
        self.concluded: set[str] = set()
        self.problems: list[str] = []
        self.on_start: Callable[[], None] = lambda: None

    async def echo(self, args: Args, run: ToolRun) -> str:
        self.on_start()
        if self.alone:
            self.problems.append("a call began while a call that must be alone was running")
        self.shared += 1
        if self.shared > PARALLEL:
            self.problems.append(f"{self.shared} calls ran beside each other")
        try:
            await asyncio.sleep(0)
            await asyncio.sleep(0)
        finally:
            self.shared -= 1
        return args.text

    async def slow(self, args: Args, run: ToolRun) -> str:
        self.on_start()
        if self.shared or self.alone:
            self.problems.append("a call that must be alone began beside another")
        self.alone += 1
        try:
            for _ in range(3):
                await asyncio.sleep(0)
        finally:
            self.alone -= 1
        return args.text

    async def fail(self, args: Args, run: ToolRun) -> str:
        raise ToolFailure("it did not work")

    async def conclude(self, args: Args, run: ToolRun) -> str:
        if self.shared or self.alone:
            self.problems.append("a call that must be alone began beside another")
        run.conclude_turn()
        self.concluded.add(run.call_id)
        return "done"

    def tools(self) -> tuple[Tool[Args, str], ...]:
        return (
            Tool(
                name="echo",
                description="Echo.",
                parameters=Args,
                execute=self.echo,
                concurrency_safe=True,
            ),
            Tool(name="slow", description="Take a while.", parameters=Args, execute=self.slow),
            Tool(
                name="fail",
                description="Fail.",
                parameters=Args,
                execute=self.fail,
                concurrency_safe=True,
            ),
            Tool(
                name="conclude", description="End the turn.", parameters=Args, execute=self.conclude
            ),
        )


PARALLEL = 2

calls = st.sampled_from(
    [
        Call(name="echo"),
        Call(name="echo", arguments={"text": "hello"}),
        Call(name="slow"),
        Call(name="slow", arguments={"text": "alone"}),
        Call(name="fail"),
        Call(name="conclude"),
        Call(name="nowhere"),
        Call(name="echo", arguments='{"text": '),
    ]
)

calling = st.builds(
    Reply,
    text=st.sampled_from(["", "Looking."]),
    calls=st.lists(calls, min_size=1, max_size=4).map(tuple),
)

replies = st.one_of(
    calling,
    calling,
    calling,
    calling,
    st.builds(Reply, text=st.sampled_from(["Done.", "Fine.", "Here it is."])),
    st.builds(Reply, text=st.sampled_from(["Done.", "Fine.", "Here it is."])),
    st.builds(Reply, reasoning=st.just("Let me think."), text=st.just("Answer.")),
    st.just(Reply(text="This sentence never", finish="max-tokens")),
    st.just(Reply(text="I w", failure=LlmFailure(message="overloaded", code="SERVER"))),
    st.just(Reply()),
)

actions = st.sampled_from(
    [
        "task",
        "task",
        "task",
        "steer",
        "steer",
        "inject",
        "cancel",
        "cancel-and-keep",
        "wait",
        "wake",
    ]
)


def conversation_problems(messages: Sequence[Message]) -> list[str]:
    """Say what is wrong with a conversation as a model would be sent it."""
    problems: list[str] = []
    waiting: list[str] = []
    for position, message in enumerate(messages):
        if isinstance(message, SystemMessage):
            if position != 0:
                problems.append(f"message {position}: instructions that are not first")
        elif isinstance(message, ToolMessage):
            if not waiting or waiting[0] != message.call_id:
                problems.append(f"message {position}: a result that answers no call in turn")
            else:
                waiting.pop(0)
        else:
            if waiting:
                problems.append(f"message {position}: calls {waiting} were left without results")
                waiting = []
            if isinstance(message, AssistantMessage):
                waiting = [call.call_id for call in message.tool_calls]
            else:
                assert isinstance(message, UserMessage)
    if waiting:
        problems.append(f"at the end: calls {waiting} were left without results")
    return problems


def log_problems(
    events: Sequence[SessionEvent[EventData]], ended_by: Collection[str] = ()
) -> list[str]:
    """Say what is wrong with the structure of a session log.

    ``ended_by`` holds the ids of the calls whose tool asked for its turn to end.
    """
    problems: list[str] = []
    turn: int | None = None
    step: int | None = None
    last_turn = 0
    last_step = 0
    cause = False
    ended = False
    dispatched: list[str] = []
    for event in events:
        data = event.data
        where = f"event {event.seq} ({event.type})"
        if isinstance(data, TurnStartEvent):
            if turn is not None:
                problems.append(f"{where}: a turn begins inside turn {turn}")
            if data.turn != last_turn + 1:
                problems.append(f"{where}: turn {data.turn} follows turn {last_turn}")
            turn, last_turn, last_step, dispatched = data.turn, data.turn, 0, []
            ended = False
        elif isinstance(data, TurnEndEvent):
            if turn != data.turn or step is not None:
                problems.append(f"{where}: does not close the open turn cleanly")
            turn = None
        elif isinstance(data, StepStartEvent):
            if turn != data.turn or step is not None or data.step != last_step + 1:
                problems.append(f"{where}: a step that does not follow")
            if data.step > 1 and not cause:
                problems.append(f"{where}: a further step without new input or tool results")
            if ended:
                problems.append(f"{where}: a further step after a tool had ended the turn")
            step = last_step = data.step
            cause = False
        elif isinstance(data, StepEndEvent):
            if turn != data.turn or step != data.step:
                problems.append(f"{where}: does not close the open step")
            step = None
        elif isinstance(data, AssistantMessageEvent | ToolCallEvent | ToolResultEvent):
            if step is None:
                problems.append(f"{where}: outside a step")
            if isinstance(data, ToolCallEvent):
                dispatched.append(data.call_id)
            elif isinstance(data, ToolResultEvent):
                cause = True
                ended = ended or (data.error is None and data.message.call_id in ended_by)
                if data.message.call_id not in dispatched:
                    problems.append(f"{where}: a result without a call that was written down")
        elif isinstance(data, UserMessageEvent):
            cause = True
            if turn is None or step is not None:
                problems.append(f"{where}: input outside the beginning of a step")
    if turn is not None:
        problems.append(f"turn {turn} was left open")
    return problems


class Run:
    """One example: the program, what was asked of it, and what it must not have done."""

    def __init__(self, agent: Agent, model: Scripted, bench: Bench) -> None:
        self.agent = agent
        self.model = model
        self.bench = bench
        self.given: dict[str, str] = {}
        self.given_at: dict[str, int] = {}
        self.cancelled = False
        self.dropped: set[str] = set()
        self.asked_at: dict[int, int] = {}
        self.waiting_at: dict[int, set[str]] = {}
        self.requests_when_cancelled: dict[int, int] = {}
        self.problems: list[str] = []

    def give(self, kind: str, text: str) -> None:
        self.given[text] = kind
        self.given_at[text] = len(self.agent.session.events)
        if kind == "task":
            self.agent.followup(text)
        elif kind == "steer":
            self.agent.steer(text)
        else:
            self.agent.inject(text)

    def cancel(self, *, keep_inbox: bool) -> None:
        """Cancel, and remember how many requests the open turn had made by then."""
        session = self.agent.session
        state = TURN.of(session)
        if state.in_turn:
            self.requests_when_cancelled.setdefault(state.turn, len(self.model.requests))
            # From here on the open turn may take nothing more from the inbox.
            self.asked_at.setdefault(state.turn, len(session.events))
            self.waiting_at.setdefault(state.turn, set(waiting_texts(session)))
        self.cancelled = True
        waited = waiting_texts(session)
        if not keep_inbox:
            # What waits at this moment is dropped. Nothing else may ever go missing.
            self.dropped.update(waited)
        self.agent.cancel("budget", keep_inbox=keep_inbox)
        # A listener may have given new input while the agent was being cancelled.
        if not keep_inbox and set(waited) & set(waiting_texts(session)):
            self.problems.append("a cancellation left input in the inbox")

    def watch(self, committed: Committed) -> None:
        data = committed.event.data
        if isinstance(data, TurnEndEvent) and data.turn in self.requests_when_cancelled:
            before = self.requests_when_cancelled[data.turn]
            if len(self.model.requests) != before:
                self.problems.append(f"turn {data.turn} asked the model after it was cancelled")

    def check(self, stored: Sequence[SessionEvent[EventData]] | None) -> None:
        session = self.agent.session
        events = session.events
        assert self.problems == []
        assert self.bench.problems == []
        assert log_problems(events, self.bench.concluded) == []
        assert conversation_problems(session.messages) == []
        for number, request in enumerate(self.model.requests):
            assert conversation_problems(request.messages) == [], f"request {number}"
        for number, (earlier, later) in enumerate(pairwise(self.model.requests), start=1):
            # Nothing rewrites what the model was sent before: a conversation only grows.
            kept = later.messages[: len(earlier.messages)] == earlier.messages
            assert kept, f"request {number} does not begin with what request {number - 1} held"
        assert closing_events(events, "interrupted") == []
        state = TURN.of(session)
        assert not state.in_turn
        assert not state.in_step
        heard = [
            data.message.content[0].text
            for event in events
            if isinstance(data := event.data, UserMessageEvent)
        ]
        turn = 0
        for event in events:
            data = event.data
            if isinstance(data, TurnStartEvent):
                turn = data.turn
            elif isinstance(data, UserMessageEvent) and turn in self.asked_at:
                # A turn that is being cancelled takes nothing more: not what waited at that
                # moment, and not what was given after it.
                text = data.message.content[0].text
                assert text not in self.waiting_at[turn], (
                    f"{text!r} waited when turn {turn} was cancelled, and the turn took it"
                )
                assert self.given_at[text] < self.asked_at[turn], (
                    f"{text!r} was given after turn {turn} was cancelled, and the turn took it"
                )
        waiting = waiting_texts(session)
        assert len(set(heard)) == len(heard), "an input reached the model twice"
        assert not set(heard) & set(waiting), "an input both reached the model and still waits"
        # Each input is in exactly one place: with the model, in the inbox, or dropped by a
        # cancellation that was asked to drop what waited at that moment.
        assert not self.dropped & set(heard), "an input that was dropped reached the model"
        assert not self.dropped & set(waiting), "an input that was dropped waits again"
        assert sorted(heard + waiting) == sorted(set(self.given) - self.dropped), (
            "an input was lost"
        )
        if not self.cancelled:
            # Left alone, an agent comes to rest only when every task and steer was read.
            assert [self.given[text] for text in waiting] == ["inject"] * len(waiting)
        if stored is not None:
            assert list(stored) == list(events)


def waiting_texts(session: Session) -> list[str]:
    """The texts of the input that waits in the inbox of a session."""
    inbox = INBOX.of(session)
    return [item.message.content[0].text for item in (*inbox.next_step, *inbox.next_turn)]


async def build(root: Path | None, script: list[Reply]) -> tuple[Context, Run]:
    ctx = Context()
    await ctx.plugin(columbina.session, None if root is None else {"root": root})
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 5})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.agent)
    await ctx.plugin(columbina.agent_loop, {"max_parallel_tool_calls": PARALLEL})
    model = Scripted(script)
    bench = Bench()
    ctx.use(LLM).adapters.add(ctx, "scripted", model)
    ctx.use(TOOLS).add(ctx, *bench.tools())
    spec = AgentSpec(cwd=Path.cwd(), model=MODEL, system_prompt="You are helpful.")
    agent = await ctx.use(AGENTS).create(ctx, spec, session_id="s-1")
    return ctx, Run(agent, model, bench)


class Complaints(logging.Handler):
    """Collects what Columbina logs as an error. Nothing in these examples should be one."""

    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def scenario(data: st.DataObject, root: Path | None) -> None:
    """Play one example in an event loop of its own, and let nothing in it log an error."""
    complaints = Complaints()
    logging.getLogger("columbina").addHandler(complaints)
    try:
        asyncio.run(play(data, root))
    finally:
        logging.getLogger("columbina").removeHandler(complaints)
    assert complaints.messages == []


async def play(data: st.DataObject, root: Path | None) -> None:
    script = data.draw(st.lists(replies, min_size=2, max_size=12), label="script")
    ctx, run = await build(root, script)
    agent = run.agent
    session: Session = agent.session
    events_left: list[int] = []
    chunks_left: list[int] = []
    tools_left: list[int] = []
    keep_at_an_event: list[bool] = []
    change_of_mind: list[tuple[str, str]] = []

    def at_a_tool() -> None:
        # Ask for a cancellation while a tool is running: from the tool itself, as it begins.
        if tools_left:
            tools_left[0] -= 1
            if tools_left[0] <= 0:
                tools_left.clear()
                run.cancel(keep_inbox=True)

    run.bench.on_start = at_a_tool

    def at_an_event(committed: Committed) -> None:
        # Ask for a cancellation from inside the turn, as a chosen event is written.
        run.watch(committed)
        if events_left:
            events_left[0] -= 1
            if events_left[0] <= 0:
                events_left.clear()
                run.cancel(keep_inbox=keep_at_an_event.pop())
                # Whoever cancels may say at once what to do instead.
                while change_of_mind:
                    run.give(*change_of_mind.pop())

    def at_a_chunk(frame: StreamFrame) -> None:
        # The same, but while a reply streams, after a chosen number of chunks.
        if chunks_left:
            chunks_left[0] -= 1
            if chunks_left[0] <= 0:
                chunks_left.clear()
                run.cancel(keep_inbox=True)

    agent.ctx.on(SESSION_EVENT, at_an_event)
    agent.ctx.on(STREAM, at_a_chunk)
    try:
        for number in range(data.draw(st.integers(min_value=1, max_value=7), label="actions")):
            # Every example begins with a task, so that the model is asked at least once.
            action = "task" if number == 0 else data.draw(actions, label="action")
            if action in ("task", "steer", "inject"):
                run.give(action, f"input {number}")
            elif action == "cancel":
                run.cancel(keep_inbox=False)
            elif action == "cancel-and-keep":
                run.cancel(keep_inbox=True)
            elif action == "wake":
                agent.wake()
            else:
                await asyncio.wait_for(agent.when_idle(), timeout=10)
            trap = data.draw(
                st.sampled_from(["none", "none", "none", "event", "chunk", "tool"]), label="trap"
            )
            if trap == "event":
                at = data.draw(st.integers(min_value=1, max_value=12), label="at")
                keep = data.draw(st.booleans(), label="keep")
                instead = data.draw(st.sampled_from(["nothing", "task", "steer"]), label="then")
                # Set last what springs the trap, so that it is never half set.
                keep_at_an_event[:] = [keep]
                change_of_mind[:] = [] if instead == "nothing" else [(instead, f"instead {number}")]
                events_left[:] = [at]
            elif trap == "chunk":
                chunks_left[:] = [data.draw(st.integers(min_value=1, max_value=9), label="at")]
            elif trap == "tool":
                tools_left[:] = [data.draw(st.integers(min_value=1, max_value=3), label="at")]
            for _ in range(data.draw(st.integers(min_value=0, max_value=6), label="yields")):
                await asyncio.sleep(0)
        await asyncio.wait_for(agent.when_idle(), timeout=10)
        events_left.clear()
        chunks_left.clear()
        tools_left.clear()
        change_of_mind.clear()
        if run.cancelled:
            # What a cancellation kept is run once the agent is woken, and then it rests again.
            agent.wake()
            await asyncio.wait_for(agent.when_idle(), timeout=10)
        await session.flush()
        stored = None
        if root is not None and len(session):
            stored = ctx.use(SESSIONS).read(Path.cwd(), "s-1").events
        run.check(stored)
        leftover = [task for task in asyncio.all_tasks() if task is not asyncio.current_task()]
        assert len(leftover) == 1, "only the driver of the agent may still be running"
    finally:
        await asyncio.wait_for(ctx.fiber.dispose(), timeout=10)


@settings(max_examples=250, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(st.data())
def test_the_log_holds_together_whatever_is_asked_and_whenever_it_is_cancelled(
    data: st.DataObject,
) -> None:
    scenario(data, None)


@settings(max_examples=40, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(st.data())
def test_what_is_stored_is_what_happened(data: st.DataObject) -> None:
    with tempfile.TemporaryDirectory() as directory:
        scenario(data, Path(directory))


def test_the_checks_of_these_tests_find_what_they_look_for() -> None:
    from columbina.llm import TextBlock, ToolCallBlock

    def user(text: str) -> UserMessage:
        return UserMessage(content=(TextBlock(text=text),))

    asking = AssistantMessage(content=(ToolCallBlock(call_id="a", name="echo", arguments="{}"),))
    answer = ToolMessage(call_id="a", content=(TextBlock(text="ok"),))
    other = ToolMessage(call_id="b", content=(TextBlock(text="ok"),))
    system = SystemMessage(content=(TextBlock(text="x"),))

    assert conversation_problems([system, user("hi"), asking, answer, user("more")]) == []
    assert conversation_problems([user("hi"), system]) != []
    assert conversation_problems([user("hi"), asking, user("more")]) != []
    assert conversation_problems([user("hi"), asking]) != []
    assert conversation_problems([user("hi"), asking, other]) != []
    assert conversation_problems([user("hi"), answer]) != []

    def events(*data: EventData) -> list[SessionEvent[EventData]]:
        return [SessionEvent(seq=seq, time=0, data=item) for seq, item in enumerate(data)]

    good = events(
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        StepEndEvent(turn=1, step=1),
        TurnEndEvent(turn=1, reason="completed"),
    )
    assert log_problems(good) == []
    assert log_problems(good[:-1]) != []
    assert log_problems(events(TurnStartEvent(turn=2))) != []
    assert log_problems(events(TurnStartEvent(turn=1), TurnStartEvent(turn=2))) != []
    assert log_problems(events(TurnStartEvent(turn=1), StepStartEvent(turn=1, step=2))) != []
    assert log_problems(events(TurnStartEvent(turn=1), StepEndEvent(turn=1, step=1))) != []
    assert (
        log_problems(
            events(
                TurnStartEvent(turn=1),
                StepStartEvent(turn=1, step=1),
                StepEndEvent(turn=1, step=1),
                StepStartEvent(turn=1, step=2),
                StepEndEvent(turn=1, step=2),
                TurnEndEvent(turn=1, reason="completed"),
            )
        )
        != []
    )
    assert (
        log_problems(
            events(
                TurnStartEvent(turn=1),
                StepStartEvent(turn=1, step=1),
                TurnEndEvent(turn=1, reason="completed"),
            )
        )
        != []
    )
    assert (
        log_problems(
            events(
                TurnStartEvent(turn=1),
                StepStartEvent(turn=1, step=1),
                ToolResultEvent(message=answer),
            )
        )
        != []
    )
    assert (
        log_problems(
            events(TurnStartEvent(turn=1), ToolCallEvent(call_id="a", name="x", arguments="{}"))
        )
        != []
    )
    two_steps = events(
        TurnStartEvent(turn=1),
        StepStartEvent(turn=1, step=1),
        ToolCallEvent(call_id="a", name="x", arguments="{}"),
        ToolResultEvent(message=answer),
        StepEndEvent(turn=1, step=1),
        StepStartEvent(turn=1, step=2),
        StepEndEvent(turn=1, step=2),
        TurnEndEvent(turn=1, reason="completed"),
    )
    assert log_problems(two_steps) == []
    assert log_problems(two_steps, ended_by={"b"}) == []
    assert log_problems(two_steps, ended_by={"a"}) != []


async def test_an_input_that_is_lost_is_noticed_also_after_a_cancellation_that_dropped_others() -> (
    None
):
    from columbina.agent import clear_inbox

    async def example(*, lose_the_second: bool) -> Run:
        ctx, run = await build(None, [Reply(text="Done.")])
        try:
            run.give("inject", "before the cancellation")
            run.cancel(keep_inbox=False)
            run.give("inject", "after the cancellation")
            if lose_the_second:
                clear_inbox(run.agent.session)
            await asyncio.wait_for(run.agent.when_idle(), timeout=5)
        finally:
            await ctx.fiber.dispose()
        return run

    kept = await example(lose_the_second=False)
    kept.check(None)
    assert kept.dropped == {"before the cancellation"}

    lost = await example(lose_the_second=True)
    with pytest.raises(AssertionError, match="an input was lost"):
        lost.check(None)
