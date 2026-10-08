"""Named entries that come and go with the plugins that add them."""

from collections.abc import Iterator, Mapping
from typing import TypeVar

from tack.kernel.context import Context
from tack.kernel.errors import RegistryConflictError
from tack.kernel.fiber import Disposer, EffectHandle

ValueT = TypeVar("ValueT")


class Registry(Mapping[str, ValueT]):
    """Named entries, each owned by the plugin instance that added it.

    A registry reads like a mapping from names to values, in the order the
    entries were added. The only way to add an entry is :meth:`add`, which ties
    the entry to a fiber, so the entry leaves when that fiber unloads. Two
    registries are equal only if they are the same registry.

    Args:
        noun: What an entry is called in error messages, such as ``"tool"``.
    """

    __slots__ = ("_entries", "_noun")

    def __init__(self, noun: str) -> None:
        self._entries: dict[str, ValueT] = {}
        self._noun = noun

    def add(self, ctx: Context, name: str, value: ValueT) -> EffectHandle:
        """Add an entry that belongs to the fiber of ``ctx``.

        Returns:
            A handle that removes the entry before its fiber unloads.

        Raises:
            RegistryConflictError: The name is taken.
            InactiveEffectError: The fiber of ``ctx`` is not loaded.
        """
        entries = self._entries

        def remove() -> None:
            del entries[name]

        def setup() -> Disposer:
            if name in entries:
                raise RegistryConflictError(f"the {self._noun} {name!r} is already registered")
            entries[name] = value
            return remove

        return ctx.effect(setup)

    def __getitem__(self, name: str) -> ValueT:
        return self._entries[name]

    def __iter__(self) -> Iterator[str]:
        return iter(self._entries)

    def __len__(self) -> int:
        return len(self._entries)

    def __eq__(self, other: object) -> bool:
        return self is other

    def __hash__(self) -> int:
        return id(self)

    def __repr__(self) -> str:
        return f"Registry({self._noun!r}, {list(self._entries)!r})"
