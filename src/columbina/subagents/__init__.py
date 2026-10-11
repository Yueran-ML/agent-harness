"""Subagents: helpers that an agent hands a task to.

A task that stands by itself can be done by another agent, which works on it
with a conversation of its own and gives back what it found. The agent that
handed it over reads the answer and none of the steps that led to it. So a
search through a code base costs the conversation one answer, and several
such tasks can be worked on side by side.

This module is a plugin. Loading it provides :data:`SUBAGENTS`. It starts no
subagent itself: that is the job of a provider, which is a plugin of its own
and registers itself with ``subagents.providers``. What is decided here is
the same for every provider.

Who may hand a task on
    An agent that a person talks to may. A subagent may not, unless the
    config says that tasks go deeper than one hand.
What a subagent is made from
    It works in the directory of its parent, and it is like its parent
    unless a kind of subagent says otherwise: the same model, the same
    system prompt, the same tools. A kind can name fewer tools and fewer
    steps, never more, and the tool that hands tasks on is taken from every
    subagent. A kind of a project cannot choose the model, because a model
    costs the money of the user, and a project is the work of somebody
    else. What kinds there are is in :mod:`columbina.subagents.types`.
How many run at once
    No more than the config allows. A task beyond that waits for a place
    before anything of its subagent is made.
What is handed back
    The last thing that the subagent said, if its turn came to its end.
    Anything else is a failure that says how the turn ended. What the
    subagent had written until then goes with the failure, marked as
    unfinished: it is never passed off as an answer.
What the parent keeps of it
    Two events in its log, one when the subagent has started and one when it
    has ended, and what the subagent cost, counted as a request of the
    parent that is no part of its conversation.

When the call that waits for a subagent is cancelled, the subagent is stopped
for its parent, and whatever happens it is closed in the end.
"""

import asyncio
import logging
from collections.abc import Collection
from dataclasses import dataclass
from pathlib import Path
from typing import ClassVar, Protocol

from pydantic import BaseModel, Field, field_validator

from columbina.agent import Agent, AgentSpec, CancelCause
from columbina.fs import locate
from columbina.kernel import Context, Registry, ServiceKey
from columbina.llm import Usage
from columbina.project import root_of
from columbina.session import (
    SESSIONS,
    AsideEvent,
    EventData,
    SessionClosedError,
    TurnEndReason,
)
from columbina.subagents import types
from columbina.subagents.types import Found, Scan, SubagentType
from columbina.text import whole
from columbina.tools import TOOLS, Tools

__all__ = [
    "SUBAGENTS",
    "TOOL",
    "Child",
    "Config",
    "Order",
    "Outcome",
    "Provider",
    "SubagentEndedEvent",
    "SubagentFailed",
    "SubagentStartedEvent",
    "Subagents",
    "Task",
    "apply",
    "inject",
    "name",
]

name = "subagents"
inject = (SESSIONS, TOOLS)

logger = logging.getLogger(__name__)

TOOL = "subagent"
"""The name of the tool that hands a task to a subagent. No subagent has it."""

PURPOSE = "subagent"
"""What the cost of a subagent is written down as in the log of its parent."""


class Config(BaseModel):
    """The config of the subagents.

    Attributes:
        provider: The name of the provider that starts them.
        max_running: How many of them run at once.
        max_depth: How many hands a task may pass through: with 1, an agent
            hands a task to a subagent, and that one does it itself.
        max_steps: How many steps the turn of a subagent may take. A kind of
            subagent can name fewer.
        home: The directory of kinds of subagent in the home of Columbina.
    """

    provider: str = Field(default="spawn", min_length=1)
    max_running: int = Field(default=4, ge=1)
    max_depth: int = Field(default=1, ge=1)
    max_steps: int = Field(default=50, ge=1)
    home: Path | None = None

    @field_validator("home")
    @classmethod
    def _whole(cls, given: Path | None) -> Path | None:
        # From which directory a path that begins nowhere would be meant, nobody can tell.
        if given is not None and not given.is_absolute():
            raise ValueError(f"{given} does not begin at a root. Give the whole path")
        return given


