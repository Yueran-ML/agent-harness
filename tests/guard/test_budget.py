"""The budget of a turn: how many steps, how many tokens and how much time it may take."""

import pytest
from pydantic import BaseModel, ValidationError

from columbina.agent import PRE_STEP, Agent, StepDecision, StepInfo
from columbina.guard import budget
from columbina.kernel import Next
from columbina.llm import Usage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import TurnEndEvent
from columbina.tools import Tool, ToolRun
from support.loop import Harness, idle


class Args(BaseModel):
    pass


async def did(args: Args, run: ToolRun) -> str:
    return "did it"


def works(usage: Usage | None = None) -> Reply:
    """A reply that calls a tool, so that another step follows it."""
    return Reply(calls=(Call(name="work"),), usage=usage)


async def loaded(harness: Harness, **config: object) -> None:
    harness.tools.add(
        harness.ctx, Tool(name="work", description="Do it.", parameters=Args, execute=did)
    )
    await harness.ctx.plugin(budget, config)


def ends(agent: Agent) -> list[TurnEndEvent]:
    return [event.data for event in agent.session.events if isinstance(event.data, TurnEndEvent)]


async def turn(agent: Agent, text: str = "Go.") -> None:
    agent.followup(text)
    await idle(agent)


# Steps


async def test_a_turn_that_would_take_a_step_too_many_is_ended_before_it(
    harness: Harness,
) -> None:
    await loaded(harness, max_steps=2)
    agent = await harness.agent(works(), works(), works(), Reply(text="Done."))

    await turn(agent)

    assert ends(agent) == [
        TurnEndEvent(turn=1, reason="blocked", detail="the turn has taken its 2 steps")
    ]
    # The model was asked twice, and the calls of both replies were carried out.
    assert len(harness.model.requests) == 2
    assert harness.model.remaining == 2


async def test_a_turn_that_takes_just_the_steps_that_it_has_ends_like_any_other(
    harness: Harness,
) -> None:
    await loaded(harness, max_steps=2)
    agent = await harness.agent(works(), Reply(text="Done."))

    await turn(agent)

    assert ends(agent) == [TurnEndEvent(turn=1, reason="completed")]


async def test_one_step_is_a_limit_too(harness: Harness) -> None:
    await loaded(harness, max_steps=1)
    agent = await harness.agent(works(), Reply(text="never"))

    await turn(agent)

    assert ends(agent) == [
        TurnEndEvent(turn=1, reason="blocked", detail="the turn has taken its 1 step")
    ]
    assert len(harness.model.requests) == 1


async def test_going_on_from_a_reply_that_was_cut_off_is_a_step_that_counts(
    harness: Harness,
) -> None:
    await loaded(harness, max_steps=2)
    cut = Reply(text="and on", finish="max-tokens")
    agent = await harness.agent(cut, cut, cut)

    await turn(agent)

    # The loop would take a reply up three times. The budget has room for one.
    assert ends(agent) == [
        TurnEndEvent(turn=1, reason="blocked", detail="the turn has taken its 2 steps")
    ]
    assert len(harness.model.requests) == 2


async def test_the_steps_that_the_spec_of_an_agent_names_count_before_those_of_the_config(
    harness: Harness,
) -> None:
    await loaded(harness, max_steps=3)
    strict = await harness.agent(works(), works(), session_id="strict", max_steps=1)
    await turn(strict)
    assert ends(strict)[0].detail == "the turn has taken its 1 step"

    # An agent whose spec names none goes by the config.
    harness.model.add(works(), works(), works(), works())
    other = await harness.agent(session_id="other")
    await turn(other)
    assert ends(other)[0].detail == "the turn has taken its 3 steps"


