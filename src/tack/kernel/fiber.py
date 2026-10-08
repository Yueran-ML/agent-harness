"""Fibers and the effects they own.

A fiber is the kernel's unit of ownership. Everything registered through a
context is recorded as an effect on the fiber that owns the context, and is
undone when that fiber unloads.

Loads, unloads and asynchronous cleanups run in tasks that belong to the
kernel. Cancelling a caller that waits for one of them never interrupts it. If
one of those tasks is itself cancelled, as ``asyncio.run`` does on exit to
whatever is still running, the fiber keeps a truthful state, and the next
caller that waits for it resumes the interrupted work. One thing cannot be
resumed: a cleanup that had already started when its task was cancelled. It is
reported in the log and counts as done.
"""

# Fibers, effect handles and the shared kernel state reach into one another.
# pyright: reportPrivateUsage=false

from __future__ import annotations

import asyncio
import contextvars
import enum
import inspect
import itertools
import logging
import weakref
from collections.abc import Awaitable, Callable, Coroutine, Generator, Iterator
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeAlias, cast

from tack.kernel.errors import InactiveEffectError, InvalidPluginError, KernelError
from tack.kernel.events import EmitEvent, HookTable, run_emit
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


def _spawn(
    coroutine: Coroutine[Any, Any, None], name: str, mark: Lifecycle | None = None
) -> asyncio.Task[None]:
    """Start a task that belongs to the kernel.

    The task runs under ``mark``, or under no lifecycle mark when none is
    given. It must not inherit the mark of whatever code happened to start it:
    a driver started from inside a disposer is not part of that disposer.

    The task is created directly instead of through the loop's task factory,
    so it never starts to run before this function returns. The kernel records
    each task right after creating it and relies on that order.
    """
    context = contextvars.copy_context()
    context.run(_lifecycle.set, mark)
    return asyncio.Task(coroutine, loop=asyncio.get_running_loop(), name=name, context=context)


class FiberState(enum.Enum):
    """Where a fiber is in its lifecycle."""

    PENDING = "pending"
    LOADING = "loading"
    ACTIVE = "active"
    FAILED = "failed"
    UNLOADING = "unloading"
    DISPOSED = "disposed"


@dataclass(frozen=True, slots=True)
class FiberStateChange:
    """The payload of :data:`FIBER_STATE_CHANGED`."""

    fiber: Fiber
    previous: FiberState
    current: FiberState


FIBER_STATE_CHANGED: EmitEvent[FiberStateChange] = EmitEvent("kernel/fiber-state-changed")
"""Emitted, outside every scope, each time a fiber enters a new state."""


@dataclass(slots=True, eq=False)
class Lifecycle:
    """Marks code as the ``apply`` or a disposer of ``fiber``.

    The mark is kept in a context variable, so a task started by that code
    inherits it. ``running`` turns false once the ``apply`` or the disposer has
    finished: a task that outlives it is ordinary code again.

    The marked code may hand the kernel a future or a task of its own to wait
    for. ``awaited`` is that future. The mark stops counting the moment it is
    done, without waiting for the kernel to notice.
    """

    fiber: Fiber
    running: bool = True
    awaited: asyncio.Future[Any] | None = None

    @property
    def live(self) -> bool:
        """Whether the code this mark describes is still running."""
        return self.running and not (self.awaited is not None and self.awaited.done())

    def adopt(self, awaitable: object) -> None:
        """Extend the mark to a future or task that the marked code returned.

        A task that already existed runs in a context of its own, so it cannot
        inherit the mark. It is looked up by identity instead.
        """
        if isinstance(awaitable, asyncio.Future):
            self.awaited = cast("asyncio.Future[Any]", awaitable)
        if isinstance(awaitable, asyncio.Task):
            _adopted[cast("asyncio.Task[Any]", awaitable)] = self


_lifecycle: contextvars.ContextVar[Lifecycle | None] = contextvars.ContextVar(
    "tack-lifecycle", default=None
)

_adopted: weakref.WeakKeyDictionary[asyncio.Task[Any], Lifecycle] = weakref.WeakKeyDictionary()


