"""Calling again what fails now and then, such as a request over a network."""

import functools
import time


def retry(attempts=3, exceptions=(Exception,), delay=0.0, backoff=2.0, sleep=time.sleep):
    """Make a function try again when it fails.

    The decorated function is called up to ``attempts`` times in all. When a
    call raises one of ``exceptions``, the wrapper waits and calls it again:
    ``delay`` seconds before the second attempt, and ``backoff`` times as long
    before each attempt after that. If the last attempt fails as well, its
    exception is raised. An exception of any other kind is raised at once.

    Args:
        attempts: How many times the function may be called, at least 1.
        exceptions: The exceptions that are worth another attempt.
        delay: How many seconds to wait before the second attempt.
        backoff: What each wait is multiplied by for the next one.
        sleep: What waits, ``time.sleep`` unless another is given.

    Raises:
        ValueError: ``attempts`` is less than 1.
    """
    if attempts < 1:
        raise ValueError(f"attempts must be at least 1, not {attempts}")

    def decorate(function):
        @functools.wraps(function)
        def wrapper(*args, **kwargs):
            wait = delay
            for attempt in range(1, attempts + 1):
                try:
                    return function(*args, **kwargs)
                except exceptions:
                    if attempt == attempts:
                        raise
                    sleep(wait)
                    wait *= backoff

        return wrapper

    return decorate
