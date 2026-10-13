"""The service process: commands, and what is told of where the agent stands."""

import asyncio
import threading

import pytest

import columbina.approval
import columbina.commands.permissions
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina import cancellation
from columbina.agent import Agent
from columbina.apps.serve import server as server_module
from columbina.apps.serve.rpc import INVALID_PARAMS, LEAVING, NOT_IDLE
from columbina.commands import COMMANDS, Command
from columbina.compaction import EVENTS as COMPACTION_EVENTS
from columbina.compaction import CompactionStartEvent
from columbina.llm.adapters.scripted import Reply
from columbina.sandbox import SANDBOX
from columbina.session import Listed, SessionError, Sessions, UserMessageEvent
from support.sandboxes import Holding
from support.served import Served
from support.terminal import until

NOT_AT_REST = (
    NOT_IDLE,
    "The agent is not at rest, and a command waits until it is. session/cancel stops the agent.",
)


def statuses(served: Served) -> list[str]:
    return [told["status"] for told in served.far.told("session.status")]


async def rested(served: Served) -> None:
    await served.far.until(lambda: statuses(served)[-1] != "running")


class Slow:
    """A command that has the agent do something, which takes until it is let go or stopped.

    Attributes:
        begun: Set when the agent has begun with it.
        let_go: Set by a test to have it come to its end.
        ended: How the command ended, once it has.
    """

    def __init__(self) -> None:
        self.begun = asyncio.Event()
        self.let_go = asyncio.Event()
        self.ended: list[str] = []

    async def __call__(self, agent: Agent, rest: str) -> str:
        async def work() -> None:
            # Like the command that sums up a conversation: it writes down that it begins.
            agent.session.append(CompactionStartEvent(why="manual", tokens=0))
            self.begun.set()
            await self.let_go.wait()

        try:
            await cancellation.guard(agent.run_maintenance(work), what="what takes long")
        except cancellation.CancelledFromInside:
            self.ended.append("the agent was stopped")
            return "It was stopped."
        self.ended.append("it is done")
        return "It is done."


class Twice:
    """A command that has the agent work, waits for something else, and has it work again.

    Attributes:
        begun: Set when the agent has begun with the first piece of work.
        first: Set by a test to have the first piece of work come to its end.
        between: Set by a test once the command may go on to the second piece.
        waiting: Set when the command waits between its two pieces of work.
    """

    def __init__(self) -> None:
        self.begun = asyncio.Event()
        self.first = asyncio.Event()
        self.waiting = asyncio.Event()
        self.between = asyncio.Event()

    async def __call__(self, agent: Agent, rest: str) -> str:
        async def once() -> None:
            agent.session.append(CompactionStartEvent(why="manual", tokens=0))
            self.begun.set()
            await self.first.wait()

        async def again() -> None:
            agent.session.append(CompactionStartEvent(why="manual", tokens=0))

        await agent.run_maintenance(once)
        self.waiting.set()
        await self.between.wait()
        # An agent that has begun a turn meanwhile refuses this.
        await agent.run_maintenance(again)
        return "Twice."


def slow(served: Served) -> Slow:
    """Give the program a command that takes long, by the name ``/slow``."""
    harness = served.harness
    command = Slow()
    harness.sessions.declare(harness.ctx, *COMPACTION_EVENTS)
    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "slow", Command(command, "Take long."))
    return command


# Commands


async def test_a_command_is_carried_out_and_what_it_says_is_the_answer(served: Served) -> None:
    await served.opened()

    answer = await served.far.result("session/command", {"line": "/help"})

    assert answer == {
        "text": "/help   List the commands.\n"
        "/model  Show the model, or name another.\n"
        "A line that does not begin with a slash goes to the agent."
    }
    # The model is told nothing of it.
    assert len(served.agent().session.events) == 0
    assert statuses(served) == ["idle"]


async def test_the_space_around_a_command_does_not_count(served: Served) -> None:
    await served.opened()

    answer = await served.far.result("session/command", {"line": "  /model \n"})

    assert answer == {"text": "The model is script. Another one is named with /model ID."}


async def test_a_command_that_there_is_not_is_answered_in_a_sentence_and_is_no_error(
    served: Served,
) -> None:
    await served.opened()

    answer = await served.far.result("session/command", {"line": "/frobnicate now"})

    assert answer == {"text": "There is no command /frobnicate. /help lists the commands."}