class SubagentStartedEvent(EventData):
    """A subagent has started. This is written into the log of its parent.

    Attributes:
        call_id: The id of the call that handed the task over.
        session_id: The id of the session of the subagent.
        kind: The kind of the subagent, if the call named one.
        description: What the task is, in a few words for a person.
    """

    event_type: ClassVar[str] = "subagent/started"
    ignorable: ClassVar[bool] = True

    call_id: str
    session_id: str
    kind: str | None
    description: str


class SubagentEndedEvent(EventData):
    """A subagent has ended. This is written into the log of its parent.

    Attributes:
        call_id: The id of the call that handed the task over.
        session_id: The id of the session of the subagent.
        reason: Why its turn ended, in the words of the end of a turn, or
            ``unknown`` if nobody can say.
        detail: More about the reason, as its own log has it.
        steps: How many steps its turn took.
    """

    event_type: ClassVar[str] = "subagent/ended"
    ignorable: ClassVar[bool] = True

    call_id: str
    session_id: str
    reason: str
    detail: str | None
    steps: int


@dataclass(frozen=True, slots=True)
class Task:
    """What an agent hands to a subagent.

    Attributes:
        call_id: The id of the call that hands it over.
        description: What it is, in a few words for a person.
        prompt: The task itself. The subagent knows nothing else.
        kind: The name of the kind of subagent that is to do it, or ``None``
            for one that is like the agent that hands it over.
    """

    call_id: str
    description: str
    prompt: str
    kind: str | None = None


@dataclass(frozen=True, slots=True)
class Order:
    """What a provider is asked for.

    Attributes:
        task: The task.
        spec: What the subagent is made from.
        depth: How many hands the task has passed through when the subagent
            has it: 1 for the subagent of an agent that a person talks to.
    """

    task: Task
    spec: AgentSpec
    depth: int


@dataclass(frozen=True, slots=True)
class Outcome:
    """How the turn of a subagent ended.

    Attributes:
        reason: Why it ended, or ``None`` if it did not come to an end that
            anybody wrote down.
        detail: More about the reason.
        text: The last thing that the subagent said which is not empty, or
            nothing if it said nothing.
        steps: How many steps the turn took.
        usage: What the subagent cost.
    """

    reason: TurnEndReason | None
    detail: str | None
    text: str
    steps: int
    usage: Usage


class Child(Protocol):
    """A subagent that a provider has started on its task."""

    @property
    def session_id(self) -> str:
        """The id of its session."""
        ...

    async def ended(self) -> Outcome:
        """Wait until its turn is over, and return how it ended."""
        ...

    def stop(self, cause: CancelCause) -> None:
        """Ask it to stop its turn. This only asks: wait with :meth:`ended`."""
        ...

    async def close(self) -> None:
        """End it. Nothing of it runs afterwards."""
        ...


class Provider(Protocol):
    """Something that can start a subagent."""

    async def start(self, parent: Agent, order: Order) -> Child:
        """Make a subagent of ``parent`` and give it its task.

        What the subagent may do is fixed before this returns.
        """
        ...


class SubagentFailed(Exception):  # noqa: N818 - named after what happened
    """A task was not done by a subagent. The message is for the agent that handed it over.

    Attributes:
        code: A stable code for programs.
        reason: Why the turn of the subagent ended, if it had one that ended.
        partial: What the subagent had written. It is no answer.
    """

    def __init__(
        self, message: str, *, code: str, reason: str | None = None, partial: str = ""
    ) -> None:
        super().__init__(message)
        self.code = code
        self.reason = reason
        self.partial = partial


def _how(outcome: Outcome) -> str:
    """Say how a turn ended that did not come to its end, in words for a model."""
    reason, detail = outcome.reason, outcome.detail
    if reason is None:
        return "it ended without its turn having come to an end"
    if reason == "aborted":
        return f"its turn was stopped ({detail or 'nobody said by whom'})"
    if reason == "max-tokens":
        return "a reply of its model was cut off at the limit of its length"
    if reason == "error":
        return detail or "a request to its model failed"
    if reason == "blocked":
        return detail or "its turn was ended before it was over"
    return f"its turn ended as {reason}"


