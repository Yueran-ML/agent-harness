"""Put small faults into the code, one at a time, and see whether the tests notice.

Usage, from the root of the repository::

    uv run python tests/mutation/mutate.py loop
    uv run python tests/mutation/mutate.py read tests/builtin_tools/test_read.py

The first argument names a set of ``mutants.py``. Further arguments take the
place of the tests that the set names, which is how one finds out what a
single test file notices by itself.

First the tests are run on the code as it is, and they have to pass. Then
each mutant is written into its file, pytest is run until the first failure,
and the file is put back. A run in which a test fails has found the mutant.
The exit code is 0 if every mutant was found.

The script changes files under ``src`` while it runs. It refuses to start if
``src`` has changes that are not committed, so that ``git checkout -- src``
always puts things right should it be killed half way.
"""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests"))

from mutation.mutants import SETS, Mutant  # noqa: E402 - needs the path above

SECONDS_FOR_ONE_RUN = 600


def mutated(mutant: Mutant, source: str) -> str | None:
    """Return ``source`` with the mutant in it, or ``None`` if the mutant no longer fits."""
    for old, new in mutant.changes:
        if source.count(old) != 1:
            return None
        source = source.replace(old, new)
    return source


class NothingTestedError(Exception):
    """Pytest ended in a way that says nothing about the code: it ran no test, or it broke."""


def verdict(code: int | None) -> bool:
    """Tell from the exit code of pytest whether the tests noticed a fault.

    Args:
        code: The exit code, or ``None`` if the run did not end in time.

    Raises:
        NothingTestedError: The code is neither of the three that say something.
    """
    if code == 0:
        return False
    # 1: a test failed. 2: the run was cut short, which is what a module that no longer
    # imports does to it. No end in time: tests that hang have noticed, in their way.
    if code in (1, 2, None):
        return True
    raise NothingTestedError(f"pytest ended with the exit code {code}")


def run(tests: tuple[str, ...], storage: str) -> int | None:
    """Run the tests and return the exit code of pytest, or ``None`` after too long a time."""
    command = [sys.executable, "-m", "pytest", "-q", "-x", "-p", "no:cacheprovider", *tests]
    # Examples that fail against a mutant are of no use to later runs of the real code.
    environment = os.environ | {"HYPOTHESIS_STORAGE_DIRECTORY": storage}
    try:
        done = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            capture_output=True,
            timeout=SECONDS_FOR_ONE_RUN,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return None
    return done.returncode


def main(arguments: list[str]) -> int:
    if not arguments or arguments[0] not in SETS:
        print(f"usage: mutate.py {{{','.join(SETS)}}} [TESTS...]", file=sys.stderr)
        return 2
    chosen = SETS[arguments[0]]
    tests = tuple(arguments[1:]) or chosen.tests
    status = subprocess.run(
        ["git", "status", "--porcelain", "--", "src"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    if status.stdout.strip():
        print("src has changes that are not committed: commit or stash them first", file=sys.stderr)
        return 2
    missed: list[str] = []
    with tempfile.TemporaryDirectory() as storage:
        before = run(tests, storage)
        if before != 0:
            print(
                f"the tests do not pass on the code as it is (pytest ended with {before}), "
                "so they can say nothing about a mutant",
                file=sys.stderr,
            )
            return 2
        for mutant in chosen.mutants:
            if mutant.only_on not in (None, sys.platform):
                print(f"elsewhere  {mutant.name}", flush=True)
                continue
            path = ROOT / mutant.path
            original = path.read_text(encoding="utf-8")
            changed = mutated(mutant, original)
            if changed is None:
                print(f"NO LONGER FITS  {mutant.name}", flush=True)
                missed.append(mutant.name)
                continue
            try:
                path.write_text(changed, encoding="utf-8", newline="\n")
                found = verdict(run(tests, storage))
            finally:
                path.write_text(original, encoding="utf-8", newline="\n")
            print(f"{'found   ' if found else 'MISSED  '}  {mutant.name}", flush=True)
            if not found:
                missed.append(mutant.name)
    total = sum(mutant.only_on in (None, sys.platform) for mutant in chosen.mutants)
    print(f"{total - len(missed)} of {total} mutants found by {' '.join(tests)}")
    return 1 if missed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
