"""What the model is sent, over a session in which a bit of everything happens.

The requests of the session are kept as text in ``snapshots/a_session.txt``,
which is the place to read what the model sees after a tool failed, after the
user steered, and after a turn was cut short.
"""

import asyncio
from pathlib import Path

from pydantic import BaseModel

from support.loop import Harness, idle
from support.snapshot import stored
from tack.kernel import Next
from tack.llm import render_requests
from tack.llm.adapters.scripted import Call, Reply
from tack.session import TurnEndEvent
from tack.tools import PRE_EXECUTE, Decision, Deny, Invocation, Tool, ToolFailure, ToolRun

SNAPSHOTS = Path(__file__).parent / "snapshots"


class File(BaseModel):
    path: str


class Nothing(BaseModel):
    pass


async def test_what_the_model_is_sent_over_a_session(harness: Harness) -> None:
    building = asyncio.Event()

    async def look(args: File, run: ToolRun) -> str:
        return f"{args.path} has 3 lines"

    async def remove(args: File, run: ToolRun) -> str:
        raise ToolFailure(f"{args.path} is in use")

    async def build(args: Nothing, run: ToolRun) -> str:
        building.set()
        await asyncio.Event().wait()
        return "built"

    async def guard(invocation: Invocation, proceed: Next[Invocation, Decision]) -> Decision:
        if isinstance(invocation.args, File) and invocation.args.path.startswith("secret"):
            return Deny(reason="secrets stay closed")
        return await proceed(invocation)

    harness.tools.add(
        harness.ctx,
        Tool(
            name="look",
            description="Look at a file.",
            parameters=File,
            execute=look,
            concurrency_safe=True,
        ),
        Tool(name="remove", description="Remove a file.", parameters=File, execute=remove),
        Tool(name="build", description="Build the project.", parameters=Nothing, execute=build),
    )
    agent = await harness.agent(
        # Turn 1: calls that end in every way a call can end.
        Reply(
            reasoning="Two files first.",
            text="I will look around.",
            calls=(
                Call(name="look", arguments={"path": "a.txt"}),
                Call(name="look", arguments={"path": "b.txt"}),
                Call(name="remove", arguments={"path": "c.txt"}),
                Call(name="look", arguments={"path": "secret.txt"}),
                Call(name="look", arguments='{"path": '),
                Call(name="open", arguments={"path": "d.txt"}),
            ),
        ),
        Reply(text="Two files are there. The rest could not be done."),
        # Turn 2: context that waited for the task, and a steer while the turn runs.
        Reply(text="Summarising a.txt.", hold=True),
        Reply(text="It is short."),
        # Turn 3: a reply that is cut short.
        Reply(reasoning="This will take a while.", text="First, ", hold=True),
        # Turn 4: a tool that is cut short.
        Reply(calls=(Call(name="build"),)),
        # Turn 5: what is left of all that.
        Reply(text="Going on."),
    )
    agent.ctx.on(PRE_EXECUTE, guard)

    agent.followup("Look at a.txt and b.txt, then tidy up.")
    await idle(agent)

    agent.inject("The user has a.txt open.")
    agent.followup("Summarise it.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.steer("Keep it short.")
    harness.model.release()
    await idle(agent)

    agent.followup("Explain the build.")
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.cancel()
    await idle(agent)

    agent.followup("Build it.")
    await asyncio.wait_for(building.wait(), timeout=5)
    agent.cancel()
    await idle(agent)

    agent.followup("Go on.")
    await idle(agent)

    ends = [e.data.reason for e in agent.session.events if isinstance(e.data, TurnEndEvent)]
    assert ends == ["completed", "completed", "aborted", "aborted", "completed"]
    seen = render_requests(harness.model.requests) + "\n"
    assert seen == stored(SNAPSHOTS / "a_session.txt", seen)
