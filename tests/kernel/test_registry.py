"""A registry holds named entries, each owned by the plugin instance that added it."""

import pytest

from columbina.kernel import Context, InactiveEffectError, Plugin, Registry, RegistryConflictError


async def test_an_entry_is_found_by_its_name(ctx: Context) -> None:
    tools = Registry[int]("tool")
    tools.add(ctx, "read", 1)

    assert tools["read"] == 1
    assert tools.get("read") == 1
    assert "read" in tools
    assert tools.get("write") is None
    assert "write" not in tools
    with pytest.raises(KeyError):
        tools["write"]


async def test_entries_come_in_the_order_they_were_added(ctx: Context) -> None:
    tools = Registry[int]("tool")
    tools.add(ctx, "write", 2)
    tools.add(ctx, "read", 1)

    assert list(tools) == ["write", "read"]
    assert list(tools.values()) == [2, 1]
    assert list(tools.items()) == [("write", 2), ("read", 1)]
    assert len(tools) == 2


async def test_a_name_can_be_taken_only_once(ctx: Context) -> None:
    tools = Registry[int]("tool")
    tools.add(ctx, "read", 1)

    with pytest.raises(RegistryConflictError, match="the tool 'read' is already registered"):
        tools.add(ctx, "read", 2)

    assert tools["read"] == 1


async def test_an_entry_leaves_with_the_plugin_that_added_it(ctx: Context) -> None:
    tools = Registry[int]("tool")

    def apply(inner: Context, config: object) -> None:
        tools.add(inner, "read", 1)

    fiber = await ctx.plugin(Plugin("reader", apply))
    assert list(tools) == ["read"]

    await fiber.dispose()

    assert list(tools) == []


async def test_an_entry_can_be_taken_back_through_its_handle(ctx: Context) -> None:
    tools = Registry[int]("tool")
    handle = tools.add(ctx, "read", 1)

    await handle.dispose()

    assert "read" not in tools


async def test_a_handle_that_was_used_does_not_remove_a_later_entry(ctx: Context) -> None:
    tools = Registry[int]("tool")
    handle = tools.add(ctx, "read", 1)
    await handle.dispose()
    tools.add(ctx, "read", 2)

    await handle.dispose()

    assert tools["read"] == 2


async def test_a_plugin_that_has_unloaded_cannot_add_an_entry(ctx: Context) -> None:
    tools = Registry[int]("tool")
    kept: list[Context] = []

    def apply(inner: Context, config: object) -> None:
        kept.append(inner)

    fiber = await ctx.plugin(Plugin("gone", apply))
    await fiber.dispose()

    with pytest.raises(InactiveEffectError):
        tools.add(kept[0], "read", 1)

    assert list(tools) == []


def test_two_registries_are_equal_only_if_they_are_the_same_one() -> None:
    tools = Registry[int]("tool")

    assert tools == tools
    assert tools != Registry[int]("tool")
    assert len({tools, tools, Registry[int]("tool")}) == 2


def test_a_registry_describes_itself() -> None:
    assert repr(Registry[int]("tool")) == "Registry('tool', [])"
