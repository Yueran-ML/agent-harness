"""The service process: how hard the model thinks, as ``/effort`` chooses it."""

import asyncio
import contextlib
import itertools
import json
from collections.abc import AsyncGenerator
from typing import Any

import pytest
from pydantic import BaseModel

import columbina.subagents
import columbina.subagents.spawn
import columbina.system_prompt
from columbina.agent import PRE_STEP, Agent, StepDecision, StepInfo
from columbina.apps import common
from columbina.apps.effort import LEFT, ULTRA, Reasoning, Style, offered
from columbina.apps.serve import server
from columbina.commands import COMMANDS
from columbina.compaction import SUMMARY_SOURCE
from columbina.kernel import Next
from columbina.llm import LLM, LlmRequest
from columbina.llm.adapters import anthropic_messages, openai_chat
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import AssistantMessageEvent, UserMessageEvent
from columbina.subagents import SUBAGENTS, Task
from columbina.tools import Tool, ToolRun
from support.loop import Harness, context, user
from support.served import Served, loaded
from support.thinking import Thinking, instructions_now, system_text, told_of_the_effort
from support.wires import Far

LEVELS = ["default", "off", "low", "high", "max", "ultra"]


class NoArgs(BaseModel):
    pass


async def noop(args: NoArgs, run: ToolRun) -> str:
    return "ok"


NOOP = Tool(name="noop", description="Do nothing.", parameters=NoArgs, execute=noop)


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

    before = await asked(served, "Hello.")
    await said(served, "/effort ultra")
    at_ultra = await asked(served, "Plan the work.")
    still = await asked(served, "Go on.")
    await said(served, "/effort default")
    after = await asked(served, "And now?")
    later = await asked(served, "Once more.")
    # An agent of another scope is told nothing of it.
    other = await served.harness.agent(Reply(text="Hi."), session_id="another")
    other.followup("Hello.")
    await other.when_idle()

    # The model is told once, in a message of its own before the input of the turn, and told
    # once that it is over. What it sees of it stays where it was.
    assert [told_of_the_effort(r) for r in (before, at_ultra, still, after, later)] == [
        [],
        [ULTRA],
        [ULTRA],
        [ULTRA, LEFT],
        [ULTRA, LEFT],
    ]
    assert at_ultra.messages[-2:] == (context("effort", ULTRA), user("Plan the work."))
    assert after.messages[-2:] == (context("effort", LEFT), user("And now?"))
    assert told_of_the_effort(served.harness.model.requests[-1]) == []
    # The instructions that the conversation opened with hold at every level, and none are
    # said to take their place: the service reads them from its cache.
    requests = (before, at_ultra, still, after, later)
    assert len({system_text(request) for request in requests}) == 1
    assert ULTRA not in system_text(at_ultra)
    assert all(instructions_now(request) == system_text(request) for request in requests)
    # Each request goes on from the one before it, as it was.
    for earlier, next_one in itertools.pairwise(requests):
        assert next_one.messages[: len(earlier.messages)] == earlier.messages
    # Not a word of it is told as if the user had said it.
    sources = [
        event["data"]["source"]
        for told in served.far.told("session.event")
        if (event := told["event"])["type"] == "user/message"
    ]
    assert sources.count("context/effort") == 2
    assert sources.count("user") == 5


