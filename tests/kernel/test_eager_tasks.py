"""The kernel must behave the same when the event loop starts tasks eagerly."""

import asyncio
import sys
from collections.abc import AsyncIterator, Awaitable

import pytest

from columbina.kernel import Context, Disposer, EffectHandle, FiberState, Plugin

pytestmark = pytest.mark.skipif(
    sys.version_info < (3, 12), reason="eager task factories arrived in Python 3.12"
)


@pytest.fixture
async def eager_tasks() -> AsyncIterator[None]:
    """Make the running loop start every new task before create_task returns."""
    loop = asyncio.get_running_loop()
    loop.set_task_factory(getattr(asyncio, "eager_task_factory"))  # noqa: B009
    yield
    loop.set_task_factory(None)


@pytest.mark.usefixtures("eager_tasks")
async def test_cleanup_stays_in_reverse_order_under_an_eager_task_factory() -> None:
    ctx = Context()
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("older"))

        async def undo() -> None:
            log.append("newer starts")
            await inner.fiber.dispose()
            log.append("newer ends")

        return undo

    fiber = await ctx.plugin(Plugin("demo", apply))
    await fiber.dispose()

    assert log == ["newer starts", "newer ends", "older"]
    assert fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


@pytest.mark.usefixtures("eager_tasks")
async def test_a_plugin_loads_in_the_background_under_an_eager_task_factory() -> None:
    ctx = Context()
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        log.append("applied")

    fiber = ctx.plugin(Plugin("demo", apply))
    assert log == []

    await fiber

    assert log == ["applied"]
    await ctx.fiber.dispose()


@pytest.mark.usefixtures("eager_tasks")
async def test_a_disposer_that_disposes_its_own_handle_in_an_eager_task_is_still_waited_for() -> (
    None
):
    ctx = Context()
    gate = asyncio.Event()
    log: list[str] = []
    handles: list[EffectHandle] = []
    spawned: list[asyncio.Future[None]] = []

    async def cleanup() -> None:
        await gate.wait()
        log.append("cleaned up")

    def undo() -> Awaitable[None]:
        spawned.append(asyncio.ensure_future(handles[0].dispose()))
        return cleanup()

    handles.append(ctx.effect(lambda: undo))
    waiter = asyncio.ensure_future(handles[0].dispose())
    await asyncio.sleep(0)
    try:
        assert not waiter.done()
        assert not handles[0].disposed
    finally:
        gate.set()
    await asyncio.wait_for(waiter, timeout=1)

    assert log == ["cleaned up"]
    assert handles[0].disposed
    await ctx.fiber.dispose()
