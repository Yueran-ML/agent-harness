"""The loop and tools: calls are carried out, and every call gets its result."""

import asyncio

import pytest
from pydantic import BaseModel

import columbina.agent_loop
from columbina.agent import INBOX, Agent
from columbina.kernel import Context, Next
from columbina.llm import AssistantMessage, TextBlock, ToolCallBlock, ToolMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import (
    SESSION_EVENT,
    AssistantMessageEvent,
    Committed,
    EventData,
    SessionEvent,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
)
from columbina.tools import PRE_EXECUTE, Decision, Deny, Invocation, Tool, ToolFailure, ToolRun
from support.loop import SYSTEM, Harness, idle, types, user


class Args(BaseModel):
    name: str = ""


class Board:
    """Tools that write down when they start and end, and wait at a gate when there is one."""

    def __init__(self) -> None:
        self.log: list[str] = []
        self.gates: dict[str, asyncio.Event] = {}
        self.began: dict[str, asyncio.Event] = {}
        self.runs: dict[str, ToolRun] = {}
        self.seen: dict[str, str] = {}

    def gate(self, name: str) -> asyncio.Event:
        return self.gates.setdefault(name, asyncio.Event())

    def begun(self, name: str) -> asyncio.Event:
        return self.began.setdefault(name, asyncio.Event())

    async def work(self, args: Args, run: ToolRun) -> str:
        self.log.append(f"start {args.name}")
        self.runs[args.name] = run
        self.seen[args.name] = run.session.events[-1].type
        self.begun(args.name).set()
        try:
            if args.name in self.gates:
                await self.gates[args.name].wait()
        except asyncio.CancelledError:
            self.log.append(f"stopped {args.name}")
            raise
        self.log.append(f"end {args.name}")
        return f"did {args.name}"

    async def fail(self, args: Args, run: ToolRun) -> str:
        raise ToolFailure(f"cannot do {args.name}")

    async def finish(self, args: Args, run: ToolRun) -> str:
        run.conclude_turn()
        return "handed over"


def shared(*names: str) -> tuple[Call, ...]:
    return tuple(Call(name="shared", arguments={"name": name}) for name in names)


def exclusive(*names: str) -> tuple[Call, ...]:
    return tuple(Call(name="exclusive", arguments={"name": name}) for name in names)


