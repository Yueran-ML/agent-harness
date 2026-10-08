"""A service that is being withdrawn while its dependents are still cleaning up."""

import asyncio

import pytest

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
)

CLOCK = ServiceKey[int]("clock")
STORE = ServiceKey[int]("store")


async def a_few_turns() -> None:
    for _ in range(5):
        await asyncio.sleep(0)


class SlowDependent:
    """A plugin that injects the clock and whose cleanup waits to be released."""

    def __init__(self, log: list[str]) -> None:
        self.log = log
        self.cleaning = asyncio.Event()
        self.release = asyncio.Event()
        self.plugin = Plugin("dependent", self.apply, inject=(CLOCK,))

    def apply(self, ctx: Context, config: object) -> Disposer:
        now = ctx.use(CLOCK)
        self.log.append(f"dependent loads against {now}")

        async def undo() -> None:
            self.log.append("dependent cleanup starts")
            self.cleaning.set()
            await self.release.wait()
            self.log.append("dependent cleanup ends")

        return undo


async def test_a_provider_waits_for_dependents_of_a_service_it_already_withdrew(
    ctx: Context,
) -> None:
    log: list[str] = []
    handles: list[EffectHandle] = []
    dependent = SlowDependent(log)

    def provider(inner: Context, config: object) -> Disposer:
        handles.append(inner.provide(CLOCK, 1))
        return lambda: log.append("provider resource closed")

    provider_fiber = await ctx.plugin(Plugin("provider", provider))
    await ctx.plugin(dependent.plugin)

    withdrawal = asyncio.ensure_future(handles[0].dispose())
    await asyncio.wait_for(dependent.cleaning.wait(), timeout=1)
    disposal = asyncio.ensure_future(provider_fiber.dispose())
    await a_few_turns()
    try:
        assert log == ["dependent loads against 1", "dependent cleanup starts"]
    finally:
        dependent.release.set()
    await asyncio.wait_for(asyncio.gather(withdrawal, disposal), timeout=1)

    assert log == [
        "dependent loads against 1",
        "dependent cleanup starts",
        "dependent cleanup ends",
        "provider resource closed",
    ]


async def test_a_name_stays_taken_until_its_dependents_have_unloaded(ctx: Context) -> None:
    log: list[str] = []
    dependent = SlowDependent(log)
    handle = ctx.provide(CLOCK, 1)
    fiber = await ctx.plugin(dependent.plugin)

    withdrawal = asyncio.ensure_future(handle.dispose())
    await asyncio.wait_for(dependent.cleaning.wait(), timeout=1)

    try:
        assert ctx.get(CLOCK) is None
        with pytest.raises(ServiceConflictError, match="still being withdrawn by 'root'"):
            ctx.provide(CLOCK, 2)
    finally:
        dependent.release.set()
    await asyncio.wait_for(withdrawal, timeout=1)
    assert fiber.state is FiberState.PENDING

    ctx.provide(CLOCK, 2)
    await ctx.settled()

    assert fiber.state is FiberState.ACTIVE
    assert log == [
        "dependent loads against 1",
        "dependent cleanup starts",
        "dependent cleanup ends",
        "dependent loads against 2",
    ]


async def test_a_withdrawal_cannot_be_overtaken_by_a_dependent_providing_the_name(
    ctx: Context,
) -> None:
    """Two plugins that would wait for each other if the name could change hands early."""
    old = ctx.provide(CLOCK, 1)

    def provide_store(inner: Context, config: object) -> None:
        inner.provide(STORE, inner.use(CLOCK))

    first = await ctx.plugin(Plugin("first", provide_store, inject=(CLOCK,)))
    second = await ctx.plugin(Plugin("second", lambda inner, config: None, inject=(STORE,)))

    withdrawal = asyncio.ensure_future(old.dispose())
    await asyncio.sleep(0)
    with pytest.raises(ServiceConflictError, match="still being withdrawn"):
        second.ctx.provide(CLOCK, 2)

    await asyncio.wait_for(withdrawal, timeout=1)

    assert first.state is FiberState.PENDING
    assert second.state is FiberState.PENDING
    assert ctx.get(CLOCK) is None