def handed_back(outcome: Outcome) -> str:
    """Return what a subagent gives back to the agent that handed it a task.

    Raises:
        SubagentFailed: Its turn did not come to its end, or it said nothing.
    """
    if outcome.reason == "completed":
        if outcome.text.strip():
            return outcome.text
        raise SubagentFailed(
            "The subagent ended its turn without an answer. Hand the task over again and say "
            "what is to be reported, or do the task yourself.",
            code="SUBAGENT_FAILED",
            reason="completed",
        )
    message = f"The subagent did not finish its task: {_how(outcome)}."
    if outcome.text.strip():
        message += (
            "\nWhat it had written until then follows. It is not a finished answer."
            f"\n\n{outcome.text}"
        )
    raise SubagentFailed(
        message, code="SUBAGENT_FAILED", reason=outcome.reason, partial=outcome.text
    )


class Subagents:
    """Hands tasks to subagents, through the provider that the config names.

    Attributes:
        providers: The providers, by name.
    """

    __slots__ = ("_config", "_places", "_said", "_tools", "providers")

    def __init__(self, config: Config, tools: Tools) -> None:
        self.providers: Registry[Provider] = Registry("provider of subagents")
        self._config = config
        self._tools = tools
        self._places = asyncio.Semaphore(config.max_running)
        self._said: set[tuple[str, ...]] = set()

    def _look(self, cwd: Path, there: Collection[str]) -> tuple[Found, list[tuple[str, ...]]]:
        """Look into every place where kinds may lie. This blocks.

        Returns:
            What was found, and what there is to say about it in the log.
        """
        project = root_of(locate(cwd, ".").path)
        scans: list[Scan] = []
        notes: list[tuple[str, ...]] = []
        for root in types.roots(project, self._config.home):
            try:
                scans.append(types.usable(types.scan(root), there))
            except OSError as error:
                # A place that cannot be looked into holds nothing this time.
                notes.append(("place", str(root.path), error.strerror or str(error)))
        found = types.merged(scans)
        notes += [("invalid", str(one.file), one.reason) for one in found.invalid]
        notes += [("passed over", str(one.file)) for one in found.shadowed]
        return found, notes

    async def found(self, cwd: Path) -> Found:
        """Look for the kinds of subagent anew, and return the ones that can be used.

        A kind that names a tool which is not there cannot.

        Args:
            cwd: The working directory of the agent that asks.
        """
        there = frozenset(self._tools.registry)
        found, notes = await asyncio.to_thread(self._look, cwd, there)
        for note in notes:
            # Each of these is written to the log once.
            if note in self._said:
                continue
            self._said.add(note)
            kind, place, *why = note
            if kind == "place":
                logger.warning(
                    "the kinds of subagent in %s cannot be looked for now: %s", place, why[0]
                )
            elif kind == "invalid":
                logger.warning("%s is no kind of subagent that can be used: %s", place, why[0])
            else:
                logger.info("the kind in %s is passed over for one of the same name", place)
        return found

    def _spec(self, parent: Agent, kind: SubagentType | None) -> AgentSpec:
        """Return what a subagent of ``parent`` is made from."""
        config = self._config
        spec = parent.spec
        # No subagent has a tool that its parent has not, and none hands a task on.
        tools = frozenset(self._tools.registry) if spec.tools is None else spec.tools
        steps = config.max_steps
        model = spec.model
        prompt = spec.system_prompt
        if kind is not None:
            if kind.tools is not None:
                tools &= frozenset(kind.tools)
            if kind.max_steps is not None:
                steps = min(steps, kind.max_steps)
            if kind.prompt is not None:
                prompt = kind.prompt
            if kind.model is not None:
                if kind.root.own:
                    model = model.model_copy(update={"model": kind.model})
                else:
                    self._say_of_the_model(kind)
        return AgentSpec(
            cwd=spec.cwd,
            model=model,
            system_prompt=prompt,
            loop=spec.loop,
            tools=tools - {TOOL},
            max_steps=steps,
        )

    def _say_of_the_model(self, kind: SubagentType) -> None:
        note = ("model", str(kind.file), str(kind.model))
        if note not in self._said:
            self._said.add(note)
            logger.warning(
                "the kind of subagent in %s names the model %r. A kind of a project cannot "
                "choose the model, so its subagents ask the model of the agent that hands "
                "them a task. A kind in the home of Columbina can",
                kind.file,
                kind.model,
            )

    async def run(self, parent: Agent, task: Task) -> str:
        """Hand a task to a subagent of ``parent``, and return what it gives back.

        This waits until the turn of the subagent is over. If whoever waits is
        cancelled meanwhile, the subagent is stopped for its parent.

        Raises:
            SubagentFailed: The task may not be handed on, no subagent could
                be started for it, or the one that was started did not come
                back with an answer.
        """
        config = self._config
        depth = parent.session.delegation_depth
        if depth >= config.max_depth:
            raise SubagentFailed(
                "A subagent cannot hand its task on any further. Do the task yourself.",
                code="SUBAGENT_TOO_DEEP",
            )
        provider = self.providers.get(config.provider)
        if provider is None:
            raise SubagentFailed(
                "No subagent can be started: nothing provides them under the name "
                f"{config.provider!r}.",
                code="NO_SUBAGENTS",
            )
        kind: SubagentType | None = None
        if task.kind is not None:
            found = await self.found(parent.cwd)
            kind = found.types.get(task.kind)
            if kind is None:
                names = ", ".join(found.types)
                listed = f"The kinds are: {names}." if names else "There are no kinds."
                raise SubagentFailed(
                    f"There is no kind of subagent named {task.kind!r}. {listed} "
                    "Leave `type` out for a subagent that is like you.",
                    code="UNKNOWN_SUBAGENT_TYPE",
                )
        order = Order(task, self._spec(parent, kind), depth + 1)
        # The place is taken before anything of the subagent is made.
        async with self._places:
            return await self._run(provider, parent, order)

    async def _run(self, provider: Provider, parent: Agent, order: Order) -> str:
        task = order.task
        try:
            child = await provider.start(parent, order)
        except SubagentFailed:
            raise
        except Exception as error:
            raise SubagentFailed(
                f"The subagent could not be started: {error}.", code="SUBAGENT_NOT_STARTED"
            ) from error
        _write(
            parent,
            SubagentStartedEvent(
                call_id=task.call_id,
                session_id=child.session_id,
                kind=task.kind,
                description=whole(task.description),
            ),
        )
        outcome: Outcome | None = None
        try:
            try:
                outcome = await child.ended()
            except asyncio.CancelledError:
                # Whoever asked no longer waits. The subagent stops, and how far it came is
                # written down before the cancellation goes on its way.
                child.stop("parent")
                outcome = await child.ended()
                raise
            return handed_back(outcome)
        finally:
            try:
                _record(parent, task, child.session_id, outcome)
            finally:
                await child.close()


