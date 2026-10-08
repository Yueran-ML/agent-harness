"""Event declarations.

An event is declared once, as a module-level constant of one of five kinds. The
kind fixes how the event is dispatched. Every event carries a single payload.
"""

from collections.abc import Awaitable, Callable
from typing import ClassVar, Generic, TypeAlias, TypeVar

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
