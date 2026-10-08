"""The context: the handle through which code reaches the kernel."""

# pyright: reportPrivateUsage=false

from __future__ import annotations

from collections.abc import Callable
from typing import Any, overload

from tack.kernel.events import (
    BailEvent,
    BailListener,
    EmitEvent,
    EmitListener,
    Event,
    Hook,
    PayloadT,
    ResultT,
    run_bail,
    run_emit,
)
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

    @overload
    def on(
        self, event: EmitEvent[PayloadT], listener: EmitListener[PayloadT], *, prepend: bool = False
    ) -> EffectHandle: ...

    @overload
    def on(
        self,
        event: BailEvent[PayloadT, ResultT],
        listener: BailListener[PayloadT, ResultT],
        *,
        prepend: bool = False,
    ) -> EffectHandle: ...

    def on(
        self, event: Event, listener: Callable[..., Any], *, prepend: bool = False
    ) -> EffectHandle:
        """Listen to ``event`` until the owning fiber unloads.

        Listeners run in registration order. ``prepend`` puts this listener
        ahead of those already registered.

        Raises:
            EventModeError: The listener does not fit the kind of the event, or
                the event name is already declared as another kind.
            InactiveEffectError: The owning fiber is not loaded.
        """
        hooks = self._kernel.hooks
        hook = Hook(listener)

        def setup() -> Disposer:
            hooks.add(event, hook, prepend=prepend)
            return lambda: hooks.remove(event, hook)

        return self.effect(setup)

    def emit(self, event: EmitEvent[PayloadT], payload: PayloadT) -> None:
        """Notify every listener of ``event``.

        A listener that raises is logged and does not stop the others.
        """
        run_emit(self._kernel.hooks.select(event, EmitEvent), event, payload)

    def bail(self, event: BailEvent[PayloadT, ResultT], payload: PayloadT) -> ResultT | None:
        """Ask each listener in turn and return the first answer that is not ``None``.

        A listener that raises stops the dispatch and the error reaches the caller.
        """
        return run_bail(self._kernel.hooks.select(event, BailEvent), payload)
