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
    Next,
    ParallelEvent,
    ParallelListener,
    PayloadT,
    ResultT,
    SerialEvent,
    SerialListener,
    WaterfallEvent,
    WaterfallListener,
    run_bail,
    run_emit,
    run_parallel,
    run_serial,
    run_waterfall,
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

    @overload
    def on(
        self,
        event: ParallelEvent[PayloadT],
        listener: ParallelListener[PayloadT],
        *,
        prepend: bool = False,
    ) -> EffectHandle: ...

    @overload
    def on(
        self,
        event: SerialEvent[PayloadT, ResultT],
        listener: SerialListener[PayloadT, ResultT],
        *,
        prepend: bool = False,
    ) -> EffectHandle: ...

    @overload
    def on(
        self,
        event: WaterfallEvent[PayloadT, ResultT],
        listener: WaterfallListener[PayloadT, ResultT],
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

    async def parallel(self, event: ParallelEvent[PayloadT], payload: PayloadT) -> None:
        """Run every listener of ``event`` concurrently and wait for all of them.

        A listener that raises is logged and does not stop the others.
        """
        await run_parallel(self._kernel.hooks.select(event, ParallelEvent), event, payload)

    async def serial(
        self, event: SerialEvent[PayloadT, ResultT], payload: PayloadT
    ) -> ResultT | None:
        """Await each listener in turn and return the first answer that is not ``None``.

        A listener that raises stops the dispatch and the error reaches the caller.
        """
        return await run_serial(self._kernel.hooks.select(event, SerialEvent), payload)

    async def waterfall(
        self,
        event: WaterfallEvent[PayloadT, ResultT],
        payload: PayloadT,
        final: Next[PayloadT, ResultT],
    ) -> ResultT:
        """Run the listeners of ``event`` as middleware around ``final``.

        The first listener is outermost. Each listener receives the payload and
        a ``next`` function that runs the rest of the chain; it may pass a
        different payload on, call ``next`` again to retry, or return without
        calling it to short-circuit. Errors reach the caller.
        """
        hooks = self._kernel.hooks.select(event, WaterfallEvent)
        return await run_waterfall(hooks, payload, final)
