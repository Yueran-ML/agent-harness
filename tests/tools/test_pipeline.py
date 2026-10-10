"""The tool pipeline: what happens between a call of the model and its result."""

import asyncio
import contextlib
import dataclasses
import logging
import threading
from collections.abc import Awaitable, Callable
from typing import Any, cast

import pytest
from pydantic import BaseModel, PrivateAttr, ValidationError, field_validator, model_serializer

import columbina.tools
from columbina import cancellation
from columbina.kernel import Context, Next, Plugin, RegistryConflictError
from columbina.llm import TextBlock, ToolCallBlock, ToolMessage
from columbina.session import ToolError, ToolResultEvent
from columbina.tools import (
    EXECUTE,
    POST_EXECUTE,
    PRE_EXECUTE,
    RESULT,
    TOOLS,
    Accept,
    Allow,
    Block,
    Decision,
    Deny,
    Executed,
    Invocation,
    Tool,
    ToolFailure,
    ToolOutcome,
    ToolRun,
    Tools,
    Verdict,
)


class EchoArgs(BaseModel):
    text: str = "hi"


class Journal:
    """Tools for these tests, and a record of what they did."""

    def __init__(self) -> None:
        self.ran: list[str] = []
        self.cleaned_up = False
        self.started = asyncio.Event()

    async def echo(self, args: EchoArgs, run: ToolRun) -> str:
        self.ran.append(args.text)
        return args.text

    async def count(self, args: EchoArgs, run: ToolRun) -> int:
        return len(args.text)

    async def fail(self, args: EchoArgs, run: ToolRun) -> str:
        raise ToolFailure(f"There is no file {args.text}.", code="NOT_FOUND")

    async def crash(self, args: EchoArgs, run: ToolRun) -> str:
        raise RuntimeError("boom")

    async def hang(self, args: EchoArgs, run: ToolRun) -> str:
        self.started.set()
        try:
            await asyncio.Event().wait()
        finally:
            self.cleaned_up = True
        return "never"

    async def hang_and_fail_to_clean_up(self, args: EchoArgs, run: ToolRun) -> str:
        self.started.set()
        try:
            await asyncio.Event().wait()
        finally:
            raise RuntimeError("cleanup failed")

    async def cancel_itself(self, args: EchoArgs, run: ToolRun) -> str:
        raise asyncio.CancelledError

    async def conclude(self, args: EchoArgs, run: ToolRun) -> str:
        run.conclude_turn()
        return "done"


def tool(
    name: str, execute: Callable[[EchoArgs, ToolRun], Awaitable[object]], **options: Any
) -> Tool[EchoArgs, object]:
    return Tool(
        name=name,
        description=f"The {name} tool.",
        parameters=EchoArgs,
        execute=execute,
        **options,
    )


def call(name: str, arguments: str = "{}", call_id: str = "call-1") -> ToolCallBlock:
    return ToolCallBlock(call_id=call_id, name=name, arguments=arguments)


@pytest.fixture
def journal() -> Journal:
    return Journal()


# A call that works


async def test_a_call_runs_the_tool_and_returns_what_the_model_reads(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))
    asked = call("echo", '{"text": "hello"}')

    outcome = await tools.run(ctx, asked, run)

    assert outcome == ToolOutcome(call=asked, text="hello", value="hello")
    assert journal.ran == ["hello"]


async def test_an_outcome_becomes_the_result_event_of_the_log(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo), tool("fail", journal.fail))

    good = await tools.run(ctx, call("echo", '{"text": "hello"}'), run)
    bad = await tools.run(ctx, call("fail", '{"text": "a.txt"}', call_id="call-2"), run)

    assert good.to_event() == ToolResultEvent(
        message=ToolMessage(call_id="call-1", content=(TextBlock(text="hello"),))
    )
    assert bad.to_event() == ToolResultEvent(
        message=ToolMessage(
            call_id="call-2", content=(TextBlock(text="There is no file a.txt."),), is_error=True
        ),
        error=ToolError(code="NOT_FOUND", reason="There is no file a.txt."),
    )


async def test_a_tool_can_say_how_its_value_reads(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def render(args: EchoArgs, value: int) -> str:
        return f"{args.text!r} has {value} characters"

    tools.add(ctx, tool("count", journal.count, render=render))

    outcome = await tools.run(ctx, call("count", '{"text": "hello"}'), run)

    assert outcome.text == "'hello' has 5 characters"
    assert outcome.value == 5


async def test_a_value_without_a_rendering_is_converted_to_text(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("count", journal.count))

    outcome = await tools.run(ctx, call("count", '{"text": "hello"}'), run)

    assert outcome.text == "5"


async def test_a_result_without_text_says_so(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))

    outcome = await tools.run(ctx, call("echo", '{"text": ""}'), run)

    assert outcome.text == "(no output)"
    assert outcome.error is None


async def test_a_tool_can_ask_for_the_turn_to_end(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("conclude", journal.conclude), tool("echo", journal.echo))

    concluding = await tools.run(ctx, call("conclude"), run)

    assert concluding.concludes_turn
    assert not ToolOutcome(call=call("echo"), text="x").concludes_turn


# A call that does not work


async def test_a_call_of_a_tool_that_is_not_there_is_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo), tool("count", journal.count))

    outcome = await tools.run(ctx, call("ehco"), run)

    assert outcome.error == ToolError(
        code="UNKNOWN_TOOL", reason="There is no tool named 'ehco'. The tools are: echo, count."
    )
    assert outcome.text == "There is no tool named 'ehco'. The tools are: echo, count."
    assert outcome.value is None


