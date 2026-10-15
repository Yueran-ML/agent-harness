"""A service process in the loop of a test, with the interface that the test plays."""

import asyncio
from dataclasses import dataclass
from typing import Any

import columbina.apps.serve
import columbina.commands
from columbina.agent import Agent
from columbina.apps import FRONTEND
from columbina.apps.serve.wire import WIRE
from support.loop import MODEL, Harness
from support.wires import Far

__all__ = ["WINDOW", "Served", "loaded"]

WINDOW = 32_000
"""How many tokens the model of a process that knows its window takes."""


@dataclass
class Served:
    """A service process in the loop of a test, and the interface that a test plays.

    Attributes:
        harness: The services that the process is built on, with a model
            whose replies a test writes down.
        far: The far end of the wire.
        running: The process at work. It gives the exit code.
    """

    harness: Harness
    far: Far
    running: "asyncio.Future[int]"

    async def greeted(self) -> dict[str, Any]:
        """Ask for the first thing that is asked for, and return the answer."""
        return await self.far.result("initialize", {"protocol": 1})

    async def opened(self, session_id: str | None = None) -> dict[str, Any]:
        """Greet if nobody has, open a session, and return the answer."""
        if not self.far.heard:
            await self.greeted()
        params = {} if session_id is None else {"session_id": session_id}
        return await self.far.result("session/open", params)

    def agent(self) -> Agent:
        """The one agent that is running."""
        (agent,) = self.harness.agents.live.values()
        return agent

    async def left(self) -> int:
        """Wait for the process to come to its end, and return its exit code."""
        return await asyncio.wait_for(self.running, 10)


async def loaded(harness: Harness, far: Far | None, **config: Any) -> "asyncio.Future[int]":
    """Load the service process on top of the loop of the harness, and start it."""
    if far is not None:
        harness.ctx.provide(WIRE, far)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(
        columbina.apps.serve, {"model": MODEL.model_dump(), "cwd": harness.cwd, **config}
    )
    return asyncio.ensure_future(harness.ctx.use(FRONTEND).run())