def results(agent: Agent) -> list[tuple[str, str, str | None]]:
    """The tool results of a session: call id, text, and error code."""
    return [
        (
            event.data.message.call_id,
            event.data.message.content[0].text,
            event.data.error.code if event.data.error else None,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def calls_and_results(agent: Agent) -> list[str]:
    """The tool calls and results of a session, in the order of the log."""

    def label(event: SessionEvent[EventData]) -> str | None:
        if isinstance(event.data, ToolCallEvent):
            return f"call {event.data.call_id}"
        if isinstance(event.data, ToolResultEvent):
            return f"result {event.data.message.call_id}"
        return None

    return [text for event in agent.session.events if (text := label(event)) is not None]


@pytest.fixture
def board(harness: Harness) -> Board:
    made = Board()
    harness.tools.add(
        harness.ctx,
        Tool(
            name="shared",
            description="Work that may overlap.",
            parameters=Args,
            execute=made.work,
            concurrency_safe=True,
        ),
        Tool(name="exclusive", description="Work alone.", parameters=Args, execute=made.work),
        Tool(name="fail", description="Fail.", parameters=Args, execute=made.fail),
        Tool(name="finish", description="End the turn.", parameters=Args, execute=made.finish),
    )
    return made


# One call


async def test_a_tool_call_is_carried_out_and_its_result_goes_back_to_the_model(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(
        Reply(text="I will do it.", calls=shared("a")), Reply(text="It is done.")
    )

    agent.followup("Do a.")
    await idle(agent)

    assert types(agent.session, 5) == [
        "step/start",
        "request/header",
        "assistant/message",
        "tool/call",
        "tool/result",
        "step/end",
        "step/start",
        "assistant/message",
        "step/end",
        "turn/end",
    ]
    asked = ToolCallBlock(call_id="call-1", name="shared", arguments='{"name": "a"}')
    assert agent.session.events[8].data == ToolCallEvent(
        call_id="call-1", name="shared", arguments='{"name": "a"}'
    )
    assert harness.model.requests[1].messages == (
        SYSTEM,
        user("Do a."),
        AssistantMessage(content=(TextBlock(text="I will do it."), asked)),
        ToolMessage(call_id="call-1", content=(TextBlock(text="did a"),)),
    )
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_a_tool_is_told_about_its_call_after_the_call_is_in_the_log(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(Reply(calls=shared("a")), Reply(text="Done."))

    agent.followup("Do a.")
    await idle(agent)

    run = board.runs["a"]
    assert run.call_id == "call-1"
    assert run.cwd == harness.cwd
    assert run.session is agent.session
    assert board.seen["a"] == "tool/call"


async def test_a_turn_goes_on_for_as_long_as_the_model_calls_tools(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(
        Reply(calls=shared("a")), Reply(calls=exclusive("b")), Reply(text="Both done.")
    )

    agent.followup("Do a, then b.")
    await idle(agent)

    assert board.log == ["start a", "end a", "start b", "end b"]
    assert len(harness.model.requests) == 3
    starts = [e.data for e in agent.session.events if isinstance(e.data, StepStartEvent)]
    assert [start.step for start in starts] == [1, 2, 3]


# Calls that do not work


async def test_a_call_that_does_not_work_is_a_result_too_and_the_turn_goes_on(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(
        Reply(
            calls=(
                Call(name="nowhere"),
                Call(name="shared", arguments='{"name": 5}'),
                Call(name="fail", arguments={"name": "x"}),
            )
        ),
        Reply(text="I see."),
    )

    agent.followup("Try.")
    await idle(agent)

    assert [(call_id, code) for call_id, _, code in results(agent)] == [
        ("call-1", "UNKNOWN_TOOL"),
        ("call-2", "INVALID_ARGS"),
        ("call-3", "TOOL_ERROR"),
    ]
    assert results(agent)[2][1] == "cannot do x"
    assert calls_and_results(agent) == [
        "call call-1",
        "result call-1",
        "call call-2",
        "result call-2",
        "call call-3",
        "result call-3",
    ]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")
    assert len(harness.model.requests) == 2


async def test_a_listener_of_the_agent_can_refuse_a_call(harness: Harness, board: Board) -> None:
    async def never(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny(reason="not today")

    agent = await harness.agent(Reply(calls=shared("a")), Reply(text="Fine."))
    other = await harness.agent(Reply(calls=shared("b")), Reply(text="Done."), session_id="s-2")
    agent.ctx.on(PRE_EXECUTE, never)

    agent.followup("Do a.")
    await idle(agent)
    other.followup("Do b.")
    await idle(other)

    assert results(agent) == [("call-1", "The call was not allowed: not today", "DENIED")]
    assert [code for _, _, code in results(other)] == [None]
    assert board.log == ["start b", "end b"]


# Several calls in one reply


async def test_results_are_written_in_the_order_of_the_model(
    harness: Harness, board: Board
) -> None:
    board.gate("a")
    agent = await harness.agent(Reply(calls=shared("a", "b")), Reply(text="Done."))
    agent.followup("Do a and b.")
    await asyncio.wait_for(board.begun("b").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ["start a", "start b", "end b"]
    assert calls_and_results(agent) == ["call call-1", "call call-2"]

    board.gate("a").set()
    await idle(agent)

    assert calls_and_results(agent) == [
        "call call-1",
        "call call-2",
        "result call-1",
        "result call-2",
    ]
    assert [text for _, text, _ in results(agent)] == ["did a", "did b"]


async def test_a_result_is_written_as_soon_as_all_before_it_are_in(
    harness: Harness, board: Board
) -> None:
    board.gate("b")
    agent = await harness.agent(Reply(calls=shared("a", "b")), Reply(text="Done."))
    agent.followup("Do a and b.")
    await asyncio.wait_for(board.begun("b").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert calls_and_results(agent) == ["call call-1", "call call-2", "result call-1"]

    board.gate("b").set()
    await idle(agent)

    assert calls_and_results(agent)[-1] == "result call-2"


async def test_a_call_that_must_be_alone_waits_and_makes_the_ones_after_it_wait(
    harness: Harness, board: Board
) -> None:
    board.gate("a")
    board.gate("x")
    agent = await harness.agent(
        Reply(calls=(*shared("a"), *exclusive("x"), *shared("b"))), Reply(text="Done.")
    )
    agent.followup("Do a, x and b.")
    await asyncio.wait_for(board.begun("a").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ["start a"]

    board.gate("a").set()
    await asyncio.wait_for(board.begun("x").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ["start a", "end a", "start x"]

    board.gate("x").set()
    await idle(agent)

    assert board.log == ["start a", "end a", "start x", "end x", "start b", "end b"]
    assert [text for _, text, _ in results(agent)] == ["did a", "did x", "did b"]


def watched(harness: Harness, board: Board) -> None:
    """Write down on the board when the checks of a call are asked."""

    async def guard(ctx: Context, invocation: Invocation) -> Deny | None:
        board.log.append(f"check {invocation.call.arguments}")
        return None

    harness.tools.guards.add(harness.ctx, "watching", guard)


async def test_a_call_that_must_be_alone_is_checked_only_when_it_can_run(
    harness: Harness, board: Board
) -> None:
    watched(harness, board)
    board.gate("a")
    agent = await harness.agent(Reply(calls=(*shared("a"), *exclusive("x"))), Reply(text="Done."))
    agent.followup("Do a and x.")
    await asyncio.wait_for(board.begun("a").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ['check {"name": "a"}', "start a"]

    board.gate("a").set()
    await idle(agent)

    assert board.log == [
        'check {"name": "a"}',
        "start a",
        "end a",
        'check {"name": "x"}',
        "start x",
        "end x",
    ]


async def test_a_call_that_waits_for_room_is_checked_only_when_there_is_room(
    harness: Harness, board: Board
) -> None:
    await harness.ctx.plugin(columbina.agent_loop, {"name": "narrow", "max_parallel_tool_calls": 1})
    watched(harness, board)
    board.gate("a")
    agent = await harness.agent(Reply(calls=shared("a", "b")), Reply(text="Done."), loop="narrow")
    agent.followup("Do a and b.")
    await asyncio.wait_for(board.begun("a").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ['check {"name": "a"}', "start a"]

    board.gate("a").set()
    await idle(agent)

    assert board.log[2:4] == ["end a", 'check {"name": "b"}']


async def test_only_so_many_calls_run_beside_each_other(harness: Harness, board: Board) -> None:
    await harness.ctx.plugin(columbina.agent_loop, {"name": "narrow", "max_parallel_tool_calls": 2})
    for name in ("a", "b", "c"):
        board.gate(name)
    agent = await harness.agent(
        Reply(calls=shared("a", "b", "c")), Reply(text="Done."), loop="narrow"
    )
    agent.followup("Do a, b and c.")
    await asyncio.wait_for(board.begun("b").wait(), timeout=5)
    await asyncio.sleep(0.02)

    assert board.log == ["start a", "start b"]

    board.gate("b").set()
    await asyncio.wait_for(board.begun("c").wait(), timeout=5)

    assert board.log == ["start a", "start b", "end b", "start c"]

    board.gate("a").set()
    board.gate("c").set()
    await idle(agent)

    assert [text for _, text, _ in results(agent)] == ["did a", "did b", "did c"]


@pytest.mark.parametrize("limit", [0, -1])
async def test_at_least_one_call_must_be_allowed_to_run(harness: Harness, limit: int) -> None:
    with pytest.raises(ValueError, match="max_parallel_tool_calls"):
        await harness.ctx.plugin(
            columbina.agent_loop, {"name": "none", "max_parallel_tool_calls": limit}
        )


# Ending a turn from a tool


async def test_a_tool_can_end_the_turn_once_the_results_are_in(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(
        Reply(calls=(*shared("a"), Call(name="finish"))), Reply(text="never asked for")
    )

    agent.followup("Do a and hand over.")
    await idle(agent)

    assert [text for _, text, _ in results(agent)] == ["did a", "handed over"]
    assert len(harness.model.requests) == 1
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")
    assert harness.model.remaining == 1


# Steering and cancelling while tools run


async def test_a_steer_that_arrives_while_tools_run_follows_their_results(
    harness: Harness, board: Board
) -> None:
    board.gate("a")
    agent = await harness.agent(Reply(calls=shared("a")), Reply(text="Stopping."))
    agent.followup("Do a.")
    await asyncio.wait_for(board.begun("a").wait(), timeout=5)

    agent.steer("Stop after this.")
    board.gate("a").set()
    await idle(agent)

    assert harness.model.requests[1].messages[-2:] == (
        ToolMessage(call_id="call-1", content=(TextBlock(text="did a"),)),
        user("Stop after this."),
    )


async def test_cancelling_stops_the_tools_and_gives_every_call_a_result(
    harness: Harness, board: Board
) -> None:
    board.gate("b")
    agent = await harness.agent(
        Reply(calls=(*shared("a", "b"), *exclusive("x"), *shared("c"))), Reply(text="never")
    )
    agent.followup("Do a, b, x and c.")
    await asyncio.wait_for(board.begun("b").wait(), timeout=5)
    await asyncio.sleep(0.02)

    agent.cancel()
    await idle(agent)

    assert board.log == ["start a", "end a", "start b", "stopped b"]
    assert [(call_id, code) for call_id, _, code in results(agent)] == [
        ("call-1", None),
        ("call-2", "TOOL_CANCELLED"),
        ("call-3", "ABORTED_BEFORE_DISPATCH"),
        ("call-4", "ABORTED_BEFORE_DISPATCH"),
    ]
    order = calls_and_results(agent)
    numbers = (1, 2, 3, 4)
    expected = [f"{kind} call-{n}" for kind in ("call", "result") for n in numbers]
    assert sorted(order) == sorted(expected)
    assert all(order.index(f"call call-{n}") < order.index(f"result call-{n}") for n in numbers)
    assert order[-4:] == ["result call-2", "result call-3", "call call-4", "result call-4"]
    assert results(agent)[1][1] == (
        "The call was cancelled while it ran. It may have taken effect in part."
    )
    assert results(agent)[2][1] == "The turn was cancelled before this call ran."
    assert types(agent.session)[-2:] == ["step/end", "turn/end"]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert len(harness.model.requests) == 1


async def test_cancelling_a_call_that_runs_alone_says_that_it_ran(
    harness: Harness, board: Board
) -> None:
    board.gate("x")
    agent = await harness.agent(Reply(calls=exclusive("x")), Reply(text="never"))
    agent.followup("Do x.")
    await asyncio.wait_for(board.begun("x").wait(), timeout=5)

    agent.cancel()
    await idle(agent)

    assert board.log == ["start x", "stopped x"]
    assert [(call_id, code) for call_id, _, code in results(agent)] == [
        ("call-1", "TOOL_CANCELLED")
    ]


async def test_the_calls_of_a_reply_that_was_cut_short_get_results_too(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(Reply(text="First", calls=shared("a"), hold=True))
    agent.followup("Do a.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.cancel()
    await idle(agent)

    interrupted = next(
        event.data
        for event in agent.session.events
        if isinstance(event.data, AssistantMessageEvent)
    )
    assert interrupted.interrupted
    assert [call.call_id for call in interrupted.message.tool_calls] == ["call-1"]
    assert calls_and_results(agent) == ["call call-1", "result call-1"]
    assert results(agent) == [
        ("call-1", "The turn was cancelled before this call ran.", "ABORTED_BEFORE_DISPATCH")
    ]
    assert board.log == []


async def test_a_new_turn_after_a_cancellation_sends_a_history_that_holds_together(
    harness: Harness, board: Board
) -> None:
    board.gate("a")
    agent = await harness.agent(Reply(calls=shared("a", "b")), Reply(text="Hello!"))
    agent.followup("Do a and b.")
    await asyncio.wait_for(board.begun("a").wait(), timeout=5)
    agent.cancel()
    await idle(agent)

    agent.followup("Say hello.")
    await idle(agent)

    messages = harness.model.requests[1].messages
    asked = messages[2]
    assert isinstance(asked, AssistantMessage)
    answered = [message.call_id for message in messages if isinstance(message, ToolMessage)]
    assert answered == [call.call_id for call in asked.tool_calls]
    assert messages[-1] == user("Say hello.")


# Review round 6


async def test_a_listener_that_cancels_as_a_tool_call_is_written_stops_the_turn(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(Reply(calls=shared("a")), Reply(text="never"))

    def at_dispatch(committed: Committed) -> None:
        if committed.event.type == "tool/call":
            agent.cancel("budget")

    agent.ctx.on(SESSION_EVENT, at_dispatch)
    agent.followup("Do a.")
    await idle(agent)

    assert results(agent) == [
        ("call-1", "The turn was cancelled before this call ran.", "ABORTED_BEFORE_DISPATCH")
    ]
    assert board.log == []
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="budget")
    assert len(harness.model.requests) == 1


async def test_every_call_gets_its_result_even_if_the_turn_is_cancelled_twice(
    harness: Harness,
) -> None:
    began = asyncio.Event()
    cleaning = asyncio.Event()
    gate = asyncio.Event()

    running: list[str] = []

    async def linger(args: Args, run: ToolRun) -> str:
        running.append(run.call_id)
        if len(running) == 2:
            began.set()
        try:
            await asyncio.Event().wait()
        finally:
            cleaning.set()
            await gate.wait()
        return "never"

    harness.tools.add(
        harness.ctx,
        Tool(
            name="linger",
            description="Take long to stop.",
            parameters=Args,
            execute=linger,
            concurrency_safe=True,
        ),
    )
    agent = await harness.agent(
        Reply(calls=(Call(name="linger"), Call(name="linger"))), Reply(text="never")
    )
    agent.followup("Linger twice.")
    await asyncio.wait_for(began.wait(), timeout=5)
    agent.cancel()
    await asyncio.wait_for(cleaning.wait(), timeout=5)
    (turn,) = [
        task
        for task in asyncio.all_tasks()
        if getattr(task.get_coro(), "__qualname__", "") == "LoopAgent._turn"
    ]

    turn.cancel()
    await idle(agent)

    assert [(call_id, code) for call_id, _, code in results(agent)] == [
        ("call-1", "TOOL_CANCELLED"),
        ("call-2", "TOOL_CANCELLED"),
    ]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")

    gate.set()
    await asyncio.sleep(0.02)

    assert len(results(agent)) == 2


async def test_a_turn_that_a_tool_ended_does_not_go_on_for_context_that_waits(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(Reply(calls=(Call(name="finish"),), hold=True), Reply(text="never"))
    agent.followup("Hand over.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.inject("Context for later.")
    harness.model.release()
    await idle(agent)

    assert len(harness.model.requests) == 1
    assert [item.message for item in INBOX.of(agent.session).next_step] == [
        user("Context for later.")
    ]
    assert agent.session.events[-1].data == TurnEndEvent(turn=1, reason="completed")


async def test_a_steer_that_arrives_while_a_tool_ends_the_turn_opens_the_next_turn(
    harness: Harness, board: Board
) -> None:
    agent = await harness.agent(
        Reply(calls=(Call(name="finish"),), hold=True), Reply(text="On it.")
    )
    agent.followup("Hand over.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    agent.steer("Wait, one more thing.")
    harness.model.release()
    await idle(agent)

    ends = [e.data for e in agent.session.events if isinstance(e.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]
    assert harness.model.requests[1].messages[-1] == user("Wait, one more thing.")
