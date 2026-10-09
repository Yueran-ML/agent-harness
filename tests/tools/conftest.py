from pathlib import Path

import pytest

import columbina.tools
from columbina.kernel import Context
from columbina.session import CORE_EVENTS, Session
from columbina.tools import TOOLS, ToolRun, Tools


@pytest.fixture
async def tools(ctx: Context) -> Tools:
    """The tool service, loaded as a plugin."""
    await ctx.plugin(columbina.tools)
    return ctx.use(TOOLS)


@pytest.fixture
def run(ctx: Context, tmp_path: Path) -> ToolRun:
    """What a tool is told about a call in these tests."""
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})
    return ToolRun(call_id="call-1", cwd=tmp_path, session=session)