async def test_a_spec_sets_a_limit_where_the_config_sets_none(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(works(), works(), works(), max_steps=2)

    await turn(agent)

    assert ends(agent)[0].detail == "the turn has taken its 2 steps"


async def test_every_turn_has_its_steps_anew(harness: Harness) -> None:
    await loaded(harness, max_steps=2)
    agent = await harness.agent(works(), works(), works(), Reply(text="Done."))
    await turn(agent)
    assert ends(agent)[0].reason == "blocked"

    # The turn after it begins with a step that is its first.
    await turn(agent, "Go on.")

    assert ends(agent)[1] == TurnEndEvent(turn=2, reason="completed")


# Tokens


async def test_a_turn_that_has_used_its_tokens_takes_no_further_step(harness: Harness) -> None:
    await loaded(harness, max_tokens=1000)
    spent = Usage(input_tokens=300, output_tokens=40, cache_read_tokens=200, cache_write_tokens=60)
    agent = await harness.agent(works(spent), works(spent), works(spent), Reply(text="never"))

    await turn(agent)

    # Six hundred after the first step, twelve hundred after the second: all four counts.
    assert ends(agent) == [
        TurnEndEvent(
            turn=1, reason="blocked", detail="the turn has used 1200 tokens, and it has 1000"
        )
    ]
    assert len(harness.model.requests) == 2


async def test_tokens_that_come_to_just_the_limit_are_the_limit(harness: Harness) -> None:
    await loaded(harness, max_tokens=600)
    spent = Usage(input_tokens=600)
    agent = await harness.agent(works(spent), Reply(text="never"))

    await turn(agent)

    assert ends(agent)[0].detail == "the turn has used 600 tokens, and it has 600"


async def test_what_earlier_turns_used_does_not_count_against_a_turn(harness: Harness) -> None:
    await loaded(harness, max_tokens=1000)
    spent = Usage(input_tokens=700)
    agent = await harness.agent(
        Reply(text="One.", usage=spent), works(spent), Reply(text="Two.", usage=spent)
    )
    await turn(agent)

    await turn(agent, "Again.")

    # Seven hundred in the turn before, seven hundred in this one when its second step began.
    assert [end.reason for end in ends(agent)] == ["completed", "completed"]


# Time


async def test_a_turn_that_has_run_for_its_time_takes_no_further_step(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    now = [100.0]
    monkeypatch.setattr(budget, "_now", lambda: now[0])
    await loaded(harness, max_seconds=60)

    async def passes(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        # Every step takes forty seconds.
        decision = await proceed(info)
        now[0] += 40
        return decision

    agent = await harness.agent(works(), works(), works(), Reply(text="never"))
    agent.ctx.on(PRE_STEP, passes)

    await turn(agent)

    # The first step began at once, the second after forty seconds, and before the third
    # eighty had passed.
    assert ends(agent) == [
        TurnEndEvent(
            turn=1, reason="blocked", detail="the turn has run for 80 seconds, and it has 60"
        )
    ]
    assert len(harness.model.requests) == 2


async def test_the_time_of_a_turn_begins_with_its_first_step(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    now = [0.0]
    monkeypatch.setattr(budget, "_now", lambda: now[0])
    await loaded(harness, max_seconds=60)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    await turn(agent)

    # A long time passes between the two turns.
    now[0] = 5000.0
    await turn(agent, "Again.")

    assert [end.reason for end in ends(agent)] == ["completed", "completed"]


# Nothing to do


async def test_without_a_limit_a_turn_takes_the_steps_that_it_needs(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(*(works() for _ in range(12)), Reply(text="Done."))

    await turn(agent)

    assert ends(agent) == [TurnEndEvent(turn=1, reason="completed")]
    assert len(harness.model.requests) == 13


async def test_two_agents_do_not_share_a_budget(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    await loaded(harness, max_tokens=1000)
    spent = Usage(input_tokens=700)
    first = await harness.agent(works(spent), Reply(text="One.", usage=spent), session_id="a")
    await turn(first)

    harness.model.add(works(spent), Reply(text="Two.", usage=spent))
    second = await harness.agent(session_id="b")
    await turn(second)

    # Each had used seven hundred when its second step began.
    assert ends(first) == [TurnEndEvent(turn=1, reason="completed")]
    assert ends(second) == [TurnEndEvent(turn=1, reason="completed")]


async def test_a_step_that_the_budget_refuses_is_asked_about_by_nobody_after_it(
    harness: Harness,
) -> None:
    await loaded(harness, max_steps=1)
    asked: list[int] = []

    async def noting(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        asked.append(info.step)
        return await proceed(info)

    agent = await harness.agent(works(), Reply(text="never"))
    agent.ctx.on(PRE_STEP, noting)

    await turn(agent)

    # Whoever makes a conversation shorter before a step has nothing to do for this one.
    assert asked == [1]


async def test_the_budget_is_asked_before_those_that_were_there_before_it(
    harness: Harness,
) -> None:
    asked: list[int] = []

    async def noting(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        asked.append(info.step)
        return await proceed(info)

    # Something that listens was loaded first.
    harness.ctx.on(PRE_STEP, noting)
    await loaded(harness, max_steps=1)
    agent = await harness.agent(works(), Reply(text="never"))

    await turn(agent)

    assert asked == [1]


@pytest.mark.parametrize(
    "bad", [{"max_steps": 0}, {"max_tokens": 0}, {"max_seconds": 0}, {"max_steps": 1.5}]
)
def test_a_limit_that_lets_nothing_happen_is_refused(bad: dict[str, object]) -> None:
    with pytest.raises(ValidationError):
        budget.Config.model_validate(bad)
