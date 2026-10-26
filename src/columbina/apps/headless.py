"""Running one task without a user interface.

This module is a plugin. It provides the frontend of ``columbina run``: one agent
is given one task, and when the agent is at rest the answer is printed.

A run may be given a JSON Schema that the answer has to fit, for another
program that reads it. The model is told the schema with the task and asked
for JSON alone. An answer that is no such JSON is sent back to it, with what
is wrong, so many times; the JSON is printed in one line where it fits, and
the run fails where it does not. A schema is checked as
:mod:`columbina.json_schema` checks one.

The answer is written by a model, and a terminal carries out the control
characters it is sent. So the answer is printed as the model wrote it only
where the output is a file or another program. On a terminal, and in whatever
is said about the run, such characters are shown and not sent. The lines of
JSON write them by their numbers, which is the same JSON.
"""

import asyncio
import json
import re
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field, JsonValue

from columbina import json_schema
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
    tools_used,
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
        output_schema: The file of a JSON Schema that the answer has to fit,
            if the answer is for a program.
        schema_attempts: How many times an answer that does not fit is sent
            back to the model.
    """

    model: CallConfig
    cwd: Path
    task: str = ""
    system_prompt: str = DEFAULT_PROMPT
    json_lines: bool = False
    session_id: str | None = None
    output_schema: Path | None = None
    schema_attempts: int = Field(default=2, ge=0, strict=True)


def _emit(line: dict[str, JsonValue]) -> None:
    sys.stdout.write(escaped(json.dumps(line, ensure_ascii=False)) + "\n")
    sys.stdout.flush()


def _tell(message: str) -> None:
    """Say something about the run to whoever started it."""
    print(harmless(f"columbina: {message}"), file=sys.stderr)


_FENCE = re.compile(r"^```[A-Za-z]*\n(.*)\n```$", re.DOTALL)
"""A block of code in Markdown that the whole answer stands in."""


def _schema(path: Path) -> Mapping[str, Any]:
    """Read the schema that the answer has to fit.

    Raises:
        ValueError: It cannot be read, is no JSON, or cannot be checked here.
    """
    try:
        return json_schema.checked(json.loads(path.read_text(encoding="utf-8")))
    except OSError as error:
        raise ValueError(f"the schema {path} cannot be read: {error.strerror or error}") from None
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"the schema {path} is no JSON: {error}") from None
    except json_schema.SchemaError as error:
        raise ValueError(f"the schema {path} cannot be checked: {error}") from None


def _asked_with(task: str, schema: Mapping[str, Any]) -> str:
    """Return the task with what the answer has to be."""
    written = json.dumps(schema, ensure_ascii=False, indent=2)
    return (
        f"{task}\n\nWhen you are done, answer with JSON alone, which a program reads: no words "
        f"before or after it. It has to fit this JSON Schema:\n```json\n{written}\n```"
    )


def _read(answer: str, schema: Mapping[str, Any]) -> tuple[JsonValue, list[str]]:
    """Return the JSON of an answer, and how it does not fit the schema."""
    text = answer.strip()
    fenced = _FENCE.match(text)
    if fenced is not None:
        text = fenced.group(1)
    try:
        value: JsonValue = json.loads(text)
    except json.JSONDecodeError as error:
        return None, [f"the answer is no JSON: {error.msg} at character {error.pos}"]
    return value, json_schema.problems(value, schema)


def _sent_back(wrong: list[str]) -> str:
    said = "; ".join(wrong[:10])
    more = f"; and {len(wrong) - 10} more" if len(wrong) > 10 else ""
    return (
        f"Your answer does not fit the schema: {said}{more}. Answer again with the JSON alone, "
        "which fits the schema."
    )


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


def _fitted(
    schema: Mapping[str, Any] | None, reason: str, output: JsonValue, wrong: list[str]
) -> dict[str, JsonValue]:
    """Return what the last line of JSON says of an answer that was to fit a schema."""
    if schema is None or reason != "completed":
        return {}
    if wrong:
        return {"output": None, "output_problems": list[JsonValue](wrong)}
    return {"output": output}


class Headless:
    """Runs one task and reports how it went."""

    __slots__ = ("_agents", "_config", "_ctx", "_sessions")

    def __init__(self, ctx: Context, config: Config, agents: Agents, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions

    async def run(self) -> int:
        """Run the task. Return 0 if the turn was completed, and 1 otherwise.

        With a schema, 0 is also for an answer that fits it, and a schema
        that cannot be used ends the run before it begins, with 2.
        """
        config = self._config
        schema = None
        task = config.task
        if config.output_schema is not None:
            try:
                schema = _schema(config.output_schema)
            except ValueError as error:
                _tell(str(error))
                return 2
            task = _asked_with(task, schema)
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
        output: JsonValue = None
        wrong: list[str] = []
        attempts = 0
        while True:
            asked_from = len(session)
            agent.followup(task)
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
            events = [event.data for event in session.events[asked_from:]]
            ends = [data for data in events if isinstance(data, TurnEndEvent)]
            # A reply that was cut off and taken up again is one answer with what went on.
            said = replies(events)
            end = ends[-1]
            answer = said[-1] if said else ""
            if schema is None or end.reason != "completed":
                break
            output, wrong = _read(answer, schema)
            if not wrong or attempts == config.schema_attempts:
                break
            attempts += 1
            task = _sent_back(wrong)
        total = USAGE.of(session)
        calls = tools_used(session.events[before:]).tools
        await agent.close()
        if config.json_lines:
            _emit(
                {
                    "type": "final",
                    "reason": end.reason,
                    "detail": end.detail,
                    "code": end.code,
                    "text": answer,
                    **_fitted(schema, end.reason, output, wrong),
                    "calls": total.calls - spent.calls,
                    "usage": {
                        "input_tokens": total.usage.input_tokens - spent.usage.input_tokens,
                        "output_tokens": total.usage.output_tokens - spent.usage.output_tokens,
                        "cache_read_tokens": total.usage.cache_read_tokens
                        - spent.usage.cache_read_tokens,
                        "cache_write_tokens": total.usage.cache_write_tokens
                        - spent.usage.cache_write_tokens,
                    },
                    "tools": {tool: use.model_dump() for tool, use in calls.items()},
                }
            )
        else:
            if schema is not None and end.reason == "completed" and not wrong:
                answer = json.dumps(output, ensure_ascii=False)
            if answer:
                print(harmless(answer) if sys.stdout.isatty() else answer)
            if schema is not None and end.reason == "completed" and wrong:
                _tell(f"the answer does not fit the schema: {'; '.join(wrong)}")
            if end.reason == "error":
                _tell(f"the turn ended with an error: {end.detail}")
            elif end.reason != "completed":
                detail = end.reason if end.detail is None else f"{end.reason} ({end.detail})"
                _tell(f"the turn ended early: {detail}")
        return 0 if end.reason == "completed" and not wrong else 1

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
