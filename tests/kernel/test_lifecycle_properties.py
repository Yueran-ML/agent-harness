"""Property test: random plugin graphs under random loads, disposals and withdrawals.

Each generated scenario declares a few services and a few plugins. Every
service has one owner: the root context, one of the plugins, or nobody. Every
plugin injects some of the services, and may be slow to load or to clean up. A
scenario then plays a random sequence of steps against a real kernel and
checks, each time the kernel comes to rest, that the kernel is exactly where
the declarations say it should be.
"""

import asyncio
from collections import Counter
from dataclasses import dataclass

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from tack.kernel import (
    Context,
    Disposer,
    EffectHandle,
    Fiber,
    FiberState,
    Plugin,
    ServiceConflictError,
    ServiceKey,
)

ROOT = -1
NOBODY = -2


class Token:
    """Stands in for one provided service instance."""


@dataclass(frozen=True)
class Scenario:
    owners: tuple[int, ...]
    """The owner of each service: a plugin index, ``ROOT`` or ``NOBODY``."""
    injects: tuple[frozenset[int], ...]
    """The services each plugin injects."""
    slow: tuple[tuple[bool, bool], ...]
    """Whether each plugin yields to the event loop while loading and while cleaning up."""
    steps: tuple[tuple[str, int], ...]


PLUGIN_ACTIONS = ("add", "dispose", "dispose_later")
# Loads and provisions are listed more often than their opposites, so that most
# scenarios spend their steps on a populated graph rather than an empty one.
ACTIONS = (
    *("add",) * 5,
    *("provide",) * 3,
    "withdraw",
    "withdraw_later",
    "dispose",
    "dispose_later",
    *("yield",) * 3,
    "settle",
)


async def linger() -> None:
    """Stay unfinished for a few turns of the event loop."""
    for _ in range(4):
        await asyncio.sleep(0)


@st.composite
def scenarios(draw: st.DrawFn) -> Scenario:
    plugin_count = draw(st.integers(min_value=2, max_value=4))
    service_count = draw(st.integers(min_value=1, max_value=3))
    owner = st.sampled_from([ROOT, ROOT, NOBODY, *range(plugin_count), *range(plugin_count)])
    owners = tuple(draw(owner) for _ in range(service_count))
    services = st.integers(min_value=0, max_value=service_count - 1)
    injects = tuple(frozenset(draw(st.sets(services, max_size=2))) for _ in range(plugin_count))
    slow = tuple(draw(st.tuples(st.booleans(), st.booleans())) for _ in range(plugin_count))
    raw_steps = draw(
        st.lists(
            st.tuples(st.sampled_from(ACTIONS), st.integers(min_value=0, max_value=11)),
            min_size=4,
            max_size=40,
        )
    )
    steps = tuple(
        (action, number % (plugin_count if action in PLUGIN_ACTIONS else service_count))
        for action, number in raw_steps
    )
    return Scenario(owners, injects, slow, steps)