async def test_a_command_that_fails_says_so_in_its_answer(served: Served) -> None:
    await served.opened()
    harness = served.harness

    async def broken(agent: Agent, rest: str) -> str:
        raise RuntimeError("the broom broke")

    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "sweep", Command(broken, "Sweep."))

    answer = await served.far.result("session/command", {"line": "/sweep"})

    assert answer == {"text": "The command /sweep failed: RuntimeError: the broom broke"}
    # And the agent is free for the next one.
    assert (await served.far.result("session/command", {"line": "/model"}))["text"]


@pytest.mark.parametrize(
    ("params", "wrong"),
    [
        ({"line": "help"}, "A command begins with a slash."),
        ({"line": "  "}, "A command begins with a slash."),
        ({"line": ""}, "A command begins with a slash."),
        ({}, "The params of session/command are not right: line: Field required."),
        (
            {"line": 7},
            "The params of session/command are not right: line: Input should be a valid string.",
        ),
    ],
)
async def test_what_is_no_command_is_refused(
    served: Served, params: dict[str, object], wrong: str
) -> None:
    await served.opened()

    assert await served.far.refusal("session/command", params) == (INVALID_PARAMS, wrong)


async def test_a_command_waits_until_the_agent_is_at_rest_and_is_refused_until_then(
    served: Served,
) -> None:
    await served.opened()
    model = served.harness.model
    model.add(Reply(text="Thinking...", hold=True))
    await served.far.result("session/prompt", {"text": "Think."})
    await model.held()

    refused = await served.far.refusal("session/command", {"line": "/model other"})
    model.release()
    await rested(served)

    assert refused == NOT_AT_REST
    # Nothing came of it.
    assert (await served.far.result("session/command", {"line": "/model"}))["text"].startswith(
        "The model is script."
    )


async def test_a_command_that_is_given_right_after_a_task_finds_the_agent_at_work(
    served: Served,
) -> None:
    await served.opened()
    served.harness.model.add(Reply(text="Hello!"))

    # Neither waits for the answer to the other.
    task = served.far.ask("session/prompt", {"text": "Say hello."})
    command = served.far.ask("session/command", {"line": "/model other"})

    assert (await served.far.answer(task))["result"] == {}
    assert (await served.far.answer(command))["error"]["code"] == NOT_IDLE
    await rested(served)


async def test_while_a_command_takes_its_time_the_agent_is_at_work_and_takes_no_other_command(
    served: Served,
) -> None:
    await served.opened()
    command = slow(served)

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    during = statuses(served)
    refused = await served.far.refusal("session/command", {"line": "/help"})
    command.let_go.set()
    answer = await served.far.answer(asked)
    await rested(served)

    assert during == ["idle", "running"]
    assert refused == NOT_AT_REST
    assert answer["result"] == {"text": "It is done."}
    assert statuses(served) == ["idle", "running", "idle"]


async def test_one_command_is_carried_out_at_a_time_whatever_it_is_about(served: Served) -> None:
    await served.opened()
    harness = served.harness
    let_go = asyncio.Event()

    async def waiting(agent: Agent, rest: str) -> str:
        # It asks nothing of the agent, which rests all the while.
        await let_go.wait()
        return "Waited."

    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "wait", Command(waiting, "Wait."))

    asked = served.far.ask("session/command", {"line": "/wait"})
    refused = await served.far.refusal("session/command", {"line": "/help"})
    let_go.set()
    answer = await served.far.answer(asked)

    assert refused == NOT_AT_REST
    assert answer["result"] == {"text": "Waited."}
    # The agent did nothing, so nothing was told of where it stands.
    assert statuses(served) == ["idle"]
    assert (await served.far.result("session/command", {"line": "/help"}))["text"]


async def test_a_command_that_takes_its_time_is_stopped_like_a_turn(served: Served) -> None:
    await served.opened()
    command = slow(served)

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    # This is heard while the command is carried out.
    assert await served.far.result("session/cancel") == {}
    answer = await served.far.answer(asked)
    await rested(served)

    assert answer["result"] == {"text": "It was stopped."}
    assert command.ended == ["the agent was stopped"]
    assert statuses(served) == ["idle", "running", "idle"]


