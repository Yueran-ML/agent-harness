"""Fibers and the effects they own.

A fiber is the kernel's unit of ownership. Everything registered through a
context is recorded as an effect on the fiber that owns the context, and is
undone when that fiber unloads.
"""

# Fibers, effect handles and the shared kernel state reach into one another.
# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
import contextlib
import enum
import inspect
import itertools
import logging
from collections.abc import Awaitable, Callable, Generator, Iterator
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeAlias

from pydantic import ValidationError

from tack.kernel.errors import InactiveEffectError, InvalidPluginError, KernelError
from tack.kernel.events import HookTable
from tack.kernel.services import ServiceKey

if TYPE_CHECKING:
    from tack.kernel.context import Context
    from tack.kernel.plugin import Plugin

logger = logging.getLogger("tack.kernel")

Disposer: TypeAlias = Callable[[], Awaitable[None] | None]
"""Undoes one effect. It may be a plain function or a coroutine function."""

Epoch: TypeAlias = tuple[int, ...]
"""Identifies the set of providers a fiber is loaded against."""


def _being_cancelled() -> bool:
    """Return whether the current task itself has been asked to cancel.

    A ``CancelledError`` also surfaces when the awaited work was cancelled.
    This tells the two apart.
    """
    task = asyncio.current_task()
    return task is not None and task.cancelling() > 0


async def _drain(awaitable: Awaitable[None]) -> None:
    await awaitable


class FiberState(enum.Enum):
    """Where a fiber is in its lifecycle."""

    PENDING = "pending"
    LOADING = "loading"
    ACTIVE = "active"
    FAILED = "failed"
    UNLOADING = "unloading"
    DISPOSED = "disposed"


@dataclass(slots=True, eq=False)
class Provision:
    """A service value and the fiber that provides it.

    ``serial`` is unique to this act of providing. A dependent records the
    serials it loaded against, so it reloads whenever a service is provided
    anew, even by the same fiber.

    ``withdrawn`` is set when the provider starts to withdraw the service. The
    service is unavailable from then on, but the provision keeps its name
    until every dependent has finished unloading.
    """

    key: ServiceKey[Any]
    value: Any
    fiber: Fiber
    serial: int
    withdrawn: bool = False


