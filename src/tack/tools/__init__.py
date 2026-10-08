"""Tools: what a model can ask the harness to do, and how a call is carried out.

This module is a plugin. Loading it provides :data:`TOOLS`.
"""

from pydantic import BaseModel, Field

from tack.kernel import Context
from tack.tools.pipeline import (
    EXECUTE,
    POST_EXECUTE,
    PRE_EXECUTE,
    RESULT,
    TOOLS,
    Accept,
    Allow,
    Block,
    Decision,
    Deny,
    Executed,
    Invocation,
    PendingCall,
    ToolOutcome,
    Tools,
    Verdict,
)
from tack.tools.tool import Tool, ToolFailure, ToolRun

__all__ = [
    "EXECUTE",
    "POST_EXECUTE",
    "PRE_EXECUTE",
    "RESULT",
    "TOOLS",
    "Accept",
    "Allow",
    "Block",
    "Config",
    "Decision",
    "Deny",
    "Executed",
    "Invocation",
    "PendingCall",
    "Tool",
    "ToolFailure",
    "ToolOutcome",
    "ToolRun",
    "Tools",
    "Verdict",
    "apply",
    "name",
]

name = "tools"


class Config(BaseModel):
    """The config of the tool service.

    Attributes:
        timeout_seconds: How long a call may take if its tool sets no limit
            of its own. Without it there is no limit.
    """

    timeout_seconds: float | None = Field(default=None, gt=0)


def apply(ctx: Context, config: Config) -> None:
    """Provide the tool service."""
    ctx.provide(TOOLS, Tools(config.timeout_seconds))
