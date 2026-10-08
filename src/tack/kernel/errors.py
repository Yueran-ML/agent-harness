"""Errors raised by the kernel."""


class KernelError(Exception):
    """Base class of every error the kernel raises deliberately."""


class InactiveEffectError(KernelError):
    """An effect was registered on a plugin instance that is not loaded."""


class LifecycleError(KernelError):
    """Code that runs while a plugin loads or unloads asked for something impossible there."""


class ServiceConflictError(KernelError):
    """A service was provided while another provider already holds its name."""


class ServiceUnavailableError(KernelError):
    """A service was requested while no active plugin provides it."""


class ServiceNotInjectedError(KernelError):
    """A plugin used a service that it did not declare in ``inject``."""


class EventModeError(KernelError, TypeError):
    """An event was dispatched or subscribed in a way its kind does not allow."""


class InvalidPluginError(KernelError, TypeError):
    """An object passed to ``Context.plugin`` is not a plugin."""


class CompositionError(KernelError):
    """A composition layer is malformed or cannot be applied."""
