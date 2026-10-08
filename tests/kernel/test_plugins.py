import asyncio
import logging
from types import SimpleNamespace

import pytest
from pydantic import BaseModel, ValidationError

from tack.kernel import (
    Context,
    Disposer,
    EmitEvent,
    FiberState,
    InactiveEffectError,
    InvalidPluginError,
    Plugin,
    resolve_plugin,
)

PING = EmitEvent[str]("test/ping")


class Greeting(BaseModel):
    name: str = "world"


def nothing(ctx: Context, config: object) -> None:
    return None


async def test_a_plugin_loads_in_the_background_and_can_be_awaited(ctx: Context) -> None:
    seen: list[Context] = []

    def apply(inner: Context, config: object) -> None:
        seen.append(inner)

    fiber = ctx.plugin(Plugin("demo", apply))
    assert fiber.state is FiberState.PENDING
    assert seen == []

    assert await fiber is fiber

    assert fiber.state is FiberState.ACTIVE
    assert seen == [fiber.ctx]
    assert fiber.name == "demo"
    assert fiber.parent is ctx.fiber
    assert fiber.ctx is not ctx
    assert repr(fiber) == f"<Fiber 'demo' #{fiber.uid} active>"


async def test_apply_may_be_a_coroutine_function(ctx: Context) -> None:
    states: list[FiberState] = []

    async def apply(inner: Context, config: object) -> None:
        await asyncio.sleep(0)
        states.append(inner.fiber.state)

    fiber = await ctx.plugin(Plugin("demo", apply))

    assert states == [FiberState.LOADING]
    assert fiber.state is FiberState.ACTIVE


async def test_the_disposer_returned_by_apply_runs_last_in_first_out(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("first effect"))
        inner.effect(lambda: lambda: log.append("second effect"))
        return lambda: log.append("returned disposer")

    fiber = await ctx.plugin(Plugin("demo", apply))
    await fiber.dispose()

    assert log == ["returned disposer", "second effect", "first effect"]
    assert fiber.state is FiberState.DISPOSED


async def test_config_is_validated_against_the_model(ctx: Context) -> None:
    seen: list[Greeting] = []

    def apply(inner: Context, config: Greeting) -> None:
        seen.append(config)

    fiber = await ctx.plugin(Plugin("demo", apply, config=Greeting), {"name": "tack"})

    assert seen == [Greeting(name="tack")]
    assert fiber.config == Greeting(name="tack")


async def test_a_missing_config_uses_the_model_defaults(ctx: Context) -> None:
    fiber = await ctx.plugin(Plugin("demo", nothing, config=Greeting))

    assert fiber.config == Greeting(name="world")


async def test_config_without_a_model_is_passed_through(ctx: Context) -> None:
    fiber = await ctx.plugin(Plugin("demo", nothing), {"anything": 1})

    assert fiber.config == {"anything": 1}


async def test_an_invalid_config_fails_without_running_apply(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    calls: list[str] = []

    def apply(inner: Context, config: Greeting) -> None:
        calls.append("applied")

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("demo", apply, config=Greeting), {"name": ["not", "text"]})

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, ValidationError)
    assert "demo" in caplog.text
    with pytest.raises(ValidationError):
        await fiber
    assert calls == []


async def test_a_plugin_that_raises_fails_and_rolls_back(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)
        inner.effect(lambda: lambda: log.append("rolled back"))
        raise RuntimeError("cannot start")

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        fiber = ctx.plugin(Plugin("demo", apply))
        with pytest.raises(RuntimeError, match="cannot start"):
            await fiber

    assert fiber.state is FiberState.FAILED
    assert isinstance(fiber.error, RuntimeError)
    assert log == ["rolled back"]
    assert "cannot start" in caplog.text

    ctx.emit(PING, "after failure")
    assert log == ["rolled back"]


