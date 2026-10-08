"""The context: the handle through which code reaches the kernel."""

# pyright: reportPrivateUsage=false

from __future__ import annotations

from collections.abc import Callable

from tack.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState


class Context:
    """The handle through which code reaches the kernel.

    Everything registered through a context belongs to the fiber that owns the
    context and is undone when that fiber unloads. Calling ``Context()``
    creates the root context of a new application.
    """

    __slots__ = ("_kernel", "fiber")

    def __init__(self) -> None:
        self._kernel = KernelState()
        self.fiber = Fiber(self._kernel, None)
        self.fiber.ctx = self

    def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
        """Run ``setup`` now and undo it when the owning fiber unloads.

        ``setup`` performs a registration and returns the function that undoes
        it, or ``None`` when there is nothing to undo. The returned handle can
        undo the effect earlier.

        Raises:
            InactiveEffectError: The owning fiber is not loaded. ``setup`` is
                not run.
        """
        return self.fiber.effect(setup)
