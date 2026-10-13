"""Hooks: commands of the user that run at certain moments of a session.

This module is a plugin. Loading it runs the hooks that the user and the
project have written down, at five moments.

- ``SessionStart``: when the first turn of an agent begins. What the hooks
  write is given to the model as context.
- ``UserPromptSubmit``: before a model is sent something that a person said.
  A hook can stop that input, which ends the turn, or add to it for the model.
- ``PreToolUse``: before a tool is called. A hook can stop the call or have
  somebody asked about it.
- ``PostToolUse``: after a tool has done its work. A hook can put a reason in
  the place of the result, or add something after it.
- ``Stop``: when a turn is about to end by itself. A hook can give a reason
  for the model to go on.

A hook that lets a call through has only said that it has nothing against
it. The rules and the sandbox are asked all the same: a hook can make things
stricter and never looser. So the hooks stand among the guards of the tools,
which can refuse a call and cannot let one through.

No input of a person reaches a model that the hooks have not been asked
about. A model is sent what it sees when a step begins. So before a step
begins, the hooks are asked about everything that a person said since the
last step that began, be it taken up by this step or by one that never came
to begin because something else stopped it.

The hooks of the user are in a file in the home of the program, and those of
a project in ``.columbina/hooks.json`` at its root. The file of a project
counts only once the user has trusted it, see :mod:`columbina.hooks.config`,
and a link in its place is not followed. Both files are read anew when a
turn begins, so a change needs no restart.

A subagent is no conversation of a person. What its tools do is checked by
the two hooks of the tools like that of any agent. The other three moments
are about a person and a session of theirs: no session of a person starts
with a subagent, what it is given was written by an agent, and whether its
turn may end is not for a hook to say that was written for a conversation.
So those three are not run for it.

Nothing that goes wrong with a hook goes wrong for the turn. A hook that
fails, a file that cannot be read and a mistake in this module are written
to the log, and the session goes on as if that hook were not there. What
the hooks beside it decided stands. Each run of a hook leaves two events in
the log of its session.

What a hook file holds is in :mod:`columbina.hooks.config`, and how a hook
is run and its answer read in :mod:`columbina.hooks.run`.
"""

