"""The context: the handle through which code reaches the kernel."""

# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import Any, overload

from columbina.kernel.errors import (
    LifecycleError,
    ServiceConflictError,
    ServiceNotInjectedError,
    ServiceUnavailableError,
)
from columbina.kernel.events import (
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
from columbina.kernel.fiber import Disposer, EffectHandle, Fiber, KernelState, Provision
from columbina.kernel.plugin import resolve_plugin
from columbina.kernel.services import ServiceKey, ServiceT


class Context:
    """The handle through which code reaches the kernel.

    Everything registered through a context belongs to the fiber that owns the
    context and is undone when that fiber unloads. Calling ``Context()``
    creates the root context of a new application. The kernel is driven by
    asyncio and must be used from within a running event loop.

    Attributes:
        fiber: The fiber that owns what is registered through this context.
        scope_key: The object identifying the scope this context belongs to,
            or ``None`` outside every scope.
    """

    __slots__ = ("_kernel", "fiber", "scope_key")

    def __init__(self) -> None:
        self._kernel = KernelState()
        self.fiber = Fiber(self._kernel, None)
        self.fiber.ctx = self
        self.scope_key: object | None = None

    @classmethod
    def _owned_by(cls, kernel: KernelState, fiber: Fiber, scope_key: object | None) -> Context:
        ctx = cls.__new__(cls)
        ctx._kernel = kernel
        ctx.fiber = fiber
        ctx.scope_key = scope_key
        fiber.ctx = ctx
        return ctx

    def scope(self, key: object) -> Context:
        """Create a scope identified by ``key`` and return its context.

        Listeners registered through the returned context, and by plugins
        loaded through it, hear only the events dispatched in the same scope
        and those dispatched outside every scope. Events dispatched through it
        reach those listeners and the ones registered outside every scope. A
        scope created inside another scope replaces the key.

        The scope is disposed with the owning fiber, or earlier by disposing
        the ``fiber`` of the returned context.

        Raises:
            InactiveEffectError: The owning fiber is not loaded.
        """
        parent = self.fiber
        parent._require_loaded()
        fiber = Fiber(self._kernel, parent)
        scoped = Context._owned_by(self._kernel, fiber, key)
        self._own(fiber)
        return scoped

    def _own(self, child: Fiber) -> None:
        """Make the owning fiber dispose ``child`` when it unloads."""
        kernel = self._kernel
        child._handle = self.fiber.effect(
            lambda: child._until_disposed,
            waits_for=lambda caller: kernel.waits_for(child, caller),
        )

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
        Context._owned_by(self._kernel, fiber, self.scope_key)
        self._own(fiber)
        self._kernel.fibers[fiber.uid] = fiber
        fiber._refresh()
        return fiber

    def provide(self, key: ServiceKey[ServiceT], value: ServiceT) -> EffectHandle:
        """Offer ``value`` as the service named by ``key``.

        The service is withdrawn when the owning fiber unloads or the returned
        handle is disposed. Other fibers see it once the owning fiber is active.
        Plugins that inject the service load when it appears.

        Withdrawing makes the service unavailable at once and then waits for
        the plugins that inject it to unload. The name stays taken until they
        have, so a replacement can be provided only after the withdrawal has
        finished.

        Raises:
            ServiceConflictError: Another provider holds the name, or the name
                is still being withdrawn.
            InactiveEffectError: The owning fiber is not loaded.
        """
        kernel = self._kernel
        provision = Provision(key, value, self.fiber, kernel.next_uid())

        def withdraw() -> Awaitable[None] | None:
            provision.withdrawn = True
            kernel.notify(key)
            if not kernel.has_unsettled_dependents(key):
                del kernel.services[key]
                return None
            return release_the_name()

        async def release_the_name() -> None:
            try:
                await kernel.dependents_settled(key)
            finally:
                del kernel.services[key]

        def setup() -> Disposer:
            existing = kernel.services.get(key)
            if existing is not None:
                problem = (
                    "still being withdrawn by" if existing.withdrawn else "already provided by"
                )
                raise ServiceConflictError(
                    f"service {key.name!r} is {problem} {existing.fiber.name!r}"
                )
            kernel.services[key] = provision
            kernel.notify(key)
            return withdraw

        return self.fiber.effect(
            setup, waits_for=lambda caller: kernel.withdrawal_waits_for(key, caller)
        )

    def get(self, key: ServiceKey[ServiceT]) -> ServiceT | None:
        """Return the service named by ``key``, or ``None`` if it is not available.

        Use this for a service the caller can do without. Nothing unloads the
        caller when such a service goes away, so ask again each time instead of
        keeping the result.
        """
        provision = self._kernel.lookup(key, self.fiber)
        return None if provision is None else provision.value

    def use(self, key: ServiceKey[ServiceT]) -> ServiceT:
        """Return the service named by ``key``.

        Raises:
            ServiceNotInjectedError: The caller is inside a plugin, and neither
                that plugin nor an enclosing one injects or provides the service.
            ServiceUnavailableError: No active plugin provides the service.
        """
        kernel = self._kernel
        if not kernel.declared(key, self.fiber):
            raise ServiceNotInjectedError(
                f"{self.fiber.name!r} uses service {key.name!r} without declaring it in inject"
            )
        provision = kernel.lookup(key, self.fiber)
        if provision is None:
            raise ServiceUnavailableError(f"service {key.name!r} is not available")
        return provision.value

    def fibers(self) -> tuple[Fiber, ...]:
        """Return every plugin fiber that has not been disposed, oldest first."""
        return tuple(self._kernel.fibers.values())

    async def settled(self) -> None:
        """Wait until no plugin in the application is loading or unloading.

        Raises:
            LifecycleError: Called from an ``apply`` or a disposer, or from a
                task one of them started and is still running alongside. The
                caller would be waiting for itself, directly or through
                another plugin that waits for it.
        """
        kernel = self._kernel
        if kernel.lifecycle_fiber() is not None:
            raise LifecycleError(
                "settled() cannot be awaited while a plugin loads or unloads: "
                "the caller would be waiting for itself"
            )
        while True:
            for fiber in tuple(kernel.fibers.values()):
                # Carry on any work whose driver was cancelled part-way.
                fiber._kick()
            pending = [driver for driver in kernel.drivers if not driver.done()]
            if not pending:
                return
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
        hook = Hook(listener, self.scope_key)

        def setup() -> Disposer:
            hooks.add(event, hook, prepend=prepend)
            return lambda: hooks.remove(event, hook)

        return self.effect(setup)

    def _select(self, event: Event, kind: type[Event]) -> tuple[Hook, ...]:
        return self._kernel.hooks.select(event, kind, self.scope_key)

    def emit(self, event: EmitEvent[PayloadT], payload: PayloadT) -> None:
        """Notify every listener of ``event``.

        A listener that raises is logged and does not stop the others.
        """
        run_emit(self._select(event, EmitEvent), event, payload)

    def bail(self, event: BailEvent[PayloadT, ResultT], payload: PayloadT) -> ResultT | None:
        """Ask each listener in turn and return the first answer that is not ``None``.

        A listener that raises stops the dispatch and the error reaches the caller.
        """
        return run_bail(self._select(event, BailEvent), event, payload)

    async def parallel(self, event: ParallelEvent[PayloadT], payload: PayloadT) -> None:
        """Run every listener of ``event`` concurrently and wait for all of them.

        A listener that raises is logged and does not stop the others.
        """
        await run_parallel(self._select(event, ParallelEvent), event, payload)

    async def serial(
        self, event: SerialEvent[PayloadT, ResultT], payload: PayloadT
    ) -> ResultT | None:
        """Await each listener in turn and return the first answer that is not ``None``.

        A listener that raises stops the dispatch and the error reaches the caller.
        """
        return await run_serial(self._select(event, SerialEvent), payload)

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
        return await run_waterfall(self._select(event, WaterfallEvent), payload, final)
