"""Calls that are made again, with nothing changed since."""

from collections.abc import Awaitable, Callable

import pytest
from pydantic import BaseModel, JsonValue, ValidationError

from columbina.agent import Agent
from columbina.guard import repeats
from columbina.kernel import Next
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent
from columbina.tools import (
    PRE_EXECUTE,
    Decision,
    Deny,
    Invocation,
    Tool,
    ToolRun,
)
from columbina.tools.tool import Effect
from support.loop import Harness, idle


class Args(BaseModel):
    path: str = ""
    lines: int = 0


async def did(args: Args, run: ToolRun) -> str:
    return f"did {args.path}"


async def failed(args: Args, run: ToolRun) -> str:
    raise RuntimeError("no")


def tool(
    name: str, effect: Effect, execute: Callable[[Args, ToolRun], Awaitable[str]] = did
) -> Tool[Args, str]:
    return Tool(name=name, description="Do it.", parameters=Args, execute=execute, effect=effect)


async def loaded(harness: Harness, **config: object) -> None:
    harness.tools.add(
        harness.ctx,
        tool("look", "read"),
        tool("change", "write"),
        tool("command", "run"),
        tool("think", "none"),
        tool("fail", "read", failed),
    )
    await harness.ctx.plugin(repeats, config)


def calls(*made: Call) -> Reply:
    return Reply(calls=made)


def look(path: str = "a.txt", **more: JsonValue) -> Call:
    return Call(name="look", arguments={"path": path, **more})


def results(agent: Agent) -> list[str]:
    return [
        "".join(block.text for block in event.data.message.content)
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def codes(agent: Agent) -> list[str | None]:
    return [
        None if event.data.error is None else event.data.error.code
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


async def turn(agent: Agent, text: str = "Go.") -> None:
    agent.followup(text)
    await idle(agent)


WARNED = "(This is the same call as before, made {} times in this turn"


async def test_the_third_time_is_told_and_the_fifth_does_not_run(harness: Harness) -> None:
    await loaded(harness)
    script = [calls(look()) for _ in range(5)]
    agent = await harness.agent(*script, Reply(text="Done."))

    await turn(agent)

    said = results(agent)
    assert said[:2] == ["did a.txt", "did a.txt"]
    assert said[2] == "did a.txt\n" + repeats.warning(3)
    assert said[2].startswith("did a.txt\n" + WARNED.format(3))
    assert said[3] == "did a.txt\n" + repeats.warning(4)
    assert codes(agent) == [None, None, None, None, repeats.REPEATED_CALL]
    assert said[4] == "The call was not allowed: " + repeats.refusal(5)
    assert "made 4 times in this turn already" in said[4]


async def test_arguments_are_the_same_whatever_their_order_and_spaces(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        calls(Call(name="look", arguments='{"path": "a.txt", "lines": 2}')),
        calls(Call(name="look", arguments='{"lines":2,"path":"a.txt"}')),
        calls(look(lines=2)),
        Reply(text="Done."),
    )

    await turn(agent)

    assert results(agent)[2].endswith(repeats.warning(3))


async def test_other_arguments_and_other_tools_are_other_calls(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        calls(look("a.txt")),
        calls(look("b.txt")),
        calls(look("a.txt", lines=1)),
        calls(Call(name="think", arguments={"path": "a.txt"})),
        calls(look("a.txt")),
        Reply(text="Done."),
    )

    await turn(agent)

    assert all("same call" not in said for said in results(agent))


async def test_a_write_that_went_through_clears_the_count(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        calls(look()),
        calls(look()),
        calls(Call(name="change", arguments={"path": "a.txt"})),
        calls(look()),
        calls(look()),
        calls(Call(name="change", arguments={"path": "a.txt"})),
        Reply(text="Done."),
    )

    await turn(agent)

    assert all("same call" not in said for said in results(agent))


async def test_a_command_clears_the_count_of_the_others_and_not_its_own(
    harness: Harness,
) -> None:
    await loaded(harness)
    command = Call(name="command", arguments={"path": "pytest"})
    agent = await harness.agent(
        calls(look()),
        calls(look()),
        calls(command),
        calls(look()),
        calls(command),
        calls(command),
        Reply(text="Done."),
    )

    await turn(agent)

    said = results(agent)
    assert said[3] == "did a.txt"
    assert said[4] == "did pytest"
    assert said[5] == "did pytest\n" + repeats.warning(3)


async def test_a_call_that_failed_counts_and_changes_nothing(harness: Harness) -> None:
    await loaded(harness, warn_at=2, refuse_at=3)
    failing = Call(name="fail", arguments={"path": "a.txt"})
    agent = await harness.agent(calls(failing), calls(failing), calls(failing), Reply(text="Ok."))

    await turn(agent)

    assert codes(agent) == ["TOOL_ERROR", "TOOL_ERROR", repeats.REPEATED_CALL]


async def test_a_new_turn_counts_afresh(harness: Harness) -> None:
    await loaded(harness, warn_at=2, refuse_at=3)
    agent = await harness.agent(
        calls(look()),
        calls(look()),
        Reply(text="Done."),
        calls(look()),
        Reply(text="Done again."),
    )

    await turn(agent)
    await turn(agent, "Again.")

    said = results(agent)
    assert said[1].endswith(repeats.warning(2))
    assert said[2] == "did a.txt"


async def test_a_call_that_a_later_check_refused_is_not_counted(harness: Harness) -> None:
    await loaded(harness, warn_at=2, refuse_at=3)
    refused: list[bool] = [True, True, False]

    async def refuse(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        if refused.pop(0):
            return Deny("not now")
        return await proceed(invocation)

    harness.ctx.on(PRE_EXECUTE, refuse)
    agent = await harness.agent(calls(look()), calls(look()), calls(look()), Reply(text="Ok."))

    await turn(agent)

    assert codes(agent) == ["DENIED", "DENIED", None]
    assert results(agent)[2] == "did a.txt"


def test_the_limits_come_in_order() -> None:
    assert repeats.Config(warn_at=2, refuse_at=3).refuse_at == 3
    with pytest.raises(ValidationError, match="refuse_at must come after warn_at"):
        repeats.Config(warn_at=3, refuse_at=3)
    with pytest.raises(ValidationError):
        repeats.Config(warn_at=1)
