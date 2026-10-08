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
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING, TypeAlias

from tack.kernel.errors import InactiveEffectError

if TYPE_CHECKING:
    from tack.kernel.context import Context

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


class KernelState:
    """State shared by every fiber and context of one application."""

    __slots__ = ("_uids", "drivers")

    def __init__(self) -> None:
        self._uids = itertools.count()
        self.drivers: set[asyncio.Task[None]] = set()

    def next_uid(self) -> int:
        """Return a fiber identifier that has not been used before."""
        return next(self._uids)


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
    """The owner of a set of effects.

    Disposing a fiber undoes its effects in reverse order of registration, one
    at a time, and waits for each to finish.
    """

    __slots__ = (
        "_disposed",
        "_driver",
        "_effects",
        "_kernel",
        "_loaded",
        "_wanted",
        "ctx",
        "parent",
        "state",
        "uid",
    )

    ctx: Context

    def __init__(self, kernel: KernelState, parent: Fiber | None) -> None:
        self.uid = kernel.next_uid()
        self.parent = parent
        self.state = FiberState.ACTIVE
        self._kernel = kernel
        self._effects: list[EffectHandle] = []
        self._wanted: Epoch | None = ()
        self._loaded: Epoch | None = ()
        self._driver: asyncio.Task[None] | None = None
        self._disposed = False

    @property
    def name(self) -> str:
        """A short label for logs and error messages."""
        return "root"

    def effect(self, setup: Callable[[], Disposer | None]) -> EffectHandle:
        """Run ``setup`` now and keep the disposer it returns.

        Raises:
            InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
        """
        if self.state not in (FiberState.LOADING, FiberState.ACTIVE):
            raise InactiveEffectError(
                f"cannot register an effect on {self.name!r} while it is {self.state.value}"
            )
        handle = EffectHandle(self, setup())
        self._effects.append(handle)
        return handle

    async def dispose(self) -> None:
        """Unload the fiber for good and wait until its effects are undone."""
        self._retire()
        await self.settled()

    async def settled(self) -> None:
        """Wait until no load or unload of this fiber is in flight."""
        while (driver := self._driver) is not None and driver is not asyncio.current_task():
            await asyncio.shield(driver)

    def _retire(self) -> None:
        if self._disposed:
            return
        self._disposed = True
        self._refresh()

    def _refresh(self) -> None:
        self._wanted = None if self._disposed else ()
        self._kick()

    def _needs_work(self) -> bool:
        return self._loaded is not None and self._loaded != self._wanted

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
                await self._unload()
        finally:
            driver = self._driver
            self._driver = None
            if driver is not None:
                self._kernel.drivers.discard(driver)
            self._rest()

    async def _unload(self) -> None:
        self._set_state(FiberState.UNLOADING)
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
        elif handle in effects:
            effects.remove(handle)

    def _rest(self) -> None:
        if self._loaded is not None:
            self._set_state(FiberState.ACTIVE)
        elif self._disposed:
            self._set_state(FiberState.DISPOSED)
        else:
            self._set_state(FiberState.PENDING)

    def _set_state(self, state: FiberState) -> None:
        self.state = state
