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
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")


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


async def test_a_disposer_may_dispose_its_fiber_through_a_task() -> None:
    ctx = Context()

    def apply(inner: Context, config: object) -> Disposer:
        def undo() -> asyncio.Future[None]:
            return asyncio.ensure_future(inner.fiber.dispose())

        return undo

    fiber = await ctx.plugin(Plugin("self", apply))

    await asyncio.wait_for(fiber.dispose(), timeout=1)

    assert fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


async def test_the_application_wide_wait_is_refused_in_a_task_spawned_by_apply() -> None:
    ctx = Context()

    async def apply(inner: Context, config: object) -> None:
        await asyncio.gather(inner.settled())

    fiber = ctx.plugin(Plugin("waiter", apply))
    await asyncio.wait_for(fiber.settled(), timeout=1)

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, LifecycleError)
    await ctx.fiber.dispose()


async def test_a_task_that_outlives_apply_may_wait_for_the_application(ctx: Context) -> None:
    go = asyncio.Event()
    outcomes: list[str] = []
    tasks: list[asyncio.Future[None]] = []

    async def background(inner: Context) -> None:
        await go.wait()
        await inner.settled()
        outcomes.append("waited")

    def apply(inner: Context, config: object) -> None:
        tasks.append(asyncio.ensure_future(background(inner)))

    await ctx.plugin(Plugin("spawner", apply))

    go.set()
    await asyncio.wait_for(tasks[0], timeout=1)

    assert outcomes == ["waited"]


async def test_a_dependent_may_dispose_its_provider_from_its_own_disposer() -> None:
    ctx = Context()
    log: list[str] = []

    def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, 1)

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    def dependent(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await provider_fiber.dispose()
            log.append("dependent undone")

        return undo

    dependent_fiber = await ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))

    await asyncio.wait_for(provider_fiber.dispose(), timeout=1)

    assert log == ["dependent undone"]
    assert provider_fiber.state is FiberState.DISPOSED
    assert dependent_fiber.state is FiberState.PENDING
    await ctx.fiber.dispose()


async def test_disposing_a_dependent_that_disposes_its_provider_does_not_hang() -> None:
    ctx = Context()

    def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, 1)

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    def dependent(inner: Context, config: object) -> Disposer:
        return provider_fiber.dispose

    dependent_fiber = await ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))

    await asyncio.wait_for(dependent_fiber.dispose(), timeout=1)
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert dependent_fiber.state is FiberState.DISPOSED
    assert provider_fiber.state is FiberState.DISPOSED
    await ctx.fiber.dispose()


async def a_few_turns() -> None:
    for _ in range(5):
        await asyncio.sleep(0)


async def test_a_disposer_that_disposes_its_fiber_and_carries_on_does_not_stall_the_loop() -> None:
    ctx = Context()
    log: list[str] = []

    async def undo() -> None:
        await ctx.fiber.dispose()
        await asyncio.sleep(0)
        log.append("undone")

    handle = ctx.effect(lambda: undo)

    await asyncio.wait_for(handle.dispose(), timeout=1)
    await asyncio.wait_for(ctx.fiber.dispose(), timeout=1)

    assert log == ["undone"]
    assert ctx.fiber.state is FiberState.DISPOSED


async def test_a_provider_waits_for_a_dependent_that_disposed_it_while_loading() -> None:
    ctx = Context()
    entered = asyncio.Event()
    gate = asyncio.Event()
    log: list[str] = []

    def provider(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, 1)
        return lambda: log.append("provider cleaned up")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))

    async def dependent(inner: Context, config: object) -> Disposer:
        inner.use(CLOCK)
        await provider_fiber.dispose()
        entered.set()
        await gate.wait()
        return lambda: log.append("dependent cleaned up")

    ctx.plugin(Plugin("dependent", dependent, inject=(CLOCK,)))
    await asyncio.wait_for(entered.wait(), timeout=1)
    await a_few_turns()
    try:
        assert log == []
    finally:
        gate.set()
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert log == ["dependent cleaned up", "provider cleaned up"]
    await ctx.fiber.dispose()


async def test_a_task_started_by_a_disposer_waits_normally_once_the_disposer_has_returned() -> None:
    ctx = Context()
    release = asyncio.Event()
    log: list[str] = []
    tasks: list[asyncio.Future[None]] = []

    async def older() -> None:
        await release.wait()
        log.append("older")

    async def after() -> None:
        await ctx.fiber.dispose()
        log.append(f"after returned while {ctx.fiber.state.value}")

    async def undo() -> None:
        tasks.append(asyncio.ensure_future(after()))

    ctx.effect(lambda: older)
    handle = ctx.effect(lambda: undo)

    await handle.dispose()
    await a_few_turns()
    try:
        assert log == []
    finally:
        release.set()
    await asyncio.wait_for(tasks[0], timeout=1)

    assert log == ["older", "after returned while disposed"]


async def test_a_dependent_may_withdraw_the_service_it_injects_from_its_own_disposer() -> None:
    ctx = Context()
    log: list[str] = []
    handle = ctx.provide(CLOCK, 1)

    def apply(inner: Context, config: object) -> Disposer:
        async def undo() -> None:
            await handle.dispose()
            log.append("undone")

        return undo

    fiber = await ctx.plugin(Plugin("dependent", apply, inject=(CLOCK,)))

    await asyncio.wait_for(fiber.dispose(), timeout=1)
    await asyncio.wait_for(ctx.settled(), timeout=1)

    assert log == ["undone"]
    assert fiber.state is FiberState.DISPOSED
    assert ctx.get(CLOCK) is None
    await asyncio.wait_for(handle.dispose(), timeout=1)
    assert handle.disposed
    await ctx.fiber.dispose()
