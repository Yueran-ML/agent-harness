import re
from importlib.metadata import version

import tack


def test_version_matches_the_installed_distribution() -> None:
    assert tack.__version__ == version("tack-harness")


def test_version_looks_like_a_release_number() -> None:
    assert re.match(r"^\d+\.\d+\.\d+", tack.__version__)