async def test_a_task_that_comes_while_a_command_is_carried_out_waits_for_it(
    served: Served,
) -> None:
    await served.opened()
    command = slow(served)
    served.harness.model.add(Reply(text="Hello!"))

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    await served.far.result("session/prompt", {"text": "Say hello."})
    await asyncio.sleep(0.05)
    before = len(served.harness.model.requests)
    seen = kinds(served).count("agent/inbox/spliced")
    command.let_go.set()
    await served.far.answer(asked)
    await rested(served)

    assert before == 0
    # The task is kept back here until the command is over. The log has nothing of it yet.
    assert seen == 0
    assert len(served.harness.model.requests) == 1
    # One stretch of work: the command, and then the turn.
    assert statuses(served) == ["idle", "running", "idle"]


def waiting(served: Served, name: str = "wait") -> asyncio.Event:
    """Give the program a command that asks nothing of the agent and waits: ``/wait``.

    Returns:
        What a test sets to have the command come to its end.
    """
    harness = served.harness
    let_go = asyncio.Event()

    async def wait(agent: Agent, rest: str) -> str:
        await let_go.wait()
        return "Waited."

    harness.ctx.use(COMMANDS).registry.add(harness.ctx, name, Command(wait, "Wait."))
    return let_go


def kinds(served: Served) -> list[str]:
    """What the process sent, in short: an event by its type, where the agent stands, or else
    whether it is an answer."""
    told: list[str] = []
    for message in served.far.heard:
        if message.get("method") == "session.event":
            told.append(message["params"]["event"]["type"])
        elif message.get("method") == "session.status":
            told.append(message["params"]["status"])
        else:
            told.append("an answer")
    return told


def said_by_the_user(served: Served) -> list[str]:
    """What the user said in the session that is open, as the log has it."""
    return [
        block.text
        for event in served.agent().session.events
        if isinstance(event.data, UserMessageEvent) and event.data.source in ("user", "steer")
        for block in event.data.message.content
    ]


@pytest.mark.parametrize("method", ["session/prompt", "session/steer"])
async def test_what_is_given_right_after_a_command_comes_after_the_command(
    served: Served, method: str
) -> None:
    await served.opened()
    command = slow(served)
    served.harness.model.add(Reply(text="Hello!"))

    # One line right after the other. The command has not begun when the second is read.
    asked = served.far.ask("session/command", {"line": "/slow"})
    given = served.far.ask(method, {"text": "Say hello."})
    await asyncio.wait_for(command.begun.wait(), 5)
    taken = await served.far.answer(given)
    before = len(served.harness.model.requests)
    command.let_go.set()
    answer = await served.far.answer(asked)
    await until(lambda: len(served.harness.model.requests) == 1)
    await rested(served)

    # The command was carried out, and the agent did not begin a turn before it.
    assert taken["result"] == {}
    assert before == 0
    assert answer["result"] == {"text": "It is done."}
    assert command.ended == ["it is done"]
    assert said_by_the_user(served) == ["Say hello."]


async def test_what_is_given_during_a_command_stays_in_the_order_in_which_it_came(
    served: Served,
) -> None:
    await served.opened()
    command = slow(served)
    served.harness.model.add(Reply(text="One."), Reply(text="Two."))

    asked = served.far.ask("session/command", {"line": "/slow"})
    # The first comes before the command has begun and is kept back for it.
    await served.far.result("session/prompt", {"text": "First."})
    await asyncio.wait_for(command.begun.wait(), 5)
    # The second comes while the agent is at work on the command. It does not go ahead.
    await served.far.result("session/prompt", {"text": "Second."})
    command.let_go.set()
    await served.far.answer(asked)
    await until(lambda: len(served.harness.model.requests) == 2)
    await rested(served)

    assert said_by_the_user(served) == ["First.", "Second."]


