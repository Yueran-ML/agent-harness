"""The command that shows the list of things to do."""

from pydantic import JsonValue

import columbina.commands
import columbina.todo
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import todos as todos_command
from columbina.llm.adapters.scripted import Call, Reply
from columbina.todo import TOOL
from support.loop import Harness, idle


def wrote(*things: tuple[str, str]) -> Reply:
    items: list[JsonValue] = [{"content": content, "status": status} for content, status in things]
    return Reply(calls=(Call(name=TOOL, arguments={"items": items}),))


async def loaded(harness: Harness) -> Commands:
    await harness.ctx.plugin(columbina.todo)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(todos_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent) -> None:
    agent.followup("Go on.")
    await idle(agent)


async def test_the_list_is_shown_as_it_stands(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(
        wrote(("Read the check", "completed"), ("Mend calc.py", "in_progress")),
        wrote(
            ("Mend calc.py", "completed"), ("Run the check", "in_progress"), ("Say so", "pending")
        ),
        Reply(text="On it."),
    )
    await turn(agent)

    said = await commands.run(agent, "/todos")

    assert said == "[x] Mend calc.py\n[>] Run the check\n[ ] Say so"
    # The model is told nothing about it, and the list is as it was.
    assert await commands.run(agent, "/todos") == said


async def test_an_agent_that_wrote_nothing_down_has_nothing_to_show(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(wrote(("Read the check", "pending")), wrote(), Reply(text="Done."))

    assert await commands.run(agent, "/todos") == "The agent has written down nothing to do."
    await turn(agent)
    # A list that was emptied is no list.
    assert await commands.run(agent, "/todos") == "The agent has written down nothing to do."


async def test_the_command_is_listed_and_leaves_with_its_plugin(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.commands)
    plugin = await harness.ctx.plugin(todos_command)
    commands = harness.ctx.use(COMMANDS)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")
    assert "/todos  Show the list of things to do that the agent keeps." in listed

    await plugin.dispose()

    assert await commands.run(agent, "/todos") == (
        "There is no command /todos. /help lists the commands."
    )
