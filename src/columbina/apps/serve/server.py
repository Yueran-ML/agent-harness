"""The service process at work: what it is asked for, and what it tells without being asked.

Another program holds the conversation here, an interface in a window. It
sends requests, a line each, and is sent answers. What happens in the
session is sent to it as it happens: every event of the log, every chunk of a
reply that is being written, and where the agent stands.

A request is done in the order in which it came. A line is read, and what it
asks for is done before the next one is read, so something that is said to a
turn on its way does not overtake the task that began the turn. Two kinds of
request may take long, a command and the list of the stored sessions. What
they need to look at is looked at when they come, like everything else, and
the work itself is done on the side. So a request that stops the agent is
heard while a command has the agent sum up the conversation.

A command that was taken comes before whatever is asked for after it, although
its work begins a little later than the next line is read. What is given to
the agent while a command is carried out is kept back until the command is
over, and an order to stop that comes before the command has begun keeps it
from beginning.

Where the agent stands is told from one place, :meth:`Server._look`. It is
asked whenever something may have changed, and says something only when what
it would say differs from what it said last. What it tells is where the agent
stands, and not what was asked of this program: a command that is carried out
at once changes nothing, and one that has the agent work is told of when the
agent writes down that it has begun.

A call that needs a yes is put to the interface as a request of this side.
Only a plain yes lets the call go ahead. Whatever else comes back, an error
included, is a no.
"""

import asyncio
import functools
import logging
import sys
from collections.abc import AsyncIterator, Awaitable, Callable
from pathlib import Path
from typing import Any, cast

from pydantic import BaseModel, ConfigDict, Field, ValidationError

