"""Agents: what every agent offers, the inbox, and the service that creates agents.

This module is a plugin. Loading it provides :data:`AGENTS`. It does not run
a single turn: that is the job of a loop, which is a plugin of its own and
registers itself with ``agents.loops``.

The service also holds the sources of context: what a model is told, beside
the words of the user, when a turn begins. A loop asks them. One source is
registered here, the note that the turn before was cut short.
"""

from columbina.agent.agent import (
    AGENTS,
    CONTEXT_PREFIX,
    Agent,
    AgentBusyError,
    AgentError,
    AgentFactory,
    Agents,
    AgentSpec,
    CancelCause,
    ContextKind,
    ContextSource,
)
from columbina.agent.context import INTERRUPTION
from columbina.agent.events import (
    EXCEPTION,
    NO_DECISION,
    PRE_STEP,
    REQUEST,
    REQUEST_ERROR,
    RUNTIME_FACTS,
    STREAM,
    TURN_STOPPING,
    Enter,
    ErrorDecision,
    Facts,
    GiveUp,
    Reject,
    RequestDraft,
    RequestFailure,
    Retry,
    StepDecision,
    StepInfo,
    StreamFrame,
    TurnStopping,
)
from columbina.agent.inbox import (
    INBOX,
    Arrivals,
    Inbox,
    InboxItem,
    InboxSplicedEvent,
    Queue,
    arrivals,
    claim_for_step,
    claim_task,
    clear_inbox,
    enqueue,
)
from columbina.agent.replies import CUT_OFF, replies
from columbina.kernel import Context
from columbina.session import SESSIONS

__all__ = [
    "AGENTS",
    "CONTEXT_PREFIX",
    "CUT_OFF",
    "EXCEPTION",
    "INBOX",
    "NO_DECISION",
    "PRE_STEP",
    "REQUEST",
    "REQUEST_ERROR",
    "RUNTIME_FACTS",
    "STREAM",
    "TURN_STOPPING",
    "Agent",
    "AgentBusyError",
    "AgentError",
    "AgentFactory",
    "AgentSpec",
    "Agents",
    "Arrivals",
    "CancelCause",
    "ContextKind",
    "ContextSource",
    "Enter",
    "ErrorDecision",
    "Facts",
    "GiveUp",
    "Inbox",
    "InboxItem",
    "InboxSplicedEvent",
    "Queue",
    "Reject",
    "RequestDraft",
    "RequestFailure",
    "Retry",
    "StepDecision",
    "StepInfo",
    "StreamFrame",
    "TurnStopping",
    "apply",
    "arrivals",
    "claim_for_step",
    "claim_task",
    "clear_inbox",
    "enqueue",
    "inject",
    "name",
    "replies",
]

name = "agent"
inject = (SESSIONS,)


def apply(ctx: Context, config: object) -> None:
    """Provide the agent service and register the events of the inbox."""
    sessions = ctx.use(SESSIONS)
    sessions.declare(ctx, InboxSplicedEvent)
    agents = Agents(sessions)
    agents.context.add(ctx, "interruption", INTERRUPTION)
    ctx.provide(AGENTS, agents)
