"""The states a fiber passes through, as reported by FIBER_STATE_CHANGED."""

import asyncio
import logging

import pytest

from tack.kernel import (
    FIBER_STATE_CHANGED,
    Context,
    Disposer,
    EmitEvent,
    FiberState,
    FiberStateChange,
    Plugin,
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")
PING = EmitEvent[str]("test/ping")

PENDING = FiberState.PENDING
LOADING = FiberState.LOADING
ACTIVE = FiberState.ACTIVE
FAILED = FiberState.FAILED
UNLOADING = FiberState.UNLOADING
DISPOSED = FiberState.DISPOSED


def states_of(ctx: Context, name: str) -> list[FiberState]:
    """Record every state the fiber called ``name`` enters from now on."""
    seen: list[FiberState] = []

    def listener(change: FiberStateChange) -> None:
        if change.fiber.name == name:
            seen.append(change.current)

    ctx.on(FIBER_STATE_CHANGED, listener)
    return seen


def nothing(ctx: Context, config: object) -> None:
    return None


async def test_a_plugin_reports_each_state_it_passes_through(ctx: Context) -> None:
    seen = states_of(ctx, "demo")

    fiber = await ctx.plugin(Plugin("demo", nothing))
    await fiber.dispose()

    assert seen == [LOADING, ACTIVE, UNLOADING, DISPOSED]


async def test_a_change_carries_the_fiber_and_both_states(ctx: Context) -> None:
    changes: list[FiberStateChange] = []
    ctx.on(FIBER_STATE_CHANGED, changes.append)

    fiber = await ctx.plugin(Plugin("demo", nothing))

    assert [(change.fiber, change.previous, change.current) for change in changes] == [
        (fiber, PENDING, LOADING),
        (fiber, LOADING, ACTIVE),
    ]


async def test_a_failed_load_is_unwound_before_it_is_reported_failed(ctx: Context) -> None:
    seen = states_of(ctx, "broken")

    def apply(inner: Context, config: object) -> None:
        raise RuntimeError("cannot start")

    fiber = ctx.plugin(Plugin("broken", apply))
    await fiber.settled()

    assert seen == [LOADING, UNLOADING, FAILED]


async def test_a_dependent_returns_to_pending_and_loads_again(ctx: Context) -> None:
    seen = states_of(ctx, "dependent")
    handle = ctx.provide(CLOCK, 1)
    await ctx.plugin(Plugin("dependent", nothing, inject=(CLOCK,)))

    await handle.dispose()
    ctx.provide(CLOCK, 2)
    await ctx.settled()

    assert seen == [LOADING, ACTIVE, UNLOADING, PENDING, LOADING, ACTIVE]


async def test_a_plugin_disposed_while_loading_never_becomes_active(ctx: Context) -> None:
    seen = states_of(ctx, "demo")
    entered = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def apply(inner: Context, config: object) -> Disposer:
        entered.set()
        await release.wait()
        return lambda: log.append("undone")

    fiber = ctx.plugin(Plugin("demo", apply))
    await asyncio.wait_for(entered.wait(), timeout=1)
    disposal = asyncio.ensure_future(fiber.dispose())
    await asyncio.sleep(0)

    release.set()
    await disposal

    assert log == ["undone"]
    assert seen == [LOADING, UNLOADING, DISPOSED]


async def test_scopes_and_the_root_report_their_disposal() -> None:
    ctx = Context()
    changes: list[tuple[str, FiberState]] = []
    ctx.on(FIBER_STATE_CHANGED, lambda change: changes.append((change.fiber.name, change.current)))
    scoped = ctx.scope(object())

    await scoped.fiber.dispose()
    await ctx.fiber.dispose()

    # The listener belongs to the root, so it is gone before the root is disposed.
    assert changes == [("scope", UNLOADING), ("scope", DISPOSED), ("root", UNLOADING)]


async def test_a_failing_state_listener_does_not_disturb_the_lifecycle(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    def failing(change: FiberStateChange) -> None:
        raise RuntimeError("observer broke")

    ctx.on(FIBER_STATE_CHANGED, failing)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = await ctx.plugin(Plugin("demo", nothing))

    assert fiber.state is ACTIVE
    assert "observer broke" in caplog.text


async def test_an_observer_that_raises_cancelled_error_cannot_corrupt_a_load(
    ctx: Context,
) -> None:
    log: list[str] = []

    def observe(change: FiberStateChange) -> None:
        if change.fiber.name == "victim" and change.current is LOADING:
            inner = change.fiber.ctx
            inner.effect(lambda: lambda: log.append("cleaned"))
            inner.on(PING, log.append)
            raise asyncio.CancelledError

    ctx.on(FIBER_STATE_CHANGED, observe)

    fiber = await ctx.plugin(Plugin("victim", nothing))
    assert fiber.state is ACTIVE

    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["cleaned"]
    assert fiber.state is DISPOSED