import asyncio
import contextlib
import logging
import time
import weakref
from collections.abc import Callable, Mapping
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
from columbina.fs import FS, locate
from columbina.hooks.config import (
    TOOL_EVENTS,
    Group,
    Hook,
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
    StepStartEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.session.store import SESSION_FILE
from columbina.tools import (
    POST_EXECUTE,
    TOOLS,
    Accept,
    Block,
    Deny,
    Executed,
    Invocation,
    ToolFailure,
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
inject = (AGENTS, SESSIONS, TOOLS)

logger = logging.getLogger(__name__)

_FILE_TOOLS = ("read", "write", "edit")
_SOURCE = "hook/"
_HUMAN = ("user", "steer")
_STOPPED = "[A hook of the user stopped what was said here: {}]"
_NOT_ACTED_ON = "what the %s hooks answered could not be acted on"

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
        # Where the inputs lie that the hooks let through, of those that no model was sent yet.
        self._passed: weakref.WeakKeyDictionary[Session, set[int]] = weakref.WeakKeyDictionary()
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

    async def _one(
        self,
        session: Session,
        cwd: Path,
        project: Path,
        event: str,
        hook: Hook,
        told: bytes,
        shell: str,
    ) -> Result:
        """Run one hook, write its run into the log, and return what it answered.

        What goes wrong here goes wrong for this one hook. It counts as a
        hook that failed, which stops nothing, and what it answered before
        things went wrong stands.
        """
        result = Result("failed")
        try:
            session.append(HookInvokedEvent(event=event, command=hook.command))
            began = _now()
            try:
                result = await run(
                    hook,
                    event,
                    told,
                    shell=shell,
                    cwd=cwd,
                    project=project,
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
                    "the %s hook %r was stopped after %g seconds", event, hook.command, hook.timeout
                )
            if result.message:
                logger.warning("the %s hook %r says: %s", event, hook.command, result.message)
        except Exception as error:
            logger.error("the %s hook %r could not be run", event, hook.command, exc_info=error)
        return result

    async def _run(
        self,
        session: Session,
        cwd: Path,
        event: str,
        more: Callable[[], Mapping[str, object]],
        names: tuple[str, ...] = (),
        *,
        anew: bool = False,
    ) -> Result:
        """Run the hooks of a moment and return what they come to. This fails for nobody.

        Args:
            session: The session.
            cwd: The working directory of its agent.
            event: The moment.
            more: Works out what this moment adds to what every hook is told.
                It is asked only if there is a hook to tell.
            names: The names that the tool of this moment goes by, if it has one.
            anew: Whether the hook files are read again first.
        """
        results: list[Result] = []
        try:
            loaded = await self._hooks(session, cwd, anew=anew)
            groups = [
                group
                for group in loaded.groups.get(event, ())
                if event not in TOOL_EVENTS or matches(group.matcher, names)
            ]
            shell = self._shell
            if groups and shell is None:
                self._once("no hook is run: the shell of this system is not installed")
            elif groups and shell is not None:
                hooks = [hook for group in groups for hook in group.hooks]
                told = payload(event, session.id, self._transcript(session, cwd), cwd, more())
                for hook in hooks:
                    answered = await self._one(
                        session, cwd, loaded.project, event, hook, told, shell
                    )
                    # One by one: what a hook answered stands, whatever becomes of the next.
                    results.append(answered)
        except Exception as error:
            logger.error("running the %s hooks failed", event, exc_info=error)
        # What the hooks answered before something went wrong is what they answered.
        return merged(results, self._config.max_context_chars)

    # The five moments

    async def session_start(self, agent: Agent) -> str | None:
        """Run the hooks of a session that begins, once, and give what they write to the model."""
        session = agent.session
        if session.delegation_depth or session in self._started:
            # No session of a person begins with a subagent.
            return None
        self._started.add(session)
        turns = sum(isinstance(event.data, TurnStartEvent) for event in session.events)
        source = "resume" if turns > 1 else "startup"
        told = {"source": source}
        result = await self._run(session, agent.cwd, "SessionStart", lambda: told, anew=True)
        return result.context or None

    def _waiting(self, session: Session) -> list[int]:
        """Return where the inputs of a person lie that no model was sent and no hook let through.

        A model is sent what it sees when a step begins. So whatever a
        person said that lies after the last step that began, and is still
        seen, has reached no model. A step may have taken it up and never
        begun: something else stopped that step, or the turn failed before
        it came to ask.
        """
        events = session.events
        began = next(
            (event.seq for event in reversed(events) if isinstance(event.data, StepStartEvent)), -1
        )
        # What lies before a step that began was sent, and is nothing to remember any more.
        passed = {seq for seq in self._passed.get(session, ()) if seq > began}
        self._passed[session] = passed
        waiting: list[int] = []
        for seq in reversed(session.surface):
            if seq <= began:
                break
            data = events[seq].data
            if isinstance(data, UserMessageEvent) and data.source in _HUMAN and seq not in passed:
                waiting.append(seq)
        waiting.reverse()
        return waiting

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Run the hooks for what a person said and a model has not been sent yet."""
        agent = info.agent
        session = agent.session
        if session.delegation_depth:
            # What a subagent is given was written by an agent, and no person said it.
            return await proceed(info)
        stopped: list[str] = []
        # The hook files are read again when a turn begins.
        anew = info.step == 1
        for seq in self._waiting(session):
            data = session.events[seq].data
            assert isinstance(data, UserMessageEvent)
            told = {"prompt": _text(data.message)}
            result = await self._run(
                session, agent.cwd, "UserPromptSubmit", lambda told=told: told, anew=anew
            )
            anew = False
            try:
                if result.decision == "block":
                    # Left where it is, the model would read it in the next turn all the same.
                    note = _note("UserPromptSubmit", _STOPPED.format(result.reason))
                    session.append(note, replaces=[seq])
                elif result.context:
                    session.append(_note("UserPromptSubmit", result.context))
            except Exception as error:
                logger.error(_NOT_ACTED_ON, "UserPromptSubmit", exc_info=error)
            if result.decision == "block":
                # Every other input is looked at all the same: the turn ends, and what is
                # left where a model sees it will be read by the turn after this one.
                stopped.append(result.reason)
            else:
                self._passed[session].add(seq)
        if stopped:
            return Reject(f"a hook stopped the input: {stopped[0]}")
        return await proceed(info)

    async def guard(self, ctx: Context, invocation: Invocation) -> Deny | None:
        """Run the hooks for a call that is about to be made, and refuse it if they say so.

        Args:
            ctx: The context of the agent that makes the call. Whoever is asked
                about the call is asked through it, so that the one who answers
                is the one who answers for this agent.
            invocation: The call.
        """
        run_ = invocation.run
        tool = invocation.tool.name
        result = await self._run(
            run_.session,
            run_.cwd,
            "PreToolUse",
            lambda: self._about(invocation),
            names_of(tool),
        )
        if result.decision == "block":
            return Deny(f"a hook stopped the call: {result.reason}", "HOOK_BLOCKED")
        if result.decision == "ask":
            approval = ctx.get(APPROVAL)
            outcome = "unavailable"
            if approval is not None:
                why = result.reason or "A hook wants somebody asked about this call."
                target = invocation.target
                outcome = await approval.ask(
                    ctx,
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
        # Nothing against it. Whether it may run is for the other guards to say.
        return None

    async def after_a_call(self, executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        """Run the hooks for a call that a tool has carried out."""
        verdict = await proceed(executed)
        if not isinstance(verdict, Accept):
            return verdict
        invocation = executed.invocation
        run_ = invocation.run
        result = await self._run(
            run_.session,
            run_.cwd,
            "PostToolUse",
            lambda: {**self._about(invocation), "tool_response": verdict.text},
            names_of(invocation.tool.name),
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
        if session.delegation_depth:
            # Such a hook was written for the turns of a conversation.
            return
        turn, count = self._kept_going.get(session, (stopping.turn, 0))
        if turn != stopping.turn:
            count = 0
        told = {"stop_hook_active": count > 0}
        result = await self._run(session, agent.cwd, "Stop", lambda: told)
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
        try:
            agent.steer(result.reason, source=_SOURCE + "Stop")
        except Exception as error:
            logger.error(_NOT_ACTED_ON, "Stop", exc_info=error)

    def _about(self, invocation: Invocation) -> dict[str, object]:
        """Return what a hook is told about a call."""
        tool = invocation.tool.name
        given: dict[str, object] = dict(invocation.args.model_dump(mode="json"))
        path = given.get("path")
        if tool in _FILE_TOOLS and isinstance(path, str):
            # The hooks that are most common, such as a formatter, go by this. A path that
            # names no file is left for the tool to refuse.
            with contextlib.suppress(ToolFailure):
                seam = self._ctx.get(FS)
                # A model may name the file the way the place of its commands has it.
                here = path if seam is None else seam.here(invocation.run.session, path)
                given["file_path"] = str(locate(invocation.run.cwd, here).path)
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
    ctx.use(TOOLS).guards.add(ctx, name, bridge.guard)
    ctx.on(POST_EXECUTE, bridge.after_a_call)
    ctx.on(TURN_STOPPING, bridge.stopping)
