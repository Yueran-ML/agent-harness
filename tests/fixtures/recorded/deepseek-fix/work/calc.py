"""A small calculator."""


def mean(values: list[float]) -> float:
    """Return the mean of the values."""
    return sum(values) / (len(values) - 1)
