"""Durations written the short way, such as "1h30m", and their numbers of seconds."""

import re

UNIT_SECONDS = {"d": 86400, "h": 3600, "m": 60, "s": 1}

_PART = re.compile(r"(\d+)([dhms])")


def parse_duration(text: str) -> int:
    """Return the number of seconds in a duration such as "1h30m", "2d 4h" or "45s".

    A duration is one part or more. Each part is a whole number followed
    directly by a unit: d (days), h (hours), m (minutes) or s (seconds), in
    lower or upper case. Parts may be separated by spaces, and spaces before
    and after the duration are ignored. Any other text is not a duration and
    raises ValueError.
    """
    cleaned = text.strip().lower()
    if not re.fullmatch(r"\d+[dhms](?: *\d+[dhms])*", cleaned):
        raise ValueError(f"not a duration: {text!r}")
    return sum(int(number) * UNIT_SECONDS[unit] for number, unit in _PART.findall(cleaned))


def format_duration(seconds: int) -> str:
    """Write a number of seconds the short way, largest unit first: 5400 gives "1h30m"."""
    if seconds < 0:
        raise ValueError("a duration cannot be negative")
    if seconds == 0:
        return "0s"
    parts = []
    for unit, size in UNIT_SECONDS.items():
        count, seconds = divmod(seconds, size)
        if count:
            parts.append(f"{count}{unit}")
    return "".join(parts)
