"""Cancelling a caller, or code that cancels itself, must not lose a cleanup."""

import asyncio
import logging

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EmitEvent,
    Fiber,
    FiberState,
    KernelError,
    ParallelEvent,
    Plugin,
    ServiceKey,
)

PING = EmitEvent[str]("test/ping")
FLUSH = ParallelEvent[str]("test/flush")
CLOCK = ServiceKey[int]("clock")


def nothing(ctx: Context, config: object) -> None:
    return None


async def a_few_turns() -> None:
    """Let every task that is ready to run take several steps."""
    for _ in range(5):
        await asyncio.sleep(0)


def driver_of(fiber: Fiber) -> asyncio.Task[object]:
    """Find the background task that is loading or unloading ``fiber``."""
    name = f"tack-fiber-{fiber.uid}"
    return next(task for task in asyncio.all_tasks() if task.get_name() == name)


async def test_cancelling_the_caller_does_not_abandon_a_cleanup() -> None:
    ctx = Context()
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def undo() -> None:
        started.set()
        await release.wait()
        log.append("cleaned")

    handle = ctx.effect(lambda: undo)
    caller = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert not handle.disposed

    release.set()
    await ctx.fiber.dispose()

    assert log == ["cleaned"]
    assert handle.disposed


async def test_a_parent_waits_for_a_child_whose_disposal_was_cancelled(ctx: Context) -> None:
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    def child(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            started.set()
            await release.wait()
            log.append("child cleaned")

        return undo

    def parent(inner: Context, config: object) -> None:
        inner.effect(lambda: lambda: log.append("parent resource released"))
        inner.plugin(Plugin("child", child))

    parent_fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()
    child_fiber = ctx.fibers()[1]

    caller = asyncio.ensure_future(child_fiber.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert child_fiber.state is FiberState.UNLOADING

    disposal = asyncio.ensure_future(parent_fiber.dispose())
    await a_few_turns()
    assert log == []

    release.set()
    await disposal

    assert log == ["child cleaned", "parent resource released"]
    assert child_fiber.state is FiberState.DISPOSED


async def test_a_cancellation_raised_by_apply_is_a_failed_load(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    async def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        raise asyncio.CancelledError

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("cancelled", apply))
        with pytest.raises(KernelError, match="cancelled while loading"):
            await fiber

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, KernelError)
    ctx.emit(PING, "after")
    assert log == []


@pytest.mark.parametrize("kind", ["plain function", "coroutine function"])
async def test_a_cancellation_raised_by_a_disposer_does_not_stop_the_unload(
    kind: str, caplog: pytest.LogCaptureFixture
) -> None:
    ctx = Context()
    log: list[str] = []

    def cancelled() -> None:
        raise asyncio.CancelledError

    async def cancelled_later() -> None:
        await asyncio.sleep(0)
        raise asyncio.CancelledError

    ctx.effect(lambda: lambda: log.append("older"))
    ctx.effect(lambda: cancelled if kind == "plain function" else cancelled_later)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["older"]
    assert ctx.fiber.state is FiberState.DISPOSED
    assert "cancelled" in caplog.text


async def test_an_unload_whose_driver_was_cancelled_is_resumed_by_dispose() -> None:
    ctx = Context()
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def slow() -> None:
        started.set()
        await release.wait()
        log.append("slow")

    ctx.effect(lambda: lambda: log.append("older"))
    ctx.effect(lambda: slow)
    disposal = asyncio.ensure_future(ctx.fiber.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)

    driver_of(ctx.fiber).cancel()
    await a_few_turns()
    assert not disposal.done()

    release.set()
    await asyncio.wait_for(disposal, timeout=1)

    assert log == ["slow", "older"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_load_whose_driver_was_cancelled_is_rolled_back(ctx: Context) -> None:
    entered = asyncio.Event()
    log: list[str] = []

    async def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        inner.effect(lambda: lambda: log.append("rolled back"))
        entered.set()
        await asyncio.Event().wait()

    fiber = ctx.plugin(Plugin("slow", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)

    driver_of(fiber).cancel()
    await a_few_turns()

    assert fiber.state is FiberState.FAILED
    assert log == ["rolled back"]
    ctx.emit(PING, "after")
    assert log == ["rolled back"]
    with pytest.raises(KernelError, match="cancelled while loading"):
        await fiber


async def test_a_driver_cancelled_before_it_ran_is_replaced_for_the_next_waiter(
    ctx: Context,
) -> None:
    fiber = ctx.plugin(Plugin("unlucky", nothing))
    driver_of(fiber).cancel()

    assert fiber.state is FiberState.PENDING
    await asyncio.wait_for(fiber.settled(), timeout=1)

    # Whoever waits next starts the load again.
    assert fiber.state is FiberState.ACTIVE


async def test_a_disposer_may_return_a_future() -> None:
    ctx = Context()
    log: list[str] = []

    async def cleanup() -> None:
        await asyncio.sleep(0)
        log.append("cleaned")

    def undo() -> asyncio.Future[None]:
        return asyncio.ensure_future(cleanup())

    ctx.effect(lambda: undo)
    await ctx.fiber.dispose()

    assert log == ["cleaned"]


async def test_parallel_isolates_a_listener_that_is_cancelled(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    finished: list[str] = []

    async def cancelled(payload: str) -> None:
        future: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        future.cancel()
        await future

    async def slow(payload: str) -> None:
        await a_few_turns()
        finished.append(payload)

    ctx.on(FLUSH, cancelled)
    ctx.on(FLUSH, slow)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        await ctx.parallel(FLUSH, "x")

    assert finished == ["x"]
    assert "was cancelled" in caplog.text


async def test_cancelling_a_parallel_dispatch_cancels_its_listeners(ctx: Context) -> None:
    started = asyncio.Event()
    cancelled: list[str] = []

    async def listener(payload: str) -> None:
        started.set()
        try:
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            cancelled.append(payload)
            raise

    ctx.on(FLUSH, listener)
    dispatch = asyncio.ensure_future(ctx.parallel(FLUSH, "x"))
    await asyncio.wait_for(started.wait(), timeout=1)

    dispatch.cancel()
    with pytest.raises(asyncio.CancelledError):
        await dispatch

    assert cancelled == ["x"]


async def test_a_withdrawal_outlasts_a_dependent_whose_driver_was_cancelled(ctx: Context) -> None:
    started = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)
        inner.effect(lambda: lambda: log.append("older"))

        async def newer() -> None:
            started.set()
            await release.wait()
            log.append("newer")

        inner.effect(lambda: newer)

    handle = ctx.provide(CLOCK, 1)
    fiber = await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    withdrawal = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)
    driver_of(fiber).cancel()
    await a_few_turns()
    try:
        assert not withdrawal.done()
    finally:
        release.set()
    await asyncio.wait_for(withdrawal, timeout=1)

    assert log == ["newer", "older"]
    assert fiber.state is FiberState.PENDING


async def test_a_rollback_survives_a_cleanup_that_cancels_itself(ctx: Context) -> None:
    started = asyncio.Event()
    log: list[str] = []

    async def cancelled_cleanup() -> None:
        raise asyncio.CancelledError

    async def apply(inner: Context, config: object) -> None:
        inner.effect(lambda: lambda: log.append("older"))
        inner.effect(lambda: cancelled_cleanup)
        started.set()
        await asyncio.Event().wait()

    fiber = ctx.plugin(Plugin("slow", apply))
    await asyncio.wait_for(started.wait(), timeout=1)

    driver_of(fiber).cancel()
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert log == ["older"]
    assert fiber.state is FiberState.FAILED


async def test_a_cleanup_cancelled_before_it_ran_is_run_for_the_next_waiter(ctx: Context) -> None:
    release = asyncio.Event()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

        async def undo() -> None:
            await release.wait()
            log.append("dependent cleaned")

        inner.effect(lambda: undo)

    handle = ctx.provide(CLOCK, 1)
    await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    def cancel_the_release_before_it_starts() -> None:
        name = f"tack-effect-{ctx.fiber.uid}"
        next(task for task in asyncio.all_tasks() if task.get_name() == name).cancel()

    caller = asyncio.ensure_future(handle.dispose())
    asyncio.get_running_loop().call_soon(cancel_the_release_before_it_starts)
    with pytest.raises(asyncio.CancelledError):
        await caller
    assert not handle.disposed

    release.set()
    await asyncio.wait_for(handle.dispose(), timeout=1)

    assert handle.disposed
    assert log == ["dependent cleaned"]
    ctx.provide(CLOCK, 2)


async def test_a_cleanup_cancelled_after_it_started_is_reported(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()
    started = asyncio.Event()

    async def undo() -> None:
        started.set()
        await asyncio.Event().wait()

    handle = ctx.effect(lambda: undo)
    waiter = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(started.wait(), timeout=1)

    name = f"tack-effect-{ctx.fiber.uid}"
    next(task for task in asyncio.all_tasks() if task.get_name() == name).cancel()
    with (
        caplog.at_level(logging.ERROR, logger="tack.kernel"),
        pytest.raises(asyncio.CancelledError),
    ):
        await waiter

    assert "was cancelled before it finished" in caplog.text
    assert handle.disposed
    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)
