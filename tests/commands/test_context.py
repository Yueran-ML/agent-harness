"""The command that says what the conversation is made of."""

from pydantic import BaseModel

import columbina.commands
import columbina.compaction.basic
import columbina.token_meter
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import context as context_command
from columbina.compaction import SUMMARY_SOURCE
from columbina.llm import CallConfig, TextBlock, UserMessage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import UserMessageEvent
from columbina.token_meter import TOKEN_METER
from columbina.tools import Tool, ToolRun
from support.loop import Harness, idle

MODEL = CallConfig(provider="scripted", model="script", context_window=20_000)


class Args(BaseModel):
    size: int = 1


async def said(args: Args, run: ToolRun) -> str:
    return "word " * args.size


async def loaded(harness: Harness) -> Commands:
    harness.tools.add(
        harness.ctx,
        Tool(name="look", description="Look.", parameters=Args, execute=said),
        Tool(name="peek", description="Peek.", parameters=Args, execute=said),
    )
    await harness.ctx.plugin(columbina.token_meter)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(context_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


def note(agent: Agent, source: str, text: str) -> None:
    message = UserMessage(content=(TextBlock(text=text),))
    agent.session.append(UserMessageEvent(message=message, source=source))


async def test_the_parts_of_the_conversation_are_said_with_the_room_that_is_left(
    harness: Harness,
) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(
        Reply(
            calls=(
                Call(name="look", arguments={"size": 400}),
                Call(name="peek", arguments={"size": 40}),
                Call(name="look", arguments={"size": 4}),
                Call(name="look", arguments={"size": 1}),
            )
        ),
        Reply(text="Done."),
        model=MODEL,
    )
    await turn(agent, "Look around.")
    note(agent, "context/instructions", "Be brief.")
    note(agent, SUMMARY_SOURCE, "Earlier, things were said.")

    told = await commands.run(agent, "/context")

    lines = told.split("\n")
    assert lines[0].startswith("The conversation comes to about ")
    assert lines[0].endswith(
        " tokens of the 20,000 that the model takes (" + lines[0].split("(")[1]
    )
    assert [line.split(":")[0] for line in lines[1:-1]] == [
        "- what the program tells the model of its work",
        "- instructions of the project and of the user, and facts of this run",
        "- the tools that are declared (2)",
        "- the summary of the earlier part",
        "- what was written to the model",
        "- what the model wrote",
        "- what tools gave back",
    ]
    assert lines[-1].startswith("The largest results of tools: look ")
    assert ", peek " in lines[-1]
    assert lines[-1].count("look") == 2


async def test_without_a_window_the_size_is_said_alone(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="Hi."))

    told = await commands.run(agent, "/context")

    assert told.split("\n")[0].endswith(" tokens. How many the model takes is not known.")
    assert "results of tools" not in told
    assert "summary" not in told


async def test_before_any_request_nothing_is_declared(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(model=MODEL)

    told = context_command.described(agent, harness.ctx.use(TOKEN_METER))

    assert "tools that are declared" not in told
    assert "of the 20,000 that the model takes" in told


async def test_the_rate_of_the_provider_is_applied_to_each_part(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="Hi."), model=MODEL)
    await turn(agent, "Hello.")
    meter = harness.ctx.use(TOKEN_METER)
    measured = meter.measure(agent.session)

    told = await commands.run(agent, "/context")

    assert f"about {measured.tokens:,} tokens" in told


async def test_the_command_is_listed(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")

    assert "/context  Say what the conversation is made of, and how much room is left." in listed
    assert columbina.compaction.basic is not None
