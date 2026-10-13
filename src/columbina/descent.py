"""Which processes descend from which, told from what a system says of each."""

from collections.abc import Callable, Mapping

__all__ = ["descended"]


def descended(
    held: Mapping[int, int],
    parents: Mapping[int, int],
    made: Callable[[int], int | None],
) -> dict[int, int]:
    """Find the processes that descend from the ones that are held.

    A system says of each process which one made it, and says it with a
    number. A number is given out again once its process is gone, so the
    number alone may name a stranger. A process counts as made by another
    only if it is the younger of the two: while the other has lived, nobody
    else had its number.

    Whoever calls this ends what it finds. So it takes too little rather than
    too much: a process whose time cannot be found out is left alone, and so
    is one that is exactly as old as the one it names.

    Args:
        held: The processes to start from, each with the time it was made.
            The caller sees to it that the number of each goes on naming it
            while this looks.
        parents: Every process, with the number of the one that made it.
        made: Tells when a process was made, or ``None`` if that cannot be
            found out. It is asked only about a process that names one of
            the run, since asking may cost something.

    Returns:
        What descends from the held ones, each with the time it was made.
        None of the held ones is among them.
    """
    children: dict[int, list[int]] = {}
    for process, parent in parents.items():
        children.setdefault(parent, []).append(process)
    known = dict(held)
    found: dict[int, int] = {}
    waiting = list(held)
    while waiting:
        parent = waiting.pop()
        for child in children.get(parent, ()):
            if child in known:
                continue
            when = made(child)
            if when is None or when <= known[parent]:
                continue
            known[child] = found[child] = when
            waiting.append(child)
    return found
