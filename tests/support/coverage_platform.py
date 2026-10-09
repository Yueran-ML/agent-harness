"""Tells coverage which lines cannot run on the platform it is measuring.

Code that only one platform can run says so on the line that begins it::

    if sys.platform == "win32":  # pragma: windows only
        ...
    else:  # pragma: posix only
        ...

Each block is counted where it can run and left out where it cannot. Without
this, both blocks would have to be left out everywhere, and a line that no
test reaches on its own platform would go unnoticed.

Coverage loads this module before it measures anything, so it imports nothing
of the package under test. It is found by its dotted name from the root of the
repository, which is why coverage is run as ``python -m coverage``.
"""

import sys
from typing import Any

from coverage import CoveragePlugin
from coverage.plugin_support import Plugins
from coverage.types import TConfigurable

__all__ = ["coverage_init"]

_ELSEWHERE = "posix" if sys.platform == "win32" else "windows"


class _Platform(CoveragePlugin):
    """Leaves out the lines that are marked for the other platform."""

    def configure(self, config: TConfigurable) -> None:
        excluded = config.get_option("report:exclude_lines")
        assert isinstance(excluded, list)
        config.set_option("report:exclude_lines", [*excluded, rf"#\s*pragma: {_ELSEWHERE} only"])


def coverage_init(reg: Plugins, options: dict[str, Any]) -> None:
    """Register the plugin. Coverage calls this by name."""
    reg.add_configurer(_Platform())
