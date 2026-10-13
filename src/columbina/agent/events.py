"""What a running agent tells the plugins around it, and what it asks them.

These are kernel events: they are for code that runs in the same process and
wants to watch or steer a turn while it happens. What a turn leaves behind is
in the session log.

A listener of one of the questions that fails ends the turn with an error.
"""

from dataclasses import dataclass, field

from columbina.agent.agent import Agent
from columbina.kernel import EmitEvent, SerialEvent, WaterfallEvent
from columbina.llm.types import CallConfig, Chunk, LlmFailure
from columbina.session import TurnEndReason

NO_DECISION = "NO_DECISION"
"""The code of a turn that was blocked because a listener answered nothing about a step."""

EXCEPTION = "EXCEPTION"
"""The code of a turn that ended with an error of this program, and not of a model.

A request that failed for good gives the code of its failure, as the service
of the model or its adapter named it.
"""


@dataclass(frozen=True, slots=True)
class StepInfo:
    """A step that is about to begin.

    Attributes:
        agent: The agent.
        turn: The number of the turn.
        step: The number that the step will have in that turn.
    """

    agent: Agent
    turn: int
    step: int


@dataclass(frozen=True, slots=True)
class Enter:
    """The answer that lets a step begin."""


@dataclass(frozen=True, slots=True)
class Reject:
    """The answer that keeps a step from beginning, which ends the turn as ``blocked``.

    Attributes:
        reason: Why, in words for a person.
        code: Why, for a program: a word that whoever refuses chooses for
            each of its reasons, such as ``STEP_BUDGET``. It goes into the
            end of the turn as it is.
    """

    reason: str
    code: str | None = None


StepDecision = Enter | Reject
"""What a listener of :data:`PRE_STEP` answers."""

PRE_STEP: WaterfallEvent[StepInfo, StepDecision] = WaterfallEvent("agent/pre-step")
"""Asked before every step, after the input of the step is in the log."""


@dataclass(frozen=True, slots=True)
class RequestDraft:
    """A request that is about to be made.

    Attributes:
        agent: The agent.
        turn: The number of the turn.
        step: The number of the step in that turn.
        attempt: 1 for the first request of the step, and one more for every
            time it is tried again.
        config: The call config that the spec of the agent asks for.
    """

    agent: Agent
    turn: int
    step: int
    attempt: int
    config: CallConfig


REQUEST: WaterfallEvent[RequestDraft, CallConfig] = WaterfallEvent("agent/request")
"""Asked before every request. A listener answers the call config to use."""


@dataclass(frozen=True, slots=True)
class StreamFrame:
    """One chunk of a reply, as it arrives.

    Attributes:
        agent: The agent that is being answered.
        turn: The number of the turn.
        step: The number of the step in that turn.
        chunk: The chunk.
    """

    agent: Agent
    turn: int
    step: int
    chunk: Chunk


STREAM: EmitEvent[StreamFrame] = EmitEvent("agent/assistant-stream")
"""Emitted for every chunk of every reply, the finish included."""


@dataclass(frozen=True, slots=True)
class RequestFailure:
    """A request that has failed. The attempt is in the log already.

    Attributes:
        agent: The agent.
        turn: The number of the turn.
        step: The number of the step in that turn.
        attempt: Which attempt of the step failed, counted from 1.
        failure: Why it failed.
    """

    agent: Agent
    turn: int
    step: int
    attempt: int
    failure: LlmFailure


@dataclass(frozen=True, slots=True)
class Retry:
    """The answer that has the request made again, in the same step."""


@dataclass(frozen=True, slots=True)
class GiveUp:
    """The answer that lets the failure stand, which ends the turn with an error."""


ErrorDecision = Retry | GiveUp
"""What a listener of :data:`REQUEST_ERROR` answers."""

REQUEST_ERROR: WaterfallEvent[RequestFailure, ErrorDecision] = WaterfallEvent("agent/request-error")
"""Asked when a request has failed. A listener that wants to wait first does so itself."""


@dataclass(frozen=True, slots=True)
class TurnStopping:
    """A turn that is about to end by itself.

    Attributes:
        agent: The agent.
        turn: The number of the turn.
        reason: The reason the turn would end with: ``completed`` or ``max-tokens``.
    """

    agent: Agent
    turn: int
    reason: TurnEndReason


TURN_STOPPING: SerialEvent[TurnStopping, None] = SerialEvent("agent/turn-stopping")
"""Told when a turn is about to end by itself. A listener that steers the agent keeps it going."""


@dataclass(frozen=True, slots=True)
class Facts:
    """What a model is told about where it runs, gathered when a turn begins.

    Attributes:
        agent: The agent whose turn begins.
        lines: The facts, each under a name. A listener adds what it knows.
    """

    agent: Agent
    lines: dict[str, str] = field(default_factory=dict[str, str])


RUNTIME_FACTS: EmitEvent[Facts] = EmitEvent("agent/runtime-facts")
"""Emitted to gather the facts about a run. A plugin that knows one adds it.

Whoever tells the model the facts emits this: see :mod:`columbina.context.runtime`.
"""
