"""``columbina acp``: conversations that an editor holds over the Agent Client Protocol."""

import asyncio
import contextlib
import logging
from collections.abc import AsyncGenerator, AsyncIterator
from pathlib import Path
from typing import Any, cast

import pytest
import yaml
from pydantic import BaseModel

import columbina.approval
import columbina.apps.acp
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina.agent import STREAM, Agent, StreamFrame
from columbina.approval import APPROVAL, ApprovalRequest, Outcome
from columbina.apps import FRONTEND
from columbina.apps.acp import said_in, stop_of, title_of
from columbina.apps.launcher import main
from columbina.apps.serve import wire as wire_module
from columbina.apps.serve.rpc import INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, RpcError
from columbina.apps.serve.wire import WIRE
from columbina.llm import LlmFailure, TextBlock, TextDelta
from columbina.llm.adapters.scripted import Call, Reply
from columbina.permissions import PERMISSIONS
from columbina.session import SESSION_EVENT, Committed, TurnEndEvent
from columbina.tools import Tool, ToolRun
from support.launch import profile
from support.loop import MODEL, Harness
from support.wires import Far

LOGGER = "columbina.apps.acp"


class Looked(BaseModel):
    path: str


async def peeked(args: Looked, run: ToolRun) -> str:
    if args.path == "missing":
        raise FileNotFoundError("there is no such file")
    return f"what {args.path} holds"


class Editor:
    """The program, loaded on the loop of a test, and the editor that the test plays."""

    def __init__(self, harness: Harness, far: Far, running: "asyncio.Future[int]") -> None:
        self.harness = harness
        self.far = far
        self.running = running

    async def session(self) -> str:
        """Greet, begin a session in the working directory, and return its id."""
        await self.far.result("initialize", {"protocolVersion": 1})
        made = await self.far.result(
            "session/new", {"cwd": str(self.harness.cwd), "mcpServers": []}
        )
        return made["sessionId"]

    def agent(self) -> Agent:
        (agent,) = self.harness.agents.live.values()
        return agent

    def updates(self, kind: str) -> list[dict[str, Any]]:
        return [
            told["update"]
            for told in self.far.told("session/update")
            if told["update"]["sessionUpdate"] == kind
        ]


@contextlib.asynccontextmanager
async def editing(harness: Harness) -> AsyncGenerator[Editor]:
    far = Far()
    harness.ctx.provide(WIRE, far)
    await harness.ctx.plugin(columbina.apps.acp, {"model": MODEL.model_dump(), "cwd": harness.cwd})
    running = asyncio.ensure_future(harness.ctx.use(FRONTEND).run())
    yield Editor(harness, far, running)
    if not running.done():
        far.hang_up()
    assert await asyncio.wait_for(running, 10) == 0
    assert far.closed
    assert far.answered_too_often() == []


@pytest.fixture
async def editor(harness: Harness) -> AsyncIterator[Editor]:
    async with editing(harness) as held:
        yield held


def peek_tool(harness: Harness) -> None:
    harness.tools.add(
        harness.ctx,
        Tool(name="peek", description="Look.", parameters=Looked, execute=peeked, effect="read"),
    )


# Greeting and sessions


async def test_the_editor_is_told_what_this_side_can_do(editor: Editor) -> None:
    greeted = await editor.far.result(
        "initialize", {"protocolVersion": 3, "clientCapabilities": {}}
    )

    assert greeted["protocolVersion"] == 1
    assert greeted["agentCapabilities"]["loadSession"] is False
    assert greeted["agentCapabilities"]["promptCapabilities"] == {
        "image": False,
        "audio": False,
        "embeddedContext": True,
    }
    assert greeted["authMethods"] == []
    assert greeted["agentInfo"]["name"] == "columbina"


