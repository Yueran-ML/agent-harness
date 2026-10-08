import asyncio
from types import SimpleNamespace

import pytest

from tack.kernel import (
    Context,
    Disposer,
    FiberState,
    InvalidPluginError,
    Plugin,
    ServiceKey,
    ServiceNotInjectedError,
    resolve_plugin,
)


class Clock:
    def __init__(self, now: int) -> None:
        self.now = now


CLOCK = ServiceKey[Clock]("clock")
STORE = ServiceKey[dict[str, int]]("store")
INDEX = ServiceKey[list[str]]("index")


def consumer(log: list[str]) -> Plugin:
    """A plugin that needs the clock and records each load and unload."""

    def apply(ctx: Context, config: object) -> Disposer:
        clock = ctx.use(CLOCK)
        log.append(f"load {clock.now}")
        return lambda: log.append(f"unload {clock.now}")

    return Plugin("consumer", apply, inject=(CLOCK,))


def clock_provider(now: int, log: list[str] | None = None) -> Plugin:
    def apply(ctx: Context, config: object) -> Disposer:
        ctx.provide(CLOCK, Clock(now))
        return lambda: None if log is None else log.append(f"provider {now} cleanup")

    return Plugin(f"clock-{now}", apply)


async def test_a_plugin_waits_for_the_services_it_injects(ctx: Context) -> None:
    log: list[str] = []

    fiber = await ctx.plugin(consumer(log))

    assert fiber.state is FiberState.PENDING
    assert fiber.missing == (CLOCK,)
    assert ctx.fiber.missing == ()
    assert log == []


async def test_a_pending_plugin_loads_when_its_service_is_provided(ctx: Context) -> None:
    log: list[str] = []
    fiber = await ctx.plugin(consumer(log))

    ctx.provide(CLOCK, Clock(1))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert fiber.missing == ()
    assert log == ["load 1"]


async def test_a_plugin_loads_at_once_when_its_services_exist(ctx: Context) -> None:
    log: list[str] = []
    ctx.provide(CLOCK, Clock(1))

    fiber = await ctx.plugin(consumer(log))

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1"]