async def test_a_failed_plugin_can_be_disposed(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> None:
        raise RuntimeError("cannot start")

    fiber = ctx.plugin(Plugin("demo", apply))
    with pytest.raises(RuntimeError):
        await fiber

    await fiber.dispose()

    assert fiber.state is FiberState.DISPOSED
    assert await fiber is fiber


async def test_apply_must_return_a_disposer_or_none(ctx: Context) -> None:
    def apply(inner: Context, config: object) -> int:
        return 42

    fiber = ctx.plugin(Plugin("demo", apply))  # pyright: ignore[reportArgumentType]

    with pytest.raises(InvalidPluginError, match="disposer or None"):
        await fiber
    assert fiber.state is FiberState.FAILED


async def test_disposing_a_plugin_removes_its_registrations(ctx: Context) -> None:
    log: list[str] = []

    def apply(inner: Context, config: object) -> None:
        inner.on(PING, log.append)

    fiber = await ctx.plugin(Plugin("demo", apply))
    ctx.emit(PING, "before")

    await fiber.dispose()
    ctx.emit(PING, "after")

    assert log == ["before"]
    assert fiber not in ctx.fibers()


async def test_nested_plugins_are_disposed_with_their_parent(ctx: Context) -> None:
    log: list[str] = []
    children: list[object] = []

    def child(inner: Context, config: object) -> Disposer:
        return lambda: log.append("child")

    def parent(inner: Context, config: object) -> Disposer:
        inner.effect(lambda: lambda: log.append("parent effect before child"))
        children.append(inner.plugin(Plugin("child", child)))
        return lambda: log.append("parent")

    fiber = await ctx.plugin(Plugin("parent", parent))
    await ctx.settled()

    await fiber.dispose()

    assert log == ["parent", "child", "parent effect before child"]
    assert [nested.state for nested in ctx.fibers()] == []


async def test_a_parent_can_wait_for_its_child_while_loading(ctx: Context) -> None:
    states: list[FiberState] = []

    async def parent(inner: Context, config: object) -> None:
        child = await inner.plugin(Plugin("child", nothing))
        states.append(child.state)

    await ctx.plugin(Plugin("parent", parent))

    assert states == [FiberState.ACTIVE]


async def test_disposing_the_root_disposes_every_plugin() -> None:
    ctx = Context()
    first = await ctx.plugin(Plugin("first", nothing))
    second = await ctx.plugin(Plugin("second", nothing))

    await ctx.fiber.dispose()

    assert first.state is FiberState.DISPOSED
    assert second.state is FiberState.DISPOSED
    assert ctx.fibers() == ()


async def test_plugins_cannot_be_added_to_a_disposed_fiber() -> None:
    ctx = Context()
    await ctx.fiber.dispose()

    with pytest.raises(InactiveEffectError):
        ctx.plugin(Plugin("late", nothing))

    assert ctx.fibers() == ()


async def test_settled_waits_for_every_plugin(ctx: Context) -> None:
    async def slow(inner: Context, config: object) -> None:
        await asyncio.sleep(0)
        await asyncio.sleep(0)

    first = ctx.plugin(Plugin("first", slow))
    second = ctx.plugin(Plugin("second", slow))

    await ctx.settled()

    assert first.state is FiberState.ACTIVE
    assert second.state is FiberState.ACTIVE
    assert ctx.fibers() == (first, second)


async def test_a_plugin_can_dispose_itself_while_loading(ctx: Context) -> None:
    async def apply(inner: Context, config: object) -> None:
        await inner.fiber.dispose()

    fiber = ctx.plugin(Plugin("demo", apply))
    await ctx.settled()

    assert fiber.state is FiberState.DISPOSED


async def test_a_module_like_object_is_a_plugin(ctx: Context) -> None:
    seen: list[Greeting] = []

    def apply(inner: Context, config: Greeting) -> None:
        seen.append(config)

    module = SimpleNamespace(name="greeter", apply=apply, Config=Greeting)
    fiber = await ctx.plugin(module, {"name": "tack"})

    assert fiber.name == "greeter"
    assert seen == [Greeting(name="tack")]


async def test_a_bare_function_is_a_plugin(ctx: Context) -> None:
    def greeter(inner: Context, config: object) -> None:
        return None

    fiber = await ctx.plugin(greeter)

    assert fiber.name == "greeter"


def test_resolve_plugin_returns_a_plugin_unchanged() -> None:
    plugin = Plugin("demo", nothing)

    assert resolve_plugin(plugin) is plugin


def test_resolve_plugin_reads_the_module_name_when_no_name_is_given() -> None:
    module = SimpleNamespace(__name__="pkg.greeter", apply=nothing)

    assert resolve_plugin(module).name == "pkg.greeter"


@pytest.mark.parametrize(
    "candidate",
    [
        42,
        SimpleNamespace(apply="not callable"),
        SimpleNamespace(apply=nothing, Config=dict),
        SimpleNamespace(apply=nothing, name=7),
    ],
)
def test_resolve_plugin_rejects_other_objects(candidate: object) -> None:
    with pytest.raises(InvalidPluginError):
        resolve_plugin(candidate)


def test_a_plugin_checks_what_it_is_given() -> None:
    with pytest.raises(InvalidPluginError, match="needs a name"):
        Plugin("", nothing)

    with pytest.raises(InvalidPluginError, match="must be callable"):
        Plugin("demo", "not callable")  # pyright: ignore[reportArgumentType]

    with pytest.raises(InvalidPluginError, match="pydantic model"):
        Plugin("demo", nothing, config=dict)


def test_pyright_rejects_a_config_model_that_apply_does_not_accept() -> None:
    """A static check: the ignore below fails the type check once it is unnecessary."""

    class Other(BaseModel):
        size: int = 0

    def apply(inner: Context, config: Greeting) -> None:
        return None

    plugin = Plugin("mismatched", apply, config=Other)  # pyright: ignore[reportArgumentType]

    assert plugin.config is Other