class KernelState:
    """State shared by every fiber and context of one application."""

    __slots__ = ("_uids", "drivers", "fibers", "hooks", "services", "working")

    def __init__(self) -> None:
        self._uids = itertools.count()
        self.drivers: set[asyncio.Task[None]] = set()
        self.fibers: dict[int, Fiber] = {}
        self.hooks = HookTable()
        self.services: dict[ServiceKey[Any], Provision] = {}
        self.working: dict[asyncio.Task[Any], Fiber] = {}

    def next_uid(self) -> int:
        """Return a fiber identifier that has not been used before."""
        return next(self._uids)

    @contextlib.contextmanager
    def working_for(self, fiber: Fiber) -> Generator[None]:
        """Mark the current task as running the ``apply`` of ``fiber``."""
        task = asyncio.current_task()
        assert task is not None
        self.working[task] = fiber
        try:
            yield
        finally:
            del self.working[task]

    def in_lifecycle_code(self) -> bool:
        """Return whether the current task is running an ``apply`` or a disposer."""
        return asyncio.current_task() in self.working

    def inside(self, fiber: Fiber) -> bool:
        """Return whether the current task is loading or cleaning up ``fiber``.

        This is also true while it runs the ``apply`` or a disposer of a fiber
        below ``fiber``. Such code must not wait for ``fiber`` to settle: the
        fiber cannot settle before that code returns.
        """
        task = asyncio.current_task()
        worker = None if task is None else self.working.get(task)
        return worker is not None and any(above is fiber for above in worker.lineage())

    def lookup(self, key: ServiceKey[Any], viewer: Fiber) -> Provision | None:
        """Return the provision of ``key`` if ``viewer`` can see it.

        A service is visible to everyone while its provider is active. The
        provider itself and the fibers below it also see the service while the
        provider is loading or unloading. Nobody sees a service that is being
        withdrawn.
        """
        provision = self.services.get(key)
        if provision is None or provision.withdrawn:
            return None
        provider = provision.fiber
        if provider.state is FiberState.ACTIVE:
            return provision
        if any(fiber is provider for fiber in viewer.lineage()):
            return provision
        return None

    def available(self, key: ServiceKey[Any]) -> Provision | None:
        """Return the provision of ``key`` if dependents may load against it."""
        provision = self.services.get(key)
        if provision is None or provision.withdrawn:
            return None
        if provision.fiber.state is not FiberState.ACTIVE:
            return None
        return provision

    def declared(self, key: ServiceKey[Any], viewer: Fiber) -> bool:
        """Return whether ``viewer`` is entitled to rely on ``key``.

        A plugin may rely on a service that it provides itself, and on one that
        it or an enclosing plugin injects: the kernel unloads the plugin before
        an injected service goes away. Code outside every plugin may rely on
        any service.
        """
        provision = self.services.get(key)
        provider = None if provision is None else provision.fiber
        in_own_plugin = True
        inside_a_plugin = False
        for fiber in viewer.lineage():
            if in_own_plugin and fiber is provider:
                return True
            if fiber.plugin is not None:
                if key in fiber.plugin.inject:
                    return True
                in_own_plugin = False
                inside_a_plugin = True
        return not inside_a_plugin

    def notify(self, key: ServiceKey[Any]) -> None:
        """Re-evaluate every fiber that injects ``key``."""
        for fiber in tuple(self.fibers.values()):
            if fiber.injects(key):
                fiber._refresh()

    def announce(self, provider: Fiber) -> None:
        """Re-evaluate every fiber that injects a service of ``provider``."""
        for provision in tuple(self.services.values()):
            if provision.fiber is provider:
                self.notify(provision.key)

    def has_unsettled_dependents(self, key: ServiceKey[Any]) -> bool:
        """Return whether a fiber that injects ``key`` is loading or unloading."""
        return any(
            fiber.injects(key) and fiber._live_driver() is not None
            for fiber in self.fibers.values()
        )

    async def dependents_settled(self, key: ServiceKey[Any]) -> None:
        """Wait until the fibers that inject ``key`` have finished changing state.

        Call this only while ``key`` is unavailable. A dependent then has
        nothing to load against, so waiting for it to settle is waiting for it
        to unload.
        """
        for fiber in tuple(self.fibers.values()):
            if fiber.injects(key):
                await fiber.settled()

    async def all_dependents_settled(self, provider: Fiber) -> None:
        """Wait for the dependents of every service that ``provider`` holds.

        This includes services the provider has begun to withdraw and whose
        dependents are still unloading.
        """
        for provision in tuple(self.services.values()):
            if provision.fiber is provider:
                await self.dependents_settled(provision.key)


class EffectHandle:
    """A registered effect. Disposing the handle undoes the effect once.

    A disposer that returns an awaitable is driven by a task of its own, so
    that cancelling whoever asked for the disposal does not interrupt the
    cleanup.
    """

    __slots__ = ("_disposer", "_done", "_fiber", "_pending")

    def __init__(self, fiber: Fiber, disposer: Disposer | None) -> None:
        self._fiber = fiber
        self._disposer = disposer
        self._pending: asyncio.Task[None] | None = None
        self._done = False

    @property
    def disposed(self) -> bool:
        """Whether the effect has been undone."""
        return self._done

    async def dispose(self) -> None:
        """Undo the effect and wait until that is finished.

        Only the first call runs the disposer. A caller that waits for it
        receives the error it raises, if any. A call made after it has finished
        returns at once, and so does a call made from inside the disposer.
        """
        self._begin()
        pending = self._pending
        if pending is None or pending is asyncio.current_task():
            return
        if pending.done():
            self._settle(pending)
            return
        await asyncio.shield(pending)

    def _begin(self) -> None:
        """Start undoing the effect without waiting for it."""
        if self._done or self._pending is not None:
            return
        disposer, self._disposer = self._disposer, None
        result: Awaitable[None] | None = None
        try:
            if disposer is not None:
                result = disposer()
        except BaseException:
            self._finish()
            raise
        if not inspect.isawaitable(result):
            self._finish()
            return
        fiber = self._fiber
        coroutine = result if inspect.iscoroutine(result) else _drain(result)
        task = asyncio.get_running_loop().create_task(coroutine, name=f"tack-effect-{fiber.uid}")
        self._pending = task
        fiber._kernel.working[task] = fiber
        task.add_done_callback(self._settle)

    def _settle(self, task: asyncio.Task[None]) -> None:
        self._fiber._kernel.working.pop(task, None)
        self._finish()

    def _finish(self) -> None:
        if not self._done:
            self._done = True
            self._fiber._forget(self)