async def test_without_any_tool_the_error_says_so(ctx: Context, tools: Tools, run: ToolRun) -> None:
    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.text == "There is no tool named 'echo'. There are no tools."


# Tools for one caller and not for another


def test_a_caller_can_be_told_about_some_of_the_tools_only(
    ctx: Context, tools: Tools, journal: Journal
) -> None:
    tools.add(
        ctx, tool("echo", journal.echo), tool("count", journal.count), tool("fail", journal.fail)
    )

    assert [schema.name for schema in tools.schemas()] == ["echo", "count", "fail"]
    # They come in the order in which they were added, whatever order they are named in.
    some = tools.schemas(only=frozenset({"fail", "echo"}))
    assert [schema.name for schema in some] == ["echo", "fail"]
    assert tools.schemas(only=frozenset[str]()) == ()
    # A name that no tool has adds nothing.
    named = tools.schemas(only=frozenset({"count", "nowhere"}))
    assert [schema.name for schema in named] == ["count"]


async def test_a_tool_that_a_caller_may_not_use_is_not_there_for_it(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo), tool("count", journal.count))

    refused = tools.begin(ctx, call("echo"), run, only=frozenset({"count"}))

    assert refused.outcome is not None
    # The tools that the answer names are the ones that the caller may use.
    assert refused.outcome.error == ToolError(
        code="UNKNOWN_TOOL", reason="There is no tool named 'echo'. The tools are: count."
    )
    assert journal.ran == []
    # One that it may use is taken up as ever.
    taken = tools.begin(ctx, call("count"), run, only=frozenset({"count"}))
    assert taken.outcome is None


async def test_a_caller_that_may_use_no_tool_is_told_that_there_are_none(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))

    refused = tools.begin(ctx, call("echo"), run, only=frozenset[str]())

    assert refused.outcome is not None
    assert refused.outcome.text == "There is no tool named 'echo'. There are no tools."


async def test_arguments_that_do_not_fit_are_an_error_result_and_the_tool_does_not_run(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))

    outcome = await tools.run(ctx, call("echo", '{"text": 5}'), run)

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"
    assert outcome.text == (
        "The arguments do not fit the tool echo:\n- text: Input should be a valid string"
    )
    assert journal.ran == []


async def test_a_failure_that_the_tool_reports_is_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("fail", journal.fail))

    outcome = await tools.run(ctx, call("fail", '{"text": "a.txt"}'), run)

    assert outcome.error == ToolError(code="NOT_FOUND", reason="There is no file a.txt.")
    assert outcome.text == "There is no file a.txt."


async def test_a_tool_that_crashes_is_an_error_result_and_is_logged(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    tools.add(ctx, tool("crash", journal.crash))

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("crash"), run)

    assert outcome.error == ToolError(
        code="TOOL_ERROR", reason="The tool crash failed: RuntimeError: boom"
    )
    assert outcome.text == "The tool crash failed: RuntimeError: boom"
    assert "the tool 'crash' failed" in caplog.text
    assert "RuntimeError: boom" in caplog.text


async def test_a_rendering_that_crashes_is_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def render(args: EchoArgs, value: str) -> str:
        raise KeyError("missing")

    tools.add(ctx, tool("echo", journal.echo, render=render))

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="TOOL_ERROR", reason="The tool echo failed: KeyError: 'missing'"
    )


# Time limits and cancellation


async def test_a_tool_that_takes_too_long_is_stopped(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("hang", journal.hang, timeout_seconds=0.05))

    outcome = await asyncio.wait_for(tools.run(ctx, call("hang"), run), timeout=2)

    assert outcome.error == ToolError(
        code="TOOL_TIMEOUT",
        reason="The tool hang did not finish within 0.05 seconds and was stopped.",
    )
    assert journal.cleaned_up


async def test_the_service_sets_the_limit_for_tools_that_have_none(
    ctx: Context, run: ToolRun, journal: Journal
) -> None:
    await ctx.plugin(columbina.tools, {"timeout_seconds": 0.05})
    tools = ctx.use(TOOLS)
    tools.add(ctx, tool("hang", journal.hang), tool("patient", journal.hang, timeout_seconds=30))

    outcome = await asyncio.wait_for(tools.run(ctx, call("hang"), run), timeout=2)
    patient = asyncio.ensure_future(tools.run(ctx, call("patient"), run))
    await asyncio.sleep(0.15)

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_TIMEOUT"
    assert not patient.done()
    patient.cancel()
    with pytest.raises(asyncio.CancelledError):
        await patient


async def test_without_any_limit_a_tool_may_take_its_time(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("hang", journal.hang))
    assert tools.timeout_seconds is None

    running = asyncio.ensure_future(tools.run(ctx, call("hang"), run))
    await asyncio.wait_for(journal.started.wait(), timeout=1)
    await asyncio.sleep(0.05)

    assert not running.done()
    running.cancel()
    with pytest.raises(asyncio.CancelledError):
        await running


@pytest.mark.parametrize("seconds", [0, -1])
async def test_the_limit_of_the_service_must_be_positive(ctx: Context, seconds: float) -> None:
    with pytest.raises(ValidationError, match="timeout_seconds"):
        await ctx.plugin(columbina.tools, {"timeout_seconds": seconds})