async def test_a_session_is_begun_in_the_directory_that_the_editor_names(
    editor: Editor, caplog: pytest.LogCaptureFixture
) -> None:
    with caplog.at_level(logging.INFO, LOGGER):
        made = await editor.far.result(
            "session/new", {"cwd": str(editor.harness.cwd), "mcpServers": [{"name": "x"}]}
        )

    assert editor.agent().id == made["sessionId"]
    assert editor.agent().spec.cwd == editor.harness.cwd
    assert "those of the profile are used instead" in caplog.text


@pytest.mark.parametrize("cwd", ["relative", "missing"])
async def test_a_session_is_not_begun_where_there_is_no_directory(editor: Editor, cwd: str) -> None:
    named = cwd if cwd == "relative" else str(editor.harness.cwd / cwd)

    code, said = await editor.far.refusal("session/new", {"cwd": named, "mcpServers": []})

    assert code == INVALID_PARAMS
    assert "is not an absolute path of a directory" in said


async def test_a_session_that_cannot_be_begun_is_said(
    editor: Editor, monkeypatch: pytest.MonkeyPatch
) -> None:
    from columbina.agent import AgentError, Agents

    async def refused(*args: object, **more: object) -> object:
        raise AgentError("no room")

    monkeypatch.setattr(Agents, "create", refused)

    code, said = await editor.far.refusal(
        "session/new", {"cwd": str(editor.harness.cwd), "mcpServers": []}
    )

    assert (code, said) == (INTERNAL_ERROR, "The session cannot be begun: no room.")


# A prompt


async def test_a_prompt_is_answered_with_the_reply_as_it_is_written(editor: Editor) -> None:
    editor.harness.model.add(Reply(reasoning="Think.", text="Hello there."))
    session = await editor.session()

    answer = await editor.far.result(
        "session/prompt",
        {
            "sessionId": session,
            "prompt": [
                {"type": "text", "text": "Look at"},
                {"type": "resource_link", "uri": "file:///a.txt", "name": "a.txt"},
                {"type": "resource", "resource": {"uri": "file:///b.txt", "text": "bee"}},
            ],
        },
    )

    assert answer == {"stopReason": "end_turn"}
    (request,) = editor.harness.model.requests
    said = cast("TextBlock", request.messages[-1].content[-1]).text
    assert said == (
        'Look at\n\n[a.txt](file:///a.txt)\n\n<resource uri="file:///b.txt">\nbee\n</resource>'
    )
    thought = "".join(one["content"]["text"] for one in editor.updates("agent_thought_chunk"))
    written = "".join(one["content"]["text"] for one in editor.updates("agent_message_chunk"))
    assert (thought, written) == ("Think.", "Hello there.")
    assert {told["sessionId"] for told in editor.far.told("session/update")} == {session}


async def test_a_call_of_a_tool_is_told_as_it_begins_and_ends(editor: Editor) -> None:
    peek_tool(editor.harness)
    editor.harness.model.add(
        Reply(calls=(Call(name="peek", arguments={"path": "a.txt"}, call_id="c1"),)),
        Reply(calls=(Call(name="peek", arguments={"path": "missing"}, call_id="c2"),)),
        Reply(text="Done."),
    )
    session = await editor.session()

    answer = await editor.far.result(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Peek."}]}
    )

    assert answer == {"stopReason": "end_turn"}
    first, second = editor.updates("tool_call")
    assert first == {
        "sessionUpdate": "tool_call",
        "toolCallId": "c1",
        "title": "peek: a.txt",
        "kind": "read",
        "status": "pending",
        "rawInput": {"path": "a.txt"},
        "locations": [{"path": str(editor.harness.cwd / "a.txt")}],
    }
    assert second["toolCallId"] == "c2"
    done, failed = editor.updates("tool_call_update")
    assert (done["toolCallId"], done["status"]) == ("c1", "completed")
    assert done["content"] == [
        {"type": "content", "content": {"type": "text", "text": "what a.txt holds"}}
    ]
    assert (failed["toolCallId"], failed["status"]) == ("c2", "failed")


async def test_a_call_that_tells_little_of_itself_is_told_as_it_is(editor: Editor) -> None:
    editor.harness.model.add(
        Reply(calls=(Call(name="web_search", arguments={"q": 1}, call_id="c1"),)),
        Reply(calls=(Call(name="nowhere", arguments="{", call_id="c2"),)),
        Reply(text="Done."),
    )
    session = await editor.session()

    await editor.far.result(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )

    first, second = editor.updates("tool_call")
    assert (first["kind"], first["rawInput"], "locations" in first) == ("search", {"q": 1}, False)
    assert (second["title"], second["kind"], second["rawInput"]) == ("nowhere", "other", "{")


async def test_what_happens_elsewhere_is_not_told_of_a_session(
    editor: Editor, harness: Harness
) -> None:
    harness.model.add(Reply(text="Hi."))
    session = await editor.session()
    mine = editor.agent()
    await editor.far.result(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )
    other = await harness.agents.create(harness.ctx, harness.spec(), session_id="other")
    before = len(editor.far.heard)

    mine.ctx.emit(SESSION_EVENT, Committed(other.session, mine.session.events[-1]))
    mine.ctx.emit(STREAM, StreamFrame(other, 1, 1, TextDelta(index=0, text="elsewhere")))

    assert len(editor.far.heard) == before


async def test_a_prompt_that_does_not_end_when_the_editor_leaves_is_broken_off(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(columbina.apps.acp, "_LAST_WORK_SECONDS", 0.01)
    async with editing(harness) as editor:
        session = await editor.session()

        agent = editor.agent()
        rested = agent.when_idle
        waited: list[None] = []

        async def never_for_the_prompt() -> None:
            # The prompt waits for good. When the program ends the agent, it is at rest.
            waited.append(None)
            if len(waited) == 1:
                await asyncio.Event().wait()
            await rested()

        monkeypatch.setattr(agent, "when_idle", never_for_the_prompt)
        asked = editor.far.ask(
            "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
        )
        await asyncio.sleep(0.05)

    (answer,) = editor.far.answers(asked)
    assert answer["error"] == {"code": INTERNAL_ERROR, "message": "The program is leaving."}


async def test_a_prompt_that_is_cancelled_is_answered_so(editor: Editor) -> None:
    editor.harness.model.add(Reply(text="slow", hold=True))
    session = await editor.session()
    asked = editor.far.ask(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )
    await asyncio.wait_for(editor.harness.model.held(), 5)

    second = await editor.far.refusal(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Again."}]}
    )
    editor.far.say({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}})

    answer = await editor.far.answer(asked)
    assert answer["result"] == {"stopReason": "cancelled"}
    assert second == (INVALID_REQUEST, "The session answers a prompt already.")


async def test_a_prompt_that_is_cancelled_as_its_turn_ends_is_answered_so(
    editor: Editor, monkeypatch: pytest.MonkeyPatch
) -> None:
    editor.harness.model.add(Reply(text="done", hold=True))
    session = await editor.session()
    asked = editor.far.ask(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )
    await asyncio.wait_for(editor.harness.model.held(), 5)

    # The cancel comes too late to stop anything: the turn ends as it would have.
    def too_late(why: str) -> None:
        editor.harness.model.release()

    monkeypatch.setattr(editor.agent(), "cancel", too_late)

    editor.far.say({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}})

    answer = await editor.far.answer(asked)
    assert answer["result"] == {"stopReason": "cancelled"}


async def test_a_prompt_whose_turn_fails_is_refused_with_why(editor: Editor) -> None:
    failure = LlmFailure(message="the key is not valid", code="AUTH", status=401)
    editor.harness.model.add(Reply(failure=failure))
    session = await editor.session()

    code, said = await editor.far.refusal(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )

    assert code == INTERNAL_ERROR
    assert said.startswith("The turn failed: ")


async def test_a_prompt_whose_agent_has_stopped_for_good_is_refused(
    editor: Editor, monkeypatch: pytest.MonkeyPatch
) -> None:
    session = await editor.session()

    async def broken() -> None:
        raise RuntimeError("the log cannot be written")

    monkeypatch.setattr(editor.agent(), "when_idle", broken)

    code, said = await editor.far.refusal(
        "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
    )

    assert (code, said) == (INTERNAL_ERROR, "The agent has stopped: the log cannot be written.")


