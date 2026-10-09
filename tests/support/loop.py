"""What tests of the agent loop share."""

import asyncio
from dataclasses import dataclass
from pathlib import Path

from columbina.agent import Agent, Agents, AgentSpec
from columbina.kernel import Context
from columbina.llm import AssistantMessage, CallConfig, SystemMessage, TextBlock, UserMessage
from columbina.llm.adapters.scripted import Scripted, Step
from columbina.session import Session, Sessions, UserMessageEvent
from columbina.tools import Tools

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


def context(source: str, text: str) -> UserMessage:
    """Return the message that carries the text of a source of context."""
    return user(f'<context source="{source}">\n{text}\n</context>')


STOPPED = context(
    "interruption",
    "The turn before this one was stopped before it had come to its end. A tool that was "
    "running then was stopped and may have done a part of its work, and calls that had not "
    "begun did not run. See how things stand before you rely on what that turn was doing.",
)
"""What a model is told when the turn before was cancelled."""


def from_a_source(event: UserMessageEvent) -> bool:
    """Tell whether a message was written by a source of context, and not given as input."""
    return event.source.startswith("context/")


def assistant(text: str) -> AssistantMessage:
    """Return an assistant message with one block of text."""
    return AssistantMessage(content=(TextBlock(text=text),))


async def idle(agent: Agent) -> None:
    """Wait until ``agent`` is at rest, but not for ever."""
    await asyncio.wait_for(agent.when_idle(), timeout=5)