async def test_a_cleanup_that_fails_after_the_limit_is_logged(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    tools.add(ctx, tool("hang", journal.hang_and_fail_to_clean_up, timeout_seconds=0.05))

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await asyncio.wait_for(tools.run(ctx, call("hang"), run), timeout=2)

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_TIMEOUT"
    assert "cleanup failed" in caplog.text


async def test_cancelling_the_caller_stops_the_tool_and_leaves_no_outcome(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("hang", journal.hang))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)
    pending = tools.begin(ctx, call("hang"), run)

    running = asyncio.ensure_future(pending.execute())
    await asyncio.wait_for(journal.started.wait(), timeout=1)
    running.cancel()
    with pytest.raises(asyncio.CancelledError):
        await running

    assert journal.cleaned_up
    assert pending.outcome is None
    assert seen == []


async def test_a_cleanup_that_fails_does_not_swallow_a_cancellation(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    tools.add(ctx, tool("hang", journal.hang_and_fail_to_clean_up))
    pending = tools.begin(ctx, call("hang"), run)

    running = asyncio.ensure_future(pending.execute())
    await asyncio.wait_for(journal.started.wait(), timeout=1)
    running.cancel()
    with (
        caplog.at_level(logging.ERROR, logger="columbina.tools"),
        pytest.raises(asyncio.CancelledError),
    ):
        await asyncio.wait_for(running, timeout=1)

    assert pending.outcome is None
    assert "cleanup failed" in caplog.text


async def test_a_tool_that_cancels_itself_is_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("selfish", journal.cancel_itself))

    outcome = await asyncio.wait_for(tools.run(ctx, call("selfish"), run), timeout=1)

    assert outcome.error == ToolError(
        code="TOOL_ERROR", reason="The tool selfish was cancelled from inside."
    )


# Before the tool runs


