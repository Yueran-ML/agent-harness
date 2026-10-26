"""The Agent Client Protocol: a conversation held by an editor, such as Zed, over a pipe.

This module is a plugin. It provides the frontend that ``columbina acp`` runs.

An editor that speaks the Agent Client Protocol starts this program and talks to it over its
standard input and output, in messages of JSON-RPC 2.0, one to a line, as ``columbina serve``
does for the desktop application. The protocol is another one, and so is this frontend:

* ``initialize`` says which version of the protocol is spoken, and what this side can do.
* ``session/new`` begins a session in the directory that the editor names. Several may be open.
* ``session/prompt`` gives the agent of a session what the person wrote, and is answered when
  the turn is over, with why it ended.
* ``session/cancel`` stops the turn of a session. Its prompt is answered with ``cancelled``.

What happens meanwhile is told with ``session/update``: the reply and the reasoning as they are
written, and each call of a tool, with how it ended. A call that needs a yes is put to the editor
with ``session/request_permission``, and only an option that allows is a yes.

The prompt may carry text, links to resources and resources whose text is embedded. Pictures
and sound are not taken, and the editor is told so at the beginning.
"""

import asyncio
import json
import logging
import sys
from collections.abc import AsyncIterator, Awaitable, Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast

from pydantic import BaseModel, ConfigDict, Field, ValidationError

import columbina
from columbina.agent import AGENTS, STREAM, Agent, AgentError, Agents, AgentSpec, StreamFrame
from columbina.approval import APPROVAL_ASK, ApprovalRequest, Outcome
from columbina.apps import FRONTEND
from columbina.apps.common import DEFAULT_PROMPT
from columbina.apps.serve import wire as wires
from columbina.apps.serve.rpc import (
    INTERNAL_ERROR,
    INVALID_PARAMS,
    INVALID_REQUEST,
    METHOD_NOT_FOUND,
    Id,
    MalformedLineError,
    Notification,
    Request,
    Response,
    RpcError,
    answered,
    asked,
    parsed,
    refused,
    text,
    told,
)
from columbina.apps.serve.wire import WIRE, Wire
from columbina.kernel import Context
from columbina.llm import CallConfig, ReasoningDelta, TextDelta
from columbina.permissions import PERMISSIONS
from columbina.session import (
    SESSION_EVENT,
    TURN,
    Committed,
    SessionError,
    ToolCallEvent,
    ToolResultEvent,
    TurnEndEvent,
)
from columbina.tools import TOOLS

__all__ = [
    "PROTOCOL",
    "Config",
    "Server",
    "apply",
    "inject",
    "name",
    "said_in",
    "stop_of",
    "title_of",
]

name = "acp"
inject = (AGENTS,)

logger = logging.getLogger(__name__)

PROTOCOL = 1
"""The version of the Agent Client Protocol that this program speaks."""

ALLOW = "allow"
ALWAYS = "allow-always"
REJECT = "reject"

_STOPS = {
    "completed": "end_turn",
    "aborted": "cancelled",
    "blocked": "max_turn_requests",
    "max-tokens": "max_tokens",
}
"""What the protocol calls the end of a turn, by how the log calls it."""

_KINDS = {"web_fetch": "fetch", "web_search": "search", "grep": "search", "glob": "search"}
"""The kind of a tool whose effect says too little of it, by its name."""

_EFFECTS = {"read": "read", "write": "edit", "run": "execute"}
"""The kind of any other tool, by its effect."""

_TITLED_BY = ("path", "command", "url", "query", "pattern")
"""The arguments that say best what a call acts on, the best first."""

_TITLE_CHARACTERS = 80

_LAST_WORK_SECONDS = 1.0
"""How long the prompts are given to be answered when the editor has left."""

_NO_TERMINAL = (
    "columbina: acp talks over its standard input and output, and this process has none: {}"
)
_TOO_LONG = "The line is longer than this program takes, and what it held was dropped."
_INSIDE = "Something went wrong inside this program. Its log says what."