async def test_ultra_is_told_again_where_a_summary_took_it_in_the_middle_of_a_turn(
    served: Served,
) -> None:
    harness = served.harness
    harness.tools.add(harness.ctx, NOOP)
    await served.opened()
    await said(served, "/effort ultra")
    await asked(served, "Plan the work.")
    agent = served.agent()

    async def shorten(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        if info.step == 2:
            # Everything before the call of this turn goes into a summary, the note too.
            session = agent.session
            call = max(
                at
                for at, seq in enumerate(session.surface)
                if isinstance(session.events[seq].data, AssistantMessageEvent)
            )
            session.append(
                UserMessageEvent(message=user("(a summary)"), source=SUMMARY_SOURCE),
                replaces=session.surface[1:call],
            )
        return await proceed(info)

    agent.ctx.on(PRE_STEP, shorten)
    harness.model.add(Reply(calls=(Call(name="noop"),)))
    count = len(harness.model.requests)
    await asked(served, "Do it.")

    first, second = harness.model.requests[count:]
    assert told_of_the_effort(first) == [ULTRA]
    # The model sees it again before it is asked, at the end of what it sees.
    assert second.messages[1] == user("(a summary)")
    assert told_of_the_effort(second) == [ULTRA]
    assert second.messages[-1] == context("effort", ULTRA)


async def test_ultra_is_told_again_in_the_next_turn_where_a_summary_took_it(
    served: Served,
) -> None:
    await served.opened()
    await said(served, "/effort ultra")
    await asked(served, "Plan the work.")
    agent = served.agent()

    async def shortened() -> None:
        session = agent.session
        session.append(
            UserMessageEvent(message=user("(a summary)"), source=SUMMARY_SOURCE),
            replaces=session.surface[1:],
        )

    await agent.run_maintenance(shortened)
    request = await asked(served, "Go on.")

    assert request.messages[1:] == (
        user("(a summary)"),
        context("effort", ULTRA),
        user("Go on."),
    )


async def test_a_summary_that_keeps_the_team_and_drops_its_end_has_the_end_told_again(
    served: Served,
) -> None:
    # The note of ultra stood at the beginning, which a summary leaves, and the word that it is
    # over came later, which a summary took.
    await served.opened()
    await said(served, "/effort ultra")
    await asked(served, "Plan the work.")
    await said(served, "/effort default")
    await asked(served, "And now?")
    agent = served.agent()

    over = UserMessageEvent(message=context("effort", LEFT), source="context/effort")

    async def shortened() -> None:
        session = agent.session
        (ended,) = [seq for seq in session.surface if session.events[seq].data == over]
        session.append(
            UserMessageEvent(message=user("(a summary)"), source=SUMMARY_SOURCE), replaces=[ended]
        )

    await agent.run_maintenance(shortened)
    request = await asked(served, "Still?")

    assert told_of_the_effort(request) == [ULTRA, LEFT]
    assert request.messages[-2:] == (context("effort", LEFT), user("Still?"))


async def test_a_subagent_is_not_told_how_the_agent_of_the_conversation_works(
    served: Served,
) -> None:
    harness = served.harness
    await harness.ctx.plugin(columbina.subagents)
    await harness.ctx.plugin(columbina.subagents.spawn)
    await served.opened()
    await said(served, "/effort ultra")
    parent = await asked(served, "Plan the work.")
    harness.model.add(Reply(text="Three files."))

    task = Task(call_id="call-1", description="count files", prompt="Count the files.")
    assert await harness.ctx.use(SUBAGENTS).run(served.agent(), task) == "Three files."

    child = harness.model.requests[-1]
    assert told_of_the_effort(parent) == [ULTRA]
    assert told_of_the_effort(child) == []
    assert "Work at full depth" not in child.model_dump_json()
    # It asks what its parent asks of the reasoning all the same.
    assert child.config.reasoning == parent.config.reasoning


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


# What a service is sent at every level


_REASONING = {
    "deepseek": {"thinking", "reasoning_effort"},
    "anthropic": {"output_config"},
    "openai": {"reasoning_effort"},
    "none": set[str](),
}
"""What the body of each style says of how much to reason."""


def body_of(style: Style, request: LlmRequest) -> dict[str, Any]:
    """The body that the adapter of a style sends for a request."""
    if style == "anthropic":
        return anthropic_messages.build_body(
            request, anthropic_messages.Config(provider="thinking")
        )
    config = openai_chat.Config(
        provider="thinking", base_url="https://service.test", reasoning_style=style
    )
    return openai_chat.build_body(request, config)


def wire(value: object) -> str:
    """What a part of a body is, byte for byte, as it is sent."""
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def the_rest(style: str, body: dict[str, Any]) -> str:
    """What a body says besides the conversation and how much to reason."""
    left_out = {"messages", *_REASONING[style]}
    return wire({key: value for key, value in body.items() if key not in left_out})


STYLES: list[tuple[Style, Reasoning | None]] = [
    ("deepseek", None),
    ("anthropic", None),
    ("openai", "high"),
    ("none", None),
]


@pytest.mark.parametrize(("style", "configured"), STYLES, ids=[style for style, _ in STYLES])
async def test_every_level_sends_the_tools_the_instructions_and_the_conversation_as_they_were(
    harness: Harness, style: Style, configured: Reasoning | None
) -> None:
    harness.tools.add(harness.ctx, NOOP)
    config = {} if configured is None else {"reasoning": configured}
    sent: dict[str, LlmRequest] = {}
    async with serving(harness, style=style, **config) as served:
        await served.opened()
        levels = efforts(served)[-1][1]
        for level in levels:
            # The same conversation, at the default, up to the turn that is asked at the level.
            await said(served, "/effort default")
            await served.far.result("session/open", {"session_id": f"at-{level}"})
            await asked(served, "First.")
            await said(served, f"/effort {level}")
            sent[level] = await asked(served, "Second.")

    assert levels == list(offered(style, configured))
    bodies = {level: body_of(style, request) for level, request in sent.items()}
    base = bodies["default"]
    for level, body in bodies.items():
        # The tools, the instructions and every other setting go out byte for byte as they did.
        assert the_rest(style, body) == the_rest(style, base), level
        if style == "anthropic":
            # The service takes what one side says in a row as one message: the note is a
            # block of its own in the message of the turn, before what the user said.
            *history, last = body["messages"]
            *history_then, last_then = base["messages"]
            assert wire(history) == wire(history_then), level
            note = [{"type": "text", "text": wire_note(ULTRA)}] if level == "ultra" else []
            assert last["content"] == [*note, *last_then["content"]], level
        else:
            *history, last = body["messages"]
            *history_then, last_then = base["messages"]
            assert wire(history[: len(history_then)]) == wire(history_then), level
            note = [{"role": "user", "content": wire_note(ULTRA)}] if level == "ultra" else []
            assert history[len(history_then) :] == note, level
            assert last == last_then, level
    # Only what is asked of the reasoning differs.
    asked_for = {
        level: {key: body[key] for key in _REASONING[style] & set(body)}
        for level, body in bodies.items()
    }
    if style == "deepseek":
        assert asked_for["off"] == {"thinking": {"type": "disabled"}}
        assert asked_for["ultra"] == {"thinking": {"type": "enabled"}, "reasoning_effort": "max"}
    elif style == "anthropic":
        assert asked_for["low"] == {"output_config": {"effort": "low"}}
        assert asked_for["ultra"] == {"output_config": {"effort": "max"}}
    elif style == "openai":
        assert asked_for["ultra"] == asked_for["high"] == {"reasoning_effort": "high"}
    else:
        assert all(not asked for asked in asked_for.values())
    assert asked_for["default"] == {}


def wire_note(text: str) -> str:
    """The text of the message that tells the model how to work."""
    return f'<context source="effort">\n{text}\n</context>'


@pytest.mark.parametrize(("style", "configured"), STYLES, ids=[style for style, _ in STYLES])
async def test_a_conversation_whose_level_changes_goes_on_from_each_request_as_it_was(
    harness: Harness, style: Style, configured: Reasoning | None
) -> None:
    harness.tools.add(harness.ctx, NOOP)
    config = {} if configured is None else {"reasoning": configured}
    async with serving(harness, style=style, **config) as served:
        await served.opened()
        offered_here = efforts(served)[-1][1]
        # Into ultra and out of it, and through every level on offer.
        levels = [*offered_here, offered_here[0], "ultra", offered_here[-2]]
        sent: list[LlmRequest] = []
        for turn, level in enumerate(levels):
            await said(served, f"/effort {level}")
            sent.append(await asked(served, f"Turn {turn}."))

    bodies = [body_of(style, request) for request in sent]
    for earlier, later in itertools.pairwise(bodies):
        # Every request begins as the one before it did, byte for byte, so that a service
        # reads all that from its cache. What follows differs, and what is asked of the
        # reasoning.
        assert the_rest(style, later) == the_rest(style, earlier)
        assert wire(later["messages"][: len(earlier["messages"])]) == wire(earlier["messages"])
    told = [told_of_the_effort(request) for request in sent]
    assert told[-1] == [ULTRA, LEFT, ULTRA, LEFT]
