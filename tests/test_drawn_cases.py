"""What every test that draws its cases goes by."""

import os
import subprocess
import sys
from pathlib import Path
from typing import Any, cast

from hypothesis import settings

TESTS = Path(__file__).resolve().parent

TOLD = """
import sys

sys.path.insert(0, sys.argv[1])
from hypothesis import settings

from support import cases

cases.register()
default, steady = settings.get_profile("default"), settings.get_profile("steady")
print(default.derandomize, steady.derandomize, default.deadline, steady.deadline)
"""
"""A program that makes the profiles of these tests and says what the two are like."""


def profile(name: str | None = None) -> Any:
    """The settings of a profile, or the ones in force if no profile is named."""
    return cast("Any", settings() if name is None else settings.get_profile(name))


def test_no_case_is_held_to_a_time_of_its_own() -> None:
    # How long one case takes says how fast the machine is. A machine of the CI took three
    # hundred milliseconds for a case that takes a few here, and the test failed for that.
    # A test that never ends is ended by the limit that every test has.
    for name in ("default", "steady"):
        assert profile(name).deadline is None
    assert profile().deadline is None


def test_only_the_steady_cases_are_the_same_in_every_run() -> None:
    assert profile("steady").derandomize
    assert not profile("default").derandomize


def test_the_two_are_what_they_are_on_a_machine_of_a_ci_as_well() -> None:
    # Hypothesis tells such a machine by its environment, and has other settings in force
    # there from the moment it is imported: the same cases in every run, among other
    # things. A profile that names no parent is built on whatever is in force. So the first
    # time these tests ran on such a machine, the profile whose cases are to differ from
    # run to run drew the same ones every time, and the test above failed on all six.
    there = {**os.environ, "CI": "true", "GITHUB_ACTIONS": "true"}

    said = subprocess.run(
        [sys.executable, "-c", TOLD, str(TESTS)],
        env=there,
        capture_output=True,
        text=True,
        timeout=120,
        check=True,
    )

    assert said.stdout.split() == ["False", "True", "None", "None"]
