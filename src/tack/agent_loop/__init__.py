"""The standard agent loop.

This module is a plugin. Loading it registers the loop with the agent service,
under the name ``default`` unless its config says otherwise.
"""

from pydantic import BaseModel

from tack.agent import AGENTS, Agent, AgentSpec
from tack.agent_loop.loop import LoopAgent
from tack.kernel import Context
from tack.llm import LLM
from tack.session import Session
from tack.tools import TOOLS

__all__ = [
    "Config",
    "LoopAgent",
    "apply",
    "inject",
    "name",
]

name = "agent-loop"
inject = (AGENTS, LLM, TOOLS)


class Config(BaseModel):
    """The config of the loop.

    Attributes:
        name: The name under which the loop is registered. An agent whose
            spec names it is driven by this loop.
    """

    name: str = "default"


def apply(ctx: Context, config: Config) -> None:
    """Register the loop with the agent service."""
    llm = ctx.use(LLM)
    tools = ctx.use(TOOLS)

    def make(scope: Context, spec: AgentSpec, session: Session) -> Agent:
        return LoopAgent(scope, spec, session, llm, tools)

    ctx.use(AGENTS).loops.add(ctx, config.name, make)
