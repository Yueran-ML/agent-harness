"""Plan mode: the model looks and thinks, and changes nothing until the user says so."""

import pytest
from pydantic import BaseModel

import columbina.commands
from columbina import plan
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import plan as plan_command
from columbina.llm import UserMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent
from columbina.tools import Tool, ToolRun
from columbina.tools.tool import Effect
from support.loop import Harness, idle


class Args(BaseModel):
    pass


async def did(args: Args, run: ToolRun) -> str:
    return "done"


async def loaded(harness: Harness, **config: object) -> Commands:
    effects: tuple[tuple[str, Effect], ...] = (
        ("change", "write"),
        ("command", "run"),
        ("look", "read"),
    )
    for name, effect in effects:
        harness.tools.add(
            harness.ctx,
            Tool(name=name, description=name, parameters=Args, execute=did, effect=effect),
        )
    await harness.ctx.plugin(plan, config)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(plan_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str = "Go.") -> None:
    agent.followup(text)
    await idle(agent)


def codes(agent: Agent) -> list[str | None]:
    return [
        None if event.data.error is None else event.data.error.code
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def told(harness: Harness, number: int) -> str:
    """Return what the model was told in a request, all of it, as one text."""
    request = harness.model.requests[number]
    return "\n".join(
        block.text
        for message in request.messages
        if isinstance(message, UserMessage)
        for block in message.content
    )


def calls(*names: str) -> Reply:
    return Reply(calls=tuple(Call(name=name) for name in names))


async def test_in_plan_mode_nothing_is_written_or_run_and_the_model_is_told(
    harness: Harness,
) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(
        calls("look", "change", "command"),
        Reply(text="1. Change it."),
        calls("change"),
        Reply(text="Changed."),
    )

    assert await commands.run(agent, "/plan") == (
        "Plan mode: the model looks and thinks, and changes nothing. Say what you want, and /plan "
        "again when the plan is right."
    )
    await turn(agent, "Plan it.")
    assert await commands.run(agent, "/plan") == (
        "Plan mode is over: the model carries out the plan from the next turn."
    )
    await turn(agent, "Do it.")

    assert codes(agent) == [None, plan.IN_PLAN_MODE, plan.IN_PLAN_MODE, None]
    assert plan.PLANNING in told(harness, 0)
    assert plan.LEFT not in told(harness, 0)
    assert plan.LEFT in told(harness, 2)
    refused = [
        event.data.message.content[0].text
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent) and event.data.error is not None
    ]
    assert refused[0] == f"The call was not allowed: {plan.OUT_OF_PLAN_MODE}"


async def test_a_conversation_that_never_planned_is_told_nothing(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(Reply(text="Hi."))

    await turn(agent)

    assert plan.PLANNING not in told(harness, 0)
    assert not plan.planning(agent.session)


async def test_a_subagent_of_a_conversation_in_plan_mode_changes_nothing(
    harness: Harness,
) -> None:
    await loaded(harness)
    parent = await harness.agent()
    plan.set_planning(parent.session, True)
    harness.model.add(calls("change", "look"), Reply(text="Found it."))
    child = await harness.agents.create(
        harness.ctx, harness.spec(), session_id="s-child", parent=parent.id, delegation_depth=1
    )

    await turn(child)

    assert codes(child) == [plan.IN_PLAN_MODE, None]


async def test_a_subagent_whose_conversation_is_gone_is_not_held(harness: Harness) -> None:
    await loaded(harness)
    harness.model.add(calls("change"), Reply(text="Done."))
    child = await harness.agents.create(
        harness.ctx, harness.spec(), session_id="s-child", parent="gone", delegation_depth=1
    )

    await turn(child)

    assert codes(child) == [None]


async def test_plan_mode_may_ask_another_model(harness: Harness) -> None:
    commands = await loaded(harness, model="thinker")
    agent = await harness.agent(Reply(text="1. A plan."), Reply(text="Done."))

    await commands.run(agent, "/plan on")
    await turn(agent, "Plan.")
    await commands.run(agent, "/plan off")
    await turn(agent, "Do it.")

    assert [request.config.model for request in harness.model.requests] == ["thinker", "script"]


@pytest.mark.parametrize(
    ("first", "then", "said"),
    [
        ("/plan on", "/plan on", "The conversation is in plan mode already."),
        ("/plan off", "/plan off", "Plan mode is not on."),
        ("/plan", "/plan maybe", "Say /plan, /plan on or /plan off."),
    ],
)
async def test_what_the_command_cannot_do_is_said(
    harness: Harness, first: str, then: str, said: str
) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()
    await commands.run(agent, first)

    assert await commands.run(agent, then) == said


async def test_the_command_is_listed(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()

    assert "/plan  Go into plan mode, where nothing is changed, or out of it." in (
        await commands.run(agent, "/help")
    )


def test_a_model_that_is_named_as_nothing_is_none() -> None:
    assert plan.Config.model_validate({"model": ""}).model is None
    assert plan.Config.model_validate({"model": "thinker"}).model == "thinker"