async def test_what_is_given_during_a_command_that_asks_nothing_of_the_agent_waits_for_it(
    served: Served,
) -> None:
    await served.opened()
    let_go = waiting(served)
    served.harness.model.add(Reply(text="Hello!"))

    asked = served.far.ask("session/command", {"line": "/wait"})
    await served.far.result("session/prompt", {"text": "Say hello."})
    await asyncio.sleep(0.05)
    before = (len(served.harness.model.requests), said_by_the_user(served), statuses(served))
    let_go.set()
    answer = await served.far.answer(asked)
    await until(lambda: len(served.harness.model.requests) == 1)
    await rested(served)

    # Nothing of the task was in the log while the command was carried out. The agent
    # was said to be at work from the moment the task was taken: it was about to begin.
    assert before == (0, [], ["idle", "running"])
    assert answer["result"] == {"text": "Waited."}
    assert statuses(served) == ["idle", "running", "idle"]
    # Once it is given, the agent is said to be at work before its turn is told of.
    told = kinds(served)
    assert told.index("running") < told.index("turn/start")


async def test_a_command_that_is_stopped_before_it_has_begun_is_not_carried_out(
    served: Served,
) -> None:
    await served.opened()
    command = slow(served)

    # One line right after the other. Nothing runs yet when the order to stop is read.
    asked = served.far.ask("session/command", {"line": "/slow"})
    stopped = served.far.ask("session/cancel")
    answer = await served.far.answer(asked)

    assert (await served.far.answer(stopped))["result"] == {}
    assert answer["result"] == {"text": "The command /slow was stopped."}
    assert not command.begun.is_set()
    assert command.ended == []
    # Nothing was done, so nothing was told of where the agent stands.
    assert statuses(served) == ["idle"]
    # And the agent takes a command again.
    assert (await served.far.result("session/command", {"line": "/help"}))["text"]


async def test_stopping_drops_what_was_given_while_a_command_was_carried_out(
    served: Served,
) -> None:
    await served.opened()
    waiting(served)

    asked = served.far.ask("session/command", {"line": "/wait"})
    await served.far.result("session/prompt", {"text": "Say hello."})
    await served.far.result("session/cancel")
    answer = await served.far.answer(asked)
    await asyncio.sleep(0.05)

    # The command was stopped, and what waited for it was dropped with it.
    assert answer["result"] == {"text": "The command /wait was stopped."}
    assert len(served.harness.model.requests) == 0
    assert said_by_the_user(served) == []
    # The agent was about to begin for as long as the task waited, and is not any more.
    assert statuses(served) == ["idle", "running", "idle"]


async def test_what_was_given_for_a_session_is_not_given_to_the_one_that_is_opened_next(
    served: Served,
) -> None:
    await served.opened("first")
    let_go = waiting(served)
    served.harness.model.add(Reply(text="Hello!"))

    asked = served.far.ask("session/command", {"line": "/wait"})
    await served.far.result("session/prompt", {"text": "For the first one."})
    await served.far.result("session/open", {"session_id": "second"})
    # The second session has a command of its own, and something is kept back for that.
    command = slow(served)
    second = served.far.ask("session/command", {"line": "/slow"})
    await served.far.result("session/prompt", {"text": "For the second one."})
    await asyncio.wait_for(command.begun.wait(), 5)
    command.let_go.set()
    await served.far.answer(second)
    await until(lambda: len(served.harness.model.requests) == 1)
    await rested(served)
    let_go.set()
    await served.far.answer(asked)

    assert said_by_the_user(served) == ["For the second one."]


async def test_what_is_given_waits_for_the_whole_command_and_not_for_a_part_of_it(
    served: Served,
) -> None:
    await served.opened()
    harness = served.harness
    command = Twice()
    harness.sessions.declare(harness.ctx, *COMPACTION_EVENTS)
    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "twice", Command(command, "Work twice."))
    harness.model.add(Reply(text="Hello!"))

    asked = served.far.ask("session/command", {"line": "/twice"})
    await asyncio.wait_for(command.begun.wait(), 5)
    # The agent is at work for the command when this comes.
    await served.far.result("session/prompt", {"text": "Say hello."})
    command.first.set()
    await asyncio.wait_for(command.waiting.wait(), 5)
    await asyncio.sleep(0.05)
    # The agent rests, and the command is not over: the task still waits.
    between = (len(harness.model.requests), said_by_the_user(served), statuses(served))
    command.between.set()
    answer = await served.far.answer(asked)
    await until(lambda: len(harness.model.requests) == 1)
    await rested(served)

    assert between == (0, [], ["idle", "running"])
    assert answer["result"] == {"text": "Twice."}
    assert said_by_the_user(served) == ["Say hello."]
    # One stretch of work to whoever looks on: the agent was about to begin all the while.
    assert statuses(served) == ["idle", "running", "idle"]


