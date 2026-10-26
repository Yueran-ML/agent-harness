"""Whether a turn did what was asked, as the person who asked says."""

import pytest

import columbina.commands
from columbina import verdicts
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import verdict as verdict_command
from columbina.llm.adapters.scripted import Reply
from columbina.verdicts import VerdictEvent, judge, last_turn
from support.loop import Harness, idle


async def loaded(harness: Harness) -> Commands:
    await harness.ctx.plugin(verdicts)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(verdict_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str = "Go.") -> None:
    agent.followup(text)
    await idle(agent)


def said(agent: Agent) -> list[VerdictEvent]:
    return [event.data for event in agent.session.events if isinstance(event.data, VerdictEvent)]


async def test_the_last_turn_that_ended_is_judged(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    await turn(agent)
    await turn(agent)

    assert last_turn(agent.session) == 2
    assert judge(agent.session, True) == 2
    assert judge(agent.session, False, "It changed the wrong file.", turn=1) == 1
    assert said(agent) == [
        VerdictEvent(turn=2, right=True),
        VerdictEvent(turn=1, right=False, note="It changed the wrong file."),
    ]


async def test_a_turn_that_has_not_ended_cannot_be_judged(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(Reply(text="One."))

    with pytest.raises(ValueError, match="no turn has come to its end yet"):
        judge(agent.session, True)
    await turn(agent)
    for named in (0, 2):
        with pytest.raises(ValueError, match=f"turn {named} has not come to its end"):
            judge(agent.session, True, turn=named)


async def test_the_command_writes_down_what_the_person_says(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    await turn(agent)
    await turn(agent)

    assert await commands.run(agent, "/verdict right") == "Turn 2 is written down as right."
    assert await commands.run(agent, "/verdict wrong #1 not what I meant") == (
        "Turn 1 is written down as wrong."
    )
    assert await commands.run(agent, "/verdict wrong #9") == (
        "Nothing was written down: turn 9 has not come to its end."
    )
    assert (await commands.run(agent, "/verdict maybe")).startswith("Say /verdict right")
    assert said(agent) == [
        VerdictEvent(turn=2, right=True),
        VerdictEvent(turn=1, right=False, note="not what I meant"),
    ]


async def test_the_command_is_listed(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")

    assert "/verdict  Say whether the last turn did what was asked" in listed
