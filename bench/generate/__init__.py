"""The tasks that are made from a seed and not written by hand: those of SQL and of reckoning.

``python -m bench generate`` makes them again, in ``bench/tasks/sql`` and
``bench/tasks/compute``, and what was there is replaced. The seeds are fixed,
so the tasks come out the same, byte for byte, unless a generator changes.
"""

import shutil
from pathlib import Path

from bench.generate import compute, sql

__all__ = ["COMPUTE_COUNT", "SEEDS", "main"]

SEEDS = {"sql": 20261014, "sql-changed": 20261015, "compute": 20261014}
"""The seeds that the tasks of the benchmark are made from."""

COMPUTE_COUNT = 30
"""How many tasks of reckoning are made."""


def main(argv: list[str]) -> int:
    """Make the tasks again below a directory, ``bench/tasks`` if none is named. Return 0."""
    root = Path(argv[0]) if argv else Path(__file__).resolve().parents[1] / "tasks"
    for category in ("sql", "compute"):
        shutil.rmtree(root / category, ignore_errors=True)
    sql.written(sql.QUESTIONS, root / "sql", seed=SEEDS["sql"], changed=SEEDS["sql-changed"])
    compute.written(compute.made(SEEDS["compute"], COMPUTE_COUNT), root / "compute")
    print(f"{len(sql.QUESTIONS)} tasks of SQL and {COMPUTE_COUNT} of reckoning in {root}")
    return 0
