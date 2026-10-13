"""What every test that draws its cases goes by."""

from typing import Any, cast

from hypothesis import settings


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