def _current_mark() -> Lifecycle | None:
    """Return the live lifecycle mark of the calling code, if it has one."""
    task = asyncio.current_task()
    adopted = None if task is None else _adopted.get(task)
    if adopted is not None and adopted.live:
        return adopted
    mark = _lifecycle.get()
    return mark if mark is not None and mark.live else None


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

    def lifecycle_fiber(self) -> Fiber | None:
        """Return the fiber whose ``apply`` or disposer the caller is part of, if any."""
        mark = _current_mark()
        if mark is None or mark.fiber._kernel is not self:
            return None
        return mark.fiber

    def waits_for(self, target: Fiber, caller: Fiber) -> bool:
        """Return whether ``target`` cannot settle before ``caller`` has.

        To unload, a fiber waits for the fibers below it, and for the fibers
        that inject a service held by it or by a fiber below it. Each of those
        waits in the same way for others.
        """
        caller_and_above = set(caller.lineage())
        seen = {target}
        waited_for = [target]
        while waited_for:
            member = waited_for.pop()
            if member in caller_and_above:
                return True
            for provision in self.services.values():
                if all(holder is not member for holder in provision.fiber.lineage()):
                    continue
                for fiber in self.fibers.values():
                    if fiber not in seen and fiber.injects(provision.key):
                        seen.add(fiber)
                        waited_for.append(fiber)
        return False

    def would_wait_for_itself(self, target: Fiber) -> bool:
        """Return whether the caller is code that ``target`` waits for.

        Such code must not wait for ``target`` in turn: neither would finish.
        """
        caller = self.lifecycle_fiber()
        return caller is not None and self.waits_for(target, caller)

    def withdrawal_waits_for(self, key: ServiceKey[Any], caller: Fiber) -> bool:
        """Return whether withdrawing ``key`` cannot finish before ``caller`` has settled."""
        return any(
            fiber.injects(key) and self.waits_for(fiber, caller) for fiber in self.fibers.values()
        )

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
        """Return whether a fiber that injects ``key`` still has loading or unloading to do."""
        return any(fiber.injects(key) and not fiber._at_rest() for fiber in self.fibers.values())

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

    __slots__ = (
        "_awaitable",
        "_begun",
        "_disposer",
        "_done",
        "_entered",
        "_fiber",
        "_mark",
        "_pending",
        "_waits_for",
    )

    def __init__(
        self,
        fiber: Fiber,
        disposer: Disposer | None,
        waits_for: Callable[[Fiber], bool] | None = None,
    ) -> None:
        self._fiber = fiber
        self._disposer = disposer
        self._waits_for = waits_for
        self._mark = Lifecycle(fiber)
        self._awaitable: Awaitable[None] | None = None
        self._pending: asyncio.Task[None] | None = None
        self._begun = False
        self._entered = False
        self._done = False

    @property
    def disposed(self) -> bool:
        """Whether the effect has been undone."""
        return self._done

    async def dispose(self) -> None:
        """Undo the effect and wait until that is finished.

        Only the first call runs the disposer. A caller that waits for it
        receives the error it raises, if any. A call made after it has finished
        returns at once.

        An ``apply`` or a disposer that this cleanup itself waits for cannot
        wait for it in turn. Called from such code, this starts the cleanup and
        returns. That is the case for the disposer itself, and for the code of
        a plugin whose unloading the cleanup waits for: withdrawing a service
        waits for the plugins that inject it.
        """
        self._begin()
        if not self._caller_is_waited_for():
            await self._join()

    def _caller_is_waited_for(self) -> bool:
        mark = _current_mark()
        if mark is None:
            return False
        if mark is self._mark:
            return True
        return self._waits_for is not None and self._waits_for(mark.fiber)

    async def _join(self) -> None:
        """Wait until the effect is undone, whoever is asking."""
        while not self._done:
            pending = self._pending
            if pending is None:  # pragma: no cover - needs an eagerly started, unmarked task
                # Asked while the disposer is still being called.
                return
            if pending.done():
                if self._entered:
                    self._finish()
                    return
                # Cancelled before its first step: the cleanup has yet to run.
                pending = self._start()
            await asyncio.shield(pending)

    def _begin(self) -> None:
        """Start undoing the effect without waiting for it."""
        if self._begun:
            return
        self._begun = True
        disposer, self._disposer = self._disposer, None
        token = _lifecycle.set(self._mark)
        try:
            result = None if disposer is None else disposer()
        except BaseException:
            self._finish()
            raise
        finally:
            _lifecycle.reset(token)
        if inspect.isawaitable(result):
            self._mark.adopt(result)
            self._awaitable = result
            self._start()
        else:
            self._finish()

    def _start(self) -> asyncio.Task[None]:
        awaitable = self._awaitable
        assert awaitable is not None
        task = _spawn(self._run(awaitable), f"tack-effect-{self._fiber.uid}", self._mark)
        self._pending = task
        task.add_done_callback(self._settle)
        return task

    async def _run(self, awaitable: Awaitable[None]) -> None:
        self._entered = True
        try:
            await awaitable
        finally:
            # A task the disposer started must be ordinary code from here on,
            # even if it runs before this task is seen to have finished.
            self._mark.running = False

    def _settle(self, task: asyncio.Task[None]) -> None:
        if not self._entered:
            return
        if task.cancelled():
            logger.error("a cleanup of %r was cancelled before it finished", self._fiber.name)
        self._finish()

    def _finish(self) -> None:
        if self._done:
            return
        self._done = True
        self._awaitable = None
        self._mark.running = False
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
        plugin: Plugin[Any] | None = None,
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
            except Exception as error:
                # Usually a ValidationError, but a validator may raise anything.
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

    def effect(
        self,
        setup: Callable[[], Disposer | None],
        *,
        waits_for: Callable[[Fiber], bool] | None = None,
    ) -> EffectHandle:
        """Run ``setup`` now and keep the disposer it returns.

        Args:
            setup: Performs a registration and returns what undoes it.
            waits_for: For the kernel's own effects: tells whether undoing
                this one waits for a given fiber to settle.

        Raises:
            InactiveEffectError: The fiber is not loaded. ``setup`` is not run.
        """
        self._require_loaded()
        handle = EffectHandle(self, setup(), waits_for)
        self._effects.append(handle)
        return handle

    async def dispose(self) -> None:
        """Unload the fiber for good and wait until its effects are undone.

        Cancelling the caller does not interrupt the unload.

        An ``apply`` or a disposer that the fiber's unload waits for cannot
        wait for that unload in turn. Called from such code, this requests the
        disposal and returns. That is the case for the fiber's own code, for
        the code of fibers below it, and for the code of plugins that inject a
        service the fiber or a fiber below it provides.
        """
        self._retire()
        handle = self._handle
        if handle is not None:
            await handle.dispose()
        elif not self._kernel.would_wait_for_itself(self):
            await self._until_disposed()

    async def settled(self) -> None:
        """Wait until this fiber has no loading or unloading left to do.

        Cancelling the caller does not interrupt the load or unload. Called
        from code that the fiber itself waits for (see :meth:`dispose`), this
        returns at once.
        """
        if self._kernel.would_wait_for_itself(self):
            return
        while True:
            driver = self._live_driver()
            if driver is None:
                if not self._needs_work():
                    return
                # A driver was cancelled part-way. Carry on from where it stopped.
                self._kick()
                continue
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

    def _at_rest(self) -> bool:
        return self._live_driver() is None and not self._needs_work()

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
        driver = _spawn(self._drive(), f"tack-fiber-{self.uid}")
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
        self._loaded = epoch
        try:
            self._set_state(FiberState.LOADING)
            result = await self._apply(plugin)
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
            if interrupted:
                # This request has been received. Take it off the count, so
                # that the rollback can tell a second request from it.
                task = asyncio.current_task()
                assert task is not None
                task.uncancel()
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

    async def _apply(self, plugin: Plugin[Any]) -> Disposer | None:
        mark = Lifecycle(self)
        token = _lifecycle.set(mark)
        try:
            result = plugin.apply(self.ctx, self.config)
            if inspect.isawaitable(result):
                mark.adopt(result)
                result = await result
            return result
        finally:
            mark.running = False
            _lifecycle.reset(token)

    async def _unload(self) -> None:
        self._set_state(FiberState.UNLOADING)
        await self._kernel.all_dependents_settled(self)
        await self._undo_effects()
        self._loaded = None

    async def _undo_effects(self) -> None:
        while self._effects:
            handle = self._effects[-1]
            try:
                handle._begin()
                await handle._join()
            except asyncio.CancelledError:
                if _being_cancelled():
                    raise
                if handle._pending is None:
                    # A task reports its own cancellation when it settles.
                    logger.error("a cleanup of %r was cancelled before it finished", self.name)
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
        listeners = self._kernel.hooks.select(FIBER_STATE_CHANGED, EmitEvent)
        run_emit(listeners, FIBER_STATE_CHANGED, FiberStateChange(self, previous, state))
