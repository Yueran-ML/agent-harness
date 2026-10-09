import asyncio
import logging
from collections.abc import Awaitable, Callable

import pytest

from columbina.kernel import Context, FiberState, InactiveEffectError


def recorder(log: list[str], label: str) -> Callable[[], None]:
    def undo() -> None:
        log.append(label)

    return undo


def broken() -> None:
    raise RuntimeError("boom")


async def test_the_root_fiber_is_active(ctx: Context) -> None:
    assert ctx.fiber.state is FiberState.ACTIVE
    assert ctx.fiber.name == "root"
    assert ctx.fiber.parent is None
    assert ctx.fiber.ctx is ctx


async def test_effect_runs_its_setup_immediately(ctx: Context) -> None:
    log: list[str] = []
    ctx.effect(lambda: log.append("setup"))
    assert log == ["setup"]


async def test_disposing_the_fiber_undoes_effects_in_reverse_order() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "first"))
    ctx.effect(lambda: recorder(log, "second"))

    await ctx.fiber.dispose()

    assert log == ["second", "first"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_async_disposers_are_awaited_one_at_a_time() -> None:
    ctx = Context()
    log: list[str] = []

    def slow(label: str) -> Callable[[], Awaitable[None]]:
        async def undo() -> None:
            log.append(f"{label} start")
            await asyncio.sleep(0)
            log.append(f"{label} end")

        return undo

    ctx.effect(lambda: slow("first"))
    ctx.effect(lambda: slow("second"))

    await ctx.fiber.dispose()

    assert log == ["second start", "second end", "first start", "first end"]


async def test_a_handle_undoes_its_effect_once(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.effect(lambda: recorder(log, "undo"))
    assert not handle.disposed

    await handle.dispose()
    await handle.dispose()

    assert log == ["undo"]
    assert handle.disposed


async def test_an_effect_disposed_early_is_not_undone_again() -> None:
    ctx = Context()
    log: list[str] = []
    handle = ctx.effect(lambda: recorder(log, "undo"))

    await handle.dispose()
    await ctx.fiber.dispose()

    assert log == ["undo"]


async def test_disposing_an_older_effect_leaves_newer_ones_in_place() -> None:
    ctx = Context()
    log: list[str] = []
    older = ctx.effect(lambda: recorder(log, "older"))
    ctx.effect(lambda: recorder(log, "newer"))

    await older.dispose()
    assert log == ["older"]

    await ctx.fiber.dispose()
    assert log == ["older", "newer"]


async def test_concurrent_disposals_share_one_run(ctx: Context) -> None:
    gate = asyncio.Event()
    runs = 0

    async def undo() -> None:
        nonlocal runs
        runs += 1
        await gate.wait()

    handle = ctx.effect(lambda: undo)
    first = asyncio.ensure_future(handle.dispose())
    second = asyncio.ensure_future(handle.dispose())
    await asyncio.sleep(0)
    assert not first.done()
    assert not second.done()

    gate.set()
    await asyncio.gather(first, second)

    assert runs == 1


async def test_disposing_a_handle_directly_raises_its_error(ctx: Context) -> None:
    handle = ctx.effect(lambda: broken)

    with pytest.raises(RuntimeError, match="boom"):
        await handle.dispose()

    assert handle.disposed


async def test_a_failing_disposer_does_not_stop_the_others(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "first"))
    ctx.effect(lambda: broken)
    ctx.effect(lambda: recorder(log, "third"))

    with caplog.at_level(logging.ERROR, logger="columbina.kernel"):
        await ctx.fiber.dispose()

    assert log == ["third", "first"]
    assert "boom" in caplog.text


async def test_a_setup_that_raises_registers_nothing() -> None:
    ctx = Context()

    with pytest.raises(RuntimeError, match="boom"):
        ctx.effect(broken)

    await ctx.fiber.dispose()
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_effects_are_rejected_after_disposal() -> None:
    ctx = Context()
    await ctx.fiber.dispose()
    log: list[str] = []

    with pytest.raises(InactiveEffectError, match="disposed"):
        ctx.effect(lambda: log.append("setup"))

    assert log == []


async def test_effects_are_rejected_while_the_fiber_unloads() -> None:
    ctx = Context()
    seen: list[FiberState] = []
    errors: list[InactiveEffectError] = []

    def late() -> None:
        seen.append(ctx.fiber.state)
        try:
            ctx.effect(lambda: None)
        except InactiveEffectError as error:
            errors.append(error)

    ctx.effect(lambda: late)
    await ctx.fiber.dispose()

    assert seen == [FiberState.UNLOADING]
    assert len(errors) == 1


async def test_disposing_a_fiber_twice_is_harmless() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: recorder(log, "undo"))

    await asyncio.gather(ctx.fiber.dispose(), ctx.fiber.dispose())
    await ctx.fiber.dispose()

    assert log == ["undo"]


async def test_a_cancelled_waiter_does_not_interrupt_the_unload() -> None:
    ctx = Context()
    gate = asyncio.Event()
    log: list[str] = []

    async def undo() -> None:
        await gate.wait()
        log.append("undone")

    ctx.effect(lambda: undo)
    waiter = asyncio.ensure_future(ctx.fiber.dispose())
    await asyncio.sleep(0)
    waiter.cancel()
    with pytest.raises(asyncio.CancelledError):
        await waiter

    gate.set()
    await ctx.fiber.dispose()

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED
