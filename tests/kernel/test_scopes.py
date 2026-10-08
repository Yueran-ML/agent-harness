import pytest

from tack.kernel import (
    BailEvent,
    Context,
    EmitEvent,
    FiberState,
    InactiveEffectError,
    Next,
    ParallelEvent,
    Plugin,
    SerialEvent,
    ServiceKey,
    WaterfallEvent,
)

PING = EmitEvent[str]("test/ping")
ASK = BailEvent[str, str]("test/ask")
FLUSH = ParallelEvent[str]("test/flush")
DECIDE = SerialEvent[str, str]("test/decide")
RENDER = WaterfallEvent[str, str]("test/render")
NOTES = ServiceKey[list[str]]("notes")


class Agent:
    """Stands in for whatever object identifies a scope."""


async def test_a_scope_is_active_as_soon_as_it_is_created(ctx: Context) -> None:
    agent = Agent()

    scoped = ctx.scope(agent)

    assert scoped.scope_key is agent
    assert scoped.fiber.state is FiberState.ACTIVE
    assert scoped.fiber.name == "scope"
    assert scoped.fiber.parent is ctx.fiber
    assert ctx.scope_key is None
    assert ctx.fibers() == ()


async def test_a_scoped_dispatch_reaches_its_scope_and_global_listeners(ctx: Context) -> None:
    log: list[str] = []
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    ctx.on(PING, lambda payload: log.append("global"))
    first.on(PING, lambda payload: log.append("first"))
    second.on(PING, lambda payload: log.append("second"))

    first.emit(PING, "x")

    assert log == ["global", "first"]


async def test_an_unscoped_dispatch_reaches_every_listener(ctx: Context) -> None:
    log: list[str] = []
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    ctx.on(PING, lambda payload: log.append("global"))
    first.on(PING, lambda payload: log.append("first"))
    second.on(PING, lambda payload: log.append("second"))

    ctx.emit(PING, "x")

    assert log == ["global", "first", "second"]


async def test_every_dispatch_mode_respects_the_scope(ctx: Context) -> None:
    first = ctx.scope(Agent())
    second = ctx.scope(Agent())
    flushed: list[str] = []

    async def render(payload: str, next: Next[str, str]) -> str:
        return f"second({await next(payload)})"

    async def final(payload: str) -> str:
        return payload

    second.on(ASK, lambda payload: "second")
    second.on(DECIDE, lambda payload: "second")
    second.on(FLUSH, flushed.append)
    second.on(RENDER, render)

    assert first.bail(ASK, "x") is None
    assert await first.serial(DECIDE, "x") is None
    await first.parallel(FLUSH, "x")
    assert flushed == []
    assert await first.waterfall(RENDER, "x", final) == "x"

    assert second.bail(ASK, "x") == "second"
    assert await second.serial(DECIDE, "x") == "second"
    await second.parallel(FLUSH, "x")
    assert flushed == ["x"]
    assert await second.waterfall(RENDER, "x", final) == "second(x)"


async def test_a_plugin_loaded_in_a_scope_stays_in_that_scope(ctx: Context) -> None:
    log: list[str] = []
    agent = Agent()
    first = ctx.scope(agent)
    second = ctx.scope(Agent())

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)

    fiber = await first.plugin(Plugin("listener", apply))

    second.emit(PING, "second")
    first.emit(PING, "first")

    assert fiber.ctx.scope_key is agent
    assert log == ["first"]
    assert ctx.fibers() == (fiber,)


async def test_disposing_a_scope_undoes_what_was_registered_through_it(ctx: Context) -> None:
    log: list[str] = []
    scoped = ctx.scope(Agent())
    scoped.on(PING, log.append)
    scoped.effect(lambda: lambda: log.append("effect undone"))
    scoped.provide(NOTES, ["a note"])
    fiber = await scoped.plugin(Plugin("inside", lambda inner, config: None))
    assert ctx.use(NOTES) == ["a note"]

    await scoped.fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["effect undone"]
    assert scoped.fiber.state is FiberState.DISPOSED
    assert fiber.state is FiberState.DISPOSED
    assert ctx.get(NOTES) is None
    assert ctx.fiber.state is FiberState.ACTIVE


async def test_a_scope_is_disposed_with_the_fiber_that_created_it(ctx: Context) -> None:
    log: list[str] = []
    scopes: list[Context] = []

    def apply(inner: Context, config: object) -> None:
        scoped = inner.scope(Agent())
        scoped.on(PING, log.append)
        scopes.append(scoped)

    fiber = await ctx.plugin(Plugin("owner", apply))
    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == []
    assert scopes[0].fiber.state is FiberState.DISPOSED


async def test_a_nested_scope_replaces_the_key(ctx: Context) -> None:
    log: list[str] = []
    outer = ctx.scope(Agent())
    inner = outer.scope(Agent())
    outer.on(PING, lambda payload: log.append("outer"))
    inner.on(PING, lambda payload: log.append("inner"))

    inner.emit(PING, "x")

    assert log == ["inner"]
    assert inner.fiber.parent is outer.fiber


async def test_a_scope_inside_a_plugin_may_use_the_plugins_own_service(ctx: Context) -> None:
    seen: list[list[str]] = []

    def apply(inner: Context, config: object) -> None:
        inner.provide(NOTES, ["own"])
        seen.append(inner.scope(Agent()).use(NOTES))

    await ctx.plugin(Plugin("owner", apply))

    assert seen == [["own"]]


async def test_scopes_cannot_be_created_on_a_disposed_fiber() -> None:
    ctx = Context()
    await ctx.fiber.dispose()

    with pytest.raises(InactiveEffectError):
        ctx.scope(Agent())
