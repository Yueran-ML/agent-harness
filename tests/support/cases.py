"""The settings that every test goes by which draws its cases."""

import os

from hypothesis import settings

__all__ = ["PLAIN", "register"]

PLAIN = settings.get_profile("default")
"""What hypothesis goes by where nothing is said of it.

On a machine of a CI it has other settings in force from the moment it is
imported: the same cases in every run, among other things. A profile that
names no parent is built on whatever is in force. So the profiles of these
tests name this one, and what a test draws is the same kind of thing on
every machine.
"""


def register() -> None:
    """Make the two profiles of these tests, and put in force the one that is asked for.

    Tests that draw their cases at random find more over time, because every
    run draws others. Whoever needs the same answer from every run asks for
    the same cases: the check of the mutants does, since a fault that is
    found in one run and missed in the next says nothing.

    No case is held to a time of its own. How long one case takes says how
    fast the machine is, and a machine of the CI has taken many times as
    long as the one that the tests were written on. A test that does not end
    is ended by the limit that every test has.
    """
    settings.register_profile("default", parent=PLAIN, deadline=None)
    settings.register_profile("steady", parent=PLAIN, derandomize=True, deadline=None)
    settings.load_profile(os.environ.get("HYPOTHESIS_PROFILE", "default"))
