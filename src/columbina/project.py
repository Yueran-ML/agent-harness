"""Where a project begins.

Several things are looked for from the top of a project: the files that say
how work in it is done, the skills that it brings, the hooks that it wants
run, the kinds of subagent that it knows. The top is the same for all of
them, and this is where it is found.

The root of a project is the nearest directory, from the working directory
upwards, that holds ``.git``: a directory in a repository, and a file in a
work tree that was added to one. A working directory that lies in no
repository is its own project.
"""

import os
from pathlib import Path

__all__ = ["root_of", "way_down"]


def way_down(here: Path) -> list[Path]:
    """Return the directories from the root of the project to a working directory, both with it.

    Without a root, the way is the working directory alone.

    Args:
        here: The working directory, absolute.
    """
    up = [here, *here.parents]
    for steps, above in enumerate(up):
        # Nothing is followed to find this out: a link of that name counts as it lies there.
        if os.path.lexists(above / ".git"):
            return list(reversed(up[: steps + 1]))
    return [here]


def root_of(here: Path) -> Path:
    """Return the root of the project that a working directory lies in."""
    return way_down(here)[0]
