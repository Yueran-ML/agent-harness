"""Telling a cancellation of the current task from what merely looks like one."""

import asyncio
import contextlib
import gc
import logging
from collections.abc import Awaitable, Callable

import pytest

from tack import cancellation
from tack.cancellation import CancelledFromInside, guard


async def value() -> str:
    return "done"


async def wait_forever() -> str:
    await asyncio.Event().wait()
    return "never"


async def cancel_inside() -> str:
    raise asyncio.CancelledError


async def fail() -> str:
    raise ValueError("ordinary")


def waiting_then_failing(started: asyncio.Event) -> Callable[[], Awaitable[str]]:
    async def run() -> str:
        started.set()
        try:
            await asyncio.Event().wait()
        finally:
            raise RuntimeError("cleanup failed")

    return run


async def after_surviving_a_cancellation(work: Callable[[], Awaitable[str]]) -> str:
    """Run ``work`` in a task that has swallowed a cancellation before."""
    waiting = asyncio.Event()

    async def run() -> str:
        try:
            waiting.set()
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            pass
        return await work()

    task = asyncio.ensure_future(run())
    await asyncio.wait_for(waiting.wait(), timeout=1)
    task.cancel()
    return await asyncio.wait_for(task, timeout=1)


async def test_a_result_gets_through() -> None:
    assert await guard(value()) == "done"


async def test_an_ordinary_error_gets_through() -> None:
    with pytest.raises(ValueError, match="ordinary"):
        await guard(fail())


async def test_a_cancellation_of_the_current_task_gets_through() -> None:
    task = asyncio.ensure_future(guard(wait_forever()))
    await asyncio.sleep(0)
    task.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)


async def test_a_cancellation_from_inside_becomes_an_error() -> None:
    with pytest.raises(CancelledFromInside):
        await guard(cancel_inside())


async def test_a_cancellation_from_inside_is_told_apart_in_a_task_that_survived_one() -> None:
    async def work() -> str:
        try:
            return await guard(cancel_inside())
        except CancelledFromInside:
            return "told apart"

    assert await after_surviving_a_cancellation(work) == "told apart"


async def test_an_error_that_took_the_place_of_a_cancellation_gives_the_place_back(
    caplog: pytest.LogCaptureFixture,
) -> None:
    started = asyncio.Event()
    task = asyncio.ensure_future(guard(waiting_then_failing(started)()))
    await asyncio.wait_for(started.wait(), timeout=1)
    task.cancel()

    with caplog.at_level(logging.ERROR, logger="tack"), pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)

    assert "an error took the place of a cancellation" in caplog.text
    assert "cleanup failed" in caplog.text


async def test_an_error_that_merely_follows_an_old_cancellation_stays_an_error() -> None:
    async def work() -> str:
        error = RuntimeError("later")
        error.__context__ = asyncio.CancelledError()
        try:
            raise error
        except RuntimeError:
            before = cancellation.requests()
            assert before == 1
            assert not cancellation.requested_since(before)
            assert not cancellation.replaced(error, before)
            return "an error"

    assert await after_surviving_a_cancellation(work) == "an error"


async def test_a_request_that_has_not_been_delivered_does_not_count_as_replaced() -> None:
    before = cancellation.requests()
    task = cancellation.current_task()
    task.cancel()
    error = RuntimeError("unrelated")
    error.__context__ = ValueError("no cancellation")

    assert cancellation.requested_since(before)
    assert not cancellation.replaced(error, before)

    task.uncancel()
    assert not cancellation.requested_since(before)
    # Before Python 3.13 the request stays pending after uncancel(): let it arrive here.
    with contextlib.suppress(asyncio.CancelledError):
        await asyncio.sleep(0)


async def test_a_cancellation_that_was_asked_for_before_the_guard_began_gets_through() -> None:
    async def work() -> str:
        cancellation.current_task().cancel()
        return await guard(wait_forever())

    task = asyncio.ensure_future(work())

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)
    assert task.cancelled()


async def test_a_guard_that_never_began_leaves_no_coroutine_behind() -> None:
    began: list[bool] = []

    async def never() -> str:
        began.append(True)
        return "done"

    async def work() -> str:
        cancellation.current_task().cancel()
        return await guard(never())

    task = asyncio.ensure_future(work())
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)
    gc.collect()

    assert began == []


async def test_the_count_can_be_taken_once_a_request_on_its_way_has_arrived() -> None:
    async def later() -> int:
        return await cancellation.settled_requests()

    async def at_once() -> int:
        cancellation.current_task().cancel()
        return await cancellation.settled_requests()

    cancelled = asyncio.ensure_future(at_once())

    assert await later() == 0
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(cancelled, timeout=1)


async def test_a_guard_that_never_began_leaves_a_task_it_was_given_alone() -> None:
    inner = asyncio.ensure_future(value())

    async def work() -> str:
        cancellation.current_task().cancel()
        return await guard(inner)

    task = asyncio.ensure_future(work())
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)

    assert await inner == "done"