async def test_a_command_that_is_under_way_when_the_program_leaves_is_stopped_and_answered(
    served: Served,
) -> None:
    await served.opened()
    command = slow(served)

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    leaving = served.far.ask("shutdown")

    assert await served.left() == 0
    assert served.far.answers(asked) == [
        {"jsonrpc": "2.0", "id": asked, "result": {"text": "It was stopped."}}
    ]
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]


async def test_a_command_is_stopped_when_its_session_is_opened_again(served: Served) -> None:
    await served.opened("mine")
    command = slow(served)

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    again = await served.far.result("session/open", {"session_id": "mine"})
    answer = await served.far.answer(asked)

    assert answer["result"] == {"text": "It was stopped."}
    assert (again["session_id"], again["resumed"]) == ("mine", True)
    # The command was answered before the session was open again.
    heard = served.far.heard
    assert heard.index(answer) < next(
        place for place, one in enumerate(heard) if one.get("result") == again
    )
    assert (await served.far.result("session/command", {"line": "/model"}))["text"]


async def test_what_is_done_on_the_side_is_given_a_moment_to_end_when_the_program_leaves(
    served: Served,
) -> None:
    await served.opened()
    harness = served.harness
    leaving = asyncio.Event()

    async def lingering(agent: Agent, rest: str) -> str:
        # It has nothing to do with the agent, and takes a little longer than its ending.
        await leaving.wait()
        await asyncio.sleep(0.05)
        return "Done, if late."

    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "linger", Command(lingering, "Linger."))

    asked = served.far.ask("session/command", {"line": "/linger"})
    left = served.far.ask("shutdown")
    leaving.set()

    assert await served.left() == 0
    assert served.far.answers(asked) == [
        {"jsonrpc": "2.0", "id": asked, "result": {"text": "Done, if late."}}
    ]
    # It was answered before the order to leave was.
    heard = served.far.heard
    assert heard.index(served.far.answers(asked)[0]) < heard.index(served.far.answers(left)[0])


