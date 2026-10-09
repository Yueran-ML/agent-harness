import os
from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from hypothesis import settings

import columbina.agent
import columbina.agent_loop
import columbina.llm
import columbina.session
import columbina.tools
from columbina.agent import AGENTS
from columbina.kernel import Context
from columbina.llm import LLM
from columbina.llm.adapters.scripted import Scripted
from columbina.session import SESSIONS
from columbina.tools import TOOLS
from support.loop import Harness

# Tests that draw their cases at random find more over time, because every run draws others.
# Whoever needs the same answer from every run asks for the same cases: the check of the
# mutants does, since a fault that is found in one run and missed in the next says nothing.
settings.register_profile("steady", derandomize=True)
settings.load_profile(os.environ.get("HYPOTHESIS_PROFILE", "default"))


@pytest.fixture
async def ctx() -> AsyncIterator[Context]:
    """A root context that is disposed when the test ends."""
    root = Context()
    yield root
    await root.fiber.dispose()


@pytest.fixture
async def harness(ctx: Context, tmp_path: Path) -> Harness:
    """The loop with its services, a scripted model, and sessions stored in a temporary place."""
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 5})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.agent)
    await ctx.plugin(columbina.agent_loop)
    model = Scripted()
    ctx.use(LLM).adapters.add(ctx, "scripted", model)
    work = tmp_path / "work"
    work.mkdir()
    return Harness(
        ctx=ctx,
        agents=ctx.use(AGENTS),
        sessions=ctx.use(SESSIONS),
        tools=ctx.use(TOOLS),
        model=model,
        cwd=work,
    )
