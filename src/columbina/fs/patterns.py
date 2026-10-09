"""Matching the names of a path against the parts of a pattern.

A pattern for paths is made of parts, one for each name, and of parts that
stand for any number of names. Whoever reads a pattern, the glob tool or a
rule about files, has its own way to say what one part means. How the parts
are laid along the names of a path is the same for all, and it is done here.

The names are gone through once, and for each of them the places of the
pattern are kept that the match may have reached. So the work is the number
of names times the number of parts and no more, whatever the pattern is, and
no function calls itself, however long the path or the pattern.
"""

from collections.abc import Callable, Sequence
from typing import TypeVar

__all__ = ["sequence_fits"]

PartT = TypeVar("PartT")


def _with_none_skipped(parts: Sequence[object | None], reached: set[int]) -> set[int]:
    """Add the places that follow parts which stand for any number of names, taken as none."""
    wider = set(reached)
    for at in reached:
        while at < len(parts) and parts[at] is None:
            at += 1
            wider.add(at)
    return wider


def sequence_fits(
    parts: Sequence[PartT | None],
    names: Sequence[str],
    fits: Callable[[PartT, str], bool],
) -> bool:
    """Tell whether the names of a path fit the parts of a pattern.

    Args:
        parts: The parts of the pattern. One that is ``None`` stands for any
            number of names, also none.
        names: The names of the path.
        fits: Tells whether one name fits one part.
    """
    reached = _with_none_skipped(parts, {0})
    for name in names:
        onward: set[int] = set()
        for at in reached:
            if at == len(parts):
                continue
            part = parts[at]
            if part is None:
                # The name is one more of those that this part stands for.
                onward.add(at)
            elif fits(part, name):
                onward.add(at + 1)
        if not onward:
            return False
        reached = _with_none_skipped(parts, onward)
    return len(parts) in reached
