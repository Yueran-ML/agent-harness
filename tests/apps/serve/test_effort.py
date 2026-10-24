"""The service process: how hard the model thinks, as ``/effort`` chooses it."""

import asyncio
import contextlib
from collections.abc import AsyncGenerator
from typing import Any

import pytest

import columbina.system_prompt
from columbina.agent import Agent
from columbina.apps import common
from columbina.apps.effort import ULTRA
from columbina.apps.serve import server
from columbina.commands import COMMANDS
from columbina.llm import LLM, LlmRequest
from columbina.llm.adapters.scripted import Reply
from support.loop import Harness
from support.served import Served, loaded
from support.thinking import Thinking, instructions_now, system_text
from support.wires import Far

LEVELS = ["default", "off", "low", "high", "max", "ultra"]


@contextlib.asynccontextmanager
async def serving(
    harness: Harness, provider: str = "thinking", style: str = "deepseek", **model: Any
) -> AsyncGenerator[Served]:
    """Run a service process with what a test says of its model.

    Under the provider ``thinking`` the scripted model reasons as it is told, in the style that
    the test names. The provider ``scripted`` is told nothing of reasoning.
    """
    if provider == "thinking":
        thinking = Thinking(harness.model, style)
        harness.ctx.use(LLM).adapters.add(harness.ctx, "thinking", thinking)
    far = Far()
    running = await loaded(harness, far, model={"provider": provider, "model": "deep", **model})
    yield Served(harness, far, running)
    if not running.done():
        far.hang_up()
    await asyncio.wait_for(running, 10)


def efforts(served: Served) -> list[tuple[str, list[str]]]:
    """The level of effort and the levels on offer, as each status told them."""
    return [
        (told["model"]["effort"], told["model"]["efforts"])
        for told in served.far.told("session.status")
    ]


async def said(served: Served, line: str) -> str:
    """What a command answers."""
    return (await served.far.result("session/command", {"line": line}))["text"]


async def asked(served: Served, text: str) -> LlmRequest:
    """Give the agent a task, and return the request that its model was sent."""
    count = len(served.harness.model.requests)
    served.harness.model.add(Reply(text="Done."))
    await served.far.result("session/prompt", {"text": text})
    await served.far.until(
        lambda: (
            served.far.told("session.status")[-1]["status"] == "idle"
            and len(served.harness.model.requests) > count
        )
    )
    return served.harness.model.requests[-1]


async def test_a_service_that_is_told_nothing_of_reasoning_offers_the_default_and_ultra(
    served: Served,
) -> None:
    await served.opened()
    assert efforts(served)[-1] == ("default", ["default", "ultra"])

    assert await said(served, "/effort") == (
        "The effort is default. This service offers default and ultra. "
        "Another level is chosen with /effort LEVEL."
    )
    told = len(served.far.told("session.status"))
    assert await said(served, "/effort high") == (
        "This service does not take high. It offers default and ultra."
    )
    assert await said(served, "/effort loud") == (
        "loud is no level of effort. This service offers default and ultra."
    )
    # Nothing changed, so nothing more is told.
    assert len(served.far.told("session.status")) == told

    assert await said(served, "/effort  Ultra ") == "The effort is ultra from the next request on."
    # It is told with the command.
    assert efforts(served)[-1] == ("ultra", ["default", "ultra"])
    # Such a service is asked for no reasoning at ultra either.
    request = await asked(served, "Go on.")
    assert request.config.reasoning is None


async def test_a_service_that_reasons_like_deepseek_is_asked_for_the_level_that_is_chosen(
    harness: Harness,
) -> None:
    async with serving(harness) as served:
        await served.opened()
        assert efforts(served)[-1] == ("default", LEVELS)
        reasoned: list[tuple[str, str | None]] = []
        for level in ["max", "off", "low", "high", "ultra", "default"]:
            assert await said(served, f"/effort {level}") == (
                f"The effort is {level} from the next request on."
            )
            reasoned.append((level, (await asked(served, f"At {level}.")).config.reasoning))

    # Ultra asks for the most there is, and the default leaves it to the service.
    assert reasoned == [
        ("max", "max"),
        ("off", "off"),
        ("low", "low"),
        ("high", "high"),
        ("ultra", "max"),
        ("default", None),
    ]


async def test_a_level_that_is_chosen_for_a_session_that_was_left_holds_for_the_open_one(
    harness: Harness,
) -> None:
    async with serving(harness) as served:
        await served.opened("first")
        first = served.agent()
        await served.far.result("session/open", {"session_id": "second"})
        await served.far.until(
            lambda: served.far.told("session.status")[-1]["session_id"] == "second"
        )

        # The command of the session that was left comes to its end now. The level is that of
        # the program, so it is told at once, of the session that is open, and holds for it.
        await served.harness.ctx.use(COMMANDS).run(first, "/effort ultra")

        last = served.far.told("session.status")[-1]
        assert (last["session_id"], last["model"]["effort"]) == ("second", "ultra")
        assert (await asked(served, "Go.")).config.reasoning == "max"


