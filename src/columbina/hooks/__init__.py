"""Hooks: commands of the user that run at certain moments of a session.

This module is a plugin. Loading it runs the hooks that the user and the
project have written down, at five moments.

- ``SessionStart``: when the first turn of an agent begins. What the hooks
  write is given to the model as context.
- ``UserPromptSubmit``: when a step takes up something that a person said. A
  hook can stop that input, which ends the turn, or add to it for the model.
- ``PreToolUse``: before a tool is called. A hook can stop the call or have
  somebody asked about it.
- ``PostToolUse``: after a tool has done its work. A hook can put a reason in
  the place of the result, or add something after it.
- ``Stop``: when a turn is about to end by itself. A hook can give a reason
  for the model to go on.

A hook that lets a call through has only said that it has nothing against
it. The rules and the sandbox are asked all the same: a hook can make things
stricter and never looser.

The hooks of the user are in a file in the home of the program, and those of
a project in ``.columbina/hooks.json`` at its root. The file of a project
counts only once the user has trusted it, see :mod:`columbina.hooks.config`,
and a link in its place is not followed. Both files are read anew when a
turn begins, so a change needs no restart.

Nothing that goes wrong with a hook goes wrong for the turn. A hook that
fails, a file that cannot be read and a mistake in this module are written
to the log, and the session goes on as if there were no hook. Each run of a
hook leaves two events in the log of its session.

What a hook file holds is in :mod:`columbina.hooks.config`, and how a hook
is run and its answer read in :mod:`columbina.hooks.run`.
"""

import asyncio
import itertools
import logging
import time
import weakref
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import ClassVar

from pydantic import BaseModel, Field, field_validator

