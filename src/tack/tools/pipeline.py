"""The tool pipeline: what happens between a call of the model and its result."""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from typing import Any

from pydantic import BaseModel

from tack import cancellation
from tack.kernel import Context, EmitEvent, Registry, ServiceKey, WaterfallEvent
from tack.llm.types import TextBlock, ToolCallBlock, ToolMessage, ToolSchema
from tack.session.events import ToolError, ToolResultEvent
from tack.tools.tool import Tool, ToolFailure, ToolRun

logger = logging.getLogger("tack.tools")


@dataclass(frozen=True, slots=True)
class Invocation:
    """A call that is on its way to a tool.

    Attributes:
        tool: The tool that was called.
        call: The call as the model wrote it. It is never rewritten: the log,
            the frontends and the tool must all see the same arguments.
        args: The arguments, validated against the parameters of the tool.
        run: What the tool is told about the call.
    """

    tool: Tool[Any, Any]
    call: ToolCallBlock
    args: BaseModel
    run: ToolRun


@dataclass(frozen=True, slots=True)
class Allow:
    """The answer of a check that has no objection to a call."""


@dataclass(frozen=True, slots=True)
class Deny:
    """The answer of a check that refuses a call.

    Attributes:
        reason: Why, in words that the model reads.
        code: The code of the error result.
    """

    reason: str
    code: str = "DENIED"


Decision = Allow | Deny
"""What a listener of :data:`PRE_EXECUTE` answers."""


@dataclass(frozen=True, slots=True)
class Executed:
    """A call that a tool has carried out.

    Attributes:
        invocation: The call.
        value: What the tool returned.
        text: What the model would read.
    """

    invocation: Invocation
    value: object
    text: str


@dataclass(frozen=True, slots=True)
class Accept:
    """The answer of a check that lets a result through, with the text that the model reads."""

    text: str


@dataclass(frozen=True, slots=True)
class Block:
    """The answer of a check that withholds a result. The model reads the reason instead."""

    reason: str


Verdict = Accept | Block
"""What a listener of :data:`POST_EXECUTE` answers."""


@dataclass(frozen=True, slots=True)
class ToolOutcome:
    """How a call ended.

    Attributes:
        call: The call as the model wrote it.
        text: What the model reads.
        error: Set when the call did not succeed.
        value: What the tool returned, if it ran and its result got through.
        concludes_turn: Whether the tool asked for the turn to end.
    """

    call: ToolCallBlock
    text: str
    error: ToolError | None = None
    value: object = None
    concludes_turn: bool = False

    def to_event(self) -> ToolResultEvent:
        """Return the event that records the outcome in a session log."""
        message = ToolMessage(
            call_id=self.call.call_id,
            content=(TextBlock(text=self.text),),
            is_error=self.error is not None,
        )
        return ToolResultEvent(message=message, error=self.error)


PRE_EXECUTE: WaterfallEvent[Invocation, Decision] = WaterfallEvent("tools/pre-execute")
"""Asked before a tool runs. A listener answers :class:`Deny` to refuse the call."""

EXECUTE: WaterfallEvent[Invocation, object] = WaterfallEvent("tools/execute")
"""Wraps the run of a tool. The innermost step is the tool itself."""

POST_EXECUTE: WaterfallEvent[Executed, Verdict] = WaterfallEvent("tools/post-execute")
"""Asked after a tool ran. A listener may change what the model reads, or withhold it."""

RESULT: EmitEvent[ToolOutcome] = EmitEvent("tools/result")
"""Emitted once for every call, with its final outcome."""


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


async def _allow(invocation: Invocation) -> Decision:
    return Allow()


async def _run_tool(invocation: Invocation) -> object:
    return await invocation.tool.execute(invocation.args, invocation.run)


async def _accept(executed: Executed) -> Verdict:
    return Accept(text=executed.text)


