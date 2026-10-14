"""Splitting the bill of a restaurant between the people at the table."""

from money import format_cents, to_cents


def split_evenly(total_cents: int, people: int) -> list[int]:
    """Split a number of cents between people, so that the shares add up to it exactly.

    The shares differ by one cent at most: the cents that do not divide
    evenly go one each to the first people in the list.
    """
    if people < 1:
        raise ValueError("a bill is split between one person or more")
    share, left_over = divmod(total_cents, people)
    return [share + 1 if index < left_over else share for index in range(people)]


def split_bill(total: str, people: int) -> list[int]:
    """Split a bill written like "45.10" between people, and return each share in cents."""
    return split_evenly(to_cents(total), people)


def describe(total: str, names: list[str]) -> list[str]:
    """Say what each person pays, one line each, such as "Ann pays 15.04"."""
    shares = split_bill(total, len(names))
    return [f"{name} pays {format_cents(share)}" for name, share in zip(names, shares)]
