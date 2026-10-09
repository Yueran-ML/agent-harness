"""Service keys shared by the demo plugins. This module is not a plugin."""

from columbina.kernel import ServiceKey

CLOCK = ServiceKey[int]("clock")
STORE = ServiceKey[dict[str, int]]("store")
REPORT = ServiceKey[dict[str, object]]("report")