class Fiber:
    """One instance of a plugin, and the owner of everything it registered.

    A fiber loads by calling its plugin's ``apply`` and unloads by undoing the
    effects registered since, in reverse order and one at a time. Loading and
    unloading run in the background; ``await fiber`` waits for them and raises
    the error of a failed load.

    The root fiber and scope fibers have no plugin. They are active from the
    moment they are created until they are disposed.

    Attributes:
        uid: A number unique within the application.
        parent: The fiber whose context created this one, or ``None`` for the root.
        plugin: The plugin this fiber is an instance of, if any.
        config: The validated config handed to ``apply``.
        error: Why the most recent load failed, if it did.
        state: Where the fiber is in its lifecycle.
        ctx: The context whose registrations this fiber owns.
    """

    __slots__ = (
        "_broken",
        "_disposed",
        "_driver",
        "_effects",
        "_failed",
        "_handle",
        "_kernel",
        "_loaded",
        "_wanted",
        "config",
        "ctx",
        "error",
        "parent",
        "plugin",
        "state",
        "uid",
    )

    ctx: Context

    def __init__(
        self,
        kernel: KernelState,
        parent: Fiber | None,
        plugin: Plugin | None = None,
        config: object = None,
    ) -> None:
        self.uid = kernel.next_uid()
        self.parent = parent
        self.plugin = plugin
        self.config: Any = config
        self.error: Exception | None = None
        self.state = FiberState.PENDING
        self._kernel = kernel
        self._effects: list[EffectHandle] = []
        self._wanted: Epoch | None = None
        self._loaded: Epoch | None = None
        self._failed: Epoch | None = None
        self._driver: asyncio.Task[None] | None = None
        self._handle: EffectHandle | None = None
        self._disposed = False
        self._broken = False
        if plugin is None:
            self._wanted = self._loaded = ()
            self.state = FiberState.ACTIVE
        elif plugin.config is not None:
            try:
                self.config = plugin.config.model_validate({} if config is None else config)
            except ValidationError as error:
                logger.error("plugin %r has an invalid config: %s", plugin.name, error)
                self.error = error
                self._broken = True

    @property
    def name(self) -> str:
        """A short label for logs and error messages."""
        if self.plugin is not None:
            return self.plugin.name
        return "root" if self.parent is None else "scope"

    def __repr__(self) -> str:
        return f"<Fiber {self.name!r} #{self.uid} {self.state.value}>"

    def lineage(self) -> Iterator[Fiber]:
        """Yield this fiber, then its parent, and so on up to the root."""
        fiber: Fiber | None = self
        while fiber is not None:
            yield fiber
            fiber = fiber.parent

    def injects(self, key: ServiceKey[Any]) -> bool:
        """Return whether this fiber's plugin requires ``key``."""
        return self.plugin is not None and key in self.plugin.inject

    @property
    def missing(self) -> tuple[ServiceKey[Any], ...]:
        """The injected services that are not available right now."""
        inject = () if self.plugin is None else self.plugin.inject
        available = self._kernel.available
        return tuple(key for key in inject if available(key) is None)

    def __await__(self) -> Generator[Any, None, Fiber]:
        return self._ready().__await__()

    def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
        """Run ``setup`` now and keep the disposer it returns.

        Raises:
            InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
        """
        self._require_loaded()
        handle = EffectHandle(self, setup())
        self._effects.append(handle)
        return handle

    async def dispose(self) -> None:
        """Unload the fiber for good and wait until its effects are undone.

        Cancelling the caller does not interrupt the unload. Called from the
        fiber's own ``apply`` or disposers, or from those of a fiber below it,
        this requests the disposal and returns: the fiber cannot finish
        unloading before that code returns.
        """
        self._retire()
        handle = self._handle
        if self._kernel.inside(self):
            if handle is not None:
                handle._begin()
            return
        if handle is None:
            await self._until_disposed()
        else:
            await handle.dispose()

    async def settled(self) -> None:
        """Wait until no load or unload of this fiber is in flight.

        Cancelling the caller does not interrupt the load or unload. Called
        from the fiber's own ``apply`` or disposers, or from those of a fiber
        below it, this returns at once.
        """
        if self._kernel.inside(self):
            return
        while (driver := self._live_driver()) is not None:
            try:
                await asyncio.shield(driver)
            except asyncio.CancelledError:
                if _being_cancelled():
                    raise

    async def _ready(self) -> Fiber:
        await self.settled()
        if self.state is FiberState.FAILED and self.error is not None:
            raise self.error
        return self

    def _require_loaded(self) -> None:
        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
            raise InactiveEffectError(
                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
            )

    async def _until_disposed(self) -> None:
        self._retire()
        while True:
            # Start a driver again if an earlier one was cancelled part-way.
            self._kick()
            if self.state is FiberState.DISPOSED:
                return
            await self.settled()

    def _retire(self) -> None:
        self._disposed = True
        self._refresh()

    def _compute_epoch(self) -> Epoch | None:
        if self._disposed or self._broken:
            return None
        serials: list[int] = []
        for key in () if self.plugin is None else self.plugin.inject:
            provision = self._kernel.available(key)
            if provision is None:
                return None
            serials.append(provision.serial)
        return tuple(serials)

    def _refresh(self) -> None:
        wanted = self._compute_epoch()
        if wanted != self._wanted:
            self._wanted = wanted
            self._failed = None
            self.error = None
        self._kick()

    def _needs_work(self) -> bool:
        if self._loaded is not None:
            return self._loaded != self._wanted
        return self._wanted is not None and self._wanted != self._failed

    def _live_driver(self) -> asyncio.Task[None] | None:
        driver = self._driver
        if driver is not None and driver.done():
            # Cancelled before it could run, so it never cleared itself.
            self._driver = None
            self._kernel.drivers.discard(driver)
            return None
        return driver

    def _kick(self) -> None:
        if self._live_driver() is not None:
            return
        if not self._needs_work():
            self._rest()
            return
        loop = asyncio.get_running_loop()
        driver = loop.create_task(self._drive(), name=f"tack-fiber-{self.uid}")
        self._driver = driver
        self._kernel.drivers.add(driver)

    async def _drive(self) -> None:
        try:
            while self._needs_work():
                if self._loaded is not None:
                    await self._unload()
                else:
                    await self._load()
            self._rest()
        finally:
            driver = self._driver
            self._driver = None
            if driver is not None:  # pragma: no branch - always set while driving
                self._kernel.drivers.discard(driver)

    async def _load(self) -> None:
        plugin = self.plugin
        epoch = self._wanted
        assert plugin is not None
        assert epoch is not None
        self._set_state(FiberState.LOADING)
        self._loaded = epoch
        try:
            with self._kernel.working_for(self):
                result = plugin.apply(self.ctx, self.config)
                if inspect.isawaitable(result):
                    result = await result
            if result is not None:
                returned: object = result
                if not callable(returned):
                    raise InvalidPluginError(
                        f"plugin {self.name!r} returned {returned!r}; "
                        "apply must return a disposer or None"
                    )
                self._effects.append(EffectHandle(self, result))
        except (Exception, asyncio.CancelledError) as error:
            interrupted = isinstance(error, asyncio.CancelledError) and _being_cancelled()
            logger.error("plugin %r failed to load", self.name, exc_info=error)
            if isinstance(error, Exception):
                self.error = error
            else:
                self.error = KernelError(f"plugin {self.name!r} was cancelled while loading")
            self._failed = epoch
            await self._unload()
            if interrupted:
                self._rest()
                raise
            return
        if self._wanted == epoch:
            self._set_state(FiberState.ACTIVE)

    async def _unload(self) -> None:
        self._set_state(FiberState.UNLOADING)
        await self._kernel.all_dependents_settled(self)
        await self._undo_effects()
        self._loaded = None

    async def _undo_effects(self) -> None:
        while self._effects:
            try:
                await self._effects[-1].dispose()
            except asyncio.CancelledError:
                if _being_cancelled():
                    raise
                logger.error("an effect of %r was cancelled while cleaning up", self.name)
            except Exception:
                logger.exception("an effect of %r failed to clean up", self.name)

    def _forget(self, handle: EffectHandle) -> None:
        effects = self._effects
        if effects and effects[-1] is handle:
            effects.pop()
        elif handle in effects:  # pragma: no branch - a handle is forgotten once
            effects.remove(handle)

    def _rest(self) -> None:
        if self._loaded is not None:
            self._set_state(FiberState.ACTIVE)
        elif self._disposed:
            self._kernel.fibers.pop(self.uid, None)
            self._set_state(FiberState.DISPOSED)
        elif self.error is not None:
            self._set_state(FiberState.FAILED)
        else:
            self._set_state(FiberState.PENDING)

    def _set_state(self, state: FiberState) -> None:
        previous = self.state
        if previous is state:
            return
        self.state = state
        if FiberState.ACTIVE in (previous, state):
            self._kernel.announce(self)