class PendingCall:
    """One call on its way through the pipeline.

    The pipeline has two steps, so that the caller can take the first for
    every call of a reply in the order of the model, and then let the second
    overlap for calls that are safe beside each other. :meth:`authorize` asks
    the checks. :meth:`execute` runs the tool and has its result checked.

    A call that cannot run has its outcome early: the tool is unknown, the
    arguments do not fit, or a check refused. Every outcome is an ordinary
    value. A call never raises, except for a cancellation of the caller, which
    stops the tool and leaves the call without an outcome.

    Attributes:
        call: The call as the model wrote it.
        concurrency_safe: Whether the call may run beside other calls of the
            same reply.
    """

    __slots__ = (
        "_authorized",
        "_ctx",
        "_invocation",
        "_outcome",
        "_timeout",
        "call",
        "concurrency_safe",
    )

    def __init__(
        self,
        ctx: Context,
        call: ToolCallBlock,
        invocation: Invocation | None,
        timeout: float | None,
        concurrency_safe: bool,
    ) -> None:
        self.call = call
        self.concurrency_safe = concurrency_safe
        self._ctx = ctx
        self._invocation = invocation
        self._timeout = timeout
        self._authorized = False
        self._outcome: ToolOutcome | None = None

    @property
    def outcome(self) -> ToolOutcome | None:
        """How the call ended, or ``None`` while that is open."""
        return self._outcome

    def fail(self, code: str, text: str) -> ToolOutcome:
        """End the call with an error result, without running the tool.

        Raises:
            RuntimeError: The call has an outcome already.
        """
        if self._outcome is not None:
            raise RuntimeError(f"the call {self.call.call_id!r} has an outcome already")
        return self._settle(
            ToolOutcome(call=self.call, text=text, error=ToolError(code=code, reason=text))
        )

    async def authorize(self) -> None:
        """Ask the checks whether the call may run. Asking again does nothing.

        A refusal ends the call with an error result. So does a check that
        fails: a call that cannot be checked does not run.
        """
        invocation = self._invocation
        if self._authorized or self._outcome is not None or invocation is None:
            return
        try:
            decision = await cancellation.guard(
                self._ctx.waterfall(PRE_EXECUTE, invocation, _allow)
            )
        except Exception as error:
            logger.error("a check of a call of the tool %r failed", self.call.name, exc_info=error)
            self.fail(
                "CHECK_FAILED",
                f"The call could not be checked and did not run: {_describe(error)}",
            )
            return
        if isinstance(decision, Deny):
            self.fail(decision.code, f"The call was not allowed: {decision.reason}")
            return
        self._authorized = True

    async def execute(self) -> ToolOutcome:
        """Run the tool and return how the call ended.

        The checks are asked first if nobody has. A call that has ended
        returns its outcome again.
        """
        await self.authorize()
        invocation = self._invocation
        if self._outcome is not None or invocation is None:
            assert self._outcome is not None
            return self._outcome
        tool = invocation.tool
        timer = asyncio.timeout(self._timeout)
        try:
            async with timer:
                value = await cancellation.guard(
                    self._ctx.waterfall(EXECUTE, invocation, _run_tool)
                )
            text = str(value) if tool.render is None else tool.render(invocation.args, value)
        except Exception as error:
            return self._failed(tool.name, error, timed_out=timer.expired())
        executed = Executed(invocation, value, text or "(no output)")
        try:
            verdict = await cancellation.guard(self._ctx.waterfall(POST_EXECUTE, executed, _accept))
        except Exception as error:
            logger.error(
                "a check of a result of the tool %r failed", self.call.name, exc_info=error
            )
            return self.fail(
                "CHECK_FAILED",
                "The result of the call could not be checked and is withheld: " + _describe(error),
            )
        if isinstance(verdict, Block):
            return self.fail(
                "RESULT_BLOCKED", f"The result of the call is withheld: {verdict.reason}"
            )
        return self._settle(
            ToolOutcome(
                call=self.call,
                text=verdict.text or "(no output)",
                value=value,
                concludes_turn=invocation.run.concludes_turn,
            )
        )

    def _failed(self, name: str, error: Exception, *, timed_out: bool) -> ToolOutcome:
        """Turn what running the tool raised into an error result."""
        if timed_out:
            return self.fail(
                "TOOL_TIMEOUT",
                f"The tool {name} did not finish within {self._timeout:g} seconds and was stopped.",
            )
        if isinstance(error, ToolFailure):
            return self.fail(error.code, str(error))
        if isinstance(error, cancellation.CancelledFromInside):
            return self.fail("TOOL_ERROR", f"The tool {name} was cancelled from inside.")
        logger.error("the tool %r failed", name, exc_info=error)
        return self.fail("TOOL_ERROR", f"The tool {name} failed: {_describe(error)}")

    def _settle(self, outcome: ToolOutcome) -> ToolOutcome:
        self._outcome = outcome
        self._ctx.emit(RESULT, outcome)
        return outcome


class Tools:
    """The tool service: the tools that a model can call, and the pipeline that runs a call.

    Attributes:
        registry: The tools, by name.
        timeout_seconds: How long a call may take if its tool sets no limit of
            its own, or ``None`` for no limit.
    """

    __slots__ = ("registry", "timeout_seconds")

    def __init__(self, timeout_seconds: float | None) -> None:
        self.registry: Registry[Tool[Any, Any]] = Registry("tool")
        self.timeout_seconds = timeout_seconds

    def add(self, ctx: Context, *tools: Tool[Any, Any]) -> None:
        """Register tools. They leave when the fiber of ``ctx`` unloads.

        Raises:
            RegistryConflictError: A tool of the same name is registered.
        """
        for tool in tools:
            self.registry.add(ctx, tool.name, tool)

    def schemas(self) -> tuple[ToolSchema, ...]:
        """Return what a model is told about the tools, in the order they were added."""
        return tuple(tool.schema for tool in self.registry.values())

    def begin(self, ctx: Context, call: ToolCallBlock, run: ToolRun) -> PendingCall:
        """Take up a call of the model.

        The tool is looked up and the arguments are read at once. If either
        fails, the call that is returned has its outcome already.

        Args:
            ctx: The context through which the listeners of the pipeline are
                reached, which is the context of the agent.
            call: The call as the model wrote it.
            run: What the tool is told about the call.
        """
        tool = self.registry.get(call.name)
        if tool is None:
            names = ", ".join(self.registry)
            known = f"The tools are: {names}." if names else "There are no tools."
            unknown = PendingCall(ctx, call, None, None, True)
            unknown.fail("UNKNOWN_TOOL", f"There is no tool named {call.name!r}. {known}")
            return unknown
        try:
            args = tool.parse(call.arguments)
        except ToolFailure as failure:
            unfit = PendingCall(ctx, call, None, None, True)
            unfit.fail(failure.code, str(failure))
            return unfit
        try:
            safe = tool.is_concurrency_safe(args)
        except Exception:
            logger.exception(
                "the tool %r could not say whether a call may run beside others", tool.name
            )
            safe = False
        timeout = self.timeout_seconds if tool.timeout_seconds is None else tool.timeout_seconds
        return PendingCall(ctx, call, Invocation(tool, call, args, run), timeout, safe)

    async def run(self, ctx: Context, call: ToolCallBlock, run: ToolRun) -> ToolOutcome:
        """Take a call through the whole pipeline and return how it ended."""
        return await self.begin(ctx, call, run).execute()


TOOLS = ServiceKey[Tools]("tools")
"""The tool service."""
