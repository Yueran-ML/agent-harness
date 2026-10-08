"""What tests of the agent loop share."""

import asyncio
from dataclasses import dataclass
from pathlib import Path

from tack.agent import Agent, Agents, AgentSpec
from tack.kernel import Context
from tack.llm import AssistantMessage, CallConfig, SystemMessage, TextBlock, UserMessage
from tack.llm.adapters.scripted import Scripted, Step
from tack.session import Session, Sessions
from tack.tools import Tools

MODEL = CallConfig(provider="scripted", model="script")
SYSTEM = SystemMessage(content=(TextBlock(text="You are helpful."),))


@dataclass
class Harness:
    """Everything a test of the loop needs: the services, a scripted model and a place for files."""

    ctx: Context
    agents: Agents
    sessions: Sessions
    tools: Tools
    model: Scripted
    cwd: Path

    def spec(self, **changes: object) -> AgentSpec:
        """Return the spec of a test agent, with ``changes`` applied."""
        return AgentSpec.model_validate(
            {"cwd": self.cwd, "model": MODEL, "system_prompt": "You are helpful.", **changes}
        )

    async def agent(self, *script: Step, session_id: str = "s-1", **changes: object) -> Agent:
        """Create an agent whose model will give the scripted replies."""
        self.model.add(*script)
        return await self.agents.create(self.ctx, self.spec(**changes), session_id=session_id)


def types(session: Session, start: int = 0) -> list[str]:
    """Return the types of the events of a session, from ``start`` on."""
    return [event.type for event in session.events[start:]]


def user(text: str) -> UserMessage:
    """Return a user message with one block of text."""
    return UserMessage(content=(TextBlock(text=text),))


def assistant(text: str) -> AssistantMessage:
    """Return an assistant message with one block of text."""
    return AssistantMessage(content=(TextBlock(text=text),))


async def idle(agent: Agent) -> None:
    """Wait until ``agent`` is at rest, but not for ever."""
    await asyncio.wait_for(agent.when_idle(), timeout=5)
