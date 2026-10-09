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
    Agent,
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
    PRE_STEP,
    REQUEST,
    REQUEST_ERROR,
    STREAM,
    TURN_STOPPING,
    Enter,
    ErrorDecision,
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
    Inbox,
    InboxItem,
    InboxSplicedEvent,
    Queue,
    claim_for_step,
    claim_task,
    clear_inbox,
    enqueue,
)
from columbina.kernel import Context
from columbina.session import SESSIONS

__all__ = [
    "AGENTS",
    "INBOX",
    "PRE_STEP",
    "REQUEST",
    "REQUEST_ERROR",
    "STREAM",
    "TURN_STOPPING",
    "Agent",
    "AgentError",
    "AgentFactory",
    "AgentSpec",
    "Agents",
    "CancelCause",
    "ContextKind",
    "ContextSource",
    "Enter",
    "ErrorDecision",
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
    "claim_for_step",
    "claim_task",
    "clear_inbox",
    "enqueue",
    "inject",
    "name",
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
