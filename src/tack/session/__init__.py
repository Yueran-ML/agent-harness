"""The session log: typed events, appended once and never changed.

This module is a plugin. Loading it provides :data:`SESSIONS`.
"""

from pathlib import Path

from pydantic import BaseModel

from tack.kernel import Context
from tack.session.codec import (
    FORMAT_VERSION,
    SessionHeader,
    UnknownEvent,
    decode_event,
    decode_header,
    encode_event,
    encode_header,
)
from tack.session.errors import (
    InvalidEventError,
    SessionClosedError,
    SessionError,
    SessionLockedError,
    SessionStoreError,
    UnreadableSessionError,
)
from tack.session.events import (
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
from tack.session.log import SESSION_EVENT, Committed, Session, SessionStore, now_ms
from tack.session.repair import CloseReason, closing_events
from tack.session.service import SESSIONS, Sessions
from tack.session.store import SessionFile, StoredSession, read_session
from tack.session.surface import Surface

__all__ = [
    "CORE_EVENTS",
    "FORMAT_VERSION",
    "SESSIONS",
    "SESSION_EVENT",
    "AssistantAttemptEvent",
    "AssistantMessageEvent",
    "CloseReason",
    "Committed",
    "Config",
    "EndSeedEvent",
    "EventData",
    "InvalidEventError",
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
    "UnknownEvent",
    "UnreadableSessionError",
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
