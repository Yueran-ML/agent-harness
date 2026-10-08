"""Event declarations.

An event is declared once, as a module-level constant of one of five kinds. The
kind fixes how the event is dispatched. Every event carries a single payload.
"""

import asyncio
import inspect
import logging
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Any, ClassVar, Generic, TypeAlias, TypeVar

from tack.kernel.errors import EventModeError

logger = logging.getLogger("tack.kernel")

PayloadT = TypeVar("PayloadT")
ResultT = TypeVar("ResultT")


class Event:
    """Base class of the five event kinds."""

    __slots__ = ("name",)

    mode: ClassVar[str]

    def __init__(self, name: str) -> None:
        self.name = name

    def __repr__(self) -> str:
        return f"{type(self).__name__}({self.name!r})"


class EmitEvent(Event, Generic[PayloadT]):
    """A synchronous notification.

    Listeners run in order and return nothing. A listener that raises is logged
    and does not stop the others.
    """

    __slots__ = ()
    mode = "emit"


class ParallelEvent(Event, Generic[PayloadT]):
    """A notification whose listeners run concurrently and are all awaited.

    A listener that raises is logged and does not stop the others.
    """

    __slots__ = ()
    mode = "parallel"


class SerialEvent(Event, Generic[PayloadT, ResultT]):
    """A question asked of each listener in turn, awaiting each one.

    The first listener to return something other than ``None`` answers it.
    """

    __slots__ = ()
    mode = "serial"


class BailEvent(Event, Generic[PayloadT, ResultT]):
    """The synchronous form of :class:`SerialEvent`."""

    __slots__ = ()
    mode = "bail"


class WaterfallEvent(Event, Generic[PayloadT, ResultT]):
    """A chain of middleware around a final handler.

    Each listener receives the payload and a ``next`` function. Calling
    ``next`` runs the rest of the chain; returning without calling it
    short-circuits the chain.
    """

    __slots__ = ()
    mode = "waterfall"


EmitListener: TypeAlias = Callable[[PayloadT], None]
ParallelListener: TypeAlias = Callable[[PayloadT], Awaitable[None] | None]
SerialListener: TypeAlias = Callable[[PayloadT], Awaitable[ResultT | None] | ResultT | None]
BailListener: TypeAlias = Callable[[PayloadT], ResultT | None]
Next: TypeAlias = Callable[[PayloadT], Awaitable[ResultT]]
WaterfallListener: TypeAlias = Callable[[PayloadT, Next[PayloadT, ResultT]], Awaitable[ResultT]]


_PLAIN_LISTENER_MODES = frozenset({"emit", "bail"})


@dataclass(slots=True, eq=False)
class Hook:
    """One registered listener."""

    callback: Callable[..., Any]


class HookTable:
    """The listeners of every event, and the kind each event name belongs to."""

    __slots__ = ("_hooks", "_kinds")

    def __init__(self) -> None:
        self._hooks: dict[str, list[Hook]] = {}
        self._kinds: dict[str, type[Event]] = {}

    def declare(self, event: Event) -> None:
        """Record the kind of ``event``.

        Raises:
            EventModeError: The name is already in use by an event of another kind.
        """
        declared = self._kinds.setdefault(event.name, type(event))
        if declared is not type(event):
            raise EventModeError(
                f"event {event.name!r} is already declared as {declared.__name__}, "
                f"not {type(event).__name__}"
            )

    def add(self, event: Event, hook: Hook, *, prepend: bool) -> None:
        """Register ``hook`` after the existing listeners, or before them.

        Raises:
            EventModeError: The event name belongs to another kind, or the
                listener is a coroutine function and the event does not await
                its listeners.
        """
        self.declare(event)
        if event.mode in _PLAIN_LISTENER_MODES and inspect.iscoroutinefunction(hook.callback):
            raise EventModeError(
                f"{event!r} takes plain functions as listeners, not coroutine functions"
            )
        hooks = self._hooks.setdefault(event.name, [])
        if prepend:
            hooks.insert(0, hook)
        else:
            hooks.append(hook)

    def remove(self, event: Event, hook: Hook) -> None:
        """Unregister ``hook``."""
        hooks = self._hooks[event.name]
        hooks.remove(hook)
        if not hooks:
            del self._hooks[event.name]

    def select(self, event: Event, kind: type[Event]) -> tuple[Hook, ...]:
        """Return the listeners one dispatch of ``event`` should reach.

        The result is a snapshot: listeners added or removed while the dispatch
        runs do not change who it reaches.

        Raises:
            EventModeError: ``event`` is not of ``kind``.
        """
        if not isinstance(event, kind):
            raise EventModeError(f"{event!r} is dispatched with {event.mode}(), not {kind.mode}()")
        self.declare(event)
        return tuple(self._hooks.get(event.name, ()))


def run_emit(hooks: Sequence[Hook], event: Event, payload: object) -> None:
    """Call every listener, logging the ones that fail."""
    for hook in hooks:
        try:
            result = hook.callback(payload)
        except Exception:
            logger.exception("a listener of %r failed", event.name)
            continue
        if inspect.iscoroutine(result):
            result.close()
            logger.error(
                "a listener of %r returned a coroutine; %s listeners must be plain functions",
                event.name,
                event.mode,
            )


def run_bail(hooks: Sequence[Hook], payload: object) -> Any:
    """Return the first answer that is not ``None``."""
    for hook in hooks:
        result = hook.callback(payload)
        if result is not None:
            return result
    return None


async def run_parallel(hooks: Sequence[Hook], event: Event, payload: object) -> None:
    """Run every listener concurrently and wait for all, logging the ones that fail."""

    async def call(hook: Hook) -> None:
        try:
            result = hook.callback(payload)
            if inspect.isawaitable(result):
                await result
        except Exception:
            logger.exception("a listener of %r failed", event.name)

    await asyncio.gather(*(call(hook) for hook in hooks))


async def run_serial(hooks: Sequence[Hook], payload: object) -> Any:
    """Await each listener in turn and return the first answer that is not ``None``."""
    for hook in hooks:
        result = hook.callback(payload)
        if inspect.isawaitable(result):
            result = await result
        if result is not None:
            return result
    return None


async def run_waterfall(
    hooks: Sequence[Hook], payload: Any, final: Callable[[Any], Awaitable[Any]]
) -> Any:
    """Run the listeners as middleware around ``final``, outermost first."""

    async def step(index: int, value: Any) -> Any:
        if index == len(hooks):
            return await final(value)

        def following(later: Any) -> Awaitable[Any]:
            return step(index + 1, later)

        return await hooks[index].callback(value, following)

    return await step(0, payload)
