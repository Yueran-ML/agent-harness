import re
from importlib.metadata import version

import columbina


def test_version_matches_the_installed_distribution() -> None:
    assert columbina.__version__ == version("columbina")


def test_version_looks_like_a_release_number() -> None:
    assert re.match(r"^\d+\.\d+\.\d+", columbina.__version__)
