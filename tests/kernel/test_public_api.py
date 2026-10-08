"""The names the kernel exports. Changing this list is changing the public API."""

import tack.kernel

EXPORTS = {
    # Contexts, fibers and effects
    "Context",
    "Disposer",
    "EffectHandle",
    "Fiber",
    "FiberState",
    "FiberStateChange",
    "FIBER_STATE_CHANGED",
    # Plugins
    "Apply",
    "ApplyResult",
    "Plugin",
    "resolve_plugin",
    # Services
    "ServiceKey",
    # Registries
    "Registry",
    # Events
    "BailEvent",
    "BailListener",
    "EmitEvent",
    "EmitListener",
    "Event",
    "Next",
    "ParallelEvent",
    "ParallelListener",
    "SerialEvent",
    "SerialListener",
    "WaterfallEvent",
    "WaterfallListener",
    # Compositions
    "Change",
    "Composition",
    "Entry",
    "Insert",
    "Layer",
    "flatten",
    "import_plugin",
    "interpolate",
    "mount",
    "parse_layer",
    # Errors
    "CompositionError",
    "EventModeError",
    "InactiveEffectError",
    "InvalidPluginError",
    "KernelError",
    "LifecycleError",
    "RegistryConflictError",
    "ServiceConflictError",
    "ServiceNotInjectedError",
    "ServiceUnavailableError",
}


def test_the_kernel_exports_exactly_its_public_api() -> None:
    assert set(tack.kernel.__all__) == EXPORTS
    assert len(tack.kernel.__all__) == len(EXPORTS)


def test_every_exported_name_exists() -> None:
    missing = [name for name in tack.kernel.__all__ if not hasattr(tack.kernel, name)]

    assert missing == []
