"""The tool pipeline: what happens between a call of the model and its result."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any, cast

from pydantic import BaseModel

from columbina import cancellation
from columbina.kernel import Context, EmitEvent, Registry, ServiceKey, WaterfallEvent
from columbina.llm.types import TextBlock, ToolCallBlock, ToolMessage, ToolSchema
from columbina.session.events import ToolError, ToolResultEvent
from columbina.tools.tool import Tool, ToolFailure, ToolRun

logger = logging.getLogger("columbina.tools")


@dataclass(frozen=True, slots=True)
class Invocation:
    """A call that is on its way to a tool.

    Attributes:
        tool: The tool that was called.
        call: The call as the model wrote it. It is never rewritten: the log,
            the frontends and the tool must all see the same arguments.
        args: The arguments, validated against the parameters of the tool.
        run: What the tool is told about the call.
        target: What the call acts on, as the tool says it, or ``None`` if the
            tool does not say. It is worked out once, so that every check and
            every frontend goes by the same text.
    """

    tool: Tool[Any, Any]
    call: ToolCallBlock
    args: BaseModel
    run: ToolRun
    target: str | None = None


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

Guard = Callable[[Context, Invocation], Awaitable[Deny | None]]
"""A check that can refuse a call and cannot let one through.

It is given the context that the call was begun with, which is the context of
the agent, and it answers with a refusal or with nothing.
"""


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

    A call has one outcome, the first that it is given. A check counts only
    if it answers: one that fails, or answers nothing, keeps the tool from
    running and its result from getting through.

    The checks before a call come in two kinds. The listeners of
    ``tools/pre-execute`` form a chain, in which one may answer for those
    after it. The guards are asked after that chain has let the call through,
    each of them, and a guard can only refuse. So nothing in the chain can
    keep a guard from being asked.

    Attributes:
        call: The call as the model wrote it.
        concurrency_safe: Whether the call may run beside other calls of the
            same reply.
    """

    __slots__ = (
        "_authorized",
        "_ctx",
        "_guards",
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
        guards: Registry[Guard],
    ) -> None:
        self.call = call
        self.concurrency_safe = concurrency_safe
        self._ctx = ctx
        self._guards = guards
        self._invocation = invocation
        self._timeout = timeout
        self._authorized = False
        self._outcome: ToolOutcome | None = None

    @property
    def outcome(self) -> ToolOutcome | None:
        """How the call ended, or ``None`` while that is open."""
        return self._outcome

    def fail(self, code: str, text: str) -> ToolOutcome:
        """End the call with an error result, without waiting for the tool.

        If the tool is running, it is not stopped by this, but whatever it
        does from here on no longer changes the outcome.

        Raises:
            RuntimeError: The call has an outcome already.
        """
        if self._outcome is not None:
            raise RuntimeError(f"the call {self.call.call_id!r} has an outcome already")
        return self._fail(code, text)

    async def authorize(self) -> None:
        """Ask the checks whether the call may run. Asking again does nothing.

        A refusal ends the call with an error result. So does a check that
        fails or gives no answer: a call that cannot be checked does not run.
        """
        invocation = self._invocation
        if self._authorized or self._outcome is not None or invocation is None:
            return
        try:
            decision = cast(
                "object",
                await cancellation.guard(self._ctx.waterfall(PRE_EXECUTE, invocation, _allow)),
            )
        except Exception as error:
            logger.error("a check of a call of the tool %r failed", self.call.name, exc_info=error)
            self._fail(
                "CHECK_FAILED",
                f"The call could not be checked and did not run: {_describe(error)}",
            )
            return
        if isinstance(decision, Deny):
            self._fail(decision.code, f"The call was not allowed: {decision.reason}")
        elif not isinstance(decision, Allow):
            logger.error("a check of a call of the tool %r gave no decision", self.call.name)
            self._fail(
                "CHECK_FAILED",
                "The call could not be checked and did not run: a check gave no decision",
            )
        elif await self._passes_the_guards(invocation):
            self._authorized = True

    async def _passes_the_guards(self, invocation: Invocation) -> bool:
        """Ask every guard, in the order they were added, until one refuses."""
        for name, guard in list(self._guards.items()):
            what = f"the guard {name!r}"
            try:
                refusal = cast(
                    "object", await cancellation.guard(guard(self._ctx, invocation), what=what)
                )
            except Exception as error:
                logger.error(
                    "%s failed on a call of the tool %r", what, self.call.name, exc_info=error
                )
                self._fail(
                    "CHECK_FAILED",
                    f"The call could not be checked and did not run: {_describe(error)}",
                )
                return False
            if isinstance(refusal, Deny):
                self._fail(refusal.code, f"The call was not allowed: {refusal.reason}")
                return False
            if refusal is not None:
                # A guard has no way to let a call through: it refuses, or it says nothing.
                logger.error("%s gave an answer that is no refusal", what)
                self._fail(
                    "CHECK_FAILED",
                    "The call could not be checked and did not run: "
                    f"{what} gave an answer that is no refusal",
                )
                return False
        return True

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
            try:
                text = str(value) if tool.render is None else tool.render(invocation.args, value)
            except asyncio.CancelledError as error:
                # Rendering does not wait, so no cancellation of ours can arrive in it.
                raise cancellation.CancelledFromInside from error
        except Exception as error:
            return self._failed(tool.name, error, timed_out=timer.expired())
        executed = Executed(invocation, value, text or "(no output)")
        try:
            verdict = cast(
                "object",
                await cancellation.guard(self._ctx.waterfall(POST_EXECUTE, executed, _accept)),
            )
        except Exception as error:
            logger.error(
                "a check of a result of the tool %r failed", self.call.name, exc_info=error
            )
            return self._fail(
                "CHECK_FAILED",
                "The result of the call could not be checked and is withheld: " + _describe(error),
            )
        if isinstance(verdict, Accept):
            return self._settle(
                ToolOutcome(
                    call=self.call,
                    text=verdict.text or "(no output)",
                    value=value,
                    concludes_turn=invocation.run.concludes_turn,
                )
            )
        if isinstance(verdict, Block):
            return self._fail(
                "RESULT_BLOCKED", f"The result of the call is withheld: {verdict.reason}"
            )
        logger.error("a check of a result of the tool %r gave no verdict", self.call.name)
        return self._fail(
            "CHECK_FAILED",
            "The result of the call could not be checked and is withheld: a check gave no verdict",
        )

    def _failed(self, name: str, error: Exception, *, timed_out: bool) -> ToolOutcome:
        """Turn what running the tool raised into an error result."""
        if timed_out:
            return self._fail(
                "TOOL_TIMEOUT",
                f"The tool {name} did not finish within {self._timeout:g} seconds and was stopped.",
            )
        if isinstance(error, ToolFailure):
            return self._fail(error.code, str(error))
        if isinstance(error, cancellation.CancelledFromInside):
            return self._fail("TOOL_ERROR", f"The tool {name} was cancelled from inside.")
        logger.error("the tool %r failed", name, exc_info=error)
        return self._fail("TOOL_ERROR", f"The tool {name} failed: {_describe(error)}")

    def _fail(self, code: str, text: str) -> ToolOutcome:
        return self._settle(
            ToolOutcome(call=self.call, text=text, error=ToolError(code=code, reason=text))
        )

    def _settle(self, outcome: ToolOutcome) -> ToolOutcome:
        """Give the call its outcome and announce it. The first outcome stays."""
        if self._outcome is None:
            self._outcome = outcome
            self._ctx.emit(RESULT, outcome)
        return self._outcome