async def test_a_plugin_unloads_when_its_service_is_withdrawn(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.provide(CLOCK, Clock(1))
    fiber = await ctx.plugin(consumer(log))

    await handle.dispose()

    assert fiber.state is FiberState.PENDING
    assert log == ["load 1", "unload 1"]


async def test_a_plugin_reloads_when_its_service_returns(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.provide(CLOCK, Clock(1))
    fiber = await ctx.plugin(consumer(log))
    await handle.dispose()

    ctx.provide(CLOCK, Clock(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1", "unload 1", "load 2"]


async def test_replacing_a_provider_reloads_its_dependents(ctx: Context) -> None:
    log: list[str] = []
    first = await ctx.plugin(clock_provider(1))
    fiber = await ctx.plugin(consumer(log))
    await ctx.settled()

    await first.dispose()
    await ctx.plugin(clock_provider(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == ["load 1", "unload 1", "load 2"]


async def test_a_dependent_loads_only_after_its_provider_is_active(ctx: Context) -> None:
    provided = asyncio.Event()
    release = asyncio.Event()
    log: list[str] = []

    async def provider(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        provided.set()
        await release.wait()
        log.append("provider ready")

    dependent = ctx.plugin(consumer(log))
    ctx.plugin(Plugin("provider", provider))
    await asyncio.wait_for(provided.wait(), timeout=1)
    await asyncio.sleep(0)
    assert dependent.state is FiberState.PENDING

    release.set()
    await ctx.settled()

    assert dependent.state is FiberState.ACTIVE
    assert log == ["provider ready", "load 1"]


async def test_dependents_unload_before_their_provider_cleans_up(ctx: Context) -> None:
    log: list[str] = []

    def provider(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("provider resource released"))
        inner.provide(CLOCK, Clock(1))
        inner.effect(lambda: lambda: log.append("provider late effect undone"))
        return lambda: log.append("provider disposer")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))
    dependent = await ctx.plugin(consumer(log))
    await ctx.settled()

    await provider_fiber.dispose()

    assert log == [
        "load 1",
        "unload 1",
        "provider disposer",
        "provider late effect undone",
        "provider resource released",
    ]
    assert dependent.state is FiberState.PENDING


async def test_a_chain_of_dependents_unloads_from_the_far_end(ctx: Context) -> None:
    log: list[str] = []

    def first(inner: Context, config: object) -> Disposer:
        inner.provide(CLOCK, Clock(1))
        return lambda: log.append("first")

    def second(inner: Context, config: object) -> Disposer:
        inner.provide(STORE, {"now": inner.use(CLOCK).now})
        return lambda: log.append("second")

    def third(inner: Context, config: object) -> Disposer:
        inner.provide(INDEX, sorted(inner.use(STORE)))
        return lambda: log.append("third")

    ctx.plugin(Plugin("third", third, inject=(STORE,)))
    ctx.plugin(Plugin("second", second, inject=(CLOCK,)))
    head = ctx.plugin(Plugin("first", first))
    await ctx.settled()
    assert ctx.use(INDEX) == ["now"]

    await head.dispose()

    assert log == ["third", "second", "first"]
    assert [fiber.state for fiber in ctx.fibers()] == [FiberState.PENDING, FiberState.PENDING]


async def test_disposing_the_root_unloads_dependents_first() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.effect(lambda: lambda: log.append("root resource released"))
    ctx.provide(CLOCK, Clock(1))
    await ctx.plugin(consumer(log))

    await ctx.fiber.dispose()

    assert log == ["load 1", "unload 1", "root resource released"]


async def test_using_an_undeclared_service_is_an_error(ctx: Context) -> None:
    ctx.provide(CLOCK, Clock(1))

    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    with pytest.raises(ServiceNotInjectedError, match=r"'sneaky'.*'clock'.*inject"):
        await ctx.plugin(Plugin("sneaky", apply))


async def test_a_child_must_inject_a_service_its_parent_provides(ctx: Context) -> None:
    def child(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    def parent(inner: Context, config: object) -> None:
        inner.provide(CLOCK, Clock(1))
        inner.plugin(Plugin("careless", child))
        inner.plugin(Plugin("careful", child, inject=(CLOCK,)))

    await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    careless_fiber, careful_fiber = ctx.fibers()[1:]
    assert careless_fiber.state is FiberState.FAILED
    assert isinstance(careless_fiber.error, ServiceNotInjectedError)
    assert careful_fiber.state is FiberState.ACTIVE


async def test_get_reads_an_optional_service_without_declaring_it(ctx: Context) -> None:
    clock = Clock(1)
    ctx.provide(CLOCK, clock)
    seen: list[Clock | None] = []

    def apply(inner: Context, config: object) -> None:
        seen.append(inner.get(CLOCK))

    await ctx.plugin(Plugin("optional", apply))

    assert seen == [clock]


async def test_a_child_may_use_what_its_parent_injects(ctx: Context) -> None:
    ctx.provide(CLOCK, Clock(1))
    seen: list[int] = []

    def child(inner: Context, config: object) -> None:
        seen.append(inner.use(CLOCK).now)

    async def parent(inner: Context, config: object) -> None:
        await inner.plugin(Plugin("child", child))

    await ctx.plugin(Plugin("parent", parent, inject=(CLOCK,)))

    assert seen == [1]


async def test_a_failed_plugin_is_retried_when_its_service_returns(ctx: Context) -> None:
    attempts: list[int] = []

    def apply(inner: Context, config: object) -> None:
        attempts.append(inner.use(CLOCK).now)
        if len(attempts) == 1:
            raise RuntimeError("first attempt fails")

    handle = ctx.provide(CLOCK, Clock(1))
    fiber = ctx.plugin(Plugin("flaky", apply, inject=(CLOCK,)))
    with pytest.raises(RuntimeError, match="first attempt fails"):
        await fiber
    assert fiber.state is FiberState.FAILED

    await handle.dispose()
    assert fiber.state is FiberState.PENDING
    assert fiber.error is None

    ctx.provide(CLOCK, Clock(2))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert attempts == [1, 2]


async def test_a_failed_plugin_is_not_retried_while_its_services_are_unchanged(
    ctx: Context,
) -> None:
    attempts: list[int] = []

    def apply(inner: Context, config: object) -> None:
        attempts.append(inner.use(CLOCK).now)
        raise RuntimeError("always fails")

    ctx.provide(CLOCK, Clock(1))
    fiber = ctx.plugin(Plugin("broken", apply, inject=(CLOCK, STORE)))
    store = ctx.provide(STORE, {})
    await ctx.settled()
    assert fiber.state is FiberState.FAILED

    ctx.provide(INDEX, [])
    await ctx.settled()
    assert attempts == [1]

    await store.dispose()
    ctx.provide(STORE, {})
    await ctx.settled()
    assert attempts == [1, 1]


async def test_inject_is_read_from_a_module_like_plugin(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        inner.use(CLOCK)

    fiber = await ctx.plugin(SimpleNamespace(name="module", apply=apply, inject=[CLOCK]))
    assert fiber.state is FiberState.PENDING

    ctx.provide(CLOCK, Clock(1))
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE


@pytest.mark.parametrize("inject", ["clock", [CLOCK, "store"], {CLOCK}])
def test_inject_must_be_a_list_of_service_keys(inject: object) -> None:
    def apply(inner: Context, config: object) -> None:
        return None

    with pytest.raises(InvalidPluginError, match="list of service keys"):
        resolve_plugin(SimpleNamespace(name="module", apply=apply, inject=inject))
