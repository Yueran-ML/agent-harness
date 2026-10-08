from pathlib import Path

import pytest

import tack.agent
import tack.agent_loop
import tack.llm
import tack.session
import tack.tools
from support.loop import Harness
from tack.agent import AGENTS
from tack.kernel import Context
from tack.llm import LLM
from tack.llm.adapters.scripted import Scripted
from tack.session import SESSIONS
from tack.tools import TOOLS


@pytest.fixture
async def harness(ctx: Context, tmp_path: Path) -> Harness:
    """The loop with its services, sessions stored under a temporary directory."""
    await ctx.plugin(tack.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(tack.llm, {"idle_timeout_seconds": 5})
    await ctx.plugin(tack.tools)
    await ctx.plugin(tack.agent)
    await ctx.plugin(tack.agent_loop)
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
