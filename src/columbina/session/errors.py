"""Errors raised by the session log."""


class SessionError(Exception):
    """Base class of every error the session log raises deliberately."""


class InvalidEventError(SessionError):
    """An event was rejected when it was appended. The log is unchanged."""


class SessionClosedError(SessionError):
    """An event was appended to a session that is closed."""


class UnreadableSessionError(SessionError):
    """A stored session cannot be loaded, because loading it would lose or distort something."""


class SessionStoreError(SessionError):
    """A session could not be written to its store."""


class SessionLockedError(SessionStoreError):
    """A session is already open for writing somewhere else."""