class Tools:
    """The tool service: the tools that a model can call, and the pipeline that runs a call.

    Attributes:
        registry: The tools, by name.
        guards: The guards, by name. Each is asked about every call that the
            listeners of ``tools/pre-execute`` let through.
        timeout_seconds: How long a call may take if its tool sets no limit of
            its own, or ``None`` for no limit.
    """

    __slots__ = ("guards", "registry", "timeout_seconds")

    def __init__(self, timeout_seconds: float | None) -> None:
        self.registry: Registry[Tool[Any, Any]] = Registry("tool")
        self.guards: Registry[Guard] = Registry("tool guard")
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
            unknown = PendingCall(ctx, call, None, None, True, self.guards)
            unknown.fail("UNKNOWN_TOOL", f"There is no tool named {call.name!r}. {known}")
            return unknown
        try:
            args = tool.parse(call.arguments)
        except ToolFailure as failure:
            unfit = PendingCall(ctx, call, None, None, True, self.guards)
            unfit.fail(failure.code, str(failure))
            return unfit
        except (Exception, asyncio.CancelledError) as error:
            # The model of the parameters is broken: one of its validators raised. Nothing
            # waits while arguments are checked, so a cancellation here is such a fault too.
            logger.error(
                "the arguments of the tool %r could not be checked", tool.name, exc_info=error
            )
            broken = PendingCall(ctx, call, None, None, True, self.guards)
            broken.fail(
                "TOOL_ERROR",
                f"The arguments of the tool {tool.name} could not be checked: {_describe(error)}",
            )
            return broken
        try:
            target = None if tool.target is None else tool.target(args, run)
        except (Exception, asyncio.CancelledError) as error:
            # Without knowing what a call acts on, no check can judge it.
            logger.error("the tool %r could not say what a call acts on", tool.name, exc_info=error)
            aimless = PendingCall(ctx, call, None, None, True, self.guards)
            aimless.fail(
                "TOOL_ERROR",
                f"The tool {tool.name} could not say what the call acts on: {_describe(error)}",
            )
            return aimless
        try:
            safe = tool.is_concurrency_safe(args)
        except (Exception, asyncio.CancelledError):
            logger.exception(
                "the tool %r could not say whether a call may run beside others", tool.name
            )
            safe = False
        timeout = self.timeout_seconds if tool.timeout_seconds is None else tool.timeout_seconds
        invocation = Invocation(tool, call, args, run, target)
        return PendingCall(ctx, call, invocation, timeout, safe, self.guards)

    async def run(self, ctx: Context, call: ToolCallBlock, run: ToolRun) -> ToolOutcome:
        """Take a call through the whole pipeline and return how it ended."""
        return await self.begin(ctx, call, run).execute()


TOOLS = ServiceKey[Tools]("tools")
"""The tool service."""
