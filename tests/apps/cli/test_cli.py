"""The terminal interface: a conversation with an agent, held at a terminal that is a test."""

import asyncio
import logging
import os
from pathlib import Path
from typing import Any

import pytest

import columbina
import columbina.approval
import columbina.apps.cli
import columbina.commands
from columbina.agent import INBOX, Agent
from columbina.approval import APPROVAL, ApprovalRequest
from columbina.apps import FRONTEND, Frontend
from columbina.apps.cli.terminal import TERMINAL
from columbina.llm import LlmFailure, Usage
from columbina.llm.adapters.scripted import Reply
from columbina.session import TurnEndEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE
from support.loop import MODEL, Harness, from_a_source, user
from support.terminal import CTRL_D, ESCAPE, Desk, desk

STEER = chr(0xBB)


async def frontend(harness: Harness, at: Desk | None, **config: Any) -> Frontend:
    """Load the terminal interface on top of the loop of the harness."""
    if at is not None:
        harness.ctx.provide(TERMINAL, at.terminal)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(
        columbina.apps.cli, {"model": MODEL.model_dump(), "cwd": harness.cwd, **config}
    )
    return harness.ctx.use(FRONTEND)


def agent_of(harness: Harness) -> Agent:
    """The one agent that the interface talks to."""
    (agent,) = harness.agents.live.values()
    return agent


async def left(running: "asyncio.Future[int]") -> int:
    return await asyncio.wait_for(running, 5)


# A conversation


async def test_a_line_is_a_task_and_the_reply_is_shown_above_the_prompt(harness: Harness) -> None:
    harness.model.add(Reply(text="Hello!\nWhat shall we do?"))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Say hello.")
        # The empty line sets the turn apart from what comes after it.
        await at.sees("Hello!\nWhat shall we do?\n\n")
        at.enter("/exit")

        assert await left(running) == 0
    assert harness.model.requests[0].messages[-1] == user("Say hello.")


async def test_the_conversation_begins_with_a_word_about_itself(harness: Harness) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        await at.sees(
            f"Columbina {columbina.__version__}   {MODEL.model}   {harness.cwd}\n"
            "/help lists the commands. Ctrl+D leaves.\n"
            "\n"
        )
        at.type(CTRL_D)

        assert await left(running) == 0


async def test_tasks_follow_one_another_in_one_session(harness: Harness) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("Second.")
        await at.sees("Two.\n")
        at.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    ends = [event.data for event in stored.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="completed"),
    ]


