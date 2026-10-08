"""Telling a cancellation of the current task from what merely looks like one.

Code that runs somebody else's coroutine has to answer two questions when
that coroutine ends badly. A ``CancelledError`` may mean that the current task
was cancelled, or that the coroutine cancelled something of its own. And an
ordinary error may have taken the place of a cancellation, as happens when a
cleanup fails while a cancellation is passing through it.

The answers come from counting how often the task has been asked to cancel.
Take the count with :func:`requests` before the operation, and compare after.
A count that is merely above zero says nothing: a task that swallowed a
cancellation long ago keeps it.
"""

import asyncio


def current_task() -> asyncio.Task[object]:
    """Return the task that is running."""
    task = asyncio.current_task()
    assert task is not None
    return task


def requests() -> int:
    """Return how often the current task has been asked to cancel so far."""
    return current_task().cancelling()


def requested_since(before: int) -> bool:
    """Tell whether the current task has been asked to cancel since the count was ``before``."""
    return current_task().cancelling() > before


def replaced(error: BaseException, before: int) -> bool:
    """Tell whether ``error`` was raised in place of a cancellation of the current task.

    That is the case if the task has been asked to cancel since the count was
    ``before``, and the error arose while a ``CancelledError`` was being
    handled. A request that has not been delivered yet does not count: asyncio
    delivers it by itself.
    """
    if not requested_since(before):
        return False
    earlier = error.__context__
    while earlier is not None:
        if isinstance(earlier, asyncio.CancelledError):
            return True
        earlier = earlier.__context__
    return False