class Config(BaseModel):
    """The config of the frontend.

    Attributes:
        model: The model that the agents talk to, and how.
        cwd: The directory that the program was started in. The editor names the directory
            of each session itself.
        system_prompt: The instructions that open a conversation.
    """

    model: CallConfig
    cwd: Path
    system_prompt: str = DEFAULT_PROMPT


class _Params(BaseModel):
    """What a request is asked with. What a newer editor adds to it is passed over."""

    model_config = ConfigDict(extra="ignore", populate_by_name=True)


class _Initialize(_Params):
    protocol: int = Field(alias="protocolVersion", ge=0)


class _New(_Params):
    cwd: str
    mcp_servers: list[Any] = Field(alias="mcpServers", default_factory=list[Any])


class _Prompt(_Params):
    session_id: str = Field(alias="sessionId")
    prompt: list[dict[str, Any]]


class _Cancel(_Params):
    session_id: str = Field(alias="sessionId")


@dataclass
class _Held:
    """A session that the editor holds, with its agent.

    Attributes:
        agent: The agent of the session.
        prompted: The prompt that is being answered, if one is.
        cancelled: Whether the editor stopped that prompt.
    """

    agent: Agent
    prompted: "asyncio.Future[str] | None" = None
    cancelled: bool = False
    calls: set[str] = field(default_factory=set[str])


def _what_is_wrong(error: ValidationError) -> str:
    first = error.errors()[0]
    where = ".".join(str(part) for part in first["loc"])
    return f"{where}: {first['msg']}" if where else first["msg"]


def said_in(prompt: list[dict[str, Any]]) -> str:
    """Put the blocks of a prompt into the words that the agent is given.

    Raises:
        RpcError: A block is one that is not taken, or there is nothing in them.
    """
    parts: list[str] = []
    for block in prompt:
        kind = block.get("type")
        if kind == "text" and isinstance(block.get("text"), str):
            parts.append(block["text"])
        elif kind == "resource_link" and isinstance(block.get("uri"), str):
            parts.append(f"[{block.get('name') or block['uri']}]({block['uri']})")
        elif kind == "resource" and isinstance(block.get("resource"), dict):
            resource = cast("dict[str, Any]", block["resource"])
            if not isinstance(resource.get("text"), str):
                raise RpcError(INVALID_PARAMS, "A resource is taken only with its text.")
            parts.append(
                f'<resource uri="{resource.get("uri", "")}">\n{resource["text"]}\n</resource>'
            )
        else:
            raise RpcError(INVALID_PARAMS, f"A block of the type {kind!r} is not taken.")
    words = "\n\n".join(parts).strip()
    if not words:
        raise RpcError(INVALID_PARAMS, "There is nothing in the prompt to give to the agent.")
    return words


def title_of(tool: str, arguments: str) -> str:
    """Say in one line what a call is: the tool, and what it acts on if that can be told."""
    try:
        read: object = json.loads(arguments)
    except ValueError:
        return tool
    if not isinstance(read, dict):
        return tool
    given = cast("dict[str, object]", read)
    target = next((given[key] for key in _TITLED_BY if isinstance(given.get(key), str)), None)
    if not target:
        return tool
    line = " ".join(str(target).split())
    if len(line) > _TITLE_CHARACTERS:
        line = line[: _TITLE_CHARACTERS - 3] + "..."
    return f"{tool}: {line}"


def stop_of(ended: TurnEndEvent | None) -> str:
    """Say why a turn stopped, as the protocol says it.

    Raises:
        RpcError: The turn failed. The editor is told what failed instead.
    """
    if ended is None:
        return "end_turn"
    if ended.reason == "error":
        raise RpcError(
            INTERNAL_ERROR, f"The turn failed: {ended.detail or ended.code or 'an error'}."
        )
    return _STOPS.get(ended.reason, "end_turn")