@pytest.mark.parametrize(
    ("prompt", "said"),
    [
        ([{"type": "image", "data": "", "mimeType": "image/png"}], "'image' is not taken"),
        ([{"type": "resource", "resource": {"uri": "x", "blob": "AA=="}}], "only with its text"),
        ([{"type": "text", "text": "  "}], "There is nothing in the prompt"),
    ],
)
async def test_a_prompt_that_cannot_be_given_is_refused(
    editor: Editor, prompt: list[dict[str, Any]], said: str
) -> None:
    session = await editor.session()

    code, refusal = await editor.far.refusal(
        "session/prompt", {"sessionId": session, "prompt": prompt}
    )

    assert code == INVALID_PARAMS
    assert said in refusal


async def test_what_the_editor_asks_wrongly_is_refused(editor: Editor) -> None:
    far = editor.far

    assert await far.refusal("session/load", {}) == (-32601, "There is no method session/load.")
    code, said = await far.refusal("session/prompt", {"sessionId": "s"})
    assert (code, said.startswith("The params of session/prompt are not right: prompt")) == (
        INVALID_PARAMS,
        True,
    )
    assert await far.refusal("session/prompt", {"sessionId": "nobody", "prompt": []}) == (
        INVALID_PARAMS,
        "There is no session nobody.",
    )
    far.write(None)
    far.write(b"   ")
    far.write(b"{")
    await far.until(lambda: len(far.heard) == 5)
    assert [message["error"]["code"] for message in far.heard[3:]] == [INVALID_REQUEST, -32700]


async def test_what_the_editor_says_unasked_is_heard_or_passed_over(
    editor: Editor, caplog: pytest.LogCaptureFixture
) -> None:
    session = await editor.session()

    unasked: list[dict[str, Any]] = [
        {"method": "session/elsewhere", "params": {}},
        {"method": "session/cancel", "params": {}},
        {"method": "session/cancel", "params": {"sessionId": "nobody"}},
        {"method": "session/cancel", "params": {"sessionId": session}},
        {"id": 99, "result": {}},
    ]
    with caplog.at_level(logging.DEBUG, LOGGER):
        for said in unasked:
            editor.far.say({"jsonrpc": "2.0", **said})
        await editor.far.result("initialize", {"protocolVersion": 1})

    assert "session/elsewhere, which means nothing here" in caplog.text
    assert "a session/cancel names no session" in caplog.text
    assert "an answer to 99 came, and nothing waits for it" in caplog.text
    assert editor.agent().at_rest


