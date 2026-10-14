"""A small least-recently-used cache, and a decorator that memoizes a function with it."""

from collections import OrderedDict
from collections.abc import Callable, Hashable
from functools import wraps
from typing import Any

_MISSING = object()


class LRUCache:
    """A mapping that holds at most `capacity` entries.

    When an entry is added to a full cache, the entry that was used least
    recently is dropped. Reading an entry with `get` and writing it with `put`
    both count as using it; looking it up with `in` does not.
    """

    def __init__(self, capacity: int) -> None:
        if capacity < 1:
            raise ValueError("capacity must be at least 1")
        self.capacity = capacity
        self.hits = 0
        self.misses = 0
        self._entries: OrderedDict[Hashable, Any] = OrderedDict()

    def __len__(self) -> int:
        return len(self._entries)

    def __contains__(self, key: object) -> bool:
        return key in self._entries

    def keys(self) -> list[Hashable]:
        """Return the keys, from the least to the most recently used."""
        return list(self._entries)

    def get(self, key: Hashable, default: Any = None) -> Any:
        """Return the value stored under `key`, or `default` if there is none."""
        if key not in self._entries:
            self.misses += 1
            return default
        self.hits += 1
        self._entries.move_to_end(key)
        return self._entries[key]

    def put(self, key: Hashable, value: Any) -> None:
        """Store `value` under `key`, making room if the cache is full."""
        if key in self._entries:
            self._entries.move_to_end(key)
        self._entries[key] = value
        if len(self._entries) > self.capacity:
            self._entries.popitem()

    def clear(self) -> None:
        """Drop every entry, and reset the counts of hits and misses."""
        self._entries.clear()
        self.hits = 0
        self.misses = 0


def lru_cached(capacity: int) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Memoize a function of hashable positional arguments in an LRUCache.

    The cache is available as the `cache` attribute of the decorated function.
    """

    def decorate(function: Callable[..., Any]) -> Callable[..., Any]:
        cache = LRUCache(capacity)

        @wraps(function)
        def memoized(*args: Hashable) -> Any:
            value = cache.get(args, _MISSING)
            if value is _MISSING:
                value = function(*args)
                cache.put(args, value)
            return value

        memoized.cache = cache  # type: ignore[attr-defined]
        return memoized

    return decorate
