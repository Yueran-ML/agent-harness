"""The lists of mutants still fit the code they are meant for.

Running the mutants takes minutes and is done by hand. This test only keeps
the lists from going stale: when the code under a mutant changes, the mutant
has to be changed with it.
"""

from pathlib import Path

import pytest

from mutation.mutants import SETS, Mutant

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
