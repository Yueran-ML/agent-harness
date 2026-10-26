"""Tasks that run at set times, while the program that holds conversations runs.

This module is a plugin. Loading it looks at the tasks of :mod:`.store` every so many seconds for
as long as the program runs, and runs each task when it is due: an agent of its own, in the
working directory of the task, with a session of its own, is asked the prompt of the task, and
the end of its turn is written down beside the task. :data:`SCHEDULE_DONE` is emitted then, for an
interface to tell the user.

Nobody watches such a run, so nobody is asked anything: a call that would need a yes does not
run, as where nobody answers. The sandbox and the rules hold as in any conversation, in the mode
of ``workspace-write``. A task that runs is not begun again before it has ended.

The desktop application runs the service, and so runs the tasks while it is open. Nothing runs
them while it is closed: a time that was missed is made up for, or passed over, as the task says,
the next time the application is open.
"""

import asyncio
import contextlib
import logging
import time
from dataclasses import dataclass
from pathlib import Path

from pydantic import BaseModel, Field

from columbina.agent import AGENTS, Agents, AgentSpec
from columbina.apps.common import DEFAULT_PROMPT
from columbina.kernel import Context, EmitEvent
from columbina.llm import CallConfig
from columbina.permissions import PERMISSIONS
from columbina.schedules.store import Schedule, Schedules, SchedulesError, changed, due, read
from columbina.session import TurnEndEvent

__all__ = ["SCHEDULE_DONE", "Config", "Done", "Runner", "apply", "inject", "name"]

name = "schedules"
inject = (AGENTS,)

logger = logging.getLogger("columbina.schedules")


@dataclass(frozen=True, slots=True)
class Done:
    """A task that ran, or could not.

    Attributes:
        id: The task.
        name: What the user calls it.
        session_id: The session of the run, if there was one.
        end: How it ended: how its turn ended, or what kept it from running.
    """

    id: str
    name: str
    session_id: str | None
    end: str


SCHEDULE_DONE: EmitEvent[Done] = EmitEvent("schedules/done")
"""Emitted when a task has run, or could not."""


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        home: The home of the program, where the file of the tasks is.
        model: The model that a task asks.
        seconds: Every how many seconds the tasks are looked at.
    """

    home: Path
    model: CallConfig
    seconds: float = Field(default=30.0, gt=0)


def _now() -> int:
    return round(time.time() * 1000)


class Runner:
    """Looks at the tasks, and runs those that are due."""

    def __init__(self, ctx: Context, config: Config, agents: Agents) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._running: dict[str, asyncio.Task[None]] = {}
        self.clock = _now

    async def look(self) -> None:
        """Run what is due now, and pass over what is to be passed over."""
        try:
            schedules = await asyncio.to_thread(read, self._config.home)
        except SchedulesError as error:
            logger.warning("the tasks cannot be looked at: %s", error)
            return
        now = self.clock()
        for schedule in schedules.schedules:
            if schedule.id in self._running:
                continue
            what = due(schedule, now)
            if what == "skip":
                await self._noted(schedule.id, now, None, "passed over: its time was missed")
            elif what == "run":
                await self._noted(schedule.id, now, None, "running")
                task = asyncio.ensure_future(self._run(schedule))
                self._running[schedule.id] = task
                task.add_done_callback(lambda done, key=schedule.id: self._running.pop(key, None))

    async def _noted(self, key: str, at: int, session_id: str | None, end: str) -> None:
        def note(schedules: Schedules) -> Schedules:
            for schedule in schedules.schedules:
                if schedule.id == key:
                    schedule.last = at
                    schedule.last_session = session_id or schedule.last_session
                    schedule.last_end = end
            return schedules

        try:
            await asyncio.to_thread(changed, self._config.home, note)
        except SchedulesError as error:
            logger.warning("what became of the task %s cannot be written down: %s", key, error)

    async def _run(self, schedule: Schedule) -> None:
        """Run a task: an agent of its own is asked its prompt, and how that ended is noted."""
        session_id = None
        end = "it did not end"
        try:
            spec = AgentSpec(
                cwd=schedule.cwd, model=self._config.model, system_prompt=DEFAULT_PROMPT
            )
            agent = await self._agents.create(self._ctx, spec)
            session_id = agent.id
            permissions = self._ctx.get(PERMISSIONS)
            if permissions is not None:
                permissions.set_preset(agent.session, "workspace-write")
            began = len(agent.session.events)
            agent.followup(schedule.prompt)
            try:
                await agent.when_idle()
            finally:
                await agent.close()
            ended = [
                event.data.reason
                for event in agent.session.events[began:]
                if isinstance(event.data, TurnEndEvent)
            ]
            end = next(reversed(ended), end)
        except asyncio.CancelledError:
            end = "stopped: the program ended"
            raise
        except Exception as error:
            logger.warning("the task %s could not run: %s", schedule.id, error)
            end = f"it could not run: {error}"
        finally:
            with contextlib.suppress(Exception):
                await asyncio.shield(self._noted(schedule.id, self.clock(), session_id, end))
            self._ctx.emit(SCHEDULE_DONE, Done(schedule.id, schedule.name, session_id, end))

    async def run_forever(self) -> None:
        """Look at the tasks every so many seconds, until the program ends."""
        while True:
            await self.look()
            await asyncio.sleep(self._config.seconds)

    async def settled(self) -> None:
        """Wait until what runs has ended."""
        await asyncio.gather(*self._running.values(), return_exceptions=True)

    async def stop(self) -> None:
        """Stop what runs."""
        for task in list(self._running.values()):
            task.cancel()
        await asyncio.gather(*self._running.values(), return_exceptions=True)


def apply(ctx: Context, config: Config) -> None:
    """Look at the tasks and run them, for as long as the program runs."""
    runner = Runner(ctx, config, ctx.use(AGENTS))
    looking = asyncio.ensure_future(runner.run_forever())

    async def stopped() -> None:
        looking.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await looking
        await runner.stop()

    ctx.effect(lambda: stopped)
