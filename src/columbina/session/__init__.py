"""The session log: typed events, appended once and never changed.

This module is a plugin. Loading it provides :data:`SESSIONS`.
"""

from pathlib import Path

from pydantic import BaseModel

from columbina.kernel import Context
from columbina.session.codec import (
    FORMAT_VERSION,
    SessionHeader,
    UnknownEvent,
    decode_event,
    decode_header,
    encode_event,
    encode_header,
)
from columbina.session.errors import (
    InvalidEventError,
    SessionClosedError,
    SessionError,
    SessionLockedError,
    SessionStoreError,
    UnreadableSessionError,
)
from columbina.session.events import (
    CORE_EVENTS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EndSeedEvent,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    StepEndEvent,
    StepStartEvent,
    SystemMessageEvent,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnEndReason,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.session.log import SESSION_EVENT, Committed, Session, SessionStore, now_ms
from columbina.session.projection import TURN, USAGE, Projection, TurnState, UsageTotal
from columbina.session.repair import CloseReason, closing_events
from columbina.session.service import SESSIONS, Sessions
from columbina.session.store import Listed, SessionFile, StoredSession, read_session
from columbina.session.surface import Surface

__all__ = [
    "CORE_EVENTS",
    "FORMAT_VERSION",
    "SESSIONS",
    "SESSION_EVENT",
    "TURN",
    "USAGE",
    "AssistantAttemptEvent",
    "AssistantMessageEvent",
    "CloseReason",
    "Committed",
    "Config",
    "EndSeedEvent",
    "EventData",
    "InvalidEventError",
    "Listed",
    "Projection",
    "RequestHeaderEvent",
    "Session",
    "SessionClosedError",
    "SessionError",
    "SessionEvent",
    "SessionFile",
    "SessionHeader",
    "SessionLockedError",
    "SessionStore",
    "SessionStoreError",
    "Sessions",
    "StepEndEvent",
    "StepStartEvent",
    "StoredSession",
    "Surface",
    "SystemMessageEvent",
    "ToolCallEvent",
    "ToolError",
    "ToolResultEvent",
    "TurnEndEvent",
    "TurnEndReason",
    "TurnStartEvent",
    "TurnState",
    "UnknownEvent",
    "UnreadableSessionError",
    "UsageTotal",
    "UserMessageEvent",
    "apply",
    "closing_events",
    "decode_event",
    "decode_header",
    "encode_event",
    "encode_header",
    "name",
    "now_ms",
    "read_session",
]

name = "session"


class Config(BaseModel):
    """The config of the session service.

    Attributes:
        root: The directory under which sessions are stored. Without it,
            sessions live in memory only.
    """

    root: Path | None = None


def apply(ctx: Context, config: Config) -> None:
    """Provide the session service and register the core event types."""
    sessions = Sessions(config.root)
    ctx.provide(SESSIONS, sessions)
    sessions.declare(ctx, *CORE_EVENTS)
