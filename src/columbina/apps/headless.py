"""Running one task without a user interface.

This module is a plugin. It provides the frontend of ``columbina run``: one agent
is given one task, and when the agent is at rest the answer is printed.

The answer is written by a model, and a terminal carries out the control
characters it is sent. So the answer is printed as the model wrote it only
where the output is a file or another program. On a terminal, and in whatever
is said about the run, such characters are shown and not sent. The lines of
JSON write them by their numbers, which is the same JSON.
"""

import asyncio
import json
import sys
from pathlib import Path

from pydantic import BaseModel, JsonValue

from columbina.agent import AGENTS, Agent, Agents, AgentSpec, replies
from columbina.apps import FRONTEND
from columbina.apps.common import DEFAULT_PROMPT, open_agent
from columbina.apps.terminal_text import escaped, harmless
from columbina.kernel import Context
from columbina.llm.types import CallConfig, ReasoningBlock, TextBlock
from columbina.session import (
    SESSION_EVENT,
    SESSIONS,
    USAGE,
    AssistantMessageEvent,
    Committed,
    EventData,
    SessionError,
    Sessions,
    StepStartEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
)

__all__ = ["DEFAULT_PROMPT", "Config", "Headless", "apply", "inject", "name"]

name = "headless"
inject = (AGENTS, SESSIONS)


class Config(BaseModel):
    """The config of the headless frontend.

    Attributes:
        model: The model that the agent talks to, and how.
        cwd: The working directory of the agent.
        task: What the agent is asked to do.
        system_prompt: The instructions that open the conversation.
        json_lines: Whether to report the run as lines of JSON, instead of
            printing only the answer.
        session_id: The id of the session. A stored session with this id is
            continued. Without an id a new session is started.
    """

    model: CallConfig
    cwd: Path
    task: str = ""
    system_prompt: str = DEFAULT_PROMPT
    json_lines: bool = False
    session_id: str | None = None


def _emit(line: dict[str, JsonValue]) -> None:
    sys.stdout.write(escaped(json.dumps(line, ensure_ascii=False)) + "\n")
    sys.stdout.flush()


def _tell(message: str) -> None:
    """Say something about the run to whoever started it."""
    print(harmless(f"columbina: {message}"), file=sys.stderr)


def _report(data: EventData) -> list[dict[str, JsonValue]]:
    """Return the lines of JSON that tell about one event of the session."""
    if isinstance(data, StepStartEvent):
        return [{"type": "status", "turn": data.turn, "step": data.step}]
    if isinstance(data, AssistantMessageEvent):
        lines: list[dict[str, JsonValue]] = []
        for block in data.message.content:
            if isinstance(block, ReasoningBlock):
                lines.append({"type": "thinking", "text": block.text})
            elif isinstance(block, TextBlock):
                lines.append({"type": "text", "text": block.text})
        return lines
    if isinstance(data, ToolCallEvent):
        return [
            {
                "type": "tool_call",
                "id": data.call_id,
                "name": data.name,
                "arguments": data.arguments,
            }
        ]
    if isinstance(data, ToolResultEvent):
        return [
            {
                "type": "tool_result",
                "id": data.message.call_id,
                "text": "\n\n".join(block.text for block in data.message.content),
                "error": None if data.error is None else data.error.code,
            }
        ]
    return []


class Headless:
    """Runs one task and reports how it went."""

    __slots__ = ("_agents", "_config", "_ctx", "_sessions")

    def __init__(self, ctx: Context, config: Config, agents: Agents, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions

    async def run(self) -> int:
        """Run the task. Return 0 if the turn was completed, and 1 otherwise."""
        config = self._config
        try:
            agent = await self._agent()
        except (SessionError, ValueError) as error:
            _tell(str(error))
            return 1
        session = agent.session
        before = len(session)
        spent = USAGE.of(session)
        if config.json_lines:
            agent.ctx.on(SESSION_EVENT, self._follow)
            _emit(
                {
                    "type": "session",
                    "id": agent.id,
                    "cwd": str(config.cwd),
                    "provider": config.model.provider,
                    "model": config.model.model,
                }
            )
        agent.followup(config.task)
        try:
            await agent.when_idle()
        except asyncio.CancelledError:
            # The program is being interrupted: end the turn properly before going.
            agent.cancel("user")
            await asyncio.shield(agent.close())
            raise
        except Exception as error:
            _tell(f"the agent has stopped: {error}")
            return 1
        events = [event.data for event in session.events[before:]]
        ends = [data for data in events if isinstance(data, TurnEndEvent)]
        # A reply that was cut off and taken up again is one answer with what went on from it.
        said = replies(events)
        end = ends[-1]
        answer = said[-1] if said else ""
        total = USAGE.of(session)
        await agent.close()
        if config.json_lines:
            _emit(
                {
                    "type": "final",
                    "reason": end.reason,
                    "detail": end.detail,
                    "text": answer,
                    "calls": total.calls - spent.calls,
                    "usage": {
                        "input_tokens": total.usage.input_tokens - spent.usage.input_tokens,
                        "output_tokens": total.usage.output_tokens - spent.usage.output_tokens,
                        "cache_read_tokens": total.usage.cache_read_tokens
                        - spent.usage.cache_read_tokens,
                        "cache_write_tokens": total.usage.cache_write_tokens
                        - spent.usage.cache_write_tokens,
                    },
                }
            )
        else:
            if answer:
                print(harmless(answer) if sys.stdout.isatty() else answer)
            if end.reason == "error":
                _tell(f"the turn ended with an error: {end.detail}")
            elif end.reason != "completed":
                detail = end.reason if end.detail is None else f"{end.reason} ({end.detail})"
                _tell(f"the turn ended early: {detail}")
        return 0 if end.reason == "completed" else 1

    async def _agent(self) -> Agent:
        """Create the agent, on a stored session if the config names one that exists."""
        config = self._config
        spec = AgentSpec(cwd=config.cwd, model=config.model, system_prompt=config.system_prompt)
        return await open_agent(self._ctx, self._agents, self._sessions, spec, config.session_id)

    def _follow(self, committed: Committed) -> None:
        if committed.event.replaces:
            # It takes the place of something that was reported when it happened.
            return
        for line in _report(committed.event.data):
            _emit(line)


def apply(ctx: Context, config: Config) -> None:
    """Provide the frontend that runs one task."""
    ctx.provide(FRONTEND, Headless(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS)))
