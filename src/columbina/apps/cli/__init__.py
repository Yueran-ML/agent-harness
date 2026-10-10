"""The terminal interface: a conversation with an agent, in a terminal.

This module is a plugin. It provides the frontend that ``columbina`` runs when
it is started without a command.

A prompt stays open at the bottom of the terminal, and the conversation is
shown above it. A line that the user submits goes to the agent. While the
agent is at rest, the line is a new task. While a turn is running, it is
something said to the agent on its way, which the model reads when its next
step begins. A line that begins with a slash is a command for the program
itself, and it is carried out while the agent is at rest.

A command may take long: it may have the agent sum up the conversation. The
keys are read meanwhile. Escape stops what the agent does for the command,
and a line that is submitted waits until the command is over.

What is shown comes from the log of the session, so a session that is taken
up again looks the way it looked. Only the reply that is being written is
shown from its chunks, because nothing else is quick enough.

Escape stops the turn that is running. A call that needs a yes is asked
about at the prompt.
"""

import asyncio
import logging
import sys
from collections import deque
from collections.abc import Callable
from pathlib import Path
from typing import TypeVar

from pydantic import BaseModel
from rich.text import Text

import columbina
from columbina.agent import (
    AGENTS,
    REQUEST,
    STREAM,
    Agent,
    Agents,
    AgentSpec,
    RequestDraft,
    StreamFrame,
)
from columbina.approval import APPROVAL_ASK
from columbina.apps import FRONTEND
from columbina.apps.cli.approval import Responder
from columbina.apps.cli.prompt import Prompt
from columbina.apps.cli.status import status
from columbina.apps.cli.terminal import TERMINAL, real
from columbina.apps.cli.transcript import Symbols, Transcript, fit, recap
from columbina.apps.common import DEFAULT_PROMPT, open_agent
from columbina.apps.terminal_text import harmless
from columbina.commands import COMMANDS, SWITCH, Command, Commands, Opener, is_command
from columbina.kernel import Context, Next
from columbina.llm import CallConfig
from columbina.permissions import PERMISSIONS
from columbina.sandbox import SANDBOX
from columbina.session import (
    SESSION_EVENT,
    SESSIONS,
    TURN,
    Committed,
    SessionError,
    Sessions,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.token_meter import TOKEN_METER

__all__ = ["Cli", "Config", "apply", "inject", "name"]

name = "cli"
inject = (AGENTS, SESSIONS, COMMANDS)

_NOTE = "dim"
_BAD = "red"
_NAME = "bold"

_NO_TERMINAL = (
    "columbina: this is not a terminal, and the conversation needs one. "
    "`columbina run TASK` gives an agent one task without it."
)

HeardT = TypeVar("HeardT")


class Config(BaseModel):
    """The config of the terminal interface.

    Attributes:
        model: The model that the agent talks to, and how.
        cwd: The working directory of the agent.
        system_prompt: The instructions that open a conversation.
        session_id: The id of the session. A stored session with this id is
            taken up again. Without an id a new session begins.
        resume: Whether to take up the session of this working directory
            that was used last, if no id names one.
    """

    model: CallConfig
    cwd: Path
    system_prompt: str = DEFAULT_PROMPT
    session_id: str | None = None
    resume: bool = False


class _Shown(logging.Handler):
    """Shows what the program logs above the prompt, where it does not tear the typing apart."""

    def __init__(self, prompt: Prompt) -> None:
        super().__init__()
        self._prompt = prompt

    def emit(self, record: logging.LogRecord) -> None:
        style = _BAD if record.levelno >= logging.ERROR else _NOTE
        self._prompt.show([fit(f"columbina: {record.getMessage()}", style)])


class Cli:
    """A conversation with one agent at a time, at a terminal."""

    __slots__ = (
        "_agent",
        "_agents",
        "_command",
        "_commands",
        "_config",
        "_ctx",
        "_gone",
        "_leaving",
        "_model",
        "_prompt",
        "_sessions",
        "_symbols",
        "_transcript",
        "_turning",
        "_watching",
    )

    def __init__(
        self, ctx: Context, config: Config, agents: Agents, sessions: Sessions, commands: Commands
    ) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions
        self._commands = commands
        self._model = config.model
        self._agent: Agent | None = None
        self._prompt: Prompt | None = None
        self._symbols = Symbols()
        self._transcript = Transcript(self._symbols)
        self._watching: set[asyncio.Future[None]] = set()
        self._gone = asyncio.Event()
        self._leaving = False
        # Whether the agent is in a turn, or about to begin one.
        self._turning = False
        # The command that is being carried out.
        self._command: asyncio.Future[None] | None = None

    async def run(self) -> int:
        """Hold the conversation until the user leaves.

        Returns:
            0 when the user left, 1 if the session could not be opened or
            the agent stopped for good, be it in the middle of the
            conversation or while the user left, and 2 if there is no
            terminal.
        """
        terminal = self._ctx.get(TERMINAL) or real()
        if terminal is None:
            print(_NO_TERMINAL, file=sys.stderr)
            return 2
        prompt = self._prompt = Prompt(terminal, status=self.status_line)
        self._symbols = Symbols.of(terminal)
        log = logging.getLogger("columbina")
        shown = _Shown(prompt)
        # While the prompt is there, what is logged stands above it and goes nowhere else:
        # whatever wrote to the terminal by itself would tear the prompt apart.
        others = list(log.handlers)
        for other in others:
            log.removeHandler(other)
        log.addHandler(shown)
        try:
            code = await self._converse(prompt)
        finally:
            log.removeHandler(shown)
            for other in others:
                log.addHandler(other)
            await self._drop_the_command()
            failure = await self._close()
        if failure is not None and not self._gone.is_set():
            # The agent stopped for good while the program was leaving: what was left of
            # its turn could not be written down. Nobody has been told so yet.
            prompt.show([fit(f"(the agent has stopped: {failure})", _BAD)])
            return 1
        return code

    def status_line(self) -> str:
        """Return what the line below the prompt says at this moment."""
        if self._agent is None or self._prompt is None:
            return ""
        session = self._agent.session
        sandbox = self._ctx.get(SANDBOX)
        permissions = self._ctx.get(PERMISSIONS)
        mode = None if permissions is None else permissions.in_force(session)[0]
        return status(
            session,
            self._model,
            sandbox,
            busy=self._prompt.busy,
            mode=mode,
            meter=self._ctx.get(TOKEN_METER),
        )

    async def leave(self, agent: Agent, rest: str) -> str:
        """Carry out the command that ends the conversation."""
        self._leaving = True
        return ""

    async def model(self, agent: Agent, rest: str) -> str:
        """Carry out the command that shows the model, or names another one."""
        if rest:
            self._model = self._model.model_copy(update={"model": rest})
            return f"The model is {rest} from the next request on."
        return f"The model is {self._model.model}. Another one is named with /model ID."

    async def begin_anew(self, agent: Agent, rest: str) -> str:
        """Carry out the command that ends the session and begins another one."""
        opened = await self.switch(self._agents.create)
        return f"A new session begins: {opened.id}"

    async def switch(self, opener: Opener) -> Agent:
        """Have the conversation go on with the agent that ``opener`` creates.

        The agent is created first. If that fails, the conversation stays
        with the agent that it has, and the failure reaches the caller.

        The conversation goes over as soon as the other agent is there, and
        the agent that it had is ended after that. So nothing that the other
        agent does goes unheard while the one before it ends. And whatever
        happens meanwhile, the other agent is the one of the conversation: the
        next line goes to it, and it is ended when the program leaves.
        """
        prompt = self._prompt
        assert prompt is not None
        agent = await opener(self._ctx, self._spec())
        left = self._agent
        self._unwatch()
        self._adopt(agent, prompt)
        await _ended(left)
        return agent

    async def _converse(self, prompt: Prompt) -> int:
        try:
            agent = await self._open(await self._first_session(prompt), prompt)
        except (SessionError, ValueError) as error:
            prompt.show([fit(f"columbina: {error}", _BAD)])
            return 1
        self._greet(prompt, agent)
        gone = asyncio.ensure_future(self._gone.wait())
        typed: asyncio.Future[str | None] | None = None
        # The lines that were submitted and are not taken yet.
        lines: deque[str] = deque()
        try:
            while not self._leaving:
                command = self._command
                if command is None and lines:
                    self._take(prompt, lines.popleft())
                    continue
                # The keys are read while a command is carried out, so that it can be stopped.
                typed = typed or asyncio.ensure_future(prompt.line())
                waited = {typed, gone} if command is None else {typed, gone, command}
                await asyncio.wait(waited, return_when=asyncio.FIRST_COMPLETED)
                if self._gone.is_set():
                    return 1
                if command is not None and command.done():
                    self._command = None
                    self._mark(prompt)
                    # What a command let through is an error of the program, and ends it.
                    command.result()
                    continue
                line, typed = typed.result(), None
                if line is None:
                    break
                lines.append(line)
            return 0
        finally:
            gone.cancel()
            if typed is not None:
                # Nobody takes a line from the prompt any more.
                prompt.put_away()

    async def _first_session(self, prompt: Prompt) -> str | None:
        """Return the id of the session that the conversation begins on, if one is named."""
        config = self._config
        if config.session_id is not None or not config.resume:
            return config.session_id
        stored = await asyncio.to_thread(self._sessions.stored, config.cwd)
        if stored:
            return stored[0].id
        note = "(no session of this directory is stored yet. A new one begins.)"
        prompt.show([Text(note, style=_NOTE)])
        return None

    def _spec(self) -> AgentSpec:
        """Return what an agent of this conversation is created from."""
        config = self._config
        return AgentSpec(cwd=config.cwd, model=self._model, system_prompt=config.system_prompt)

    async def _open(self, session_id: str | None, prompt: Prompt) -> Agent:
        """Create the agent that the conversation begins with."""
        agent = await open_agent(self._ctx, self._agents, self._sessions, self._spec(), session_id)
        self._adopt(agent, prompt)
        return agent

    def _adopt(self, agent: Agent, prompt: Prompt) -> None:
        """Make an agent the one of the conversation, and listen to what happens to it."""
        self._agent = agent
        self._transcript = Transcript(self._symbols)
        prompt.show(recap(agent.session.events, self._symbols))
        prompt.partial = self._transcript.partial
        agent.ctx.on(SESSION_EVENT, self._heard(agent, self._event))
        agent.ctx.on(STREAM, self._heard(agent, self._chunk))
        agent.ctx.on(APPROVAL_ASK, Responder(prompt, self._ctx.get(PERMISSIONS)))
        agent.ctx.on(REQUEST, self._with_the_model)
        prompt.on_cancel = lambda: agent.cancel("user")
        # The agent may be in a turn already. Whoever waits for it to come to rest finds out.
        self._turning = True
        self._mark(prompt)
        self._watch(prompt, agent)

    def _heard(self, agent: Agent, hear: Callable[[HeardT], None]) -> Callable[[HeardT], None]:
        """Return a listener that hears ``agent`` for as long as the conversation is with it.

        The conversation may go over to another agent while this one is still
        ending. What it writes down then is in its log, and is not shown as
        if it were of the conversation.
        """

        def listener(heard: HeardT) -> None:
            if agent is self._agent:
                hear(heard)

        return listener

    def _mark(self, prompt: Prompt) -> None:
        """Say at the prompt whether something is running that Escape would stop."""
        prompt.busy = self._turning or self._command is not None

    def _greet(self, prompt: Prompt, agent: Agent) -> None:
        head = Text()
        head.append("Columbina", style=_NAME)
        head.append(harmless(f" {columbina.__version__}   {self._model.model}   {agent.cwd}"))
        hint = Text("/help lists the commands. Ctrl+D leaves.", style=_NOTE)
        prompt.show([head, hint, Text()])

    def _take(self, prompt: Prompt, line: str) -> None:
        """Do with a line what its beginning and the state of the agent say.

        The line goes to the agent that the conversation is with at this
        moment. The spaces around a line do not count, also not for whether
        it is a command.
        """
        agent = self._agent
        assert agent is not None
        line = line.strip()
        if not line:
            return
        if is_command(line):
            if self._turning:
                waits = "(a command waits until the agent is at rest. Escape stops the agent.)"
                prompt.show([Text(waits, style=_NOTE)])
                return
            self._command = asyncio.ensure_future(self._carry_out(prompt, agent, line))
            self._mark(prompt)
        elif self._turning:
            agent.steer(line)
        else:
            agent.followup(line)
            self._turning = True
            self._mark(prompt)
            self._watch(prompt, agent)

    async def _carry_out(self, prompt: Prompt, agent: Agent, line: str) -> None:
        """Carry out a command, and show what it says."""
        said = await self._commands.run(agent, line)
        if said:
            # An empty line sets the answer apart from what comes after it.
            prompt.show([*(fit(part) for part in said.splitlines()), Text()])

    async def _drop_the_command(self) -> None:
        """Stop the command that is being carried out, if one is: the program is leaving."""
        command, self._command = self._command, None
        if command is not None:
            command.cancel()
            await asyncio.gather(command, return_exceptions=True)

    def _watch(self, prompt: Prompt, agent: Agent) -> None:
        """Have somebody wait for the agent to come to rest, unless somebody does."""
        if self._watching:
            return
        watching = asyncio.ensure_future(self._rest(prompt, agent))
        self._watching.add(watching)
        watching.add_done_callback(self._watching.discard)

    def _unwatch(self) -> None:
        """Have nobody wait any longer for the agent to come to rest."""
        for watching in list(self._watching):
            watching.cancel()
        self._watching.clear()

    async def _rest(self, prompt: Prompt, agent: Agent) -> None:
        """Wait until the agent is at rest, and notice if it has stopped for good."""
        try:
            await agent.when_idle()
        except asyncio.CancelledError:
            raise
        except Exception as error:
            prompt.show([fit(f"(the agent has stopped: {error})", _BAD)])
            self._gone.set()
        finally:
            # The conversation may be with another agent by now, which has a state of its own.
            if agent is self._agent:
                self._turning = False
                self._mark(prompt)

    async def _with_the_model(
        self, draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]
    ) -> CallConfig:
        """Make a request with the model that the user named last."""
        config = await proceed(draft)
        return config.model_copy(update={"model": self._model.model})

    def _event(self, committed: Committed) -> None:
        prompt, agent = self._prompt, self._agent
        assert prompt is not None
        assert agent is not None
        data = committed.event.data
        if isinstance(data, TurnStartEvent):
            self._turning = True
            self._mark(prompt)
            # A turn can begin without a line from the keys. An agent that stops for good
            # in such a turn has to be noticed as well.
            self._watch(prompt, agent)
        elif isinstance(data, TurnEndEvent):
            self._turning = False
            self._mark(prompt)
        prompt.show(self._transcript.event(committed.event))
        prompt.partial = self._transcript.partial

    def _chunk(self, frame: StreamFrame) -> None:
        prompt = self._prompt
        assert prompt is not None
        prompt.show(self._transcript.chunk(frame.chunk))
        prompt.partial = self._transcript.partial

    async def _close(self) -> Exception | None:
        """End the agent of the conversation, properly, before the program goes.

        Returns:
            What stopped the agent for good, if something did. That can
            happen at this very moment: the turn that is stopped here leaves
            something to be written down.
        """
        self._unwatch()
        return await _ended(self._agent)


async def _ended(agent: Agent | None) -> Exception | None:
    """End an agent properly, if there is one: a turn that is running is stopped first.

    Returns:
        What stopped the agent for good, if something did.
    """
    if agent is None:
        return None
    if TURN.of(agent.session).in_turn:
        agent.cancel("user")
    await asyncio.shield(agent.close())
    try:
        await agent.when_idle()
    except Exception as error:
        return error
    return None


def apply(ctx: Context, config: Config) -> None:
    """Provide the frontend that holds a conversation at a terminal."""
    commands = ctx.use(COMMANDS)
    cli = Cli(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS), commands)
    commands.registry.add(ctx, "exit", Command(cli.leave, "Leave."))
    commands.registry.add(ctx, "model", Command(cli.model, "Show the model, or name another."))
    commands.registry.add(
        ctx, "new", Command(cli.begin_anew, "End this session and begin another one.")
    )
    ctx.provide(SWITCH, cli.switch)
    ctx.provide(FRONTEND, cli)
