"""Fibers and the effects they own.

A fiber is the kernel's unit of ownership. Everything registered through a
context is recorded as an effect on the fiber that owns the context, and is
undone when that fiber unloads.
"""

# Fibers, effect handles and the shared kernel state reach into one another.
# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
import enum
import inspect
import itertools
import logging
from collections.abc import Awaitable, Callable, Generator, Iterator
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeAlias

from pydantic import ValidationError

from tack.kernel.errors import InactiveEffectError, InvalidPluginError
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
    """

    key: ServiceKey[Any]
    value: Any
    fiber: Fiber
    serial: int


class KernelState:
    """State shared by every fiber and context of one application."""

    __slots__ = ("_uids", "drivers", "fibers", "hooks", "services")

    def __init__(self) -> None:
        self._uids = itertools.count()
        self.drivers: set[asyncio.Task[None]] = set()
        self.fibers: dict[int, Fiber] = {}
        self.hooks = HookTable()
        self.services: dict[ServiceKey[Any], Provision] = {}

    def next_uid(self) -> int:
        """Return a fiber identifier that has not been used before."""
        return next(self._uids)

    def lookup(self, key: ServiceKey[Any], viewer: Fiber) -> Provision | None:
        """Return the provision of ``key`` if ``viewer`` can see it.

        A service is visible to everyone while its provider is active. The
        provider itself and the fibers below it also see the service while the
        provider is loading or unloading.
        """
        provision = self.services.get(key)
        if provision is None:
            return None
        provider = provision.fiber
        if provider.state is FiberState.ACTIVE:
            return provision
        if any(fiber is provider for fiber in viewer.lineage()):
            return provision
        return None

    def available(self, key: ServiceKey[Any]) -> Provision | None:
        """Return the provision of ``key`` if its provider is active."""
        provision = self.services.get(key)
        if provision is None or provision.fiber.state is not FiberState.ACTIVE:
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

    async def dependents_settled(self, key: ServiceKey[Any], provider: Fiber) -> None:
        """Wait until the fibers that inject ``key`` have finished changing state.

        The provider and the fibers above it are not waited for. One of them
        may be the very fiber whose unload is waiting here.
        """
        above = set(provider.lineage())
        for fiber in tuple(self.fibers.values()):
            if fiber.injects(key) and fiber not in above:
                await fiber.settled()

    async def all_dependents_settled(self, provider: Fiber) -> None:
        """Wait for the dependents of every service that ``provider`` provides."""
        for provision in tuple(self.services.values()):
            if provision.fiber is provider:
                await self.dependents_settled(provision.key, provider)


class EffectHandle:
    """A registered effect. Disposing the handle undoes the effect once."""

    __slots__ = ("_completion", "_disposer", "_fiber")

    def __init__(self, fiber: Fiber, disposer: Disposer | None) -> None:
        self._fiber = fiber
        self._disposer = disposer
        self._completion: asyncio.Future[None] | None = None

    @property
    def disposed(self) -> bool:
        """Whether the effect has been undone."""
        return self._completion is not None and self._completion.done()

    async def dispose(self) -> None:
        """Undo the effect.

        The first call runs the disposer and re-raises whatever it raises. A
        call that arrives later, or while the disposer is still running, waits
        for that one run and returns.
        """
        if self._completion is not None:
            await asyncio.shield(self._completion)
            return
        completion = self._completion = asyncio.get_running_loop().create_future()
        try:
            if self._disposer is not None:
                result = self._disposer()
                if inspect.isawaitable(result):
                    await result
        finally:
            self._fiber._forget(self)
            completion.set_result(None)


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
        """Unload the fiber for good and wait until its effects are undone."""
        if self._handle is None:
            self._retire()
        else:
            await self._handle.dispose()
        await self.settled()

    async def settled(self) -> None:
        """Wait until no load or unload of this fiber is in flight."""
        while (driver := self._driver) is not None and driver is not asyncio.current_task():
            await asyncio.shield(driver)

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

    async def _dispose_owned(self) -> None:
        self._retire()
        await self.settled()

    def _retire(self) -> None:
        if self._disposed:
            return
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

    def _kick(self) -> None:
        if self._driver is not None:
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
        except Exception as error:
            logger.error("plugin %r failed to load", self.name, exc_info=error)
            self.error = error
            self._failed = epoch
            await self._unload()
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
