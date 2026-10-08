"""Lifecycle code that waits for its own fiber must not wait for itself."""

import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    FiberState,
    LifecycleError,
    Plugin,
)


async def test_a_plugin_can_dispose_itself_from_its_own_disposer(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await inner.fiber.dispose()
            log.append("undone")

        return undo

    fiber = await ctx.plugin(Plugin("self", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert fiber.state is FiberState.DISPOSED


async def test_a_plugin_can_dispose_itself_while_another_caller_disposes_it(
    ctx: Context,
) -> None:
    entered = asyncio.Event()
    release = asyncio.Event()

    async def apply(inner: Context, config: object) -> None:
        entered.set()
        await release.wait()
        await inner.fiber.dispose()

    fiber = ctx.plugin(Plugin("self", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)
    outside = asyncio.ensure_future(fiber.dispose())
    await asyncio.sleep(0)

    release.set()
    await asyncio.wait_for(outside, timeout=1)

    assert fiber.state is FiberState.DISPOSED


async def test_the_root_can_be_disposed_from_one_of_its_own_disposers() -> None:
    ctx = Context()
    log: list[str] = []

    async def undo() -> None:
        await ctx.fiber.dispose()
        log.append("undone")

    ctx.effect(lambda: undo)

    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_child_can_dispose_its_parent_from_its_own_disposer(ctx: Context) -> None:
    log: list[str] = []

    def child(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            parent_fiber = inner.fiber.parent
            assert parent_fiber is not None
            await parent_fiber.dispose()
            log.append("child undone")

        return undo

    def parent(inner: Context, config: object) -> None:
        inner.plugin(Plugin("child", child))

    fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert log == ["child undone"]
    assert ctx.fibers() == ()


async def test_a_handle_can_be_disposed_from_inside_its_own_disposer(ctx: Context) -> None:
    handles: list[EffectHandle] = []
    log: list[str] = []

    async def undo() -> None:
        await handles[0].dispose()
        log.append("undone")

    handles.append(ctx.effect(lambda: undo))

    await asyncio.wait_for(handles[0].dispose(), timeout=1)

    assert log == ["undone"]
    assert handles[0].disposed


async def test_awaiting_a_fiber_from_its_own_apply_does_not_wait(ctx: Context) -> None:
    states: list[FiberState] = []

    async def apply(inner: Context, config: object) -> None:
        await inner.fiber
        await inner.fiber.settled()
        states.append(inner.fiber.state)

    fiber = ctx.plugin(Plugin("self", apply))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert states == [FiberState.LOADING]
    assert fiber.state is FiberState.ACTIVE


async def test_the_application_wide_wait_is_refused_inside_a_plugin() -> None:
    # The root is disposed by hand: were the two plugins to wait for each
    # other, disposing them could never finish either.
    ctx = Context()

    async def child(inner: Context, config: object) -> None:
        await inner.settled()

    async def parent(inner: Context, config: object) -> None:
        await inner.plugin(Plugin("child", child))

    fiber = ctx.plugin(Plugin("parent", parent))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, LifecycleError)
    with pytest.raises(LifecycleError, match="settled"):
        await fiber
    await ctx.fiber.dispose()


async def test_the_application_wide_wait_is_refused_inside_a_disposer(
    caplog: pytest.LogCaptureFixture,
) -> None:
    ctx = Context()

    def apply(inner: Context, config: object) -> Disposer:
        return inner.settled

    fiber = await ctx.plugin(Plugin("waiter", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert fiber.state is FiberState.DISPOSED
    assert "LifecycleError" in caplog.text
    await ctx.fiber.dispose()
