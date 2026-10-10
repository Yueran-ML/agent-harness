"""The hooks at their five moments: when they run, what they are told, and what their answers do."""

import asyncio
import json
import logging
import os
from collections.abc import Callable
from pathlib import Path
from typing import Any, cast

import pytest
from pydantic import BaseModel

import columbina.agent
import columbina.agent_loop
import columbina.approval
import columbina.hooks
import columbina.llm
import columbina.session
import columbina.tools
from columbina import shells
from columbina.agent import AGENTS, Agent, AgentSpec
from columbina.approval import APPROVAL_ASK, ApprovalRequest, Outcome
from columbina.hooks import HookInvokedEvent, HookResultEvent
from columbina.hooks.config import Hook, trust
from columbina.hooks.run import Result
from columbina.kernel import Context, Next
from columbina.llm import LLM, LlmRequest
from columbina.llm.adapters.scripted import Call, Reply, Scripted
from columbina.session import ToolResultEvent, TurnEndEvent, UserMessageEvent
from columbina.tools import (
    POST_EXECUTE,
    PRE_EXECUTE,
    Block,
    Decision,
    Deny,
    Executed,
    Invocation,
    Tool,
    ToolFailure,
    ToolRun,
    Verdict,
)
from support.loop import MODEL, SYSTEM, Harness, idle, user

Answer = Result | Callable[[dict[str, Any]], Result]


class Ran:
    """Stands in for the run of a hook: it keeps what each hook was told, and answers for it."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, dict[str, Any]]] = []
        self.answers: dict[str, Answer] = {}
        self.places: list[tuple[Path, Path, int]] = []

    async def run(
        self,
        hook: Hook,
        event: str,
        told: bytes,
        *,
        shell: str,
        cwd: Path,
        project: Path,
        limit: int,
    ) -> Result:
        heard = json.loads(told)
        self.calls.append((event, hook.command, heard))
        self.places.append((cwd, project, limit))
        answer = self.answers.get(hook.command, Result(exit_code=0))
        return answer if isinstance(answer, Result) else answer(heard)

    def events(self) -> list[str]:
        return [event for event, _, _ in self.calls]

    def told(self, event: str) -> list[dict[str, Any]]:
        return [heard for name, _, heard in self.calls if name == event]


class Text(BaseModel):
    text: str = ""


class Where(BaseModel):
    path: str


@pytest.fixture
def ran(monkeypatch: pytest.MonkeyPatch) -> Ran:
    monkeypatch.setattr(shells, "find", lambda: "the-shell")
    stand_in = Ran()
    monkeypatch.setattr(columbina.hooks, "run", stand_in.run)
    return stand_in


def hooks_file(path: Path, hooks: dict[str, Any]) -> bytes:
    content = json.dumps({"hooks": hooks}).encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return content


def at(event: str, command: str, matcher: str | None = None) -> dict[str, Any]:
    """The hooks of one moment: one group with one command."""
    group: dict[str, Any] = {"hooks": [{"type": "command", "command": command}]}
    if matcher is not None:
        group["matcher"] = matcher
    return {event: [group]}


async def working(
    harness: Harness, tmp_path: Path, hooks: dict[str, Any], *script: Any, **config: Any
) -> Agent:
    """Load the plugin with a hook file of the user, and return an agent with three tools."""
    hooks_file(tmp_path / "home" / "hooks.json", hooks)
    await harness.ctx.plugin(
        columbina.hooks, {"user_file": tmp_path / "home" / "hooks.json", **config}
    )

    async def note(args: Text, run: ToolRun) -> str:
        return f"noted {args.text}"

    async def broken(args: Text, run: ToolRun) -> str:
        raise ToolFailure("It does not work.", code="BROKEN")

    async def edit(args: Where, run: ToolRun) -> str:
        return f"edited {args.path}"

    for name, execute, parameters in (
        ("note", note, Text),
        ("broken", broken, Text),
        ("edit", edit, Where),
        # A tool that takes a path and does not work on one file.
        ("walk", edit, Where),
    ):
        harness.tools.add(
            harness.ctx,
            Tool(
                name=name, description=f"The tool {name}.", parameters=parameters, execute=execute
            ),
        )
    return await harness.agent(*(script or (Reply(text="Hello!"),)))


async def turn(agent: Agent, text: str = "Go on.") -> None:
    agent.followup(text)
    await idle(agent)


def calls(name: str, **arguments: Any) -> Reply:
    return Reply(calls=(Call(name=name, arguments=arguments),))


def results(agent: Agent) -> list[tuple[str, str | None]]:
    return [
        (
            "".join(block.text for block in event.data.message.content),
            None if event.data.error is None else event.data.error.code,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def ends(agent: Agent) -> list[tuple[str, str | None]]:
    return [
        (event.data.reason, event.data.detail)
        for event in agent.session.events
        if isinstance(event.data, TurnEndEvent)
    ]


def from_hooks(agent: Agent) -> list[tuple[str, str]]:
    """What hooks put before the model and it still sees: the source and the text of each."""
    session = agent.session
    return [
        (data.source, "".join(block.text for block in data.message.content))
        for seq in session.surface
        if isinstance(data := session.events[seq].data, UserMessageEvent)
        and data.source.startswith("hook/")
    ]


def from_context(agent: Agent) -> list[tuple[str, str]]:
    """What sources of context put before the model and it still sees."""
    session = agent.session
    return [
        (data.source, "".join(block.text for block in data.message.content))
        for seq in session.surface
        if isinstance(data := session.events[seq].data, UserMessageEvent)
        and data.source.startswith("context/")
    ]


def logged(caplog: pytest.LogCaptureFixture, level: int = logging.WARNING) -> list[str]:
    return [
        record.getMessage()
        for record in caplog.records
        if record.name == "columbina.hooks" and record.levelno == level
    ]


# When a session begins


async def test_what_a_hook_writes_when_a_session_begins_stands_before_the_first_input(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["date"] = Result(context="Today is a holiday.", exit_code=0)
    agent = await working(
        harness, tmp_path, at("SessionStart", "date"), Reply(text="One."), Reply(text="Two.")
    )

    await turn(agent, "Say hello.")
    await turn(agent, "Say it again.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        user('<context source="hooks">\nToday is a holiday.\n</context>'),
        user("Say hello."),
    )
    # A session begins once, however many turns it has.
    assert ran.events() == ["SessionStart"]
    (told,) = ran.told("SessionStart")
    assert told == {
        "session_id": "s-1",
        "transcript_path": str(harness.sessions.directory(harness.cwd, "s-1") / "session.v1.jsonl"),
        "cwd": str(harness.cwd),
        "hook_event_name": "SessionStart",
        "source": "startup",
    }
    assert ran.places == [(harness.cwd, Path(os.path.realpath(harness.cwd)), 10000)]


async def test_a_session_that_is_taken_up_again_says_so_to_its_hooks(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["date"] = Result(context="Today is a holiday.", exit_code=0)
    agent = await working(harness, tmp_path, at("SessionStart", "date"), Reply(text="One."))
    await turn(agent)
    await agent.close()
    harness.model.add(Reply(text="Two."))
    again = await harness.agents.resume(harness.ctx, harness.spec(), "s-1")

    await turn(again)

    assert [told["source"] for told in ran.told("SessionStart")] == ["startup", "resume"]
    # What the hooks write is word of something that happened, and is given each time.
    assert [source for source, _ in from_context(again)] == ["context/hooks"] * 2


# What a person says


async def test_a_hook_is_run_for_what_a_person_says_and_for_nothing_else(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks = {**at("UserPromptSubmit", "check"), **at("SessionStart", "date")}
    ran.answers["date"] = Result(context="Today is a holiday.", exit_code=0)

    def steering(request: LlmRequest) -> Reply:
        agent.steer("And be quick.")
        agent.inject("A note of a plugin.", source="plugin")
        return calls("note", text="x")

    agent = await working(harness, tmp_path, hooks, steering, Reply(text="Done."))

    await turn(agent, "把月亮画出来")

    # Not for the context of the hook before it, and not for what a plugin put in.
    assert [told["prompt"] for told in ran.told("UserPromptSubmit")] == [
        "把月亮画出来",
        "And be quick.",
    ]


async def test_a_hook_can_add_something_to_what_a_person_says(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["check"] = Result(context="The branch is behind main.", exit_code=0)
    agent = await working(harness, tmp_path, at("UserPromptSubmit", "check"))

    await turn(agent, "Deploy.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("Deploy."),
        user("The branch is behind main."),
    )
    assert from_hooks(agent) == [("hook/UserPromptSubmit", "The branch is behind main.")]


async def test_an_input_that_a_hook_stops_ends_the_turn_and_is_taken_from_the_model(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["check"] = lambda told: (
        Result("blocked", "block", "No secrets.", exit_code=2)
        if "password" in told["prompt"]
        else Result(exit_code=0)
    )
    agent = await working(harness, tmp_path, at("UserPromptSubmit", "check"), Reply(text="Sure."))

    await turn(agent, "My password is hunter2.")

    assert ends(agent) == [("blocked", "a hook stopped the input: No secrets.")]
    assert harness.model.requests == []

    await turn(agent, "Say hello.")

    # The next turn does not read what was stopped, only that something was.
    assert harness.model.requests[0].messages == (
        SYSTEM,
        user("[A hook of the user stopped what was said here: No secrets.]"),
        user("Say hello."),
    )
    # The log still holds what was said.
    said = [
        "".join(block.text for block in event.data.message.content)
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent) and event.data.source == "user"
    ]
    assert said == ["My password is hunter2.", "Say hello."]


# Before a call


async def test_a_hook_is_told_of_a_call_by_the_name_that_other_tools_give_its_tool(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks = {**at("PreToolUse", "before"), **at("PostToolUse", "after")}
    agent = await working(
        harness,
        tmp_path,
        hooks,
        calls("edit", path="src/a.py"),
        calls("note", text="月"),
        Reply(text="Done."),
    )

    await turn(agent)

    edited = str(Path(os.path.realpath(harness.cwd)) / "src" / "a.py")
    assert [(told["tool_name"], told["tool_input"]) for told in ran.told("PreToolUse")] == [
        # A tool that works on one file also says where that file is, whole.
        ("Edit", {"path": "src/a.py", "file_path": edited}),
        ("note", {"text": "月"}),
    ]
    assert [told["tool_use_id"] for told in ran.told("PreToolUse")] == ["call-1", "call-2"]
    assert [(told["tool_name"], told["tool_response"]) for told in ran.told("PostToolUse")] == [
        ("Edit", "edited src/a.py"),
        ("note", "noted 月"),
    ]


async def test_a_group_runs_for_the_tools_that_its_matcher_names(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks = {
        "PreToolUse": [
            {"matcher": "Edit|Write", "hooks": [{"command": "for-edits"}]},
            {"matcher": "note", "hooks": [{"command": "for-notes"}]},
            {"hooks": [{"command": "for-all"}]},
        ]
    }
    agent = await working(
        harness, tmp_path, hooks, calls("edit", path="a.py"), calls("note"), Reply(text="Done.")
    )

    await turn(agent)

    assert [command for _, command, _ in ran.calls] == [
        "for-edits",
        "for-all",
        "for-notes",
        "for-all",
    ]


async def test_a_call_that_a_hook_stops_does_not_run(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["before"] = Result("blocked", "block", "Not on a Friday.", exit_code=2)
    hooks = {**at("PreToolUse", "before"), **at("PostToolUse", "after")}
    agent = await working(harness, tmp_path, hooks, calls("note", text="x"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        ("The call was not allowed: a hook stopped the call: Not on a Friday.", "HOOK_BLOCKED")
    ]
    # Nothing ran, so nothing came after it.
    assert ran.events() == ["PreToolUse"]


async def test_a_hook_that_lets_a_call_through_does_not_get_it_past_the_other_checks(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["before"] = Result(decision="allow", exit_code=0)
    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )

    async def rule(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny("No rule allows this.")

    harness.ctx.on(PRE_EXECUTE, rule)

    await turn(agent)

    assert results(agent) == [("The call was not allowed: No rule allows this.", "DENIED")]


async def asking(harness: Harness, answer: str | None) -> list[ApprovalRequest]:
    """Load the approval, with somebody who gives this answer, or with nobody."""
    await harness.ctx.plugin(columbina.approval, {"policy": "ask"})
    asked: list[ApprovalRequest] = []

    async def somebody(request: ApprovalRequest) -> Outcome:
        asked.append(request)
        return cast("Outcome", answer)

    if answer is not None:
        harness.ctx.on(APPROVAL_ASK, somebody)
    return asked


@pytest.mark.parametrize(
    ("answer", "outcome"),
    [
        ("allowed-once", [("noted x", None)]),
        (
            "rejected",
            [
                (
                    "The call was not allowed: a hook wanted somebody asked about the call, "
                    "and nobody said yes",
                    "HOOK_BLOCKED",
                )
            ],
        ),
    ],
)
async def test_a_hook_can_have_somebody_asked_about_a_call(
    harness: Harness, tmp_path: Path, ran: Ran, answer: str, outcome: list[tuple[str, str | None]]
) -> None:
    asked = await asking(harness, answer)
    ran.answers["before"] = Result(decision="ask", reason="This writes outside.", exit_code=0)
    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )

    await turn(agent)

    assert results(agent) == outcome
    (request,) = asked
    assert (request.call_id, request.tool, request.summary, request.reason, request.target) == (
        "call-1",
        "note",
        "note",
        "This writes outside.",
        None,
    )


@pytest.mark.parametrize("with_approval", [True, False])
async def test_where_nobody_can_be_asked_a_call_that_a_hook_wants_asked_about_does_not_run(
    harness: Harness, tmp_path: Path, ran: Ran, with_approval: bool
) -> None:
    if with_approval:
        await asking(harness, None)
    ran.answers["before"] = Result(decision="ask", exit_code=0)
    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )

    await turn(agent)

    assert [code for _, code in results(agent)] == ["HOOK_BLOCKED"]


# After a call


async def test_a_hook_can_put_a_reason_in_the_place_of_a_result_or_add_to_it(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["after"] = lambda told: (
        Result("blocked", "block", "It holds a key.", exit_code=0)
        if told["tool_input"]["text"] == "secret"
        else Result(context="Run the tests next.", exit_code=0)
    )
    agent = await working(
        harness,
        tmp_path,
        at("PostToolUse", "after"),
        calls("note", text="secret"),
        calls("note", text="plain"),
        Reply(text="Done."),
    )

    await turn(agent)

    assert results(agent) == [
        (
            "The result of the call is withheld: a hook withheld it: It holds a key.",
            "RESULT_BLOCKED",
        ),
        ("noted plain\n\n[A hook of the user adds:]\nRun the tests next.", None),
    ]


async def test_a_call_that_failed_has_no_hook_after_it(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks = {**at("PreToolUse", "before"), **at("PostToolUse", "after")}
    agent = await working(harness, tmp_path, hooks, calls("broken"), Reply(text="Done."))

    await turn(agent)

    assert [code for _, code in results(agent)] == ["BROKEN"]
    assert ran.events() == ["PreToolUse"]


# When a turn would end


async def test_a_hook_can_keep_the_model_going_when_its_turn_would_end(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["verify"] = lambda told: (
        Result(exit_code=0)
        if told["stop_hook_active"]
        else Result("blocked", "block", "The tests fail.", exit_code=2)
    )
    agent = await working(
        harness, tmp_path, at("Stop", "verify"), Reply(text="Done."), Reply(text="Now they pass.")
    )

    await turn(agent)

    assert [told["stop_hook_active"] for told in ran.told("Stop")] == [False, True]
    assert harness.model.requests[1].messages[-1] == user("The tests fail.")
    assert from_hooks(agent) == [("hook/Stop", "The tests fail.")]
    assert ends(agent) == [("completed", None)]


async def test_a_hook_keeps_a_turn_going_three_times_and_no_more(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    ran.answers["verify"] = Result("blocked", "block", "Not yet.", exit_code=2)
    agent = await working(
        harness, tmp_path, at("Stop", "verify"), *(Reply(text="Done.") for _ in range(8))
    )

    await turn(agent)

    assert len(harness.model.requests) == 4
    assert [told["stop_hook_active"] for told in ran.told("Stop")] == [False, True, True, True]
    assert ends(agent) == [("completed", None)]
    assert logged(caplog) == [
        "a Stop hook wants the turn to go on, and it has done so 3 times already. The turn ends"
    ]

    # The next turn has its own three.
    await turn(agent)

    assert len(harness.model.requests) == 8
    assert ran.told("Stop")[4]["stop_hook_active"] is False


async def test_how_often_a_turn_is_kept_going_is_for_the_config_to_say(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["verify"] = Result("blocked", "block", "Not yet.", exit_code=2)
    agent = await working(
        harness, tmp_path, at("Stop", "verify"), Reply(text="Done."), max_stop_continuations=0
    )

    await turn(agent)

    assert len(harness.model.requests) == 1


# The log


async def test_every_run_of_a_hook_leaves_two_events_in_the_log(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    ran.answers["before"] = Result("blocked", "block", "Not on a Friday.", exit_code=2)
    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )

    await turn(agent)

    kept = [
        event.data
        for event in agent.session.events
        if isinstance(event.data, HookInvokedEvent | HookResultEvent)
    ]
    invoked, result = kept
    assert invoked == HookInvokedEvent(event="PreToolUse", command="before")
    assert isinstance(result, HookResultEvent)
    assert (result.event, result.command, result.status, result.exit_code, result.reason) == (
        "PreToolUse",
        "before",
        "blocked",
        2,
        "Not on a Friday.",
    )
    assert result.duration_ms >= 0
    # A program that reads the log and does not know hooks may pass both over.
    assert HookInvokedEvent.ignorable
    assert HookResultEvent.ignorable


async def test_what_is_wrong_with_a_hook_is_written_to_the_log_of_the_program(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    ran.answers["before"] = Result(
        "failed", warnings=("it ended with the exit code 1",), message="3 files", exit_code=1
    )
    ran.answers["slow"] = Result("timed-out")
    hooks = {"PreToolUse": [{"hooks": [{"command": "before"}, {"command": "slow", "timeout": 5}]}]}
    agent = await working(harness, tmp_path, hooks, calls("note", text="x"), Reply(text="Done."))

    await turn(agent)

    # Neither of them stops the call.
    assert results(agent) == [("noted x", None)]
    assert logged(caplog) == [
        "the PreToolUse hook 'before': it ended with the exit code 1",
        "the PreToolUse hook 'before' says: 3 files",
        "the PreToolUse hook 'slow' was stopped after 5 seconds",
    ]


async def test_a_run_that_fails_in_a_way_that_nobody_foresaw_leaves_the_turn_alone(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    def fails(told: dict[str, Any]) -> Result:
        raise RuntimeError("something else")

    ran.answers["before"] = fails
    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )

    await turn(agent)

    assert results(agent) == [("noted x", None)]
    assert logged(caplog, logging.ERROR) == ["running the PreToolUse hooks failed"]
    # What was begun is closed in the log all the same.
    statuses = [
        event.data.status
        for event in agent.session.events
        if isinstance(event.data, HookResultEvent)
    ]
    assert statuses == ["failed"]


async def test_a_hook_is_stopped_with_the_turn_that_it_runs_in(
    harness: Harness, tmp_path: Path, ran: Ran, monkeypatch: pytest.MonkeyPatch
) -> None:
    reached = asyncio.Event()

    async def never(hook: Hook, event: str, told: bytes, **more: Any) -> Result:
        reached.set()
        await asyncio.Event().wait()
        return Result()

    agent = await working(
        harness, tmp_path, at("PreToolUse", "before"), calls("note", text="x"), Reply(text="Done.")
    )
    monkeypatch.setattr(columbina.hooks, "run", never)
    agent.followup("Go on.")
    await asyncio.wait_for(reached.wait(), timeout=5)

    agent.cancel()
    await idle(agent)

    assert [reason for reason, _ in ends(agent)] == ["aborted"]
    statuses = [
        event.data.status
        for event in agent.session.events
        if isinstance(event.data, HookResultEvent)
    ]
    assert statuses == ["failed"]


async def test_without_a_shell_no_hook_is_run_and_that_is_said_once(
    harness: Harness,
    tmp_path: Path,
    ran: Ran,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    monkeypatch.setattr(shells, "find", lambda: None)
    agent = await working(
        harness, tmp_path, at("UserPromptSubmit", "check"), Reply(text="One."), Reply(text="Two.")
    )

    await turn(agent)
    await turn(agent)

    assert ran.calls == []
    assert logged(caplog) == ["no hook is run: the shell of this system is not installed"]


# The files


async def test_the_hook_file_is_read_anew_when_a_turn_begins(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    agent = await working(
        harness, tmp_path, at("UserPromptSubmit", "old"), Reply(text="One."), Reply(text="Two.")
    )

    await turn(agent)
    hooks_file(tmp_path / "home" / "hooks.json", at("UserPromptSubmit", "new"))
    await turn(agent)

    assert [command for _, command, _ in ran.calls] == ["old", "new"]


async def test_a_hook_file_that_cannot_be_used_is_said_once_and_stops_nothing(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    agent = await working(harness, tmp_path, {}, Reply(text="One."), Reply(text="Two."))
    file = tmp_path / "home" / "hooks.json"
    file.write_bytes(b'{"hooks": {"Later": [], "Stop": "x"}')
    await turn(agent)
    file.write_bytes(b'{"hooks": {"Later": [], "Stop": "x"}}')
    await turn(agent)
    await turn_again(agent, harness)

    assert ends(agent) == [("completed", None)] * 3
    assert logged(caplog) == [
        f"the hook file {file} is not used: it is not JSON: Expecting ',' delimiter: line 1 "
        "column 37 (char 36)",
        f'in the hook file {file}: "Later" is no moment at which hooks run here, and is passed '
        "over. The moments are: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop",
        f'in the hook file {file}: Stop holds "x" and no list of groups, and is passed over',
    ]


async def turn_again(agent: Agent, harness: Harness) -> None:
    harness.model.add(Reply(text="Three."))
    await turn(agent)


async def test_without_a_hook_file_nothing_happens(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    await harness.ctx.plugin(columbina.hooks, {"user_file": tmp_path / "nowhere" / "hooks.json"})
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent)

    assert ran.calls == []
    assert ends(agent) == [("completed", None)]
    # A file that is not there is nothing to speak of.
    assert logged(caplog) == []


def project_file(harness: Harness) -> Path:
    return Path(os.path.realpath(harness.cwd)) / ".columbina" / "hooks.json"


async def test_the_hooks_of_a_project_run_only_once_its_file_is_trusted(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    record = tmp_path / "home" / "trusted-hooks.json"
    content = hooks_file(project_file(harness), at("UserPromptSubmit", "of-the-project"))
    agent = await working(
        harness,
        tmp_path,
        at("UserPromptSubmit", "of-the-user"),
        *(Reply(text="Go.") for _ in range(4)),
        trust_file=record,
    )

    await turn(agent)
    await turn(agent)

    assert [command for _, command, _ in ran.calls] == ["of-the-user", "of-the-user"]
    assert logged(caplog) == [
        f"the hooks of this project are not run: the file {project_file(harness)} is not "
        "trusted as it is now. Look at what it would run, and trust it with: columbina hooks trust"
    ]

    # The user looks at the file and says that it may run. Its hooks come after their own.
    record.write_bytes(trust(None, project_file(harness), content))
    await turn(agent)

    assert [command for _, command, _ in ran.calls][2:] == ["of-the-user", "of-the-project"]

    # The file changes, and what was trusted is not what is there any more.
    hooks_file(project_file(harness), at("UserPromptSubmit", "something-else"))
    await turn(agent)

    assert [command for _, command, _ in ran.calls][4:] == ["of-the-user"]


async def test_the_hook_file_of_a_project_can_be_left_unread(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    record = tmp_path / "home" / "trusted-hooks.json"
    content = hooks_file(project_file(harness), at("UserPromptSubmit", "of-the-project"))
    record.parent.mkdir(parents=True, exist_ok=True)
    record.write_bytes(trust(None, project_file(harness), content))
    agent = await working(harness, tmp_path, {}, trust_file=record, project=False)

    await turn(agent)

    assert ran.calls == []
    assert logged(caplog) == []


async def test_without_a_record_of_trust_no_project_is_trusted(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks_file(project_file(harness), at("UserPromptSubmit", "of-the-project"))
    agent = await working(harness, tmp_path, {})

    await turn(agent)

    assert ran.calls == []


async def test_the_hooks_of_a_project_count_without_a_hook_file_of_the_user(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    record = tmp_path / "home" / "trusted-hooks.json"
    content = hooks_file(project_file(harness), at("UserPromptSubmit", "of-the-project"))
    record.parent.mkdir(parents=True, exist_ok=True)
    record.write_bytes(trust(None, project_file(harness), content))
    await harness.ctx.plugin(columbina.hooks, {"trust_file": record})
    agent = await harness.agent(Reply(text="Hello!"))

    await turn(agent)

    assert [command for _, command, _ in ran.calls] == ["of-the-project"]


async def test_a_hook_file_that_cannot_be_read_is_said_and_stops_nothing(
    harness: Harness, tmp_path: Path, ran: Ran, caplog: pytest.LogCaptureFixture
) -> None:
    # A directory lies where each of the two files should.
    of_the_user = tmp_path / "home" / "hooks.json"
    of_the_user.mkdir(parents=True)
    project_file(harness).mkdir(parents=True)
    await harness.ctx.plugin(columbina.hooks, {"user_file": of_the_user})
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))

    await turn(agent)
    await turn(agent)

    assert ends(agent) == [("completed", None)] * 2
    first, second = logged(caplog)
    assert first.startswith(f"the hook file {of_the_user} cannot be read: ")
    assert second.startswith(f"the hook file {project_file(harness)} cannot be read: ")


async def test_a_result_that_something_else_withheld_is_not_shown_to_a_hook(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    agent = await working(
        harness, tmp_path, at("PostToolUse", "after"), calls("note", text="x"), Reply(text="Done.")
    )

    async def withholds(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return Block("It holds a key.")

    harness.ctx.on(POST_EXECUTE, withholds)

    await turn(agent)

    assert [code for _, code in results(agent)] == ["RESULT_BLOCKED"]
    assert ran.calls == []


async def test_where_sessions_are_kept_in_memory_a_hook_is_told_no_file(
    ctx: Context, tmp_path: Path, ran: Ran
) -> None:
    await ctx.plugin(columbina.session)
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 5})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.agent)
    await ctx.plugin(columbina.agent_loop)
    model = Scripted([Reply(text="Hello!")])
    ctx.use(LLM).adapters.add(ctx, "scripted", model)
    hooks_file(tmp_path / "hooks.json", at("UserPromptSubmit", "check"))
    await ctx.plugin(columbina.hooks, {"user_file": tmp_path / "hooks.json"})
    spec = AgentSpec.model_validate({"cwd": tmp_path, "model": MODEL, "system_prompt": "Be kind."})
    agent = await ctx.use(AGENTS).create(ctx, spec, session_id="s-1")

    await turn(agent)

    (told,) = ran.told("UserPromptSubmit")
    assert told["transcript_path"] == ""


async def test_only_a_tool_that_works_on_one_file_says_where_that_file_is(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    agent = await working(
        harness,
        tmp_path,
        at("PreToolUse", "before"),
        calls("walk", path="src"),
        Reply(text="Done."),
    )

    await turn(agent)

    assert [(told["tool_name"], told["tool_input"]) for told in ran.told("PreToolUse")] == [
        ("walk", {"path": "src"})
    ]


async def test_a_matcher_counts_only_at_the_moments_that_have_a_tool(
    harness: Harness, tmp_path: Path, ran: Ran
) -> None:
    hooks = {
        "Stop": [{"matcher": "Bash", "hooks": [{"command": "verify"}]}],
        "UserPromptSubmit": [{"matcher": "Bash", "hooks": [{"command": "check"}]}],
    }
    agent = await working(harness, tmp_path, hooks)

    await turn(agent)

    assert [command for _, command, _ in ran.calls] == ["check", "verify"]
