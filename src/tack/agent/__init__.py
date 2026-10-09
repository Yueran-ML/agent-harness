"""Agents: what every agent offers, the inbox, and the service that creates agents.

This module is a plugin. Loading it provides :data:`AGENTS`. It does not run
a single turn: that is the job of a loop, which is a plugin of its own and
registers itself with ``agents.loops``.
"""

from tack.agent.agent import (
    AGENTS,
    Agent,
    AgentError,
    AgentFactory,
    Agents,
    AgentSpec,
    CancelCause,
)
from tack.agent.events import (
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
from tack.agent.inbox import (
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
from tack.kernel import Context
from tack.session import SESSIONS

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
    ctx.provide(AGENTS, Agents(sessions))
