import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
    ServiceUnavailableError,
)


class Clock:
    def __init__(self, now: int) -> None:
        self.now = now


CLOCK = ServiceKey[Clock]("clock")


async def test_a_provided_service_can_be_used(ctx: Context) -> None:
    clock = Clock(1)

    ctx.provide(CLOCK, clock)

    assert ctx.use(CLOCK) is clock
    assert ctx.get(CLOCK) is clock


async def test_get_returns_none_for_a_missing_service(ctx: Context) -> None:
    assert ctx.get(CLOCK) is None


async def test_use_raises_for_a_missing_service(ctx: Context) -> None:
    with pytest.raises(ServiceUnavailableError, match="'clock'"):
        ctx.use(CLOCK)


async def test_providing_a_service_twice_is_an_error(ctx: Context) -> None:
    first = Clock(1)
    ctx.provide(CLOCK, first)

    with pytest.raises(ServiceConflictError, match="already provided by 'root'"):
        ctx.provide(CLOCK, Clock(2))

    assert ctx.use(CLOCK) is first


async def test_disposing_the_handle_withdraws_the_service(ctx: Context) -> None:
    handle = ctx.provide(CLOCK, Clock(1))

    await handle.dispose()

    assert ctx.get(CLOCK) is None


async def test_a_name_can_be_provided_again_after_withdrawal(ctx: Context) -> None:
    handle = ctx.provide(CLOCK, Clock(1))
    await handle.dispose()
    second = Clock(2)

    ctx.provide(CLOCK, second)

    assert ctx.use(CLOCK) is second


async def test_a_service_is_withdrawn_when_its_provider_is_disposed(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))

    fiber = await ctx.plugin(Plugin("clock", apply))
    assert ctx.get(CLOCK) is not None

    await fiber.dispose()

    assert ctx.get(CLOCK) is None


async def test_a_service_becomes_visible_when_its_provider_is_active(ctx: Context) -> None:
    provided = asyncio.Event()
    release = asyncio.Event()
    clock = Clock(1)

    async def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        provided.set()
        await release.wait()

    fiber = ctx.plugin(Plugin("clock", apply))
    await asyncio.wait_for(provided.wait(), timeout=1)
    assert fiber.state is FiberState.LOADING
    assert ctx.get(CLOCK) is None

    release.set()
    await fiber

    assert ctx.get(CLOCK) is clock


async def test_a_provider_sees_its_own_service_while_loading(ctx: Context) -> None:
    clock = Clock(1)
    seen: list[Clock] = []

    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        seen.append(inner.use(CLOCK))

    await ctx.plugin(Plugin("clock", apply))

    assert seen == [clock]


async def test_a_child_sees_a_service_of_its_loading_parent(ctx: Context) -> None:
    clock = Clock(1)
    seen: list[Clock | None] = []

    def child(inner: Context, config: object) -> None:
        seen.append(inner.get(CLOCK))

    async def parent(inner: Context, config: object) -> None:
        inner.provide(CLOCK, clock)
        await inner.plugin(Plugin("child", child))

    await ctx.plugin(Plugin("parent", parent))

    assert seen == [clock]


async def test_a_service_is_hidden_from_others_while_its_provider_unloads(ctx: Context) -> None:
    clock = Clock(1)
    outside: list[Clock | None] = []
    inside: list[Clock | None] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, clock)

        def undo() -> None:
            outside.append(ctx.get(CLOCK))
            inside.append(inner.get(CLOCK))

        return undo

    fiber = await ctx.plugin(Plugin("clock", apply))
    await fiber.dispose()

    assert outside == [None]
    assert inside == [clock]


async def test_a_failed_provider_withdraws_its_service(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        raise RuntimeError("cannot start")

    with pytest.raises(RuntimeError, match="cannot start"):
        await ctx.plugin(Plugin("clock", apply))

    assert ctx.get(CLOCK) is None
    ctx.provide(CLOCK, Clock(2))
