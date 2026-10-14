"""A token bucket, to limit how often something may happen."""

import time
from collections.abc import Callable


class TokenBucket:
    """Allow bursts of up to `capacity` events, and then one event every `interval` seconds.

    The bucket starts full, and never holds more than `capacity` tokens. Each
    event takes a token. While the bucket is not full, it gains a token every
    `interval` seconds, the first one `interval` seconds after a token was
    taken from the full bucket. So with a capacity of 1 and an interval of 2
    seconds, events are allowed at 0, 2, 4, ... seconds.

    `clock` returns the current time in seconds. It is `time.monotonic`
    unless another one is given, as the tests do.
    """

    def __init__(
        self, capacity: int, interval: float, clock: Callable[[], float] = time.monotonic
    ) -> None:
        if capacity < 1:
            raise ValueError("capacity must be at least 1")
        if interval <= 0:
            raise ValueError("interval must be more than 0")
        self.capacity = capacity
        self.interval = interval
        self._clock = clock
        self._tokens = capacity
        self._next_token = clock()

    def _refill(self, now: float) -> None:
        while self._tokens < self.capacity and now > self._next_token:
            self._tokens += 1
            self._next_token += self.interval

    def available(self) -> int:
        """Return how many events are allowed right now."""
        self._refill(self._clock())
        return self._tokens

    def try_acquire(self) -> bool:
        """Take a token if there is one, and return whether the event is allowed."""
        now = self._clock()
        self._refill(now)
        if self._tokens == 0:
            return False
        if self._tokens == self.capacity:
            self._next_token = now + self.interval
        self._tokens -= 1
        return True

    def wait_time(self) -> float:
        """Return how many seconds to wait before an event is allowed, 0.0 if it is allowed now."""
        now = self._clock()
        self._refill(now)
        if self._tokens > 0:
            return 0.0
        return self._next_token - now
