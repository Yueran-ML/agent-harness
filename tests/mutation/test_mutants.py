"""The lists of mutants still fit the code they are meant for.

Running the mutants takes minutes and is done by hand. This test only keeps
the lists from going stale: when the code under a mutant changes, the mutant
has to be changed with it.
"""

from pathlib import Path

import pytest

from mutation.mutants import SETS, Mutant
from mutation.mutate import NothingTestedError, verdict

ROOT = Path(__file__).resolve().parents[2]
EVERY = [(name, mutant) for name, chosen in SETS.items() for mutant in chosen.mutants]


@pytest.mark.parametrize(
    "mutant", [mutant for _, mutant in EVERY], ids=[f"{name}: {m.name}" for name, m in EVERY]
)
def test_the_text_that_a_mutant_replaces_occurs_exactly_once(mutant: Mutant) -> None:
    source = (ROOT / mutant.path).read_text(encoding="utf-8")

    for old, new in mutant.changes:
        assert source.count(old) == 1
        assert new != old
        source = source.replace(old, new)


def test_the_mutants_of_a_set_have_names_of_their_own() -> None:
    for chosen in SETS.values():
        names = [mutant.name for mutant in chosen.mutants]

        assert len(set(names)) == len(names)


def test_the_tests_that_a_set_names_exist() -> None:
    for chosen in SETS.values():
        assert chosen.tests
        for path in chosen.tests:
            assert (ROOT / path).exists()


# The runner


@pytest.mark.parametrize(
    ("code", "found"),
    [(0, False), (1, True), (2, True), (None, True)],
    ids=["all passed", "a test failed", "the run was cut short", "no end in time"],
)
def test_a_run_of_the_tests_says_whether_a_mutant_was_found(code: int | None, found: bool) -> None:
    assert verdict(code) is found


@pytest.mark.parametrize("code", [3, 4, 5], ids=["pytest broke", "wrong usage", "no test ran"])
def test_a_run_that_tested_nothing_says_nothing_about_a_mutant(code: int) -> None:
    with pytest.raises(NothingTestedError, match=f"exit code {code}"):
        verdict(code)