async def test_a_line_of_spaces_is_nothing(harness: Harness) -> None:
    harness.model.add(Reply(text="One."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("   ")
        at.enter("")
        at.enter("  Padded.  ")
        await at.sees("One.\n")
        at.type(CTRL_D)
        await left(running)

    assert len(harness.model.requests) == 1
    assert harness.model.requests[0].messages[-1] == user("Padded.")


async def test_a_line_that_is_typed_while_a_turn_runs_is_read_at_the_next_step(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="Reading a.txt.", hold=True), Reply(text="Reading b.txt."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        at.enter("Read a.txt.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.enter("No, read b.txt.")
        await at.draws("No, read b.txt.")
        await asyncio.sleep(0.05)
        harness.model.release()
        await at.sees(f"{STEER} No, read b.txt.\n")
        await at.sees("Reading b.txt.\n")
        at.type(CTRL_D)
        await left(running)

    assert harness.model.requests[1].messages[-1] == user("No, read b.txt.")


async def test_escape_stops_the_turn_and_the_next_task_is_taken(harness: Harness) -> None:
    harness.model.add(Reply(text="thinking", hold=True), Reply(text="Here I am."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.type(ESCAPE)
        await at.sees("(stopped)\n")
        at.enter("Are you there?")
        await at.sees("Here I am.\n")
        at.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    ends = [event.data for event in stored.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="aborted", detail="user"),
        TurnEndEvent(turn=2, reason="completed"),
    ]


async def test_a_turn_that_fails_says_why_and_the_conversation_goes_on(harness: Harness) -> None:
    failed = LlmFailure(message="the key is not valid", code="AUTH", status=401)
    harness.model.add(Reply(failure=failed), Reply(text="Now it works."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("Hello?")
        await at.sees("(the turn ended with an error: AUTH: the key is not valid)\n")
        at.enter("Again?")
        await at.sees("Now it works.\n")
        at.type(CTRL_D)

        assert await left(running) == 0


# Commands


async def test_a_line_that_begins_with_a_slash_is_carried_out_and_not_sent(
    harness: Harness,
) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("/help")
        await at.sees(
            "/exit   Leave.\n"
            "/help   List the commands.\n"
            "/model  Show the model, or name another.\n"
            "/new    End this session and begin another one.\n"
            "A line that does not begin with a slash goes to the agent.\n"
            "\n"
        )
        at.enter("/nowhere")
        await at.sees("There is no command /nowhere. /help lists the commands.\n\n")
        at.enter("/exit")

        assert await left(running) == 0
        # A command that has nothing to say shows nothing, not an empty line either.
        assert at.seen().endswith("There is no command /nowhere. /help lists the commands.\n\n")
    assert harness.model.requests == []


async def test_a_command_is_not_carried_out_while_a_turn_runs(harness: Harness) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.enter("/exit")
        await at.sees("(a command waits until the agent is at rest. Escape stops the agent.)\n")
        assert not running.done()
        assert INBOX.of(agent_of(harness).session).next_step == ()
        harness.model.release()
        await at.sees("thinking\n")
        at.enter("/exit")

        assert await left(running) == 0


async def test_another_model_can_be_named_and_the_next_request_goes_to_it(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("/model")
        await at.sees(f"The model is {MODEL.model}. Another one is named with /model ID.\n")
        at.enter("/model other-model")
        await at.sees("The model is other-model from the next request on.\n")
        at.enter("Second.")
        await at.sees("Two.\n")
        at.type(CTRL_D)
        await left(running)

    asked = [request.config.model for request in harness.model.requests]
    assert asked == [MODEL.model, "other-model"]


async def test_a_new_session_can_be_begun_without_leaving(harness: Harness) -> None:
    harness.model.add(Reply(text="One."), Reply(text="Two."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="first")).run())

        at.enter("First.")
        await at.sees("One.\n")
        at.enter("/new")
        await at.sees("A new session begins: ")
        at.enter("Second.")
        await at.sees("Two.\n")
        (agent,) = harness.agents.live.values()
        second = agent.id
        at.type(CTRL_D)
        await left(running)

    assert second != "first"
    assert f"A new session begins: {second}\n" in at.seen()
    # The model of the new session was not sent what the old one held.
    assert user("First.") not in harness.model.requests[1].messages
    assert harness.model.requests[1].messages[-1] == user("Second.")
    first = harness.sessions.read(harness.cwd, "first")
    said = [
        event.data.message
        for event in first.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]
    assert said == [user("First.")]


# Leaving


async def test_leaving_closes_the_agent_and_ends_a_turn_that_was_running(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)

        at.type(CTRL_D)

        assert await left(running) == 0
    stored = harness.sessions.read(harness.cwd, "s-1")
    assert stored.events[-1].data == TurnEndEvent(turn=1, reason="aborted", detail="user")
    assert list(harness.agents.live) == []


async def test_a_terminal_that_goes_away_ends_the_conversation(harness: Harness) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        at.hang_up()

        assert await left(running) == 0
    assert list(harness.agents.live) == []


async def test_without_a_terminal_there_is_no_conversation(
    harness: Harness, capsys: pytest.CaptureFixture[str]
) -> None:
    runner = await frontend(harness, None)

    code = await runner.run()

    assert code == 2
    assert capsys.readouterr().err == (
        "columbina: this is not a terminal, and the conversation needs one. "
        "`columbina run TASK` gives an agent one task without it.\n"
    )
    assert list(harness.agents.live) == []


async def test_an_agent_that_stops_for_good_ends_the_conversation(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(event: object) -> str:
        raise OSError(28, "No space left on device")

    harness.model.add(Reply(text="thinking", hold=True))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, session_id="s-1")).run())
        at.enter("Think.")
        await asyncio.wait_for(harness.model.held(), timeout=5)
        monkeypatch.setattr("columbina.session.store.encode_event", full)

        harness.model.release()
        await at.sees("(the agent has stopped: ")
        code = await left(running)
        monkeypatch.undo()

    assert code == 1
    assert "No space left on device" in at.seen()


# Sessions


async def test_an_id_that_cannot_name_a_session_ends_the_conversation(harness: Harness) -> None:
    with desk() as at:
        runner = await frontend(harness, at, session_id="../escape")

        code = await asyncio.wait_for(runner.run(), 5)

        assert code == 1
        await at.sees("columbina: '../escape' is not a usable session id")
    assert harness.model.requests == []


async def test_a_session_that_is_named_is_taken_up_and_its_last_turn_is_shown(
    harness: Harness,
) -> None:
    harness.model.add(Reply(text="The parser is fixed."), Reply(text="The lexer too."))
    await harness.ctx.plugin(columbina.commands)
    config = {"model": MODEL.model_dump(), "cwd": harness.cwd, "session_id": "s-1"}
    with desk() as at:
        provided = harness.ctx.provide(TERMINAL, at.terminal)
        loaded = await harness.ctx.plugin(columbina.apps.cli, config)
        running = asyncio.ensure_future(harness.ctx.use(FRONTEND).run())
        at.enter("Fix the parser.")
        await at.sees("The parser is fixed.\n")
        at.type(CTRL_D)
        await left(running)
        await loaded.dispose()
        await provided.dispose()

    with desk() as again:
        harness.ctx.provide(TERMINAL, again.terminal)
        await harness.ctx.plugin(columbina.apps.cli, config)
        running = asyncio.ensure_future(harness.ctx.use(FRONTEND).run())

        await again.sees(
            "(this session is taken up again. Its last turn:)\n"
            "> Fix the parser.\n"
            "The parser is fixed.\n"
        )
        again.enter("And the lexer.")
        await again.sees("The lexer too.\n")
        again.type(CTRL_D)
        await left(running)

    stored = harness.sessions.read(harness.cwd, "s-1")
    said = [
        event.data.message
        for event in stored.events
        if isinstance(event.data, UserMessageEvent) and not from_a_source(event.data)
    ]
    assert said == [user("Fix the parser."), user("And the lexer.")]


async def test_the_session_that_was_used_last_can_be_taken_up_without_its_name(
    harness: Harness,
) -> None:
    for name, at_second in (("older", 1000), ("newer", 2000)):
        session = harness.sessions.create(harness.ctx, harness.cwd, session_id=name)
        session.append(UserMessageEvent(message=user(f"A task of {name}.")))
        await session.flush()
        session.close()
        stamp = at_second * 1_000_000_000
        file = harness.sessions.directory(harness.cwd, name) / SESSION_FILE
        os.utime(file, ns=(stamp, stamp))
    harness.model.add(Reply(text="Going on."))
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, resume=True)).run())
        await at.sees("/help lists the commands.")

        at.enter("Go on.")
        await at.sees("Going on.\n")
        at.type(CTRL_D)
        await left(running)

    newer = harness.sessions.read(harness.cwd, "newer")
    assert any(isinstance(event.data, TurnEndEvent) for event in newer.events)


async def test_taking_up_a_session_where_none_is_stored_begins_a_new_one(
    harness: Harness, tmp_path: Path
) -> None:
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at, resume=True)).run())

        await at.sees("(no session of this directory is stored yet. A new one begins.)\n")
        at.type(CTRL_D)

        assert await left(running) == 0


