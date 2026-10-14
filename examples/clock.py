"""The smallest plugin of a tool: it tells the agent what time it is, in UTC."""

from datetime import UTC, datetime

from pydantic import BaseModel

from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolRun

name = "clock"
inject = (TOOLS,)


class NoArgs(BaseModel):
    """The tool takes no arguments."""


async def now(args: NoArgs, run: ToolRun) -> str:
    return datetime.now(UTC).isoformat(timespec="seconds")


def apply(ctx: Context, config: object) -> None:
    tool = Tool(
        name="now",
        description="Say what time it is now, in UTC.",
        parameters=NoArgs,
        execute=now,
    )
    ctx.use(TOOLS).add(ctx, tool)
