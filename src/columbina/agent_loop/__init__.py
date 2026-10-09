"""The standard agent loop.

This module is a plugin. Loading it registers the loop with the agent service,
under the name ``default`` unless its config says otherwise.
"""

from pydantic import BaseModel, Field

from columbina.agent import AGENTS, Agent, AgentSpec
from columbina.agent_loop.loop import LoopAgent
from columbina.kernel import Context
from columbina.llm import LLM
from columbina.session import Session
from columbina.tools import TOOLS

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
        max_parallel_tool_calls: How many tool calls of one reply may run
            beside each other, if their tools say that this is safe.
    """

    name: str = "default"
    max_parallel_tool_calls: int = Field(default=10, ge=1)


def apply(ctx: Context, config: Config) -> None:
    """Register the loop with the agent service."""
    llm = ctx.use(LLM)
    tools = ctx.use(TOOLS)
    agents = ctx.use(AGENTS)

    def make(scope: Context, spec: AgentSpec, session: Session) -> Agent:
        return LoopAgent(
            scope, spec, session, llm, tools, agents.context, config.max_parallel_tool_calls
        )

    agents.loops.add(ctx, config.name, make)
