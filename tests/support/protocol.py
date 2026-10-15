"""The types of event that the package declares, as the code has them.

The sample session of ``tests/protocol/test_tour.py`` is held to these: everything that can
happen in a session happens in it once.
"""

import importlib
import pkgutil

import columbina
from columbina.session import EventData, UnknownEvent

__all__ = ["event_kinds"]

_NOT_A_MODULE_TO_LOAD = "__main__"


def event_kinds() -> dict[str, type[EventData]]:
    """Return every type of event that a module of the package declares, by its name.

    The package is gone through for them, so a type that a new plugin brings
    is found without being named here. Types that tests declare for
    themselves are left out, and so is what stands for a type that nothing
    knows.
    """
    for module in pkgutil.walk_packages(columbina.__path__, "columbina."):
        if not module.name.endswith(_NOT_A_MODULE_TO_LOAD):
            importlib.import_module(module.name)
    found: dict[str, type[EventData]] = {}
    pending = list(EventData.__subclasses__())
    while pending:
        kind = pending.pop()
        pending.extend(kind.__subclasses__())
        if kind.__module__.startswith("columbina.") and not issubclass(kind, UnknownEvent):
            found[kind.event_type] = kind
    return dict(sorted(found.items()))
