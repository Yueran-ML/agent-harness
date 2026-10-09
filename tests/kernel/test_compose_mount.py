"""Mounting a list of entries: importing the plugins and loading them."""

import sys
from pathlib import Path

import pytest

from columbina.kernel import (
    CompositionError,
    Context,
    Entry,
    FiberState,
    ServiceKey,
    import_plugin,
    mount,
)

REPORT = ServiceKey[dict[str, object]]("report")

CLOCK = Entry(id="clock", plugin="compose_demo.clock", config={"now": 7})
REPORTER = Entry(id="reporter", plugin="compose_demo.reporter", config={"label": "demo"})
BROKEN = Entry(id="broken", plugin="compose_demo.extras:broken")


@pytest.fixture(autouse=True)
def demo_plugins_can_be_imported(monkeypatch: pytest.MonkeyPatch) -> None:
    fixtures = str(Path(__file__).parent / "fixtures")
    monkeypatch.setattr(sys, "path", [fixtures, *sys.path])


async def test_mount_loads_the_entries_whatever_their_order(ctx: Context) -> None:
    composition = mount(ctx, [REPORTER, CLOCK])
    await ctx.settled()

    assert list(composition.fibers) == ["reporter", "clock"]
    assert composition.fibers["clock"].state is FiberState.ACTIVE
    assert composition.fibers["clock"].name == "clock"
    assert ctx.use(REPORT) == {"label": "demo", "port": 0, "now": 7}
    assert composition.problems() == []


async def test_a_disabled_entry_is_not_even_imported(ctx: Context) -> None:
    missing = Entry(id="off", plugin="compose_demo.does_not_exist", disabled=True)

    composition = mount(ctx, [CLOCK, missing])
    await ctx.settled()

    assert list(composition.fibers) == ["clock"]


async def test_config_strings_are_filled_in_from_the_environment(ctx: Context) -> None:
    reporter = Entry(
        id="reporter",
        plugin="compose_demo.reporter",
        config={"label": "${LABEL:-fallback}", "port": "${PORT}"},
    )

    mount(ctx, [CLOCK, reporter], env={"PORT": "8080"})
    await ctx.settled()

    assert ctx.use(REPORT) == {"label": "fallback", "port": 8080, "now": 7}


async def test_problems_name_the_services_an_entry_is_waiting_for(ctx: Context) -> None:
    composition = mount(ctx, [REPORTER])
    await ctx.settled()

    assert composition.problems() == [
        "reporter (compose_demo.reporter) is waiting for the service 'clock'"
    ]


async def test_problems_report_an_entry_that_failed_to_load(ctx: Context) -> None:
    composition = mount(ctx, [CLOCK, BROKEN])
    await ctx.settled()

    assert composition.problems() == [
        "broken (compose_demo.extras:broken) failed: RuntimeError: cannot start"
    ]


async def test_problems_report_an_invalid_config(ctx: Context) -> None:
    clock = Entry(id="clock", plugin="compose_demo.clock", config={"now": "soon"})

    composition = mount(ctx, [clock])
    await ctx.settled()

    (problem,) = composition.problems()
    assert problem.startswith("clock (compose_demo.clock) failed: ValidationError: ")
    assert "now" in problem


async def test_nothing_is_loaded_when_an_entry_cannot_be_imported(ctx: Context) -> None:
    missing = Entry(id="missing", plugin="compose_demo.does_not_exist")

    with pytest.raises(CompositionError, match="entry 'missing': cannot import"):
        mount(ctx, [CLOCK, missing])

    assert ctx.fibers() == ()


async def test_nothing_is_loaded_when_a_variable_is_missing(ctx: Context) -> None:
    reporter = Entry(id="reporter", plugin="compose_demo.reporter", config={"port": "${PORT}"})

    with pytest.raises(CompositionError, match="entry 'reporter': environment variable 'PORT'"):
        mount(ctx, [CLOCK, reporter], env={})

    assert ctx.fibers() == ()


async def test_two_entries_cannot_share_an_id(ctx: Context) -> None:
    with pytest.raises(CompositionError, match="entry 'clock' appears twice"):
        mount(ctx, [CLOCK, CLOCK])

    assert ctx.fibers() == ()


def test_import_plugin_accepts_a_module_or_an_attribute() -> None:
    assert import_plugin("compose_demo.clock").name == "clock"
    assert import_plugin("compose_demo.extras:broken").name == "broken"


@pytest.mark.parametrize(
    ("path", "complaint"),
    [
        ("compose_demo.does_not_exist", "cannot import"),
        ("compose_demo.extras:nowhere", "has no attribute 'nowhere'"),
        ("compose_demo.extras:not_a_plugin", "is not a plugin"),
        ("compose_demo.keys", "is not a plugin"),
        ("compose_demo.explodes", "cannot import 'compose_demo.explodes': RuntimeError"),
        ("", "is not an import path"),
        (":apply", "is not an import path"),
        (".relative", "is not an import path"),
        ("compose_demo.clock:", "is not an import path"),
    ],
)
def test_import_plugin_explains_what_it_could_not_use(path: str, complaint: str) -> None:
    with pytest.raises(CompositionError, match=complaint):
        import_plugin(path)


async def test_problems_report_an_entry_that_was_disposed(ctx: Context) -> None:
    composition = mount(ctx, [CLOCK])
    await ctx.settled()

    await composition.fibers["clock"].dispose()

    assert composition.problems() == ["clock (compose_demo.clock) is disposed"]


async def test_problems_list_every_missing_service(ctx: Context) -> None:
    composition = mount(ctx, [Entry(id="needy", plugin="compose_demo.extras:needy")])
    await ctx.settled()

    assert composition.problems() == [
        "needy (compose_demo.extras:needy) is waiting for the services 'clock', 'store'"
    ]


async def test_problems_report_a_disposed_entry_even_if_it_injects_services(ctx: Context) -> None:
    composition = mount(ctx, [Entry(id="needy", plugin="compose_demo.extras:needy")])
    await ctx.settled()

    await composition.fibers["needy"].dispose()

    assert composition.problems() == ["needy (compose_demo.extras:needy) is disposed"]


async def test_a_config_that_cannot_be_validated_does_not_stop_the_other_entries(
    ctx: Context,
) -> None:
    fussy = Entry(id="fussy", plugin="compose_demo.extras:fussy", config={"number": "many"})

    composition = mount(ctx, [CLOCK, fussy])
    await ctx.settled()

    assert composition.fibers["clock"].state is FiberState.ACTIVE
    assert composition.problems() == [
        "fussy (compose_demo.extras:fussy) failed: TypeError: cannot read 'many'"
    ]
