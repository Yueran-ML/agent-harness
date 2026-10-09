"""Telling a cancellation of the current task from what merely looks like one."""

import asyncio
import contextlib
import gc
import logging
from collections.abc import Awaitable, Callable

import pytest

from columbina import cancellation
from columbina.cancellation import CancelledFromInside, guard


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


def carrying_on(started: asyncio.Event) -> Callable[[], Awaitable[str]]:
    """Code that catches the cancellation of the task it runs in, and returns as if nothing was."""

    async def run() -> str:
        started.set()
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.Event().wait()
        return "carried on"

    return run


async def test_a_cancellation_that_the_awaited_code_keeps_to_itself_still_cancels() -> None:
    started = asyncio.Event()
    task = asyncio.ensure_future(guard(carrying_on(started)()))
    await asyncio.wait_for(started.wait(), timeout=1)

    task.cancel()

    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=1)
    assert task.cancelled()


async def test_a_cancellation_asked_for_while_the_awaited_code_ended_arrives_only_once() -> None:
    async def ask_and_end() -> str:
        # The request is made while the task runs, so it has not arrived when this returns.
        cancellation.current_task().cancel()
        return "done"

    async def work() -> str:
        try:
            return await guard(ask_and_end())
        except asyncio.CancelledError:
            # What cleans up after a cancellation must be able to wait.
            await asyncio.sleep(0)
            await asyncio.sleep(0)
            return "cancelled once"

    assert await asyncio.wait_for(asyncio.ensure_future(work()), timeout=1) == "cancelled once"


async def test_code_that_takes_a_cancellation_back_is_left_alone() -> None:
    async def patient() -> str:
        try:
            async with asyncio.timeout(0.01):
                await asyncio.Event().wait()
        except TimeoutError:
            return "timed out"
        return "never"

    assert await asyncio.wait_for(guard(patient()), timeout=1) == "timed out"


async def test_a_cancellation_from_inside_becomes_an_error() -> None:
    with pytest.raises(CancelledFromInside) as raised:
        await guard(cancel_inside())

    assert str(raised.value) == "awaited code ended with a cancellation that nobody asked for"
    assert isinstance(raised.value.__cause__, asyncio.CancelledError)


async def test_the_error_names_what_was_awaited_if_it_is_told() -> None:
    with pytest.raises(CancelledFromInside) as raised:
        await guard(cancel_inside(), what="the check of a plugin")

    assert str(raised.value) == (
        "the check of a plugin ended with a cancellation that nobody asked for"
    )


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

    with caplog.at_level(logging.ERROR, logger="columbina"), pytest.raises(asyncio.CancelledError):
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
