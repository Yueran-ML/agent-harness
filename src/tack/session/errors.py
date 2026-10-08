"""Errors raised by the session log."""


class SessionError(Exception):
    """Base class of every error the session log raises deliberately."""


class InvalidEventError(SessionError):
    """An event was rejected when it was appended. The log is unchanged."""


class SessionClosedError(SessionError):
    """An event was appended to a session that is closed."""
