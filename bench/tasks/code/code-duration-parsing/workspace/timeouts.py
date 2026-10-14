"""The timeouts of the job runner, read from its settings."""

from durations import parse_duration

DEFAULTS = {"connect": "10s", "job": "2h", "retention": "30d"}


def read_timeouts(settings: dict[str, str]) -> dict[str, int]:
    """Return every timeout in seconds, using the default for those that are not set."""
    merged = {**DEFAULTS, **settings}
    timeouts = {}
    for name, text in merged.items():
        try:
            timeouts[name] = parse_duration(text)
        except ValueError as error:
            raise ValueError(f"the timeout {name!r} is wrong: {error}") from None
    return timeouts