from columbina import shells
from columbina.agent import (
    AGENTS,
    PRE_STEP,
    TURN_STOPPING,
    Agent,
    ContextSource,
    Reject,
    StepDecision,
    StepInfo,
    TurnStopping,
)
from columbina.approval import APPROVAL, ApprovalRequest
from columbina.fs import locate
from columbina.hooks.config import (
    TOOL_EVENTS,
    Group,
    HookFileError,
    Hooks,
    matches,
    names_of,
    parsed,
    trusted,
)
from columbina.hooks.files import (
    PROJECT_FILE,
    Found,
    of_the_project,
    of_the_user,
    project_of,
    record_at,
)
from columbina.hooks.run import Result, Status, merged, payload, run
from columbina.kernel import Context, Next
from columbina.llm import TextBlock, UserMessage
from columbina.session import (
    SESSIONS,
    EventData,
    Session,
    SessionError,
    Sessions,
    StepEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.session.store import SESSION_FILE
from columbina.tools import (
    POST_EXECUTE,
    PRE_EXECUTE,
    Accept,
    Block,
    Decision,
    Deny,
    Executed,
    Invocation,
    Verdict,
)

__all__ = [
    "PROJECT_FILE",
    "Config",
    "HookInvokedEvent",
    "HookResultEvent",
    "apply",
    "inject",
    "name",
]

name = "hooks"
inject = (AGENTS, SESSIONS)

logger = logging.getLogger(__name__)

_FILE_TOOLS = ("read", "write", "edit")
_SOURCE = "hook/"
_HUMAN = ("user", "steer")
_STOPPED = "[A hook of the user stopped what was said here: {}]"

_now = time.monotonic


class Config(BaseModel):
    """The config of the hooks.

    Attributes:
        user_file: The hook file of the user.
        trust_file: The record of the hook files of projects that the user
            trusts. Without it no project is trusted.
        project: Whether the hook file of the project is read at all.
        max_stop_continuations: How often in one turn a hook may keep the
            model going when the turn would end.
        max_context_chars: How many characters of a reason, and of what a
            hook adds for the model, are taken.
    """

    user_file: Path | None = None
    trust_file: Path | None = None
    project: bool = True
    max_stop_continuations: int = Field(default=3, ge=0)
    max_context_chars: int = Field(default=10000, ge=100)

    @field_validator("user_file", "trust_file")
    @classmethod
    def _whole(cls, given: Path | None) -> Path | None:
        # The session and the command line that trusts for it have to mean the same file,
        # from whatever directory each was started.
        if given is not None and not given.is_absolute():
            raise ValueError(f"{given} does not begin at a root. Give the whole path")
        return given


class HookInvokedEvent(EventData):
    """A hook is about to run.

    Attributes:
        event: The moment that it runs for.
        command: Its command.
    """

    event_type: ClassVar[str] = "hook/invoked"
    ignorable: ClassVar[bool] = True

    event: str
    command: str


class HookResultEvent(EventData):
    """A hook has run.

    Attributes:
        event: The moment that it ran for.
        command: Its command.
        status: How the run ended.
        exit_code: The exit code of the command, if it came to its end.
        duration_ms: How long the run took.
        reason: Why the hook stops something or wants somebody asked, if it
            does.
    """

    event_type: ClassVar[str] = "hook/result"
    ignorable: ClassVar[bool] = True

    event: str
    command: str
    status: Status
    exit_code: int | None
    duration_ms: int
    reason: str


@dataclass(frozen=True, slots=True)
class _Loaded:
    """The hooks that count for a session, as the files held them when a turn began."""

    project: Path
    groups: Mapping[str, tuple[Group, ...]] = field(default_factory=dict[str, tuple[Group, ...]])


def _text(message: UserMessage) -> str:
    return "".join(block.text for block in message.content)


def _note(source: str, text: str) -> UserMessageEvent:
    return UserMessageEvent(
        message=UserMessage(content=(TextBlock(text=text),)), source=_SOURCE + source
    )


class _Bridge:
    """Runs the hooks of a session at their moments, and does what they answer."""

    def __init__(self, ctx: Context, config: Config, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._sessions = sessions
        self._shell = shells.find()
        self._loaded: weakref.WeakKeyDictionary[Session, _Loaded] = weakref.WeakKeyDictionary()
        self._started: weakref.WeakSet[Session] = weakref.WeakSet()
        self._kept_going: weakref.WeakKeyDictionary[Session, tuple[int, int]] = (
            weakref.WeakKeyDictionary()
        )
        # What was said about a file, so that it is said once and not at every turn.
        self._said: set[str] = set()

    # Reading the files

    def _once(self, text: str) -> None:
        if text not in self._said:
            self._said.add(text)
            logger.warning("%s", text)

    def _held(self, found: Found) -> Hooks | None:
        """Return what a hook file holds. If it cannot be read or used, say why and return none."""
        if found.problem is not None:
            self._once(f"the hook file {found.path} cannot be read: {found.problem}")
        if found.content is None:
            return None
        try:
            return parsed(found.content)
        except HookFileError as error:
            self._once(f"the hook file {found.path} is not used: {error}")
            return None

    def _groups(self, found: Found, hooks: Hooks) -> Mapping[str, tuple[Group, ...]]:
        """Return the groups of a hook file that counts, and say what was passed over in it."""
        for warning in hooks.warnings:
            self._once(f"in the hook file {found.path}: {warning}")
        return hooks.groups

    def _read(self, cwd: Path) -> _Loaded:
        """Read the hook files that count for a working directory. This blocks."""
        config = self._config
        project = project_of(cwd)
        found: list[Mapping[str, tuple[Group, ...]]] = []
        if config.user_file is not None:
            mine = of_the_user(config.user_file)
            hooks = self._held(mine)
            if hooks is not None:
                found.append(self._groups(mine, hooks))
        if config.project:
            theirs = of_the_project(project)
            hooks = self._held(theirs)
            if hooks is not None:
                assert theirs.content is not None
                if trusted(record_at(config.trust_file), theirs.path, theirs.content):
                    found.append(self._groups(theirs, hooks))
                elif hooks.groups:
                    # A file that would run nothing needs nobody's trust, and no word here.
                    self._once(
                        f"the hooks of this project are not run: the file {theirs.path} is not "
                        "trusted as it is now. 'columbina hooks list' shows what it would run, "
                        "and 'columbina hooks trust' trusts it"
                    )
        groups: dict[str, tuple[Group, ...]] = {}
        for one in found:
            for event, listed in one.items():
                groups[event] = (*groups.get(event, ()), *listed)
        return _Loaded(project, groups)

    async def _hooks(self, session: Session, cwd: Path, *, anew: bool) -> _Loaded:
        loaded = None if anew else self._loaded.get(session)
        if loaded is None:
            loaded = self._loaded[session] = await asyncio.to_thread(self._read, cwd)
        return loaded

    # Running them

    def _transcript(self, session: Session, cwd: Path) -> Path | None:
        try:
            return self._sessions.directory(cwd, session.id) / SESSION_FILE
        except SessionError:
            # Sessions are kept in memory, so there is no file to point to.
            return None

    async def _run(
        self,
        session: Session,
        cwd: Path,
        event: str,
        more: Mapping[str, object],
        names: tuple[str, ...] = (),
        *,
        anew: bool = False,
    ) -> Result:
        """Run the hooks of a moment and return what they come to. This fails for nobody."""
        try:
            loaded = await self._hooks(session, cwd, anew=anew)
            groups = [
                group
                for group in loaded.groups.get(event, ())
                if event not in TOOL_EVENTS or matches(group.matcher, names)
            ]
            if not groups:
                return Result()
            if self._shell is None:
                self._once("no hook is run: the shell of this system is not installed")
                return Result()
            told = payload(event, session.id, self._transcript(session, cwd), cwd, more)
            results: list[Result] = []
            for group in groups:
                for hook in group.hooks:
                    session.append(HookInvokedEvent(event=event, command=hook.command))
                    began = _now()
                    result = Result("failed")
                    try:
                        result = await run(
                            hook,
                            event,
                            told,
                            shell=self._shell,
                            cwd=cwd,
                            project=loaded.project,
                            limit=self._config.max_context_chars,
                        )
                    finally:
                        # Also when the turn was cancelled meanwhile and the hook with it.
                        session.append(
                            HookResultEvent(
                                event=event,
                                command=hook.command,
                                status=result.status,
                                exit_code=result.exit_code,
                                duration_ms=round((_now() - began) * 1000),
                                reason=result.reason,
                            )
                        )
                    for warning in result.warnings:
                        logger.warning("the %s hook %r: %s", event, hook.command, warning)
                    if result.status == "timed-out":
                        logger.warning(
                            "the %s hook %r was stopped after %g seconds",
                            event,
                            hook.command,
                            hook.timeout,
                        )
                    if result.message:
                        logger.warning(
                            "the %s hook %r says: %s", event, hook.command, result.message
                        )
                    results.append(result)
            return merged(results)
        except Exception as error:
            logger.error("running the %s hooks failed", event, exc_info=error)
            return Result()

    # The five moments

    async def session_start(self, agent: Agent) -> str | None:
        """Run the hooks of a session that begins, once, and give what they write to the model."""
        session = agent.session
        if session in self._started:
            return None
        self._started.add(session)
        turns = sum(isinstance(event.data, TurnStartEvent) for event in session.events)
        source = "resume" if turns > 1 else "startup"
        result = await self._run(session, agent.cwd, "SessionStart", {"source": source}, anew=True)
        return result.context or None

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Run the hooks for what a person said and this step takes up."""
        agent = info.agent
        session = agent.session
        events = session.events
        # What this step took up stands after the end of the step before it, or after the
        # beginning of the turn.
        taken = itertools.takewhile(
            lambda event: not isinstance(event.data, StepEndEvent | TurnStartEvent),
            reversed(events),
        )
        said = [
            event.seq
            for event in taken
            if isinstance(event.data, UserMessageEvent) and event.data.source in _HUMAN
        ]
        for seq in reversed(said):
            data = events[seq].data
            assert isinstance(data, UserMessageEvent)
            result = await self._run(
                session,
                agent.cwd,
                "UserPromptSubmit",
                {"prompt": _text(data.message)},
                anew=info.step == 1 and seq == said[-1],
            )
            if result.decision == "block":
                # Left where it is, the model would read it in the next turn all the same.
                stopped = _note("UserPromptSubmit", _STOPPED.format(result.reason))
                session.append(stopped, replaces=[seq])
                return Reject(f"a hook stopped the input: {result.reason}")
            if result.context:
                session.append(_note("UserPromptSubmit", result.context))
        return await proceed(info)

    async def before_a_call(
        self, invocation: Invocation, proceed: Next[Invocation, Decision]
    ) -> Decision:
        """Run the hooks for a call that is about to be made."""
        run_ = invocation.run
        tool = invocation.tool.name
        result = await self._run(
            run_.session,
            run_.cwd,
            "PreToolUse",
            self._about(invocation),
            names_of(tool),
        )
        if result.decision == "block":
            return Deny(f"a hook stopped the call: {result.reason}", "HOOK_BLOCKED")
        if result.decision == "ask":
            approval = self._ctx.get(APPROVAL)
            outcome = "unavailable"
            if approval is not None:
                why = result.reason or "A hook wants somebody asked about this call."
                target = invocation.target
                outcome = await approval.ask(
                    self._ctx,
                    ApprovalRequest(
                        session=run_.session,
                        call_id=run_.call_id,
                        tool=tool,
                        summary=f"{tool}: {target}" if target else tool,
                        reason=why,
                        target=target,
                    ),
                )
            if outcome != "allowed-once":
                return Deny(
                    "a hook wanted somebody asked about the call, and nobody said yes",
                    "HOOK_BLOCKED",
                )
        # Nothing against it. Whether it may run is for the rules to say.
        return await proceed(invocation)

    async def after_a_call(self, executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        """Run the hooks for a call that a tool has carried out."""
        verdict = await proceed(executed)
        if not isinstance(verdict, Accept):
            return verdict
        invocation = executed.invocation
        run_ = invocation.run
        more = {**self._about(invocation), "tool_response": verdict.text}
        result = await self._run(
            run_.session, run_.cwd, "PostToolUse", more, names_of(invocation.tool.name)
        )
        if result.decision == "block":
            return Block(f"a hook withheld it: {result.reason}")
        if result.context:
            return Accept(f"{verdict.text}\n\n[A hook of the user adds:]\n{result.context}")
        return verdict

    async def stopping(self, stopping: TurnStopping) -> None:
        """Run the hooks for a turn that is about to end, and keep it going if they say so."""
        agent = stopping.agent
        session = agent.session
        turn, count = self._kept_going.get(session, (stopping.turn, 0))
        if turn != stopping.turn:
            count = 0
        result = await self._run(session, agent.cwd, "Stop", {"stop_hook_active": count > 0})
        if result.decision != "block":
            return
        if count >= self._config.max_stop_continuations:
            logger.warning(
                "a Stop hook wants the turn to go on, and it has done so %d times already. "
                "The turn ends",
                count,
            )
            return
        self._kept_going[session] = (stopping.turn, count + 1)
        agent.steer(result.reason, source=_SOURCE + "Stop")

    def _about(self, invocation: Invocation) -> dict[str, object]:
        """Return what a hook is told about a call."""
        tool = invocation.tool.name
        given: dict[str, object] = dict(invocation.args.model_dump(mode="json"))
        path = given.get("path")
        if tool in _FILE_TOOLS and isinstance(path, str):
            # The hooks that are most common, such as a formatter, go by this.
            given["file_path"] = str(locate(invocation.run.cwd, path).path)
        return {
            "tool_name": names_of(tool)[-1],
            "tool_input": given,
            "tool_use_id": invocation.run.call_id,
        }


def apply(ctx: Context, config: Config) -> None:
    """Run the hooks of the user and of the project at their moments."""
    sessions = ctx.use(SESSIONS)
    sessions.declare(ctx, HookInvokedEvent, HookResultEvent)
    bridge = _Bridge(ctx, config, sessions)
    # After the instructions of the project, and before the facts of the run.
    source = ContextSource(bridge.session_start, order=50, kind="note")
    ctx.use(AGENTS).context.add(ctx, "hooks", source)
    ctx.on(PRE_STEP, bridge.before_a_step)
    ctx.on(PRE_EXECUTE, bridge.before_a_call)
    ctx.on(POST_EXECUTE, bridge.after_a_call)
    ctx.on(TURN_STOPPING, bridge.stopping)