class Server:
    """The sessions that an editor holds over a wire, each with an agent of its own."""

    def __init__(self, ctx: Context, config: Config, agents: Agents) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._send: Callable[[str], None] = lambda line: None
        self._held: dict[str, _Held] = {}
        self._working: set[asyncio.Future[None]] = set()
        self._questions: dict[Id, asyncio.Future[Response]] = {}
        self._asked = 0
        self._methods: dict[str, tuple[type[_Params], Callable[[Any, Id], Awaitable[str]]]] = {
            "initialize": (_Initialize, self._initialize),
            "session/new": (_New, self._new),
            "session/prompt": (_Prompt, self._prompt),
        }

    async def run(self) -> int:
        """Serve the editor at the other end of the wire, until it leaves.

        Returns:
            0 when the editor left, and 2 if there is no wire to speak over.
        """
        wire = self._ctx.get(WIRE)
        if wire is None:  # pragma: no cover - takes the standard streams of its process
            try:
                wire = wires.taken()
            except OSError as error:
                print(_NO_TERMINAL.format(error), file=sys.stderr)
                return 2
        return await self.serve(wire)

    async def serve(self, wire: Wire) -> int:
        """Do what the lines of a wire ask for, until they end."""
        self._send = wire.send
        try:
            await self._hear(wire.lines())
        finally:
            await self._end()
            await wire.close()
        return 0

    # Lines

    async def _hear(self, lines: AsyncIterator[bytes | None]) -> None:
        async for line in lines:
            if line is None:
                self._send(refused(None, INVALID_REQUEST, _TOO_LONG))
                continue
            if not line.strip():
                continue
            try:
                message = parsed(line)
            except MalformedLineError as error:
                self._send(refused(error.id, error.code, error.message))
                continue
            if isinstance(message, Response):
                self._answer_came(message)
            elif isinstance(message, Request):
                await self._do(message)
            else:
                self._heard(message)

    def _heard(self, message: Notification) -> None:
        if message.method != "session/cancel":
            logger.debug("the editor says %s, which means nothing here", message.method)
            return
        try:
            params = _Cancel.model_validate(dict(message.params))
        except ValidationError:
            logger.debug("a session/cancel names no session")
            return
        held = self._held.get(params.session_id)
        if held is not None and held.prompted is not None:
            held.cancelled = True
            held.agent.cancel("user")

    async def _do(self, request: Request) -> None:
        """Do what a request asks for, and answer it.

        A prompt is answered when its turn is over, so it is done on the side: meanwhile the
        editor may stop it, or answer a question that it puts. Anything else is done before the
        next line is read, so a prompt never comes to a session that is still being begun.
        """
        method = self._methods.get(request.method)
        if method is None:
            self._send(
                refused(request.id, METHOD_NOT_FOUND, f"There is no method {request.method}.")
            )
            return
        shape, begin = method
        try:
            params = shape.model_validate(dict(request.params))
        except ValidationError as error:
            wrong = f"The params of {request.method} are not right: {_what_is_wrong(error)}."
            self._send(refused(request.id, INVALID_PARAMS, wrong))
            return
        work = self._answer(request.id, request.method, begin(params, request.id))
        if request.method != "session/prompt":
            await work
            return
        doing = asyncio.ensure_future(work)
        self._working.add(doing)
        doing.add_done_callback(self._working.discard)

    async def _answer(self, id: Id, method: str, work: Awaitable[str]) -> None:
        try:
            result = await work
        except RpcError as error:
            self._send(refused(id, error.code, error.message))
        except asyncio.CancelledError:
            self._send(refused(id, INTERNAL_ERROR, "The program is leaving."))
            raise
        except Exception as error:
            logger.error("the request %s failed", method, exc_info=error)
            self._send(refused(id, INTERNAL_ERROR, _INSIDE))
        else:
            self._send(answered(id, result))

    def _answer_came(self, answer: Response) -> None:
        waiting = None if answer.id is None else self._questions.get(answer.id)
        if waiting is None or waiting.done():
            logger.debug("an answer to %r came, and nothing waits for it", answer.id)
            return
        waiting.set_result(answer)

    # The requests

    async def _initialize(self, params: _Initialize, id: Id) -> str:
        return text(
            {
                "protocolVersion": min(PROTOCOL, params.protocol),
                "agentCapabilities": {
                    "loadSession": False,
                    "promptCapabilities": {"image": False, "audio": False, "embeddedContext": True},
                    "mcpCapabilities": {"http": False, "sse": False},
                },
                "authMethods": [],
                "agentInfo": {
                    "name": "columbina",
                    "title": "Columbina",
                    "version": columbina.__version__,
                },
            }
        )

    async def _new(self, params: _New, id: Id) -> str:
        cwd = Path(params.cwd)
        if not cwd.is_absolute() or not await asyncio.to_thread(cwd.is_dir):
            raise RpcError(
                INVALID_PARAMS,
                f"The directory {params.cwd!r} is not an absolute path of a directory.",
            )
        if params.mcp_servers:
            logger.info("the editor names MCP servers, and those of the profile are used instead")
        spec = AgentSpec(
            cwd=cwd, model=self._config.model, system_prompt=self._config.system_prompt
        )
        try:
            agent = await self._agents.create(self._ctx, spec)
        except (SessionError, AgentError) as error:
            raise RpcError(INTERNAL_ERROR, f"The session cannot be begun: {error}.") from None
        self._held[agent.id] = held = _Held(agent)
        self._listen(held)
        return text({"sessionId": agent.id})

    async def _prompt(self, params: _Prompt, id: Id) -> str:
        held = self._held.get(params.session_id)
        if held is None:
            raise RpcError(INVALID_PARAMS, f"There is no session {params.session_id}.")
        if held.prompted is not None:
            raise RpcError(INVALID_REQUEST, "The session answers a prompt already.")
        words = said_in(params.prompt)
        agent = held.agent
        held.cancelled = False
        held.prompted = waiting = asyncio.ensure_future(self._turn(agent, words))
        try:
            reason = await waiting
        finally:
            held.prompted = None
        return text({"stopReason": "cancelled" if held.cancelled else reason})

    async def _turn(self, agent: Agent, words: str) -> str:
        """Give the agent what was said, and say how the turn that it opened ended."""
        began = len(agent.session.events)
        agent.followup(words)
        try:
            await agent.when_idle()
        except Exception as error:
            raise RpcError(INTERNAL_ERROR, f"The agent has stopped: {error}.") from None
        ended = [
            event.data
            for event in agent.session.events[began:]
            if isinstance(event.data, TurnEndEvent)
        ]
        return stop_of(next(reversed(ended), None))

    # What happens in a session

    def _listen(self, held: _Held) -> None:
        agent = held.agent

        def event(committed: Committed) -> None:
            if committed.session is agent.session:
                self._event(held, committed)

        def chunk(frame: StreamFrame) -> None:
            if frame.agent is agent:
                self._chunk(agent, frame)

        agent.ctx.on(SESSION_EVENT, event)
        agent.ctx.on(STREAM, chunk)
        agent.ctx.on(APPROVAL_ASK, self._asker(held))

    def _update(self, agent: Agent, update: dict[str, Any]) -> None:
        self._send(told("session/update", text({"sessionId": agent.id, "update": update})))

    def _chunk(self, agent: Agent, frame: StreamFrame) -> None:
        piece = frame.chunk
        if isinstance(piece, TextDelta) and piece.text:
            kind = "agent_message_chunk"
        elif isinstance(piece, ReasoningDelta) and piece.text:
            kind = "agent_thought_chunk"
        else:
            return
        self._update(
            agent, {"sessionUpdate": kind, "content": {"type": "text", "text": piece.text}}
        )

    def _kind(self, tool: str) -> str:
        if tool in _KINDS:
            return _KINDS[tool]
        tools = self._ctx.get(TOOLS)
        known = None if tools is None else tools.registry.get(tool)
        return "other" if known is None else _EFFECTS.get(known.effect, "other")

    def _event(self, held: _Held, committed: Committed) -> None:
        data = committed.event.data
        agent = held.agent
        if isinstance(data, ToolCallEvent):
            held.calls.add(data.call_id)
            try:
                given: object = json.loads(data.arguments)
            except ValueError:
                given = data.arguments
            call: dict[str, Any] = {
                "sessionUpdate": "tool_call",
                "toolCallId": data.call_id,
                "title": title_of(data.name, data.arguments),
                "kind": self._kind(data.name),
                "status": "pending",
                "rawInput": given,
            }
            if isinstance(given, dict) and isinstance(
                cast("dict[str, object]", given).get("path"), str
            ):
                where = Path(cast("dict[str, str]", given)["path"])
                call["locations"] = [
                    {"path": str(where if where.is_absolute() else agent.spec.cwd / where)}
                ]
            self._update(agent, call)
        elif isinstance(data, ToolResultEvent) and data.message.call_id in held.calls:
            held.calls.discard(data.message.call_id)
            output = "".join(block.text for block in data.message.content)
            self._update(
                agent,
                {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": data.message.call_id,
                    "status": "failed" if data.error is not None else "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": output}}],
                },
            )

    # Questions

    def _asker(self, held: _Held) -> Callable[[ApprovalRequest], Awaitable[Outcome | None]]:
        agent = held.agent

        async def ask(request: ApprovalRequest) -> Outcome | None:
            if request.session is not agent.session:
                # A subagent asks: the editor knows nothing of its session.
                return None
            granting = self._ctx.get(PERMISSIONS)
            standing = None if granting is None else request.standing
            options = [{"optionId": ALLOW, "name": "Allow", "kind": "allow_once"}]
            if standing is not None:
                options.append(
                    {
                        "optionId": ALWAYS,
                        "name": f"Allow, and do not ask again about {standing}",
                        "kind": "allow_always",
                    }
                )
            options.append({"optionId": REJECT, "name": "Reject", "kind": "reject_once"})
            self._asked += 1
            question = self._asked
            waiting: asyncio.Future[Response] = asyncio.get_running_loop().create_future()
            self._questions[question] = waiting
            params = {
                "sessionId": agent.id,
                "toolCall": {
                    "toolCallId": request.call_id,
                    "title": request.summary,
                    "kind": self._kind(request.tool),
                    "status": "pending",
                    "content": [
                        {"type": "content", "content": {"type": "text", "text": request.reason}}
                    ],
                },
                "options": options,
            }
            self._send(asked(question, "session/request_permission", text(params)))
            try:
                answer = await waiting
            finally:
                del self._questions[question]
            given = None if answer.failed else answer.result
            said = cast("dict[str, Any]", given) if isinstance(given, dict) else {}
            outcome = said.get("outcome")
            chosen = (
                cast("dict[str, Any]", outcome).get("optionId")
                if isinstance(outcome, dict)
                else None
            )
            if chosen == ALWAYS and granting is not None and standing is not None:
                try:
                    granting.grant(request.session, request.tool, request.target)
                except ValueError as error:
                    logger.warning("the longer yes could not be written down: %s", error)
                return "allowed-once"
            return "allowed-once" if chosen == ALLOW else "rejected"

        return ask

    # Leaving

    async def _end(self) -> None:
        """End every agent, and what is done on the side."""
        for held in list(self._held.values()):
            agent = held.agent
            try:
                if TURN.of(agent.session).in_turn:
                    agent.cancel("user")
                await asyncio.shield(agent.close())
                await agent.when_idle()
            except Exception as error:
                logger.error("the agent of the session %s stopped for good: %s", agent.id, error)
        self._held.clear()
        # A prompt whose agent has ended is answered. What is still at work after that is
        # broken off.
        if self._working:
            await asyncio.wait(self._working, timeout=_LAST_WORK_SECONDS)
        for doing in list(self._working):
            doing.cancel()
        if self._working:
            await asyncio.wait(self._working)


def apply(ctx: Context, config: Config) -> None:
    """Provide the frontend that serves an editor over a wire."""
    ctx.provide(FRONTEND, Server(ctx, config, ctx.use(AGENTS)))
