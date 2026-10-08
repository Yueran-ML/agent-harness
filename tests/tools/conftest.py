from pathlib import Path

import pytest

import tack.tools
from tack.kernel import Context
from tack.session import CORE_EVENTS, Session
from tack.tools import TOOLS, ToolRun, Tools


@pytest.fixture
async def tools(ctx: Context) -> Tools:
    """The tool service, loaded as a plugin."""
    await ctx.plugin(tack.tools)
    return ctx.use(TOOLS)


@pytest.fixture
def run(ctx: Context, tmp_path: Path) -> ToolRun:
    """What a tool is told about a call in these tests."""
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})
    return ToolRun(call_id="call-1", cwd=tmp_path, session=session)
