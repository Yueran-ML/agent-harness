"""Telling a cancellation of the current task from what merely looks like one.

Code that runs somebody else's coroutine has to answer two questions when
that coroutine ends badly. A ``CancelledError`` may mean that the current task
was cancelled, or that the coroutine cancelled something of its own. And an
ordinary error may have taken the place of a cancellation, as happens when a
cleanup fails while a cancellation is passing through it.

The answers come from counting how often the task has been asked to cancel.
Take the count before the operation, and compare after. A count that is merely
above zero says nothing: a task that swallowed a cancellation long ago keeps
it.

One more case has to be kept apart. A request that was made while the task was
running has not been delivered yet. It arrives where the task next waits, and
it is as real as any other. So the count is taken with
:func:`settled_requests`, which lets such a request arrive first.

And the coroutine may end well although the task was cancelled meanwhile: it
caught the cancellation and went on. :func:`kept_back` raises the cancellation
then. :func:`guard` does all of this for one awaitable.
"""

import asyncio
import logging
from collections.abc import Awaitable
from typing import TypeVar

ResultT = TypeVar("ResultT")

logger = logging.getLogger("columbina.cancellation")


class CancelledFromInside(Exception):  # noqa: N818 - named after what happened
    """Awaited code ended with a ``CancelledError`` that was not meant for the current task."""


def current_task() -> asyncio.Task[object]:
    """Return the task that is running."""
    task = asyncio.current_task()
    assert task is not None
    return task


def requests() -> int:
    """Return how often the current task has been asked to cancel so far."""
    return current_task().cancelling()


async def settled_requests() -> int:
    """Return the count of :func:`requests`, once a request that is on its way has arrived.

    This waits for a moment first. A cancellation that was asked for while the
    task was running arrives during that wait, as the ``CancelledError`` that
    it is. What is counted afterwards has all been delivered.
    """
    await asyncio.sleep(0)
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


async def kept_back(before: int) -> None:
    """Raise the cancellation that awaited code kept to itself, if it did.

    Call this when awaited code has come back with a result. If the current
    task has been asked to cancel since the count was ``before``, one of two
    things happened. The request has not arrived yet, and then it arrives
    here. Or it arrived inside the awaited code, which caught it and went on,
    and then it is raised here in its place.

    Code that means to absorb a cancellation takes the request back with
    ``Task.uncancel``, as ``asyncio.timeout`` does. Such code is left alone.
    """
    if requested_since(before):
        await asyncio.sleep(0)
        raise asyncio.CancelledError


async def guard(awaitable: Awaitable[ResultT], *, what: str = "awaited code") -> ResultT:
    """Await code that is not ours, and let only a real cancellation through as one.

    A result and an ordinary error get through unchanged. Three cases are put
    right. A ``CancelledError`` that the awaited code produced by itself
    becomes :class:`CancelledFromInside`, which is an ordinary error. An error
    that took the place of a cancellation of the current task is logged, and
    the cancellation is raised again in its stead. A result that comes back
    although the task was cancelled meanwhile is dropped, and the cancellation
    is raised.

    A cancellation that was asked for before the guard began arrives before
    ``awaitable`` is started.

    Args:
        awaitable: What to await.
        what: A name for the awaited code, which the text of a
            :class:`CancelledFromInside` begins with.
    """
    try:
        before = await settled_requests()
    except BaseException:
        # A coroutine that is never awaited would complain when it is collected.
        close = getattr(awaitable, "close", None)
        if close is not None:
            close()
        raise
    try:
        result = await awaitable
    except asyncio.CancelledError as error:
        if requested_since(before):
            raise
        raise CancelledFromInside(
            f"{what} ended with a cancellation that nobody asked for"
        ) from error
    except Exception as error:
        if replaced(error, before):
            logger.error("an error took the place of a cancellation", exc_info=error)
            raise asyncio.CancelledError from error
        raise
    await kept_back(before)
    return result