# What else is shown


async def test_what_the_program_logs_is_shown_above_the_prompt_while_it_is_open(
    harness: Harness,
) -> None:
    log = logging.getLogger("columbina.somewhere")
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")

        log.warning("careful with %s", "that")
        log.error("that went wrong")
        await at.sees("columbina: careful with that\ncolumbina: that went wrong\n")
        at.type(CTRL_D)
        await left(running)
        handlers = list(logging.getLogger("columbina").handlers)

    assert not any(type(handler).__name__ == "_Shown" for handler in handlers)


async def test_the_line_below_the_prompt_tells_of_the_session(harness: Harness) -> None:
    harness.model.add(Reply(text="One.", usage=Usage(input_tokens=1200, output_tokens=5)))
    with desk() as at:
        runner = await frontend(harness, at)
        assert isinstance(runner, columbina.apps.cli.Cli)
        before = runner.status_line()
        running = asyncio.ensure_future(runner.run())

        at.enter("First.")
        await at.sees("One.\n")
        await asyncio.wait_for(agent_of(harness).when_idle(), 5)
        after = runner.status_line()
        at.type(CTRL_D)
        await left(running)

    assert before == ""
    assert after == f"{MODEL.model} | 1.2k tokens | cache 0%"


async def test_a_question_of_approval_is_put_to_the_person_at_the_terminal(
    harness: Harness,
) -> None:
    await harness.ctx.plugin(columbina.approval)
    with desk() as at:
        running = asyncio.ensure_future((await frontend(harness, at)).run())
        await at.sees("/help lists the commands.")
        agent = agent_of(harness)
        asked = ApprovalRequest(
            session=agent.session,
            call_id="c-1",
            tool="bash",
            summary="bash: git status",
            reason="No rule allows this command.",
            target="git status",
        )

        answering = asyncio.ensure_future(harness.ctx.use(APPROVAL).ask(agent.ctx, asked))
        await at.sees("bash asks for a yes:\n    git status\nNo rule allows this command.\n")
        await at.draws("[y/n]")
        at.type("y")

        assert await asyncio.wait_for(answering, 5) == "allowed-once"
        at.type(CTRL_D)
        await left(running)
