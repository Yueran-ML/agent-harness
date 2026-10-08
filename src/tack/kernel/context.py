"""The context: the handle through which code reaches the kernel."""

# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
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
from tack.kernel.plugin import resolve_plugin


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

    @classmethod
    def _owned_by(cls, kernel: KernelState, fiber: Fiber) -> Context:
        ctx = cls.__new__(cls)
        ctx._kernel = kernel
        ctx.fiber = fiber
        fiber.ctx = ctx
        return ctx

    def plugin(self, plugin: object, config: object = None) -> Fiber:
        """Load ``plugin`` as a child of the owning fiber.

        The plugin loads in the background, so this must be called while an
        event loop is running. Await the returned fiber to wait for the load
        and to receive its error if it fails. The child is disposed when the
        owning fiber unloads.

        Args:
            plugin: A :class:`Plugin`, a module or object with ``apply``, or a
                bare callable.
            config: The raw config. It is validated against the plugin's config
                model when the plugin has one.

        Raises:
            InvalidPluginError: ``plugin`` is not a plugin.
            InactiveEffectError: The owning fiber is not loaded.
        """
        spec = resolve_plugin(plugin)
        parent = self.fiber
        parent._require_loaded()
        fiber = Fiber(self._kernel, parent, spec, config)
        Context._owned_by(self._kernel, fiber)
        fiber._handle = parent.effect(lambda: fiber._dispose_owned)
        self._kernel.fibers[fiber.uid] = fiber
        fiber._refresh()
        return fiber

    def fibers(self) -> tuple[Fiber, ...]:
        """Return every plugin fiber that has not been disposed, oldest first."""
        return tuple(self._kernel.fibers.values())

    async def settled(self) -> None:
        """Wait until no plugin in the application is loading or unloading."""
        drivers = self._kernel.drivers
        current = asyncio.current_task()
        while pending := [driver for driver in drivers if driver is not current]:
            await asyncio.wait(pending)

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