async def test_a_listener_can_refuse_a_call_before_it_runs(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    seen: list[Invocation] = []

    async def no_secrets(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        seen.append(invocation)
        if "secret" in invocation.call.arguments:
            return Deny(reason="secrets stay where they are")
        return await proceed(invocation)

    registered = tool("echo", journal.echo)
    tools.add(ctx, registered)
    ctx.on(PRE_EXECUTE, no_secrets)
    asked = call("echo", '{"text": "the secret"}')

    refused = await tools.run(ctx, asked, run)
    allowed = await tools.run(ctx, call("echo", '{"text": "hello"}', call_id="call-2"), run)

    assert refused.error == ToolError(
        code="DENIED", reason="The call was not allowed: secrets stay where they are"
    )
    assert refused.text == "The call was not allowed: secrets stay where they are"
    assert allowed.text == "hello"
    assert journal.ran == ["hello"]
    assert seen[0].tool is registered
    assert seen[0].call is asked
    assert seen[0].args == EchoArgs(text="the secret")
    # The run that a listener sees tells what the caller told, and what the call acts on.
    told = seen[0].run
    assert (told.call_id, told.cwd, told.session) == (run.call_id, run.cwd, run.session)


async def test_a_refusal_can_carry_its_own_code(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def never(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny(reason="nobody is here to approve", code="APPROVAL_UNAVAILABLE")

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, never)

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error is not None
    assert outcome.error.code == "APPROVAL_UNAVAILABLE"


async def test_a_check_that_crashes_keeps_the_tool_from_running(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    async def broken(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        raise RuntimeError("policy bug")

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, broken)

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="CHECK_FAILED",
        reason="The call could not be checked and did not run: RuntimeError: policy bug",
    )
    assert journal.ran == []
    assert "policy bug" in caplog.text


async def test_a_call_is_checked_once_however_often_that_is_asked_for(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    checks: list[str] = []

    async def watch(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        checks.append(invocation.call.call_id)
        return await proceed(invocation)

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, watch)
    pending = tools.begin(ctx, call("echo"), run)

    await pending.authorize()
    await pending.authorize()
    await pending.execute()

    assert checks == ["call-1"]


async def test_running_a_call_checks_it_first_if_nobody_did(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def never(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny(reason="no")

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, never)

    outcome = await tools.begin(ctx, call("echo"), run).execute()

    assert outcome.error is not None
    assert outcome.error.code == "DENIED"
    assert journal.ran == []


async def test_a_listener_in_the_scope_of_another_agent_is_not_asked(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def never(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny(reason="not for this agent")

    tools.add(ctx, tool("echo", journal.echo))
    here = ctx.scope("agent-a")
    ctx.scope("agent-b").on(PRE_EXECUTE, never)

    outcome = await tools.run(here, call("echo"), run)

    assert outcome.error is None


# Around the tool


async def test_a_listener_can_wrap_the_tool(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    order: list[str] = []

    async def shout(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        order.append("before")
        value = await proceed(invocation)
        order.append("after")
        return str(value).upper()

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(EXECUTE, shout)

    outcome = await tools.run(ctx, call("echo", '{"text": "hello"}'), run)

    assert outcome.text == "HELLO"
    assert outcome.value == "HELLO"
    assert order == ["before", "after"]


async def test_the_limit_covers_what_is_wrapped_around_the_tool(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def stall(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        await asyncio.Event().wait()
        return await proceed(invocation)

    tools.add(ctx, tool("echo", journal.echo, timeout_seconds=0.05))
    ctx.on(EXECUTE, stall)

    outcome = await asyncio.wait_for(tools.run(ctx, call("echo"), run), timeout=2)

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_TIMEOUT"


# After the tool has run


async def test_a_listener_can_replace_what_the_model_reads(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    seen: list[Executed] = []

    async def shorten(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        seen.append(executed)
        return Accept(text=executed.text[:3] + "...")

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(POST_EXECUTE, shorten)

    outcome = await tools.run(ctx, call("echo", '{"text": "hello"}'), run)

    assert outcome.text == "hel..."
    assert outcome.value == "hello"
    assert seen[0].text == "hello"
    assert seen[0].value == "hello"
    assert seen[0].invocation.call.name == "echo"


async def test_a_listener_can_withhold_a_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def censor(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        if "password" in executed.text:
            return Block(reason="it holds a password")
        return await proceed(executed)

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(POST_EXECUTE, censor)

    withheld = await tools.run(ctx, call("echo", '{"text": "password: 123"}'), run)
    passed = await tools.run(ctx, call("echo", '{"text": "hello"}'), run)

    assert withheld.error == ToolError(
        code="RESULT_BLOCKED", reason="The result of the call is withheld: it holds a password"
    )
    assert withheld.text == "The result of the call is withheld: it holds a password"
    assert withheld.value is None
    assert passed.text == "hello"
    assert journal.ran == ["password: 123", "hello"]


async def test_a_check_of_the_result_that_crashes_withholds_the_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    async def broken(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        raise RuntimeError("filter bug")

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(POST_EXECUTE, broken)

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="CHECK_FAILED",
        reason="The result of the call could not be checked and is withheld: "
        "RuntimeError: filter bug",
    )
    assert "filter bug" in caplog.text


async def test_a_result_is_not_checked_when_the_tool_did_not_succeed(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    checked: list[Executed] = []

    async def watch(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        checked.append(executed)
        return await proceed(executed)

    tools.add(ctx, tool("fail", journal.fail))
    ctx.on(POST_EXECUTE, watch)

    await tools.run(ctx, call("fail"), run)
    await tools.run(ctx, call("nowhere"), run)

    assert checked == []


# The final outcome


async def test_every_call_is_announced_once_with_its_final_outcome(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def shorten(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return Accept(text=executed.text[:3])

    async def no_secrets(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        if "secret" in invocation.call.arguments:
            return Deny(reason="no")
        return await proceed(invocation)

    tools.add(ctx, tool("echo", journal.echo), tool("fail", journal.fail))
    ctx.on(PRE_EXECUTE, no_secrets)
    ctx.on(POST_EXECUTE, shorten)
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    outcomes = [
        await tools.run(ctx, call("echo", '{"text": "hello"}', call_id="a"), run),
        await tools.run(ctx, call("echo", '{"text": "secret"}', call_id="b"), run),
        await tools.run(ctx, call("echo", "{", call_id="c"), run),
        await tools.run(ctx, call("nowhere", call_id="d"), run),
        await tools.run(ctx, call("fail", call_id="e"), run),
    ]

    assert seen == outcomes
    assert [outcome.text for outcome in seen][:1] == ["hel"]
    assert [outcome.error.code if outcome.error else None for outcome in seen] == [
        None,
        "DENIED",
        "INVALID_ARGS",
        "UNKNOWN_TOOL",
        "NOT_FOUND",
    ]


async def test_a_finished_call_keeps_its_outcome(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))
    pending = tools.begin(ctx, call("echo", '{"text": "hello"}'), run)
    assert pending.outcome is None

    first = await pending.execute()
    second = await pending.execute()

    assert pending.outcome is first
    assert second is first
    assert journal.ran == ["hello"]


async def test_a_call_that_cannot_run_has_its_outcome_at_once(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    pending = tools.begin(ctx, call("nowhere"), run)

    assert pending.outcome is not None
    assert pending.outcome.error is not None
    assert pending.outcome.error.code == "UNKNOWN_TOOL"
    assert seen == [pending.outcome]
    await pending.authorize()
    assert await pending.execute() is pending.outcome
    assert seen == [pending.outcome]


async def test_a_call_can_be_ended_from_outside_before_it_runs(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)
    pending = tools.begin(ctx, call("echo"), run)

    outcome = pending.fail(
        "ABORTED_BEFORE_DISPATCH", "The turn was cancelled before this call ran."
    )

    assert outcome.error == ToolError(
        code="ABORTED_BEFORE_DISPATCH", reason="The turn was cancelled before this call ran."
    )
    assert seen == [outcome]
    assert await pending.execute() is outcome
    assert journal.ran == []
    with pytest.raises(RuntimeError, match="the call 'call-1' has an outcome already"):
        pending.fail("ABORTED_BEFORE_DISPATCH", "again")


# Guards


def refusing(reason: str, seen: list[str] | None = None) -> Any:
    """A guard that refuses every call, and writes down that it was asked."""

    async def guard(ctx: Context, invocation: Invocation) -> Deny | None:
        if seen is not None:
            seen.append(reason)
        return Deny(reason=reason, code="NOT_TODAY")

    return guard


def content(seen: list[str], name: str) -> Any:
    """A guard that has no objection, and writes down that it was asked."""

    async def guard(ctx: Context, invocation: Invocation) -> Deny | None:
        seen.append(name)
        return None

    return guard


async def test_a_guard_can_refuse_a_call(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "closed", refusing("the shop is closed"))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="NOT_TODAY", reason="The call was not allowed: the shop is closed"
    )
    assert journal.ran == []
    assert seen == [outcome]


async def test_a_guard_is_asked_even_if_a_listener_let_the_call_through_by_itself(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def generous(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        # Answers at once, so that no listener after it is asked.
        return Allow()

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, generous)
    tools.guards.add(ctx, "closed", refusing("the shop is closed"))

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error is not None
    assert outcome.error.code == "NOT_TODAY"
    assert journal.ran == []


async def test_guards_are_asked_in_the_order_they_were_added_until_one_refuses(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    asked: list[str] = []
    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "first", content(asked, "first"))
    tools.guards.add(ctx, "second", refusing("second says no", asked))
    tools.guards.add(ctx, "third", content(asked, "third"))

    outcome = await tools.run(ctx, call("echo"), run)

    assert asked == ["first", "second says no"]
    assert outcome.error is not None
    assert outcome.error.reason == "The call was not allowed: second says no"


async def test_a_call_runs_when_no_guard_refuses(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    asked: list[str] = []
    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "first", content(asked, "first"))
    tools.guards.add(ctx, "second", content(asked, "second"))

    outcome = await tools.run(ctx, call("echo", '{"text": "hello"}'), run)

    assert asked == ["first", "second"]
    assert outcome.error is None
    assert outcome.text == "hello"


async def test_guards_are_not_asked_about_a_call_that_a_listener_refused(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def never(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        return Deny(reason="not today")

    asked: list[str] = []
    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, never)
    tools.guards.add(ctx, "first", content(asked, "first"))

    outcome = await tools.run(ctx, call("echo"), run)

    assert asked == []
    assert outcome.error is not None
    assert outcome.error.code == "DENIED"


async def test_a_guard_is_given_the_context_that_the_call_was_begun_with(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    given: list[Context] = []

    async def guard(context: Context, invocation: Invocation) -> Deny | None:
        given.append(context)
        return None

    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "watching", guard)
    scope = ctx.scope("one agent")

    await tools.run(scope, call("echo"), run)

    assert given == [scope]


@pytest.mark.parametrize("fault", ["raises", "answers yes", "cancels itself", "is no coroutine"])
async def test_a_guard_that_fails_keeps_the_tool_from_running(
    ctx: Context,
    tools: Tools,
    run: ToolRun,
    journal: Journal,
    caplog: pytest.LogCaptureFixture,
    fault: str,
) -> None:
    async def raising(context: Context, invocation: Invocation) -> Deny | None:
        raise RuntimeError("guard bug")

    async def approving(context: Context, invocation: Invocation) -> Any:
        return Allow()

    async def cancelling(context: Context, invocation: Invocation) -> Deny | None:
        raise asyncio.CancelledError

    def plain(context: Context, invocation: Invocation) -> Any:
        raise RuntimeError("guard bug")

    guards = {
        "raises": raising,
        "answers yes": approving,
        "cancels itself": cancelling,
        "is no coroutine": plain,
    }
    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "broken", guards[fault])

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await asyncio.wait_for(tools.run(ctx, call("echo"), run), timeout=1)

    assert outcome.error is not None
    assert outcome.error.code == "CHECK_FAILED"
    assert outcome.error.reason.startswith("The call could not be checked and did not run")
    assert journal.ran == []
    assert "the guard 'broken'" in caplog.text


async def test_a_call_can_be_cancelled_while_a_guard_decides(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    asked = asyncio.Event()

    async def pondering(context: Context, invocation: Invocation) -> Deny | None:
        asked.set()
        await asyncio.Event().wait()
        return None

    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "slow", pondering)
    running = asyncio.ensure_future(tools.run(ctx, call("echo"), run))
    await asyncio.wait_for(asked.wait(), timeout=1)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=1)
    assert journal.ran == []


async def test_a_guard_leaves_with_the_plugin_that_added_it(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(TOOLS).guards.add(inner, "closed", refusing("the shop is closed"))

    tools.add(ctx, tool("echo", journal.echo))
    fiber = await ctx.plugin(Plugin("shop", apply, inject=(TOOLS,)))
    assert list(tools.guards) == ["closed"]

    await fiber.dispose()

    assert list(tools.guards) == []
    assert (await tools.run(ctx, call("echo"), run)).error is None


# What a call acts on


async def test_what_a_call_acts_on_is_worked_out_once_and_shown_to_the_checks(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    worked_out: list[str] = []

    def target(args: EchoArgs, run: ToolRun) -> str:
        worked_out.append(args.text)
        return f"{run.cwd.name}/{args.text}"

    seen: list[tuple[str, str | None]] = []

    async def listener(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        seen.append(("listener", invocation.target))
        return await proceed(invocation)

    async def guard(context: Context, invocation: Invocation) -> Deny | None:
        seen.append(("guard", invocation.target))
        return None

    tools.add(ctx, tool("echo", journal.echo, effect="write", target=target))
    tools.add(ctx, tool("plain", journal.echo))
    ctx.on(PRE_EXECUTE, listener)
    tools.guards.add(ctx, "watching", guard)

    await tools.run(ctx, call("echo", '{"text": "a.txt"}'), run)
    await tools.run(ctx, call("plain"), run)

    acted_on = f"{run.cwd.name}/a.txt"
    assert worked_out == ["a.txt"]
    assert seen == [
        ("listener", acted_on),
        ("guard", acted_on),
        ("listener", None),
        ("guard", None),
    ]


async def test_a_tool_that_cannot_say_what_a_call_acts_on_does_not_run(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    def target(args: EchoArgs, run: ToolRun) -> str:
        raise RuntimeError("target bug")

    tools.add(ctx, tool("echo", journal.echo, target=target))

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="TOOL_ERROR",
        reason="The tool echo could not say what the call acts on: RuntimeError: target bug",
    )
    assert journal.ran == []
    assert "target bug" in caplog.text


async def test_a_tool_can_refuse_a_call_when_it_says_what_the_call_acts_on(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def target(args: EchoArgs, run: ToolRun) -> str:
        raise ToolFailure("This is no path that is used here.", code="INVALID_PATH")

    asked: list[str] = []
    tools.add(ctx, tool("echo", journal.echo, target=target))
    tools.guards.add(ctx, "watching", content(asked, "watching"))

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="INVALID_PATH", reason="This is no path that is used here."
    )
    assert asked == []
    assert journal.ran == []


# What runs is what was checked


async def test_a_listener_cannot_put_another_call_in_the_place_of_the_one_that_was_checked(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, caplog: pytest.LogCaptureFixture
) -> None:
    async def swapping(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        return await proceed(dataclasses.replace(invocation, args=EchoArgs(text="another")))

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(EXECUTE, swapping)

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("echo", '{"text": "as checked"}'), run)

    assert outcome.error == ToolError(
        code="CHECK_FAILED",
        reason="The call did not run: a listener of tools/execute passed on another call "
        "than the one that was checked",
    )
    assert journal.ran == []
    assert "tools/execute" in caplog.text


async def test_a_listener_that_passes_on_the_call_it_was_given_lets_it_run(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def timing(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        return await proceed(invocation)

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(EXECUTE, timing)

    outcome = await tools.run(ctx, call("echo", '{"text": "as checked"}'), run)

    assert outcome.error is None
    assert journal.ran == ["as checked"]


class ListArgs(BaseModel):
    items: list[str] = []


@pytest.mark.parametrize("stage", ["before the checks", "after the checks"])
async def test_arguments_that_a_listener_changed_in_place_keep_the_tool_from_running(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal, stage: str
) -> None:
    async def before(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        cast("EchoArgs", invocation.args).text = "changed"
        return await proceed(invocation)

    async def after(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        cast("EchoArgs", invocation.args).text = "changed"
        return await proceed(invocation)

    asked: list[str] = []
    tools.add(ctx, tool("echo", journal.echo))
    tools.guards.add(ctx, "watching", content(asked, "watching"))
    if stage == "before the checks":
        ctx.on(PRE_EXECUTE, before)
    else:
        ctx.on(EXECUTE, after)

    outcome = await tools.run(ctx, call("echo", '{"text": "as written"}'), run)

    assert outcome.error is not None
    assert outcome.error.code == "CHECK_FAILED"
    assert "changed the arguments" in outcome.error.reason
    assert journal.ran == []
    # A guard is not asked about arguments that are no longer those of the model.
    assert asked == ([] if stage == "before the checks" else ["watching"])


async def test_a_change_inside_an_argument_is_a_change_of_the_arguments(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    ran: list[list[str]] = []

    async def take(args: ListArgs, run: ToolRun) -> str:
        ran.append(args.items)
        return "taken"

    async def adding(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        cast("ListArgs", invocation.args).items.append("smuggled")
        return await proceed(invocation)

    tools.add(ctx, Tool(name="take", description="Take.", parameters=ListArgs, execute=take))
    ctx.on(EXECUTE, adding)

    outcome = await tools.run(ctx, call("take", '{"items": ["one"]}'), run)

    assert outcome.error is not None
    assert outcome.error.code == "CHECK_FAILED"
    assert ran == []


async def test_a_tool_that_leaves_while_a_listener_holds_the_call_does_not_run(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    fiber = await ctx.plugin(lending("echo", journal))

    async def unloading(invocation: Invocation, proceed: Next[Invocation, object]) -> object:
        await fiber.dispose()
        return await proceed(invocation)

    ctx.on(EXECUTE, unloading)

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error is not None
    assert outcome.error.code == "UNKNOWN_TOOL"
    assert journal.ran == []


async def test_a_tool_is_told_what_the_checks_were_told_that_the_call_acts_on(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    told: list[str | None] = []

    async def noting(args: EchoArgs, run: ToolRun) -> str:
        told.append(run.target)
        return "noted"

    def target(args: EchoArgs, run: ToolRun) -> str:
        return f"the place of {args.text}"

    tools.add(ctx, tool("aimed", noting, target=target))
    tools.add(ctx, tool("plain", noting))

    await tools.run(ctx, call("aimed", '{"text": "a.txt"}'), run)
    await tools.run(ctx, call("plain"), run)

    assert told == ["the place of a.txt", None]


def lending(name: str, journal: Journal) -> Plugin[object]:
    """A plugin that brings one tool, and takes it away when it leaves."""

    def apply(inner: Context, config: object) -> None:
        inner.use(TOOLS).add(inner, tool(name, journal.echo))

    return Plugin(f"lends-{name}", apply, inject=(TOOLS,))


async def test_a_call_whose_tool_left_before_the_checks_does_not_run_and_is_not_checked(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    asked: list[str] = []
    fiber = await ctx.plugin(lending("echo", journal))
    tools.guards.add(ctx, "watching", content(asked, "watching"))
    pending = tools.begin(ctx, call("echo"), run)

    await fiber.dispose()
    outcome = await pending.execute()

    assert outcome.error == ToolError(
        code="UNKNOWN_TOOL", reason="The tool echo is no longer there, and the call did not run."
    )
    assert asked == []
    assert journal.ran == []


async def test_a_call_whose_tool_left_after_the_checks_does_not_run(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    fiber = await ctx.plugin(lending("echo", journal))
    pending = tools.begin(ctx, call("echo"), run)
    await pending.authorize()
    assert pending.outcome is None

    await fiber.dispose()
    outcome = await pending.execute()

    assert outcome.error is not None
    assert outcome.error.code == "UNKNOWN_TOOL"
    assert journal.ran == []


async def test_a_tool_that_took_the_name_of_the_one_that_left_is_not_run_in_its_place(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    other = Journal()
    asked: list[str] = []
    fiber = await ctx.plugin(lending("echo", journal))
    tools.guards.add(ctx, "watching", content(asked, "watching"))
    pending = tools.begin(ctx, call("echo"), run)

    await fiber.dispose()
    await ctx.plugin(lending("echo", other))
    outcome = await pending.execute()

    assert outcome.error is not None
    assert outcome.error.code == "UNKNOWN_TOOL"
    assert journal.ran == []
    assert other.ran == []
    # The checks are not asked about a call of a tool that is no longer the one it was for.
    assert asked == []


# Running calls beside each other


async def test_a_pending_call_says_whether_it_may_run_beside_others(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def only_short(args: EchoArgs) -> bool:
        return len(args.text) < 5

    def broken(args: EchoArgs) -> bool:
        raise RuntimeError("classifier bug")

    tools.add(
        ctx,
        tool("exclusive", journal.echo),
        tool("shared", journal.echo, concurrency_safe=True),
        tool("choosy", journal.echo, concurrency_safe=only_short),
        tool("broken", journal.echo, concurrency_safe=broken),
    )

    def safe(name: str, arguments: str = "{}") -> bool:
        return tools.begin(ctx, call(name, arguments), run).concurrency_safe

    assert not safe("exclusive")
    assert safe("shared")
    assert safe("choosy", '{"text": "hi"}')
    assert not safe("choosy", '{"text": "a long text"}')
    assert not safe("broken")
    assert safe("nowhere")


# The service


async def test_the_tools_are_listed_for_the_model_in_the_order_they_were_added(
    ctx: Context, tools: Tools, journal: Journal
) -> None:
    echo = tool("echo", journal.echo)
    count = tool("count", journal.count)

    tools.add(ctx, echo, count)

    assert tools.schemas() == (echo.schema, count.schema)
    assert list(tools.registry) == ["echo", "count"]
    assert tools.registry["echo"] is echo


async def test_a_tool_name_can_be_taken_only_once(
    ctx: Context, tools: Tools, journal: Journal
) -> None:
    tools.add(ctx, tool("echo", journal.echo))

    with pytest.raises(RegistryConflictError, match="the tool 'echo' is already registered"):
        tools.add(ctx, tool("echo", journal.count))


async def test_a_tool_leaves_with_the_plugin_that_brought_it(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(TOOLS).add(inner, tool("echo", journal.echo))

    fiber = await ctx.plugin(Plugin("echo-tool", apply, inject=(TOOLS,)))
    assert tools.schemas() != ()

    await fiber.dispose()

    assert tools.schemas() == ()
    outcome = await tools.run(ctx, call("echo"), run)
    assert outcome.error is not None
    assert outcome.error.code == "UNKNOWN_TOOL"


async def test_allow_is_what_a_check_answers_when_it_has_no_objection() -> None:
    assert Allow() == Allow()
    assert Deny(reason="no").code == "DENIED"


# Faults of the code around a call


async def test_parameters_whose_validation_crashes_are_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, caplog: pytest.LogCaptureFixture
) -> None:
    class Fussy(BaseModel):
        text: str

        @field_validator("text")
        @classmethod
        def broken(cls, value: str) -> str:
            raise TypeError("validator bug")

    async def work(args: Fussy, run: ToolRun) -> str:
        return "never"

    tools.add(ctx, Tool(name="fussy", description="Fuss.", parameters=Fussy, execute=work))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("fussy", '{"text": "hi"}'), run)

    assert outcome.error == ToolError(
        code="TOOL_ERROR",
        reason="The arguments of the tool fussy could not be checked: TypeError: validator bug",
    )
    assert seen == [outcome]
    assert "validator bug" in caplog.text


async def test_parameters_whose_validation_raises_a_cancellation_are_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, caplog: pytest.LogCaptureFixture
) -> None:
    class Fussy(BaseModel):
        text: str

        @field_validator("text")
        @classmethod
        def broken(cls, value: str) -> str:
            raise asyncio.CancelledError("validator bug")

    async def work(args: Fussy, run: ToolRun) -> str:
        return "never"

    tools.add(ctx, Tool(name="fussy", description="Fuss.", parameters=Fussy, execute=work))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await asyncio.wait_for(
            tools.run(ctx, call("fussy", '{"text": "hi"}'), run), timeout=1
        )

    assert outcome.error == ToolError(
        code="TOOL_ERROR",
        reason="The arguments of the tool fussy could not be checked: "
        "CancelledError: validator bug",
    )
    assert seen == [outcome]
    assert "validator bug" in caplog.text


async def test_a_tool_that_raises_a_cancellation_when_asked_whether_it_may_overlap_runs_alone(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def broken(args: EchoArgs) -> bool:
        raise asyncio.CancelledError

    tools.add(ctx, tool("broken", journal.echo, concurrency_safe=broken))

    pending = tools.begin(ctx, call("broken", '{"text": "hi"}'), run)

    assert not pending.concurrency_safe
    assert (await pending.execute()).text == "hi"


async def test_a_tool_that_keeps_its_time_limit_to_itself_still_runs_out_of_time(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    async def stubborn(args: EchoArgs, run: ToolRun) -> str:
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.Event().wait()
        return "finished anyway"

    tools.add(
        ctx,
        Tool(
            name="stubborn",
            description="Wait.",
            parameters=EchoArgs,
            execute=stubborn,
            timeout_seconds=0.02,
        ),
    )

    outcome = await asyncio.wait_for(tools.run(ctx, call("stubborn"), run), timeout=1)

    assert outcome.error == ToolError(
        code="TOOL_TIMEOUT",
        reason="The tool stubborn did not finish within 0.02 seconds and was stopped.",
    )


async def test_a_tool_that_keeps_a_cancellation_to_itself_does_not_keep_its_caller_going(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    began = asyncio.Event()

    async def stubborn(args: EchoArgs, run: ToolRun) -> str:
        began.set()
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.Event().wait()
        return "finished anyway"

    tools.add(
        ctx, Tool(name="stubborn", description="Wait.", parameters=EchoArgs, execute=stubborn)
    )
    running = asyncio.ensure_future(tools.run(ctx, call("stubborn"), run))
    await asyncio.wait_for(began.wait(), timeout=1)

    running.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(running, timeout=1)


async def test_a_rendering_that_raises_a_cancellation_is_an_error_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    def render(args: EchoArgs, value: str) -> str:
        raise asyncio.CancelledError

    tools.add(ctx, tool("echo", journal.echo, render=render))

    outcome = await asyncio.wait_for(tools.run(ctx, call("echo"), run), timeout=1)

    assert outcome.error == ToolError(
        code="TOOL_ERROR", reason="The tool echo was cancelled from inside."
    )


async def test_a_check_that_gives_no_answer_keeps_the_tool_from_running(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def forgetful(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        await proceed(invocation)
        return None  # type: ignore[return-value]

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(PRE_EXECUTE, forgetful)

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="CHECK_FAILED",
        reason="The call could not be checked and did not run: a check gave no decision",
    )
    assert journal.ran == []


async def test_a_check_of_the_result_that_gives_no_answer_withholds_the_result(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    async def forgetful(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return None  # type: ignore[return-value]

    tools.add(ctx, tool("echo", journal.echo))
    ctx.on(POST_EXECUTE, forgetful)
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)

    outcome = await tools.run(ctx, call("echo"), run)

    assert outcome.error == ToolError(
        code="CHECK_FAILED",
        reason="The result of the call could not be checked and is withheld: "
        "a check gave no verdict",
    )
    assert seen == [outcome]


async def test_a_tool_that_goes_on_after_it_was_given_up_does_not_change_the_outcome(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    release = asyncio.Event()
    began = asyncio.Event()

    async def stubborn(args: EchoArgs, run: ToolRun) -> str:
        began.set()
        await release.wait()
        return "late"

    tools.add(ctx, tool("stubborn", stubborn))
    seen: list[ToolOutcome] = []
    ctx.on(RESULT, seen.append)
    pending = tools.begin(ctx, call("stubborn"), run)
    running = asyncio.ensure_future(pending.execute())
    await asyncio.wait_for(began.wait(), timeout=1)

    given_up = pending.fail("TOOL_CANCELLED", "The call was cancelled while it ran.")
    release.set()
    late = await asyncio.wait_for(running, timeout=1)

    assert late is given_up
    assert pending.outcome is given_up
    assert seen == [given_up]


async def test_a_cancellation_that_is_on_its_way_stops_a_call_before_its_tool_runs(
    ctx: Context, tools: Tools, run: ToolRun, journal: Journal
) -> None:
    tools.add(ctx, tool("hang", journal.hang))

    async def work() -> ToolOutcome:
        cancellation.current_task().cancel()
        return await tools.run(ctx, call("hang"), run)

    task = asyncio.ensure_future(work())
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)

    assert not journal.started.is_set()


# Arguments that a tool keeps something beside


class Keeping(BaseModel):
    """Arguments beside which a tool keeps something that cannot be copied."""

    name: str = ""
    _lock: Any = PrivateAttr(default_factory=threading.Lock)


class Unsayable(BaseModel):
    """Arguments whose values cannot be written down."""

    name: str = ""

    @model_serializer
    def refuse(self) -> dict[str, str]:
        raise RuntimeError("these arguments are not for writing down")


async def test_what_a_tool_keeps_beside_its_arguments_does_not_keep_a_call_from_running(
    ctx: Context, tools: Tools, run: ToolRun
) -> None:
    async def greet(args: Keeping, run: ToolRun) -> str:
        return f"Hello, {args.name}."

    tools.add(ctx, Tool(name="greet", description="Greet.", parameters=Keeping, execute=greet))

    outcome = await tools.run(ctx, call("greet", '{"name": "you"}'), run)

    assert outcome.error is None
    assert outcome.text == "Hello, you."


async def test_arguments_that_cannot_be_put_aside_are_an_error_of_the_call(
    ctx: Context, tools: Tools, run: ToolRun, caplog: pytest.LogCaptureFixture
) -> None:
    ran: list[str] = []

    async def greet(args: Unsayable, run: ToolRun) -> str:
        ran.append(args.name)
        return "Hello."

    tools.add(ctx, Tool(name="greet", description="Greet.", parameters=Unsayable, execute=greet))

    with caplog.at_level(logging.ERROR, logger="columbina.tools"):
        outcome = await tools.run(ctx, call("greet", '{"name": "you"}'), run)

    assert outcome.error is not None
    assert outcome.error.code == "TOOL_ERROR"
    assert outcome.text.startswith(
        "The arguments of the tool greet could not be put aside for the checks: "
    )
    assert "these arguments are not for writing down" in outcome.text
    assert ran == []
    assert "could not be put aside" in caplog.text
