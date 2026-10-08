"""The plugin kernel: contexts, services, effects, events, and plugin instances."""

from tack.kernel.context import Context
from tack.kernel.errors import (
    CompositionError,
    EventModeError,
    InactiveEffectError,
    InvalidPluginError,
    KernelError,
    ServiceConflictError,
    ServiceNotInjectedError,
    ServiceUnavailableError,
)
from tack.kernel.events import (
    BailEvent,
    BailListener,
    EmitEvent,
    EmitListener,
    Event,
    Next,
    ParallelEvent,
    ParallelListener,
    SerialEvent,
    SerialListener,
    WaterfallEvent,
    WaterfallListener,
)
from tack.kernel.fiber import Disposer, EffectHandle, Fiber, FiberState
from tack.kernel.plugin import Apply, ApplyResult, Plugin, resolve_plugin
from tack.kernel.services import ServiceKey

__all__ = [
    "Apply",
    "ApplyResult",
    "BailEvent",
    "BailListener",
    "CompositionError",
    "Context",
    "Disposer",
    "EffectHandle",
    "EmitEvent",
    "EmitListener",
    "Event",
    "EventModeError",
    "Fiber",
    "FiberState",
    "InactiveEffectError",
    "InvalidPluginError",
    "KernelError",
    "Next",
    "ParallelEvent",
    "ParallelListener",
    "Plugin",
    "SerialEvent",
    "SerialListener",
    "ServiceConflictError",
    "ServiceKey",
    "ServiceNotInjectedError",
    "ServiceUnavailableError",
    "WaterfallEvent",
    "WaterfallListener",
    "resolve_plugin",
]