async def test_a_request_that_fails_inside_is_answered_all_the_same(
    editor: Editor, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    def broken(*args: object) -> str:
        raise RuntimeError("inside")

    monkeypatch.setattr(columbina.apps.acp, "text", broken)

    with caplog.at_level(logging.ERROR, LOGGER):
        code, said = await editor.far.refusal("initialize", {"protocolVersion": 1})

    assert (code, said) == (
        INTERNAL_ERROR,
        "Something went wrong inside this program. Its log says what.",
    )
    assert "the request initialize failed" in caplog.text


async def test_a_prompt_that_runs_when_the_editor_leaves_is_broken_off(harness: Harness) -> None:
    harness.model.add(Reply(text="slow", hold=True))
    async with editing(harness) as editor:
        session = await editor.session()
        asked = editor.far.ask(
            "session/prompt", {"sessionId": session, "prompt": [{"type": "text", "text": "Go."}]}
        )
        await asyncio.wait_for(harness.model.held(), 5)
        editor.far.hang_up()
        await asyncio.wait_for(editor.running, 10)

    (answer,) = editor.far.answers(asked)
    assert "result" in answer or answer["error"]["code"] == INTERNAL_ERROR


async def test_an_agent_that_cannot_be_ended_is_noted(
    harness: Harness, caplog: pytest.LogCaptureFixture, monkeypatch: pytest.MonkeyPatch
) -> None:
    with caplog.at_level(logging.ERROR, LOGGER):
        async with editing(harness) as editor:
            await editor.session()
            agent = editor.agent()

            async def broken() -> None:
                raise RuntimeError("stuck")

            monkeypatch.setattr(agent, "when_idle", broken)

    assert "stopped for good: stuck" in caplog.text


# Questions


async def put(editor: Editor, request: ApprovalRequest) -> "asyncio.Future[Outcome]":
    approval = editor.harness.ctx.use(APPROVAL)
    before = len(editor.far.asked())
    answering = asyncio.ensure_future(approval.ask(editor.agent().ctx, request))
    await editor.far.until(lambda: len(editor.far.asked()) > before or answering.done())
    return answering


def question(agent: Agent, **other: Any) -> ApprovalRequest:
    asked: dict[str, Any] = {
        "tool": "peek",
        "summary": "peek: a.txt",
        "reason": "No rule allows this.",
        "target": "a.txt",
        **other,
    }
    return ApprovalRequest(session=agent.session, call_id="c-1", **asked)


async def asking(editor: Editor, *, permissions: bool = False) -> None:
    harness = editor.harness
    if permissions:
        for plugin in (columbina.fs, columbina.sandbox):
            await harness.ctx.plugin(plugin)
    await harness.ctx.plugin(columbina.approval)
    if permissions:
        await harness.ctx.plugin(columbina.permissions, {"preset": "workspace-write"})
    peek_tool(harness)
    await editor.session()


@pytest.mark.parametrize(
    ("answer", "outcome"),
    [
        ({"outcome": {"outcome": "selected", "optionId": "allow"}}, "allowed-once"),
        ({"outcome": {"outcome": "selected", "optionId": "reject"}}, "rejected"),
        ({"outcome": {"outcome": "cancelled"}}, "rejected"),
        (None, "rejected"),
    ],
)
async def test_a_call_that_needs_a_yes_is_put_to_the_editor(
    editor: Editor, answer: dict[str, Any] | None, outcome: Outcome
) -> None:
    await asking(editor)
    answering = await put(editor, question(editor.agent()))

    (asked,) = editor.far.asked()
    assert asked["method"] == "session/request_permission"
    params = asked["params"]
    assert params["toolCall"] == {
        "toolCallId": "c-1",
        "title": "peek: a.txt",
        "kind": "read",
        "status": "pending",
        "content": [
            {"type": "content", "content": {"type": "text", "text": "No rule allows this."}}
        ],
    }
    # Without the permissions, nothing would write a longer yes down.
    assert [option["optionId"] for option in params["options"]] == ["allow", "reject"]
    if answer is None:
        editor.far.fail(asked["id"])
    else:
        editor.far.reply(asked["id"], answer)
    assert await asyncio.wait_for(answering, 5) == outcome


async def test_a_longer_yes_is_written_down(editor: Editor) -> None:
    await asking(editor, permissions=True)
    agent = editor.agent()
    answering = await put(editor, question(agent, standing="peek of a.txt"))

    (asked,) = editor.far.asked()
    always = asked["params"]["options"][1]
    assert always == {
        "optionId": "allow-always",
        "name": "Allow, and do not ask again about peek of a.txt",
        "kind": "allow_always",
    }
    editor.far.reply(asked["id"], {"outcome": {"outcome": "selected", "optionId": "allow-always"}})

    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    assert editor.harness.ctx.use(PERMISSIONS).granted(agent.session) == (("peek", "a.txt"),)


async def test_a_longer_yes_that_cannot_be_written_down_is_a_yes_for_once(
    editor: Editor, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await asking(editor, permissions=True)
    granting = editor.harness.ctx.use(PERMISSIONS)

    def refused(*args: object) -> None:
        raise ValueError("no room")

    monkeypatch.setattr(type(granting), "grant", refused)
    answering = await put(editor, question(editor.agent(), standing="peek of a.txt"))
    (asked,) = editor.far.asked()

    with caplog.at_level(logging.WARNING, LOGGER):
        editor.far.reply(
            asked["id"], {"outcome": {"outcome": "selected", "optionId": "allow-always"}}
        )
        assert await asyncio.wait_for(answering, 5) == "allowed-once"

    assert "could not be written down: no room" in caplog.text


async def test_a_question_of_another_session_is_not_put_to_the_editor(
    editor: Editor, harness: Harness
) -> None:
    await asking(editor)
    mine = editor.agent()
    other = await harness.agents.create(harness.ctx, harness.spec(), session_id="other")

    outcome = await editor.harness.ctx.use(APPROVAL).ask(mine.ctx, question(other))

    assert outcome == "unavailable"
    assert editor.far.asked() == []


# The parts


@pytest.mark.parametrize(
    ("arguments", "title"),
    [
        ('{"path": "a.txt"}', "peek: a.txt"),
        ('{"command": "git   status\\n"}', "peek: git status"),
        ('{"query": "' + "x" * 100 + '"}', "peek: " + "x" * 77 + "..."),
        ('{"other": 1}', "peek"),
        ('{"path": ""}', "peek"),
        ("[1]", "peek"),
        ("{", "peek"),
    ],
)
def test_a_call_is_titled_by_what_it_acts_on(arguments: str, title: str) -> None:
    assert title_of("peek", arguments) == title


@pytest.mark.parametrize(
    ("reason", "stop"),
    [
        ("completed", "end_turn"),
        ("aborted", "cancelled"),
        ("blocked", "max_turn_requests"),
        ("max-tokens", "max_tokens"),
        ("interrupted", "end_turn"),
    ],
)
def test_the_end_of_a_turn_is_said_as_the_protocol_says_it(reason: Any, stop: str) -> None:
    assert stop_of(TurnEndEvent(turn=1, reason=reason)) == stop
    assert stop_of(None) == "end_turn"


@pytest.mark.parametrize(
    ("ended", "said"),
    [
        (TurnEndEvent(turn=1, reason="error", detail="it broke"), "The turn failed: it broke."),
        (TurnEndEvent(turn=1, reason="error", code="AUTH"), "The turn failed: AUTH."),
        (TurnEndEvent(turn=1, reason="error"), "The turn failed: an error."),
    ],
)
def test_a_turn_that_failed_is_said_as_an_error(ended: TurnEndEvent, said: str) -> None:
    with pytest.raises(RpcError, match=said):
        stop_of(ended)


def test_a_prompt_of_links_alone_is_given() -> None:
    assert said_in([{"type": "resource_link", "uri": "file:///x"}]) == "[file:///x](file:///x)"


# The command line


def test_the_program_for_an_editor_can_be_printed_instead_of_run(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(["acp", "--cwd", str(work), "--dump-config"])

    entries = {entry["id"]: entry for entry in yaml.safe_load(capsys.readouterr().out)}
    assert code == 0
    assert entries["frontend"]["plugin"] == "columbina.apps.acp"
    assert entries["frontend"]["config"]["cwd"] == str(work)


def test_the_program_for_an_editor_speaks_over_the_wire_of_its_process(
    home: Path, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    far = Far()
    monkeypatch.setattr(wire_module, "taken", lambda: far)
    far.say({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}})
    far.say(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {"cwd": str(work), "mcpServers": []},
        }
    )
    far.hang_up()

    chosen = profile(tmp_path, extends="acp")

    code = main(["acp", "--profile", chosen, "--cwd", str(work)])

    assert code == 0
    greeted, made = far.heard
    assert greeted["result"]["protocolVersion"] == 1
    assert isinstance(made["result"]["sessionId"], str)
    assert far.closed


def test_the_program_for_an_editor_needs_a_wire(
    home: Path, work: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def none() -> object:
        raise OSError("closed")

    monkeypatch.setattr(wire_module, "taken", none)

    assert main(["acp", "--cwd", str(work)]) == 2
    assert "'columbina acp' talks over its standard input and output" in capsys.readouterr().err
