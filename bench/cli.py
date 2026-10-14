"""The commands of ``python -m bench``.

python -m bench check [DIRECTORY]      whether every task can be judged
python -m bench generate [DIRECTORY]   make the tasks of SQL and of reckoning again
"""

import sys
from collections.abc import Callable

from bench import check, generate

__all__ = ["COMMANDS", "USAGE", "main"]

USAGE = "usage: python -m bench {check,generate} [DIRECTORY]"

COMMANDS: dict[str, Callable[[list[str]], int]] = {
    "check": check.main,
    "generate": generate.main,
}
"""Each command, by its name."""


def main(argv: list[str]) -> int:
    """Run a command of the benchmark. Return what it returns, or 2 if there is no such command."""
    if not argv or argv[0] not in COMMANDS:
        print(USAGE, file=sys.stderr)
        return 2
    return COMMANDS[argv[0]](argv[1:])