def _write(parent: Agent, event: EventData) -> None:
    """Write into the log of a parent, if it is still there to be written into."""
    try:
        parent.session.append(event)
    except SessionClosedError:
        # The parent has ended, and with it everything that would read this.
        logger.debug("the session %s is closed, so %s is not written", parent.id, event.event_type)


def _record(parent: Agent, task: Task, session_id: str, outcome: Outcome | None) -> None:
    """Write down in the log of a parent how its subagent ended and what it cost."""
    _write(
        parent,
        SubagentEndedEvent(
            call_id=task.call_id,
            session_id=session_id,
            reason="unknown" if outcome is None or outcome.reason is None else outcome.reason,
            detail=None if outcome is None else outcome.detail,
            steps=0 if outcome is None else outcome.steps,
        ),
    )
    if outcome is not None and outcome.usage != Usage():
        _write(parent, AsideEvent(purpose=PURPOSE, usage=outcome.usage))


SUBAGENTS = ServiceKey[Subagents]("subagents")
"""The subagents."""


def apply(ctx: Context, config: Config) -> None:
    """Provide the subagents."""
    ctx.use(SESSIONS).declare(ctx, SubagentStartedEvent, SubagentEndedEvent)
    ctx.provide(SUBAGENTS, Subagents(config, ctx.use(TOOLS)))
