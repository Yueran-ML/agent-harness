"""How hard the model thinks: the levels, what each asks of a service, and the command."""

from collections.abc import AsyncIterator

import pytest

from columbina.apps.effort import (
    EFFORTS,
    ULTRA,
    Effort,
    Reasoning,
    effort_named,
    effort_of,
    offered,
    reasoning_of,
    style_of,
)
from columbina.kernel import Context
from columbina.llm import LLM, Chunk, LlmRequest
from support.loop import Harness
from support.thinking import Thinking

LISTED = "default, off, low, high, max and ultra"


def test_there_are_six_levels_the_least_first() -> None:
    assert EFFORTS == ("default", "off", "low", "high", "max", "ultra")


def test_a_service_that_reasons_as_deepseek_offers_every_level_and_the_others_two() -> None:
    assert offered("deepseek") == EFFORTS
    assert offered("deepseek", "low") == EFFORTS
    # What a model of the style of OpenAI that does not reason would refuse is not offered.
    assert offered("openai") == ("default", "ultra")
    assert offered("none") == ("default", "ultra")


def test_a_service_of_the_style_of_openai_offers_the_level_that_its_config_asks_for() -> None:
    # Whoever wrote it knows that the model takes it.
    assert offered("openai", "low") == ("default", "low", "ultra")
    assert offered("openai", "max") == ("default", "max", "ultra")
    # Off asks such a service for nothing, as the default does. One that is told nothing is
    # told nothing whatever its config asks.
    assert offered("openai", "off") == ("default", "ultra")
    assert offered("none", "high") == ("default", "ultra")


@pytest.mark.parametrize(
    ("effort", "deepseek"),
    [
        ("default", None),
        ("off", "off"),
        ("low", "low"),
        ("high", "high"),
        ("max", "max"),
        ("ultra", "max"),
    ],
)
def test_a_level_that_is_chosen_asks_a_service_for_so_much_reasoning_where_it_takes_any(
    effort: Effort, deepseek: Reasoning | None
) -> None:
    assert reasoning_of(effort, "deepseek") == deepseek
    # What the config asks is no matter where the level is chosen from all of them.
    assert reasoning_of(effort, "deepseek", "low") == deepseek
    # The others are asked what the config asks at every level but the default: at ultra the
    # team is what is asked for besides.
    assert reasoning_of(effort, "openai") is None
    assert reasoning_of(effort, "none") is None
    assert reasoning_of(effort, "openai", "high") == (None if effort == "default" else "high")
    assert reasoning_of(effort, "none", "low") == (None if effort == "default" else "low")


def test_what_a_config_asks_for_stands_for_a_level_that_is_offered() -> None:
    assert effort_of(None, "deepseek") == "default"
    assert effort_of("low", "deepseek") == "low"
    assert effort_of("max", "deepseek") == "max"
    assert effort_of("high", "openai") == "high"
    # A level that the service does not take is the default.
    assert effort_of("off", "openai") == "default"
    assert effort_of("low", "none") == "default"


def test_the_command_says_the_level_and_those_on_offer() -> None:
    assert effort_named("high", EFFORTS, "") == (
        None,
        f"The effort is high. This service offers {LISTED}. "
        "Another level is chosen with /effort LEVEL.",
    )
    assert effort_named("default", ("default", "ultra"), "  ")[1].startswith(
        "The effort is default. This service offers default and ultra."
    )


def test_the_command_chooses_a_level_that_is_on_offer_however_it_is_written() -> None:
    assert effort_named("default", EFFORTS, "max") == (
        "max",
        "The effort is max from the next request on.",
    )
    assert effort_named("max", ("default", "ultra"), " ULTRA ") == (
        "ultra",
        "The effort is ultra from the next request on.",
    )
    # The level in force is chosen as any other is.
    assert effort_named("high", ("default", "high", "ultra"), "high") == (
        "high",
        "The effort is high from the next request on.",
    )


def test_the_command_chooses_nothing_where_a_level_is_not_on_offer() -> None:
    assert effort_named("ultra", ("default", "high", "ultra"), "low") == (
        None,
        "This service does not take low. It offers default, high and ultra.",
    )
    assert effort_named("low", EFFORTS, "Loud") == (
        None,
        f"Loud is no level of effort. This service offers {LISTED}.",
    )


class _Plain:
    """A service that says nothing of how it is told to reason."""

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        raise AssertionError("nothing is asked of it")


class _Odd:
    reasoning_style = "anthropic"

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        raise AssertionError("nothing is asked of it")


class _OpenAI:
    reasoning_style = "openai"

    def stream(self, request: LlmRequest) -> AsyncIterator[Chunk]:
        raise AssertionError("nothing is asked of it")


async def test_the_style_of_a_service_is_what_its_adapter_says(harness: Harness) -> None:
    adapters = harness.ctx.use(LLM).adapters
    adapters.add(harness.ctx, "thinking", Thinking(harness.model))
    adapters.add(harness.ctx, "plain", _Plain())
    adapters.add(harness.ctx, "odd", _Odd())
    adapters.add(harness.ctx, "openai", _OpenAI())

    assert style_of(harness.ctx, "thinking") == "deepseek"
    assert style_of(harness.ctx, "openai") == "openai"
    # The scripted model says nothing of it, and neither does a service that is not there.
    assert style_of(harness.ctx, "scripted") == "none"
    assert style_of(harness.ctx, "plain") == "none"
    assert style_of(harness.ctx, "odd") == "none"
    assert style_of(harness.ctx, "nowhere") == "none"


def test_without_a_model_service_no_service_is_told_anything(ctx: Context) -> None:
    assert style_of(ctx, "deepseek") == "none"


def test_ultra_has_the_agent_plan_hand_parts_to_subagents_side_by_side_and_review() -> None:
    assert "plan" in ULTRA
    assert "side by side" in ULTRA
    assert "review" in ULTRA
    assert "subagent" in ULTRA