import columbina
from columbina.agent import (
    STREAM,
    Agent,
    AgentError,
    Agents,
    AgentSpec,
    StreamFrame,
)
from columbina.approval import APPROVAL, APPROVAL_ASK, ApprovalRequest, Outcome
from columbina.apps.common import DEFAULT_PROMPT, model_named, open_agent
from columbina.apps.effort import (
    ULTRA,
    Effort,
    Style,
    effort_named,
    effort_of,
    offered,
    reasoning_of,
    style_of,
)
from columbina.apps.serve import wire as wires
from columbina.apps.serve.rpc import (
    CANNOT_OPEN,
    INTERNAL_ERROR,
    INVALID_PARAMS,
    INVALID_REQUEST,
    LEAVING,
    METHOD_NOT_FOUND,
    NO_SESSION,
    NOT_IDLE,
    NOT_INITIALIZED,
    STOPPED,
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
from columbina.commands import Commands, is_command
from columbina.kernel import Context, Next
from columbina.llm import CallConfig
from columbina.permissions import PERMISSIONS
from columbina.prices import SPENDING
from columbina.sandbox import SANDBOX
from columbina.session import (
    SESSION_EVENT,
    TURN,
    Committed,
    Session,
    SessionError,
    Sessions,
    encode_event,
)
from columbina.session.store import SESSION_FILE
from columbina.skills import SKILLS, one_line
from columbina.system_prompt import SYSTEM_PROMPT_ASSEMBLE, Draft

__all__ = ["PAGE_BYTES", "PAGE_EVENTS", "PROTOCOL", "Config", "Server"]

logger = logging.getLogger(__name__)

PROTOCOL = 1
"""The version of the protocol that this program speaks."""

PAGE_EVENTS = 500
"""How many events of a session are given at once, at most."""

PAGE_BYTES = 4 * 1024 * 1024
"""About how long the events that are given at once may be together. One is always given."""

_LAST_WORK_SECONDS = 1.0
"""How long what is done on the side is given to come to its end when the program leaves."""

_NO_TERMINAL = (
    "columbina: serve talks over its standard input and output, and this process has none: {}"
)
_TOO_LONG = "The line is longer than this program takes, and what it held was dropped."
_NOT_GREETED = "Nothing was asked for yet with initialize, and that comes first."
_GREETED = "initialize was asked for already. It is asked for once."
_NO_SESSION = "No session is open. session/open opens one."
_INSIDE = "Something went wrong inside the service process. Its log says what."
_LEAVING = "The program is leaving, and this was broken off."


class Config(BaseModel):
    """The config of the service process.

    Attributes:
        model: The model that the agent talks to, and how.
        cwd: The working directory of the agent.
        system_prompt: The instructions that open a conversation.
    """

    model: CallConfig
    cwd: Path
    system_prompt: str = DEFAULT_PROMPT


class _Params(BaseModel):
    """What a request is asked with. What a newer interface adds to it is passed over."""

    model_config = ConfigDict(extra="ignore", strict=True)


class _Client(_Params):
    name: str = ""
    version: str = ""


class _Initialize(_Params):
    protocol: int = Field(ge=1)
    client: _Client = _Client()


class _Open(_Params):
    session_id: str | None = None


class _Events(_Params):
    start: int = Field(alias="from", ge=0)
    limit: int = Field(default=PAGE_EVENTS, ge=1)


class _Said(_Params):
    text: str


class _Steered(_Said):
    now: bool = False


class _Line(_Params):
    line: str


class _Nothing(_Params):
    pass


Work = Awaitable[str | None]
"""What is left to do for a request. It gives the result as JSON, or ``None`` if the
request was answered meanwhile."""


def _what_is_wrong(error: ValidationError) -> str:
    """Say in a sentence what is wrong with the params of a request."""
    first = error.errors()[0]
    where = ".".join(str(part) for part in first["loc"])
    return f"{where}: {first['msg']}" if where else first["msg"]


async def _ended(agent: Agent) -> Exception | None:
    """End an agent properly: a turn that is running is stopped first.

    The agent is ended as far as it can be, whatever goes wrong on the way. A
    session that cannot be written any more fails the order to stop its turn,
    which has to write down what it drops, and its agent is put away all the
    same.

    Returns:
        What kept the session from being written to its end, if something did.
    """
    failure: Exception | None = None
    try:
        if TURN.of(agent.session).in_turn:
            agent.cancel("user")
    except Exception as error:
        failure = error
    try:
        await asyncio.shield(agent.close())
        await agent.when_idle()
    except Exception as error:
        failure = failure or error
    return failure


class Server:
    """A conversation with one agent at a time, held by another program over a wire."""

    def __init__(
        self,
        ctx: Context,
        config: Config,
        agents: Agents,
        sessions: Sessions,
        commands: Commands,
    ) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions
        self._commands = commands
        self._model = config.model
        # The level of effort that was chosen in this run, or None to go by the config.
        self._effort: Effort | None = None
        self._send: Callable[[str], None] = lambda line: None
        self._greeted = False
        self._leaving = False
        self._ended: bool | None = None
        self._agent: Agent | None = None
        # Why the agent of the conversation has stopped for good, if it has.
        self._stopped: str | None = None
        # The agent that a command is being carried out for, and the carrying out of it.
        self._commanding: Agent | None = None
        self._carrying: asyncio.Future[str] | None = None
        # What was given to that agent since the command was taken, and waits for its end.
        self._held: list[tuple[Callable[[str], None], str]] = []
        # Whether a session of this run could not be written to its end.
        self._lost = False
        # What was allowed for the rest of a session, by the id of the session.
        self._granted: dict[str, list[tuple[str, str | None]]] = {}
        # What was said last of where the agent stands.
        self._said: str | None = None
        self._watching: asyncio.Future[None] | None = None
        # What is done on the side, and the agents that are being ended.
        self._working: set[asyncio.Future[None]] = set()
        self._ending: set[asyncio.Future[None]] = set()
        # The questions that wait for an answer of the interface, by their ids.
        self._questions: dict[Id, asyncio.Future[Response]] = {}
        self._asked = 0
        self._methods: dict[str, tuple[type[_Params], Callable[[Any, Id], Work]]] = {
            "initialize": (_Initialize, self._initialize),
            "session/list": (_Nothing, self._list),
            "session/open": (_Open, self._open),
            "session/events": (_Events, self._events),
            "session/prompt": (_Said, self._prompt),
            "session/steer": (_Steered, self._steer),
            "session/cancel": (_Nothing, self._cancel),
            "session/command": (_Line, self._command),
            "session/skills": (_Nothing, self._skills),
            "shutdown": (_Nothing, self._shutdown),
        }

    # The frontend

    async def run(self) -> int:
        """Serve whoever holds the other end of the wire, until it leaves.

        Returns:
            0 when the other side left, 1 if a session of this run could not
            be written to its end, and 2 if there is no wire to speak over.
        """
        wire = self._ctx.get(WIRE)
        if wire is None:
            try:
                wire = wires.taken()
            except OSError as error:
                print(_NO_TERMINAL.format(error), file=sys.stderr)
                return 2
        return await self.serve(wire)

    async def serve(self, wire: Wire) -> int:
        """Do what the lines of a wire ask for, until one asks to leave or they end."""
        self._send = wire.send
        try:
            await self._hear(wire.lines())
        finally:
            failed = await self._end()
            await wire.close()
        return 1 if failed else 0

    async def model(self, agent: Agent, rest: str) -> str:
        """Carry out the command that shows the model, or names another one."""
        if rest:
            self._model = model_named(self._config.model, rest)
            self._hand_down(agent)
            self._look()
            return f"The model is {rest} from the next request on."
        return f"The model is {self._model.model}. Another one is named with /model ID."

    async def effort(self, agent: Agent, rest: str) -> str:
        """Carry out the command that shows how hard the model thinks, or chooses another level."""
        chosen, said = effort_named(self._chosen_effort(), self._levels(), rest)
        if chosen is not None:
            self._effort = chosen
            self._hand_down(agent)
            self._look()
        return said

    # The model, and how hard it thinks

    def _style(self) -> Style:
        """How the service in use is told how much to reason."""
        return style_of(self._ctx, self._model.provider)

    def _levels(self) -> tuple[Effort, ...]:
        """The levels that the service in use offers, the least first."""
        return offered(self._style(), self._config.model.reasoning)

    def _chosen_effort(self) -> Effort:
        """The level of effort in force: the one that was chosen in this run, or the config's."""
        if self._effort is not None:
            return self._effort
        return effort_of(self._config.model.reasoning, self._style())

    def _call(self) -> CallConfig:
        """The model that a request of this run asks, and how much it asks it to reason.

        Until a level is chosen in this run, it asks what the config asks.
        """
        if self._effort is None:
            return self._model
        reasoning = reasoning_of(self._effort, self._style(), self._config.model.reasoning)
        return self._model.model_copy(update={"reasoning": reasoning})

    def _hand_down(self, agent: Agent) -> None:
        """Have ``agent`` ask what this run asks from its next request on.

        So does the agent of the conversation where that is another one by now, as where a
        session was opened while a command was carried out. The subagents that either makes
        from then on ask the same: a subagent is made from the spec of its parent.
        """
        call = self._call()
        agent.spec = agent.spec.model_copy(update={"model": call})
        current = self._agent
        if current is not None and current is not agent:
            current.spec = current.spec.model_copy(update={"model": call})

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
                if self._leaving:
                    return
            else:
                # Nothing that an interface says without asking has a meaning yet.
                self._passed_over(message)

    def _passed_over(self, message: Notification) -> None:
        logger.debug("the interface says %s, which means nothing here", message.method)

    async def _do(self, request: Request) -> None:
        """Do what a request asks for, or have it done on the side, and answer it."""
        method = self._methods.get(request.method)
        if method is None:
            refusal = f"There is no method {request.method}."
            self._send(refused(request.id, METHOD_NOT_FOUND, refusal))
            return
        shape, begin = method
        if not self._greeted and request.method not in ("initialize", "shutdown"):
            self._send(refused(request.id, NOT_INITIALIZED, _NOT_GREETED))
            return
        try:
            params = shape.model_validate(dict(request.params))
        except ValidationError as error:
            wrong = f"The params of {request.method} are not right: {_what_is_wrong(error)}."
            self._send(refused(request.id, INVALID_PARAMS, wrong))
            return
        try:
            # What a request looks at, it looks at now. What takes long comes after.
            work = begin(params, request.id)
        except RpcError as error:
            self._send(refused(request.id, error.code, error.message))
            return
        if request.method in ("session/command", "session/list", "session/skills"):
            doing = asyncio.ensure_future(self._answer(request, work))
            self._working.add(doing)
            doing.add_done_callback(self._working.discard)
        else:
            await self._answer(request, work)

    async def _answer(self, request: Request, work: Work) -> None:
        """Do the rest of what a request asks for, and answer it."""
        try:
            result = await work
        except RpcError as error:
            self._send(refused(request.id, error.code, error.message))
        except asyncio.CancelledError:
            # Broken off, because the program leaves. Every request is answered all the same.
            self._send(refused(request.id, LEAVING, _LEAVING))
            raise
        except Exception as error:
            logger.error("the request %s failed", request.method, exc_info=error)
            self._send(refused(request.id, INTERNAL_ERROR, _INSIDE))
        else:
            if result is not None:
                self._send(answered(request.id, result))

    def _answer_came(self, answer: Response) -> None:
        """Hand an answer of the interface to the question that waits for it."""
        waiting = None if answer.id is None else self._questions.get(answer.id)
        if waiting is None or waiting.done():
            # The question was withdrawn, or there never was one.
            logger.debug("an answer to %r came, and nothing waits for it", answer.id)
            return
        waiting.set_result(answer)

    # The requests

    async def _initialize(self, params: _Initialize, id: Id) -> str:
        if self._greeted:
            raise RpcError(NOT_INITIALIZED, _GREETED)
        self._greeted = True
        client = params.client
        who = f"{client.name} {client.version}".strip() or "an interface that gives no name"
        logger.info(
            "the interface is %s. It speaks version %d of the protocol", who, params.protocol
        )
        registry = self._commands.registry
        commands = [{"name": name, "summary": registry[name].summary} for name in sorted(registry)]
        return text(
            {
                "protocol": min(PROTOCOL, params.protocol),
                "server": {"name": "columbina", "version": columbina.__version__},
                "cwd": str(self._config.cwd),
                "commands": commands,
            }
        )

    async def _list(self, params: _Nothing, id: Id) -> str:
        stored = await asyncio.to_thread(self._sessions.stored, self._config.cwd)
        listed = [
            {"id": one.id, "modified_ms": one.modified_ms, "first_input": one.first_input}
            for one in stored
        ]
        return text({"sessions": listed})

    def _is_stored(self, session_id: str | None) -> bool:
        """Tell whether an id names a session that is stored.

        Raises:
            ValueError: The id cannot name a session.
        """
        if session_id is None or self._sessions.root is None:
            return False
        return (self._sessions.directory(self._config.cwd, session_id) / SESSION_FILE).exists()

    async def _open(self, params: _Open, id: Id) -> None:
        """Open a session, and have the conversation go on with its agent.

        The session is opened before the one that is open is ended, so the
        conversation stays where it is if the other one cannot be opened.
        Only a session that is open at this moment has to be ended first to
        be opened again.

        The answer goes out before anything is told of the new session.
        """
        named = params.session_id
        if self._ending:
            # An agent that is still ending may hold the session that is asked for.
            await asyncio.wait(self._ending)
        left = self._agent
        if left is not None and named == left.id:
            self._let_go()
            await self._put_away(left)
            left = None
        try:
            resumed = self._is_stored(named)
            agent = await open_agent(self._ctx, self._agents, self._sessions, self._spec(), named)
        except ValueError as error:
            raise RpcError(INVALID_PARAMS, f"{error}.") from None
        except (SessionError, AgentError) as error:
            raise RpcError(CANNOT_OPEN, f"The session cannot be opened: {error}.") from None
        self._let_go()
        self._adopt(agent)
        # A model or a level that was named while the agent was made holds for it too.
        self._hand_down(agent)
        self._grant_again(agent)
        opened = {
            "session_id": agent.id,
            "resumed": resumed,
            "created_at": agent.session.created_at,
            "events": len(agent.session.events),
        }
        # Nothing was let in between the agent and this: every event from here on is told.
        self._send(answered(id, text(opened)))
        self._look()
        if left is not None:
            ending = asyncio.ensure_future(self._put_away(left))
            self._ending.add(ending)
            ending.add_done_callback(self._ending.discard)

    async def _put_away(self, agent: Agent) -> None:
        """End an agent, and note it if its session could not be written to its end.

        Nobody is told of that at the time: the conversation has left the
        agent, or the program is leaving. It is told by the log, and by the
        exit code of the program.
        """
        failure = await _ended(agent)
        if failure is not None:
            logger.error("the agent of the session %s stopped for good: %s", agent.id, failure)
            self._lost = True

    def _grant_again(self, agent: Agent) -> None:
        """Give a session that is opened the longer yeses that it was given in this run.

        Such a yes is kept with the session that is open. To the person it is
        a yes for a task, and looking at another task in between takes
        nothing back.
        """
        granting = self._ctx.get(PERMISSIONS)
        if granting is None:
            return
        for tool, target in self._granted.get(agent.session.id, ()):
            try:
                granting.grant(agent.session, tool, target)
            except ValueError as error:
                logger.warning("a longer yes could not be given again: %s", error)

    async def _events(self, params: _Events, id: Id) -> str:
        events = self._open_agent().session.events
        page: list[str] = []
        size = 0
        position = params.start
        most = min(params.limit, PAGE_EVENTS)
        while position < len(events) and len(page) < most:
            line = encode_event(events[position])
            if page and size + len(line) > PAGE_BYTES:
                break
            page.append(line)
            size += len(line)
            position += 1
        following = position if position < len(events) else None
        return f'{{"events":[{",".join(page)}],"next":{text(following)}}}'

    async def _skills(self, params: _Nothing, id: Id) -> str:
        """List the skills that there are for the session that is open.

        They are looked for anew, as they are at the beginning of every turn, so a
        skill that was written a moment ago is among them. A program without skills
        has none to list.
        """
        agent = self._open_agent()
        skills = self._ctx.get(SKILLS)
        if skills is None:
            return text({"skills": []})
        found = await skills.found(agent.session, self._config.cwd)
        listed = [
            {
                "name": named,
                "description": one_line(skill.description),
                "user_invocable": skill.user_invocable,
                "model_invocable": skill.model_invocable,
            }
            for named, skill in found.skills.items()
        ]
        return text({"skills": listed})

    async def _prompt(self, params: _Said, id: Id) -> str:
        agent, said = self._working_agent(), self._words(params)
        self._give(agent, agent.followup, said)
        return "{}"

    async def _steer(self, params: _Steered, id: Id) -> str:
        """Give the agent words for its next step. With ``now``, the step that runs is cut short."""
        agent, said = self._working_agent(), self._words(params)
        self._give(agent, functools.partial(agent.steer, now=params.now), said)
        return "{}"

    def _give(self, agent: Agent, give: Callable[[str], None], said: str) -> None:
        """Give an agent what was said, or keep it back until its command is over.

        While a command is carried out the agent is spoken for, from the
        moment the command was taken until it has come to its end, whatever
        it does in between. A command has not begun its work when the next
        line is read, and it may have the agent work more than once. A turn
        that began meanwhile would come before the command, or between two
        parts of it. So what is said meanwhile is kept back here, in the
        order in which it came.
        """
        if self._commanding is agent:
            self._held.append((give, said))
        else:
            give(said)
        self._look()

    def _release(self) -> None:
        """Give the agent what was kept back for it, now that its command is over.

        What is kept back is always for the agent of the conversation: it is
        dropped when the conversation leaves an agent.
        """
        try:
            # The program may be leaving, and then the agent takes nothing more.
            if self._ended is None:
                # Each is still kept back while it is given: whoever looks at where the
                # agent stands in that moment is not to find it at rest with nothing to do.
                for give, said in self._held:
                    give(said)
        finally:
            self._held = []
        self._look()

    async def _cancel(self, params: _Nothing, id: Id) -> str:
        agent = self._open_agent()
        # What waits is dropped: here, and by the agent what it holds itself.
        self._held.clear()
        rested = agent.at_rest
        agent.cancel("user")
        if rested and self._commanding is agent and self._carrying is not None:
            # There was nothing of the agent to stop. A command that has not begun its
            # work does not begin it, and one that does none of the agent is broken off.
            self._carrying.cancel()
        return "{}"

    def _command(self, params: _Line, id: Id) -> Work:
        """Take a command that is to be carried out, and return the carrying out of it."""
        agent = self._working_agent()
        line = params.line.strip()
        if not is_command(line):
            raise RpcError(INVALID_PARAMS, "A command begins with a slash.")
        if not agent.at_rest or self._commanding is agent:
            raise RpcError(
                NOT_IDLE,
                "The agent is not at rest, and a command waits until it is. "
                "session/cancel stops the agent.",
            )
        # From this moment the agent is spoken for, whatever comes in next. The command is
        # a task of its own, so that it can be broken off before it has taken a step.
        self._commanding = agent
        self._carrying = command = asyncio.ensure_future(self._commands.run(agent, line))
        return self._carried_out(line, command)

    async def _carried_out(self, line: str, command: "asyncio.Future[str]") -> str:
        """Wait for a command, and give the agent what was kept back until it was over."""
        try:
            said = await command
        except asyncio.CancelledError:
            # Broken off from here, before it could say so itself: it was told to stop
            # before the agent had begun with it, or the program leaves.
            said = f"The command {line.partition(' ')[0].lower()} was stopped."
        finally:
            # A command of a session that was left may end while the session that is open
            # has one of its own carried out. That one is not this one's to end.
            mine = self._carrying is command
            if mine:
                self._commanding = None
                self._carrying = None
        if mine:
            self._release()
        return text({"text": said})

    async def _shutdown(self, params: _Nothing, id: Id) -> str:
        self._leaving = True
        await self._end()
        return "{}"

    # The agent of the conversation

    def _spec(self) -> AgentSpec:
        config = self._config
        return AgentSpec(cwd=config.cwd, model=self._call(), system_prompt=config.system_prompt)

    def _open_agent(self) -> Agent:
        if self._agent is None:
            raise RpcError(NO_SESSION, _NO_SESSION)
        return self._agent

    def _working_agent(self) -> Agent:
        agent = self._open_agent()
        if self._stopped is not None:
            raise RpcError(STOPPED, f"The agent has stopped: {self._stopped}.")
        return agent

    def _words(self, params: _Said) -> str:
        said = params.text.strip()
        if not said:
            raise RpcError(INVALID_PARAMS, "There is nothing in the text to give to the agent.")
        return said

    def _let_go(self) -> None:
        """Have the conversation be with no agent: what the one before says is told no more."""
        self._agent = None
        self._stopped = None
        self._said = None
        self._held = []
        if self._watching is not None:
            self._watching.cancel()
            self._watching = None

    def _adopt(self, agent: Agent) -> None:
        """Make an agent the one of the conversation, and listen to what happens to it."""
        self._agent = agent

        # What is heard in the scope of an agent is what happens there, and what happens
        # outside every scope as well. Only what is this agent's own is told, and only for
        # as long as the conversation is with it: an agent that is being ended still writes.
        def event(committed: Committed) -> None:
            if agent is self._agent and committed.session is agent.session:
                self._event(committed)

        def chunk(frame: StreamFrame) -> None:
            if agent is self._agent and frame.agent is agent:
                self._chunk(frame)

        agent.ctx.on(SESSION_EVENT, event)
        agent.ctx.on(STREAM, chunk)
        agent.ctx.on(APPROVAL_ASK, self._asker(agent))
        # Only the agent of the conversation hears this: a subagent has a scope of its own.
        agent.ctx.on(SYSTEM_PROMPT_ASSEMBLE, self._with_the_effort)

    def _event(self, committed: Committed) -> None:
        event = encode_event(committed.event)
        session = text(committed.session.id)
        self._send(told("session.event", f'{{"session_id":{session},"event":{event}}}'))
        self._look()

    def _chunk(self, frame: StreamFrame) -> None:
        chunk = text(frame.chunk.model_dump(mode="json"))
        where = f'"session_id":{text(frame.agent.id)},"turn":{frame.turn},"step":{frame.step}'
        self._send(told("session.stream", f'{{{where},"chunk":{chunk}}}'))

    async def _with_the_effort(self, draft: Draft, proceed: Next[Draft, str]) -> str:
        """Tell the agent of the conversation how to work, where the level of effort is ultra."""
        text = await proceed(draft)
        return f"{text}\n\n{ULTRA}" if self._chosen_effort() == "ultra" else text

    # Where the agent stands

    def _look(self) -> None:
        """Say where the agent stands, if that is something else than was said last."""
        agent = self._agent
        if agent is None:
            return
        if not agent.at_rest and self._watching is None:
            self._watching = asyncio.ensure_future(self._rest(agent))
        said = self._status(agent)
        if said != self._said:
            self._said = said
            self._send(told("session.status", said))

    async def _rest(self, agent: Agent) -> None:
        """Wait until the agent is at rest, and notice if it has stopped for good."""
        try:
            await agent.when_idle()
        except asyncio.CancelledError:
            raise
        except Exception as error:
            self._stopped = str(error) or type(error).__name__
        # Whoever looks next finds nobody waiting, and has somebody wait if the agent is
        # at work again by then.
        self._watching = None
        self._look()

    def _status(self, agent: Agent) -> str:
        session = agent.session
        if self._stopped is not None:
            status, detail = "stopped", self._stopped
        elif agent.at_rest and not self._held:
            status, detail = "idle", None
        else:
            # At work, or about to be: what was kept back for it begins a turn.
            status, detail = "running", None
        sandbox = self._ctx.get(SANDBOX)
        approval = self._ctx.get(APPROVAL)
        permissions = self._ctx.get(PERMISSIONS)
        # Before the first turn the log names neither. The permissions know what it will name.
        mode, policy = (None, None) if permissions is None else permissions.in_force(session)
        held = None
        if sandbox is not None:
            held = {
                "mode": mode or sandbox.mode(session),
                "strength": sandbox.strength(session),
            }
        model = self._model
        return text(
            {
                "session_id": agent.id,
                "status": status,
                "detail": detail,
                "model": {
                    "provider": model.provider,
                    "model": model.model,
                    "context_window": model.context_window,
                    "effort": self._chosen_effort(),
                    "efforts": list(self._levels()),
                },
                "sandbox": held,
                "approval": None if approval is None else policy or approval.policy(session),
                "spent": self._spent(session),
            }
        )

    def _spent(self, session: Session) -> dict[str, float] | None:
        """Say what the requests of a session have cost, in each currency, if that is known."""
        spending = self._ctx.get(SPENDING)
        if spending is None:
            return None
        return {
            currency: round(amount, 6) for currency, amount in spending.of(session).money.items()
        }

    # Questions

    def _asker(self, agent: Agent) -> Callable[[ApprovalRequest], Awaitable[Outcome | None]]:
        """Return what puts the questions of an agent to the interface."""

        async def ask(request: ApprovalRequest) -> Outcome | None:
            if agent is not self._agent or request.session is not agent.session:
                # The question is about another session, or the conversation is elsewhere
                # by now. Whoever is at the other end of the wire is not the one to ask.
                return None
            granting = self._ctx.get(PERMISSIONS)
            standing = None if granting is None else request.standing
            self._asked += 1
            question = self._asked
            waiting: asyncio.Future[Response] = asyncio.get_running_loop().create_future()
            self._questions[question] = waiting
            about = {
                "session_id": request.session.id,
                "call_id": request.call_id,
                "tool": request.tool,
                "summary": request.summary,
                "reason": request.reason,
                "target": request.target,
                "standing": standing,
            }
            self._send(asked(question, "approval/ask", text(about)))
            try:
                answer = await waiting
            finally:
                # Also when the question is withdrawn: what comes for it later means nothing.
                del self._questions[question]
            given = None if answer.failed else answer.result
            said = cast("dict[str, object]", given) if isinstance(given, dict) else {}
            if said.get("outcome") != "allowed-once":
                # Only a plain yes is a yes.
                return "rejected"
            if said.get("remember") is True and granting is not None and standing is not None:
                try:
                    granting.grant(request.session, request.tool, request.target)
                except ValueError as error:
                    logger.warning("the longer yes could not be written down: %s", error)
                else:
                    granted = self._granted.setdefault(request.session.id, [])
                    granted.append((request.tool, request.target))
            return "allowed-once"

        return ask

    # Leaving

    async def _end(self) -> bool:
        """End the agent and what is done on the side.

        Returns:
            Whether a session of this run could not be written to its end.
        """
        if self._ended is not None:
            return self._ended
        self._ended = False
        agent = self._agent
        if self._watching is not None:
            self._watching.cancel()
            self._watching = None
        if agent is not None:
            await self._put_away(agent)
        if self._ending:
            await asyncio.wait(self._ending)
        # A command whose agent was ended says so and is answered. What is still at work
        # after that is broken off.
        if self._working:
            await asyncio.wait(self._working, timeout=_LAST_WORK_SECONDS)
        for doing in list(self._working):
            doing.cancel()
        if self._working:
            await asyncio.wait(self._working)
        self._ended = self._lost
        return self._ended