async def test_a_level_that_is_chosen_while_a_session_is_opened_holds_for_that_session(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    async with serving(harness, reasoning="low") as served:
        await served.opened("first")
        commands: list[int] = []
        made = common.open_agent

        async def made_after_the_command(*given: Any) -> Agent:
            # The agent is made from what the run asked when the session was asked for, and
            # the command comes to its end before it is.
            await served.far.answer(commands[0])
            return await made(*given)

        monkeypatch.setattr(server, "open_agent", made_after_the_command)
        commands.append(served.far.ask("session/command", {"line": "/effort ultra"}))
        await served.far.result("session/open", {"session_id": "second"})

        assert efforts(served)[-1][0] == "ultra"
        assert (await asked(served, "Go.")).config.reasoning == "max"


async def test_a_run_begins_with_the_level_that_its_config_asks_for(harness: Harness) -> None:
    async with serving(harness, reasoning="low") as served:
        await served.opened()
        assert efforts(served)[-1] == ("low", LEVELS)
        assert (await asked(served, "Go.")).config.reasoning == "low"


async def test_a_service_of_the_style_of_openai_is_asked_what_its_config_asks_for(
    harness: Harness,
) -> None:
    # Whoever wrote the config knows that the model takes that level: it is offered beside the
    # default and ultra, and asked for until another one is chosen.
    async with serving(harness, style="openai", reasoning="high") as served:
        await served.opened()
        assert efforts(served)[-1] == ("high", ["default", "high", "ultra"])
        assert (await asked(served, "Go.")).config.reasoning == "high"
        assert await said(served, "/effort max") == (
            "This service does not take max. It offers default, high and ultra."
        )
        # Ultra asks what the config asks, and for the team.
        await said(served, "/effort ultra")
        assert (await asked(served, "Go on.")).config.reasoning == "high"
        await said(served, "/effort default")
        assert (await asked(served, "And now.")).config.reasoning is None
        await said(served, "/effort high")
        assert (await asked(served, "Once more.")).config.reasoning == "high"


async def test_a_level_that_the_service_does_not_take_stands_for_the_default(
    harness: Harness,
) -> None:
    # The config asks for a level that a service that is told nothing of reasoning does not
    # take. The run begins at the default, and a call asks what the config asks, which such a
    # service is not told, until a level is chosen.
    async with serving(harness, provider="scripted", reasoning="low") as served:
        await served.opened()
        assert efforts(served)[-1] == ("default", ["default", "ultra"])
        assert (await asked(served, "Go.")).config.reasoning == "low"
        await said(served, "/effort default")
        assert (await asked(served, "Go on.")).config.reasoning is None


async def test_ultra_tells_the_agent_of_the_conversation_how_to_work_and_no_other(
    served: Served,
) -> None:
    await served.harness.ctx.plugin(columbina.system_prompt)
    await served.opened()

    await said(served, "/effort ultra")
    at_ultra = await asked(served, "Plan the work.")
    await said(served, "/effort default")
    after = await asked(served, "And now?")
    # An agent of another scope, such as a subagent, is told nothing of it.
    other = await served.harness.agent(Reply(text="Hi."), session_id="another")
    other.followup("Hello.")
    await other.when_idle()

    assert ULTRA in instructions_now(at_ultra)
    assert ULTRA not in instructions_now(after)
    assert ULTRA not in instructions_now(served.harness.model.requests[-1])
    # What the conversation opened with stays as it was, and the service reads it from its cache.
    assert system_text(at_ultra) == system_text(after)


async def test_what_the_agent_asks_is_handed_down_and_kept_when_the_model_is_named(
    harness: Harness,
) -> None:
    async with serving(harness) as served:
        await served.opened("first")
        agent = served.agent()
        await said(served, "/effort max")
        # The subagents that the agent makes from now on ask what it asks.
        assert (agent.spec.model.model, agent.spec.model.reasoning) == ("deep", "max")

        await said(served, "/model deep-2")
        assert (agent.spec.model.model, agent.spec.model.reasoning) == ("deep-2", "max")
        assert efforts(served)[-1][0] == "max"
        request = await asked(served, "Go.")
        assert (request.config.model, request.config.reasoning) == ("deep-2", "max")

        await said(served, "/effort low")
        await served.far.result("session/open", {"session_id": "second"})
        second = served.harness.agents.live["second"]
        assert (second.spec.model.model, second.spec.model.reasoning) == ("deep-2", "low")
