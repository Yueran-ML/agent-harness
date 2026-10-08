"""The kernel must behave the same when the event loop starts tasks eagerly."""

import asyncio
import sys
from collections.abc import AsyncIterator

import pytest

from tack.kernel import Context, Disposer, FiberState, Plugin

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