class Run:
    """Plays one scenario against a kernel and tracks what should be true."""

    def __init__(self, scenario: Scenario) -> None:
        self.scenario = scenario
        self.ctx = Context()
        self.keys = [ServiceKey[Token](f"service-{s}") for s in range(len(scenario.owners))]
        self.fibers: dict[int, Fiber] = {}
        self.retired: list[Fiber] = []
        self.handles: dict[int, EffectHandle] = {}
        self.withdrawals: dict[int, asyncio.Future[None]] = {}
        self.tokens: dict[int, Token] = {}
        self.seen: dict[int, dict[int, Token]] = {}
        self.loaded: set[int] = set()
        self.loads: Counter[int] = Counter()
        self.unloads: Counter[int] = Counter()
        self.pending: list[asyncio.Future[None]] = []
        self.violations: list[str] = []

    def dependents_of(self, service: int) -> list[int]:
        return [p for p, injects in enumerate(self.scenario.injects) if service in injects]

    def plugin(self, index: int) -> Plugin[object]:
        provides = [s for s, owner in enumerate(self.scenario.owners) if owner == index]
        injects = sorted(self.scenario.injects[index])
        slow_to_load, slow_to_clean_up = self.scenario.slow[index]

        async def apply(ctx: Context, config: object) -> Disposer:
            self.seen[index] = {s: ctx.use(self.keys[s]) for s in injects}
            self.loaded.add(index)
            self.loads[index] += 1
            if slow_to_load:
                await linger()
            for service in provides:
                token = Token()
                ctx.provide(self.keys[service], token)
                self.tokens[service] = token

            async def undo() -> None:
                # A plugin counts as loaded until its cleanup has finished, so
                # a provider that starts cleaning up during it is caught here.
                for service in provides:
                    for dependent in self.dependents_of(service):
                        if dependent != index and dependent in self.loaded:
                            self.violations.append(
                                f"plugin {index} cleaned up while its dependent "
                                f"{dependent} was still loaded"
                            )
                if slow_to_clean_up:
                    await linger()
                for service in provides:
                    del self.tokens[service]
                self.loaded.discard(index)
                self.unloads[index] += 1

            return undo

        return Plugin(f"plugin-{index}", apply, inject=tuple(self.keys[s] for s in injects))

    async def withdraw(self, service: int, handle: EffectHandle) -> None:
        await handle.dispose()
        del self.tokens[service]
        for dependent in self.dependents_of(service):
            if dependent in self.loaded:
                self.violations.append(
                    f"service {service} was withdrawn while its dependent "
                    f"{dependent} was still loaded"
                )

    def provide(self, service: int) -> None:
        if self.scenario.owners[service] != ROOT or service in self.handles:
            return
        withdrawal = self.withdrawals.get(service)
        if withdrawal is not None and not withdrawal.done():
            with pytest.raises(ServiceConflictError):
                self.ctx.provide(self.keys[service], Token())
            return
        token = Token()
        self.handles[service] = self.ctx.provide(self.keys[service], token)
        self.tokens[service] = token

    async def step(self, action: str, target: int) -> None:
        if action == "add":
            previous = [fiber for fiber in self.retired if fiber.name == f"plugin-{target}"]
            if target not in self.fibers and all(
                fiber.state is FiberState.DISPOSED for fiber in previous
            ):
                self.fibers[target] = self.ctx.plugin(self.plugin(target))
        elif action in ("dispose", "dispose_later"):
            fiber = self.fibers.pop(target, None)
            if fiber is not None:
                self.retired.append(fiber)
                disposal = asyncio.ensure_future(fiber.dispose())
                if action == "dispose":
                    await disposal
                else:
                    self.pending.append(disposal)
        elif action == "provide":
            self.provide(target)
        elif action in ("withdraw", "withdraw_later"):
            handle = self.handles.pop(target, None)
            if handle is not None:
                withdrawal = asyncio.ensure_future(self.withdraw(target, handle))
                self.withdrawals[target] = withdrawal
                if action == "withdraw":
                    await withdrawal
                else:
                    self.pending.append(withdrawal)
        elif action == "yield":
            await asyncio.sleep(0)
        else:
            await self.settle()
            self.check()

    async def settle(self) -> None:
        while self.pending:
            pending, self.pending = self.pending, []
            await asyncio.gather(*pending)
        await self.ctx.settled()

    def expected_active(self) -> set[int]:
        """The least fixed point: who can be active given who is live and what root provides."""
        available = set(self.handles)
        active: set[int] = set()
        progressed = True
        while progressed:
            progressed = False
            for index in set(self.fibers) - active:
                if self.scenario.injects[index] <= available:
                    active.add(index)
                    available |= {
                        s for s, owner in enumerate(self.scenario.owners) if owner == index
                    }
                    progressed = True
        return active

    def check(self) -> None:
        active = self.expected_active()
        for index, fiber in self.fibers.items():
            expected = FiberState.ACTIVE if index in active else FiberState.PENDING
            assert fiber.state is expected, (index, fiber.state, expected)
            assert self.loads[index] - self.unloads[index] == (1 if index in active else 0)
            if index in active:
                for service in self.scenario.injects[index]:
                    assert self.seen[index][service] is self.tokens[service], (index, service)
        assert self.loaded == active
        for service, key in enumerate(self.keys):
            assert self.ctx.get(key) is self.tokens.get(service), service
        assert self.violations == []

    async def finish(self) -> None:
        """Withdraw one root service, then dispose the root, checking at each rest.

        The other root services stay provided, so that disposing the root also
        has dependents to unload.
        """
        await self.settle()
        self.check()
        if self.handles:
            await self.step("withdraw", min(self.handles))
            await self.settle()
            self.check()
        fibers = [*self.fibers.values(), *self.retired]
        await self.ctx.fiber.dispose()
        assert all(fiber.state is FiberState.DISPOSED for fiber in fibers)
        assert self.ctx.fibers() == ()
        assert self.loaded == set()
        assert self.loads == self.unloads
        assert all(self.ctx.get(key) is None for key in self.keys)
        assert self.violations == []


async def play(scenario: Scenario) -> None:
    run = Run(scenario)
    for action, target in scenario.steps:
        await run.step(action, target)
    await run.finish()


@settings(max_examples=300, deadline=None)
@given(scenarios())
def test_plugins_are_active_exactly_when_their_services_are(scenario: Scenario) -> None:
    asyncio.run(play(scenario))