async def test_what_does_not_end_on_the_side_is_broken_off_when_the_program_leaves(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(server_module, "_LAST_WORK_SECONDS", 0.05)
    await served.opened()
    harness = served.harness
    ended: list[str] = []

    async def stuck(agent: Agent, rest: str) -> str:
        try:
            await asyncio.Event().wait()
        finally:
            ended.append("broken off")
        return "Never."

    harness.ctx.use(COMMANDS).registry.add(harness.ctx, "stuck", Command(stuck, "Be stuck."))

    asked = served.far.ask("session/command", {"line": "/stuck"})
    await asyncio.sleep(0.05)
    leaving = served.far.ask("shutdown")

    assert await served.left() == 0
    assert ended == ["broken off"]
    # It is answered all the same, like every request, and before the order to leave is.
    assert served.far.answers(asked) == [
        {"jsonrpc": "2.0", "id": asked, "result": {"text": "The command /stuck was stopped."}}
    ]
    heard = served.far.heard
    assert heard.index(served.far.answers(asked)[0]) < heard.index(served.far.answers(leaving)[0])


async def test_what_was_kept_back_is_given_to_nobody_when_the_program_leaves(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(server_module, "_LAST_WORK_SECONDS", 0.05)
    await served.opened()
    waiting(served)

    asked = served.far.ask("session/command", {"line": "/wait"})
    given = await served.far.result("session/prompt", {"text": "Say hello."})
    leaving = served.far.ask("shutdown")

    assert await served.left() == 0
    # The task was taken, and the agent was ended before the command was over.
    assert given == {}
    assert served.far.answers(asked) == [
        {"jsonrpc": "2.0", "id": asked, "result": {"text": "The command /wait was stopped."}}
    ]
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]
    assert len(served.harness.model.requests) == 0


async def test_a_list_that_does_not_end_is_broken_off_when_the_program_leaves_and_says_so(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(server_module, "_LAST_WORK_SECONDS", 0.05)
    await served.greeted()
    looked = threading.Event()
    let_go = threading.Event()

    def never(self: Sessions, cwd: object) -> list[Listed]:
        looked.set()
        let_go.wait(20)
        return []

    monkeypatch.setattr(Sessions, "stored", never)

    try:
        asked = served.far.ask("session/list")
        await asyncio.to_thread(looked.wait, 10)
        leaving = served.far.ask("shutdown")

        assert await served.left() == 0
    finally:
        let_go.set()
    assert served.far.answers(asked) == [
        {
            "jsonrpc": "2.0",
            "id": asked,
            "error": {
                "code": LEAVING,
                "message": "The program is leaving, and this was broken off.",
            },
        }
    ]
    assert served.far.answers(leaving) == [{"jsonrpc": "2.0", "id": leaving, "result": {}}]


async def test_a_command_of_the_session_that_was_left_does_not_count_for_the_one_that_is_open(
    served: Served,
) -> None:
    await served.opened("first")
    command = slow(served)

    asked = served.far.ask("session/command", {"line": "/slow"})
    await asyncio.wait_for(command.begun.wait(), 5)
    await served.far.result("session/open", {"session_id": "second"})
    answer = await served.far.answer(asked)
    await until(lambda: list(served.harness.agents.live) == ["second"])

    # The agent of the second session rests from the start, whatever the first one was at.
    of_the_second = [
        told["status"]
        for told in served.far.told("session.status")
        if told["session_id"] == "second"
    ]
    assert of_the_second == ["idle"]
    assert answer["result"] == {"text": "It was stopped."}


async def test_a_command_of_a_session_that_was_left_keeps_nothing_back_from_the_one_that_is_open(
    served: Served,
) -> None:
    await served.opened("first")
    let_go = waiting(served)
    served.harness.model.add(Reply(text="Hello!"))

    asked = served.far.ask("session/command", {"line": "/wait"})
    await served.far.result("session/open", {"session_id": "second"})
    # The command of the first session still waits. A task for the second does not.
    await served.far.result("session/prompt", {"text": "Say hello."})
    await until(lambda: len(served.harness.model.requests) == 1)
    await rested(served)
    let_go.set()
    await served.far.answer(asked)

    assert said_by_the_user(served) == ["Say hello."]


async def test_a_command_that_outlasts_its_session_does_not_hold_up_the_one_that_is_open(
    served: Served,
) -> None:
    await served.opened("first")
    old = waiting(served)

    asked = served.far.ask("session/command", {"line": "/wait"})
    await served.far.result("session/open", {"session_id": "second"})
    # The command of the first session still waits. The second one takes one of its own.
    new = waiting(served, "pause")
    paused = served.far.ask("session/command", {"line": "/pause"})
    old.set()
    answer = await served.far.answer(asked)
    # The end of the old command ended nothing of the new one, which is still carried out.
    refused = await served.far.refusal("session/command", {"line": "/help"})
    new.set()

    assert answer["result"] == {"text": "Waited."}
    assert refused == NOT_AT_REST
    assert (await served.far.answer(paused))["result"] == {"text": "Waited."}
    assert (await served.far.result("session/command", {"line": "/help"}))["text"]


# The model


async def test_the_model_can_be_named_anew_and_the_next_request_goes_to_it(served: Served) -> None:
    await served.opened()
    served.harness.model.add(Reply(text="Hello!"))

    answer = await served.far.result("session/command", {"line": "/model swift-2"})
    told_at_once = [told["model"]["model"] for told in served.far.told("session.status")]
    await served.far.result("session/prompt", {"text": "Say hello."})
    await rested(served)

    assert answer == {"text": "The model is swift-2 from the next request on."}
    # It is told with the command, and not only when the agent is given something next.
    assert told_at_once == ["script", "swift-2"]
    assert served.harness.model.requests[0].config.model == "swift-2"
    models = [told["model"]["model"] for told in served.far.told("session.status")]
    # It is told when it changes, before anything is asked of it.
    assert models == ["script", "swift-2", "swift-2", "swift-2"]
    assert statuses(served) == ["idle", "idle", "running", "idle"]
    said = [
        event.data.message.content[0].text
        for event in served.agent().session.events
        if isinstance(event.data, UserMessageEvent)
    ]
    assert said == ["Say hello."]


async def test_the_model_that_was_named_stays_for_the_session_that_is_opened_next(
    served: Served,
) -> None:
    await served.opened("first")
    await served.far.result("session/command", {"line": "/model swift-2"})

    await served.far.result("session/open", {"session_id": "second"})
    await served.far.until(lambda: served.far.told("session.status")[-1]["session_id"] == "second")

    assert served.far.told("session.status")[-1]["model"] == {
        "provider": "scripted",
        "model": "swift-2",
        "context_window": None,
    }


async def test_a_model_that_is_named_while_no_session_is_open_is_told_with_the_next_one(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    await served.opened("mine")
    first = served.agent()

    async def torn(*args: object, **kwargs: object) -> Agent:
        raise SessionError("it is torn")

    # The session is closed to be opened again, and cannot be: none is open after that.
    monkeypatch.setattr("columbina.apps.serve.server.open_agent", torn)
    await served.far.refusal("session/open", {"session_id": "mine"})
    monkeypatch.undo()
    before = len(served.far.told("session.status"))

    # A command of the session that was open comes to its end only now.
    said = await served.harness.ctx.use(COMMANDS).run(first, "/model swift-2")
    between = len(served.far.told("session.status"))
    await served.far.result("session/open", {"session_id": "second"})

    assert said == "The model is swift-2 from the next request on."
    # There is no agent to tell anything of. The next one is told of with the model.
    assert between == before
    assert served.far.told("session.status")[-1]["model"]["model"] == "swift-2"
    # Another such command comes to its end while the second session is open. The model is
    # that of the program, so it is told at once, of the session that is open.
    await served.harness.ctx.use(COMMANDS).run(first, "/model swift-3")
    last = served.far.told("session.status")[-1]
    assert (last["session_id"], last["model"]["model"]) == ("second", "swift-3")


# What a session may do


async def permitted(served: Served, preset: str = "workspace-write") -> None:
    """Load what keeps a session in check, with the command that shows and changes it."""
    harness = served.harness
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await harness.ctx.plugin(plugin)
    await harness.ctx.plugin(columbina.permissions, {"preset": preset})
    await harness.ctx.plugin(columbina.commands.permissions)
    await served.opened()


async def test_what_a_session_may_do_is_told_from_the_start(served: Served) -> None:
    await permitted(served)

    (told,) = served.far.told("session.status")

    # Before the first turn the log names neither. What is told is what it will name.
    assert (told["sandbox"], told["approval"]) == (
        {"mode": "workspace-write", "strength": "none"},
        "ask",
    )


async def test_what_a_session_may_do_is_told_anew_when_it_is_changed(served: Served) -> None:
    await permitted(served)

    answer = await served.far.result("session/command", {"line": "/permissions read-only"})
    await served.far.until(lambda: len(served.far.told("session.status")) == 2)

    assert answer["text"].startswith("sandbox mode: read-only\n")
    first, second = served.far.told("session.status")
    assert (first["sandbox"]["mode"], second["sandbox"]["mode"]) == ("workspace-write", "read-only")
    assert second["status"] == "idle"


async def test_a_session_that_may_do_anything_and_asks_nobody_says_so(served: Served) -> None:
    await permitted(served, "full-access")

    (told,) = served.far.told("session.status")

    assert told["sandbox"]["mode"] == "danger-full-access"
    assert told["approval"] in ("ask", "never")


async def test_only_the_policy_of_approval_is_told_where_nothing_else_is_loaded(
    served: Served,
) -> None:
    await served.harness.ctx.plugin(columbina.approval, {"policy": "never"})
    await served.opened()

    (told,) = served.far.told("session.status")

    assert (told["sandbox"], told["approval"]) == (None, "never")


async def test_only_the_sandbox_is_told_where_nothing_else_is_loaded(served: Served) -> None:
    await served.harness.ctx.plugin(columbina.fs)
    await served.harness.ctx.plugin(columbina.sandbox, {"mode": "read-only"})
    await served.opened()

    (told,) = served.far.told("session.status")

    assert (told["sandbox"], told["approval"]) == ({"mode": "read-only", "strength": "none"}, None)


async def test_how_well_the_commands_of_a_session_are_held_is_told_as_what_holds_them_says_it(
    served: Served,
) -> None:
    # A desktop application shows it. Where commands run in a container it is no "none".
    ctx = served.harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, {"provider": "holding"})
    ctx.use(SANDBOX).providers.add(ctx, "holding", Holding("partial"))
    await served.opened()

    (told,) = served.far.told("session.status")

    assert told["sandbox"] == {"mode": "workspace-write", "strength": "partial"}
