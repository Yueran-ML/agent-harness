"""The command that makes the conversation shorter when a person asks for it."""

import asyncio

import pytest

import columbina.commands
import columbina.compaction.basic
import columbina.token_meter
from columbina.agent import Agent
from columbina.commands import COMMANDS, Commands
from columbina.commands import compact as compact_command
from columbina.compaction import COMPACTION, COMPACTIONS, EVENTS, Outcome, Why
from columbina.llm import CallConfig, LlmFailure
from columbina.llm.adapters.scripted import Reply
from columbina.token_meter import estimate
from support.loop import Harness, assistant, idle

MODEL = CallConfig(provider="scripted", model="script", context_window=20_000)


def words(tokens: int) -> str:
    text = "word " * round(tokens / 1.11)
    assert abs(estimate(text) - tokens) <= 2
    return text


async def loaded(harness: Harness) -> Commands:
    await harness.ctx.plugin(columbina.token_meter)
    await harness.ctx.plugin(columbina.compaction.basic)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(compact_command)
    return harness.ctx.use(COMMANDS)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


class Saying:
    """A service that makes nothing shorter and says that it did, or waits to be stopped."""

    def __init__(self, outcome: Outcome | None) -> None:
        self.outcome = outcome
        self.asked: list[tuple[Agent, Why]] = []
        self.focus: list[str | None] = []
        self.begun = asyncio.Event()

    async def compact(self, agent: Agent, why: Why, focus: str | None = None) -> Outcome:
        self.asked.append((agent, why))
        self.focus.append(focus)
        self.begun.set()
        if self.outcome is None:
            await asyncio.Event().wait()
        assert self.outcome is not None
        return self.outcome


async def saying(harness: Harness, outcome: Outcome | None) -> tuple[Commands, Saying]:
    service = Saying(outcome)
    harness.sessions.declare(harness.ctx, *EVENTS)
    harness.ctx.provide(COMPACTION, service)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(compact_command)
    return harness.ctx.use(COMMANDS), service


async def test_a_conversation_is_summed_up_when_a_person_asks_for_it(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."))

    said = await commands.run(agent, "/compact")

    last = COMPACTIONS.of(agent.session).last
    assert last is not None
    assert (last.trimmed, last.summarised, last.error) == (0, 2, None)
    assert said == (
        "The conversation was made shorter: 2 messages gave way to a summary. It came to about "
        f"{last.before} tokens and comes to about {last.after} now."
    )
    assert last.before > 8000 > last.after > 4000
    await idle(agent)
    # The agent goes on from there.
    harness.model.add(Reply(text="Three."))
    await turn(agent, "And?")
    assert agent.session.messages[-1] == assistant("Three.")


async def test_the_command_is_listed_with_what_it_is_for(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")

    assert (
        "/compact  Sum up the older part of the conversation to make room: /compact [WHAT TO KEEP]."
    ) in listed


async def test_words_after_the_command_say_what_the_summary_keeps(harness: Harness) -> None:
    commands, service = await saying(harness, Outcome(before=300, after=300))
    agent = await harness.agent()

    await commands.run(agent, "/compact   the names of the files  ")
    await commands.run(agent, "/compact")

    assert service.focus == ["the names of the files", None]


async def test_the_summary_is_asked_to_keep_what_the_person_said(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."))

    await commands.run(agent, "/compact the numbers")

    asked = harness.model.requests[-1].messages[-1]
    assert asked.content[0].text.endswith(  # type: ignore[union-attr]
        "\n\nThe user asked that the handover keep this above all, in their words:\nthe numbers"
    )


@pytest.mark.parametrize(
    ("outcome", "said"),
    [
        (
            Outcome(summarised=1, before=900, after=400),
            "The conversation was made shorter: 1 message gave way to a summary. "
            "It came to about 900 tokens and comes to about 400 now.",
        ),
        (
            Outcome(trimmed=1, before=900, after=400),
            "The conversation was made shorter: 1 long result of a tool was cut down to the "
            "beginning and the end. It came to about 900 tokens and comes to about 400 now.",
        ),
        (
            Outcome(trimmed=3, summarised=12, before=9000, after=1200),
            "The conversation was made shorter: 12 messages gave way to a summary and 3 long "
            "results of tools were cut down to the beginning and the end. It came to about "
            "9000 tokens and comes to about 1200 now.",
        ),
        (
            Outcome(trimmed=2, before=9000, after=7000, error="AUTH: no such key"),
            "The conversation was made shorter: 2 long results of tools were cut down to the "
            "beginning and the end. No summary came of it: AUTH: no such key. It came to about "
            "9000 tokens and comes to about 7000 now.",
        ),
        (
            Outcome(before=300, after=300),
            "There is nothing in this conversation that could go yet. It comes to about 300 "
            "tokens.",
        ),
        (
            Outcome(before=9000, after=9000, error="AUTH: no such key"),
            "The conversation is as it was: AUTH: no such key. It comes to about 9000 tokens.",
        ),
    ],
)
async def test_the_answer_says_what_was_done_and_what_the_conversation_comes_to(
    harness: Harness, outcome: Outcome, said: str
) -> None:
    commands, service = await saying(harness, outcome)
    agent = await harness.agent()

    assert await commands.run(agent, "/compact") == said

    assert service.asked == [(agent, "manual")]
    await idle(agent)


async def test_a_summary_that_fails_is_said_to_have_failed(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(failure=LlmFailure(message="no such key", code="AUTH", status=401)))

    said = await commands.run(agent, "/compact")

    assert said.startswith("The conversation is as it was: AUTH: no such key. It comes to about ")


async def test_an_agent_that_is_not_at_rest_is_left_to_its_work(harness: Harness) -> None:
    commands, service = await saying(harness, Outcome(before=1, after=1))
    agent = await harness.agent(Reply(text="thinking", hold=True))
    agent.followup("Go.")
    await asyncio.wait_for(harness.model.held(), timeout=5)

    said = await commands.run(agent, "/compact")

    assert said == (
        "The agent is in the middle of something. Wait until it is at rest, or stop it, and "
        "ask again."
    )
    assert service.asked == []
    harness.model.release()
    await idle(agent)


async def test_it_is_done_between_turns_and_input_waits_for_it(harness: Harness) -> None:
    commands, service = await saying(harness, None)
    agent = await harness.agent(Reply(text="One."))
    asking = asyncio.ensure_future(commands.run(agent, "/compact"))
    await asyncio.wait_for(service.begun.wait(), timeout=5)

    agent.followup("Go.")
    await asyncio.sleep(0.05)

    assert harness.model.requests == []
    agent.cancel(keep_inbox=True)
    assert await asking == "It was stopped. The conversation is as it was."
    # Nobody stopped the one who asked. The agent was told to stop, and the work with it.
    agent.wake()
    await idle(agent)
    assert agent.session.messages[-1] == assistant("One.")


async def test_the_one_who_asks_can_be_stopped_too(harness: Harness) -> None:
    commands, service = await saying(harness, None)
    agent = await harness.agent()
    asking = asyncio.ensure_future(commands.run(agent, "/compact"))
    await asyncio.wait_for(service.begun.wait(), timeout=5)

    asking.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asking
    agent.cancel()
    await idle(agent)
