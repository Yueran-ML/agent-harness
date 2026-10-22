"""The assistant in WeCom: a conversation with the person who owns the bot, in a chat of WeCom.

This module is a plugin. It provides the frontend of ``columbina wecom``.

The assistant keeps the long connection of the bot, :mod:`columbina.wecom.link`, and hears
every message that is written to it. It answers one person, its owner, in a chat of the two.
Whoever sends the code of pairing first becomes the owner. The code is shown on the computer
that the assistant runs on, holds for some minutes, and is given up after too many wrong
tries. What anybody else writes is passed over, and so is what is written in a group.

What the owner writes goes to an agent, in a session of its own that is taken up again when
the assistant starts again. Each message is a turn of its own, and the answer of the turn
answers that message: from the moment that the message is taken it says that the agent is
thinking, and once the turn is over it holds the answer. WeCom lets such an answer be written
for ten minutes. Where the agent works longer, the answer says so and the agent's answer comes
in a message of its own, and so does an answer to a message from before the assistant started
again. An answer too long for one message goes on in more. A few things are done by the
assistant itself: ``/new`` begins a new session, ``/stop`` stops the answer, ``/skills`` lists
the skills and deletes one, ``/help`` says what there is. A voice message is read as the text
that WeCom made of it. A file of text that the owner sends is kept in the directory ``uploads``
and the agent is told of it. A ``SKILL.md`` is a skill, which goes into the directory of skills
of the assistant.

WeCom may send a message again. The messages that were taken are written down, and one that
comes again is passed over, also after the assistant started again.

Another program, such as the desktop application, may run the assistant. It is then told
how the assistant is doing in lines of JSON, and stops it by closing the standard input. The
answers that are being written then say that they will not be, before the connection closes.

The agent may read, and not change anything or run anything: the profile gives it no such
tool, and nobody is there to say yes to one. Its owner may let it change the files of the
directory of the assistant, with the profile ``wecom-writable``. That directory lies apart
from the home of the program, where the program keeps the lock of the assistant, how it is
doing and which messages it took.
"""

import asyncio
import importlib.util
import json
import logging
import os
import re
import secrets
import sys
import time
from collections import deque
from collections.abc import Awaitable, Callable, Coroutine, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

import httpx
from pydantic import BaseModel, Field

import columbina
from columbina import files, frontmatter
from columbina.agent import AGENTS, Agent, AgentError, Agents, AgentSpec, replies
from columbina.apps import FRONTEND
from columbina.apps.common import open_agent, when_the_input_closes
from columbina.apps.terminal_text import harmless
from columbina.feishu.messages import parts
from columbina.feishu.store import within
from columbina.kernel import Context
from columbina.llm import CallConfig
from columbina.net import proxy_for
from columbina.permissions import PERMISSIONS
from columbina.session import (
    SESSION_EVENT,
    SESSIONS,
    Committed,
    SessionError,
    Sessions,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.session.lock import acquire
from columbina.skills import SKILLS, one_line
from columbina.skills.files import FILE, MAX_BYTES, SkillError, parsed
from columbina.wecom import media, messages, store
from columbina.wecom.app import WecomError, address
from columbina.wecom.link import EVENT, RESPOND, SEND, WELCOME, Link, request_id
from columbina.wecom.words import Language, words_for

__all__ = ["PROMPT", "Assistant", "Config", "apply", "inject", "name"]

name = "wecom"
inject = (AGENTS, SESSIONS)

logger = logging.getLogger(__name__)

PROMPT = (
    "You are Columbina, a personal assistant. You talk with your user in a chat of WeCom. "
    "Answer the way one does in a chat: to the point, and at length only where the user asks "
    "for it. What you write is shown as Markdown. A message that the user speaks reaches you as "
    "text. A file that the user sends you lies in the directory uploads, and you are told where."
)
"""The words that open a conversation of the assistant."""

SEEN = "seen.json"
"""The file that names the messages that were taken, in what the program keeps for the assistant."""

UPLOADS = "uploads"
"""The directory, in the directory of the assistant, of the files that the owner sent."""

SKILLS_DIRECTORY = "skills"
"""The directory of skills of the assistant, in its directory."""

_MOST = 20_480
"""How many bytes an answer in one message may take, as WeCom counts them in UTF-8."""

_STREAM_SECONDS = 540.0
"""How long an answer is written in the message that it answers. WeCom lets it be written for
ten minutes from its first part, and some time is left for the last."""

_SEEN_MOST = 500
"""How many messages that were taken are remembered."""

_LAST_SECONDS = 3.0
"""How long the assistant takes at most, when it stops, to say so in the answers being written."""

_PAUSES = (2.0, 5.0, 15.0)
"""How long an answer waits before it is sent again, where WeCom was not reached or asked for
fewer messages. An answer is sent once more than there are pauses."""

_TOO_OFTEN = 846607
"""The errcode with which WeCom says that the chat was sent more than it takes for now."""

_TRIES_EACH = 5
_TRIES_ALL = 20
"""How many wrong codes of pairing one sender may send, and all senders together. The last of
them gives the code up until the assistant starts again."""

_SEND_FAILED = "an answer could not be sent to WeCom"
"""How what went wrong begins where an answer could not be sent. It holds until one is."""

_NOT_NOTED = (
    "which messages were taken cannot be written down, so one that WeCom sends again after a "
    "new start may be answered twice"
)

_TEXT = {
    ".txt",
    ".md",
    ".markdown",
    ".csv",
    ".tsv",
    ".json",
    ".jsonl",
    ".yaml",
    ".yml",
    ".toml",
    ".ini",
    ".cfg",
    ".log",
    ".xml",
    ".html",
    ".htm",
    ".py",
    ".js",
    ".ts",
    ".sql",
    ".sh",
    ".tex",
    ".bib",
}
"""What a file of text is called at its end. Other files are not taken."""

_UNNAMED = "file.txt"
"""The name of a file of text that was sent without one."""

_UNSAFE = re.compile(r'[\x00-\x1f\x7f<>:"/\\|?*]')
"""What a name of a file may not hold on one system or another."""

_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
"""What the name of a skill is made of."""


class Config(BaseModel):
    """The config of the assistant in WeCom.

    Attributes:
        model: The model that the agent talks to, and how.
        cwd: The directory of the assistant: the working directory of the agent, with its
            instruction file, its skills and the files that it was sent. It lies apart from the
            home of the program.
        home: The home of the program, where the bot is kept.
        system_prompt: The instructions that open a conversation.
        language: The language of what the assistant says by itself, Chinese where none is said.
        pairing_minutes: How long a code of pairing holds.
        json_lines: Whether another program runs the assistant, such as the desktop
            application. It is then told how the assistant is doing in lines of JSON on the
            standard output, and stops it by closing the standard input.
        writable: Whether the agent is given the tools that change files. It runs only where its
            owner lets it, and changes no file outside its directory. Otherwise it changes none
            at all, whatever tools it was given.
    """

    model: CallConfig
    cwd: Path
    home: Path
    system_prompt: str = PROMPT
    language: Language | None = None
    pairing_minutes: float = Field(default=10.0, gt=0)
    json_lines: bool = False
    writable: bool = False


def _tell(message: str) -> None:
    """Say something to whoever started the assistant."""
    print(harmless(f"columbina: {message}"), file=sys.stderr)


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def _safe(name: str) -> str:
    """Return the name of a file that was sent as one that may be written on any system."""
    last = name.replace("\\", "/").rsplit("/", 1)[-1]
    cleaned = _UNSAFE.sub("_", last).strip(" .")
    return (cleaned or "file")[-120:]


def _text_like(content: bytes) -> bool:
    """Tell whether what was sent without a name reads as text."""
    if b"\x00" in content:
        return False
    try:
        content.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


class _Seen:
    """The messages that were taken, kept on disk so that one that comes again is known."""

    def __init__(self, path: Path) -> None:
        self._path = path
        try:
            found: object = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            found = []
        listed = cast("list[object]", found) if isinstance(found, list) else []
        self._ids = deque((one for one in listed if isinstance(one, str)), maxlen=_SEEN_MOST)

    def __contains__(self, message_id: str) -> bool:
        return message_id in self._ids

    def add(self, message_id: str) -> bool:
        """Note that a message was taken, and say whether that is written down."""
        self._ids.append(message_id)
        try:
            files.replaced(self._path, json.dumps(list(self._ids)))
        except OSError as error:
            logger.warning("which messages were taken cannot be written down: %s", error)
            return False
        return True


@dataclass(slots=True)
class _Pairing:
    """The code that makes whoever sends it the owner, while there is none."""

    code: str
    until: float
    tries: dict[str, int]
    total: int = 0


@dataclass(slots=True)
class _Asked:
    """A message of the owner that the agent answers in a turn of its own.

    Attributes:
        req_id: The message, as the frames that answer it name it.
        stream: The answer, as the frames that write it name it.
        began: When the answer was begun, on the clock of the assistant, once it was.
        beginning: The work of beginning it.
        expiring: What ends it where the agent works for longer than it may be written.
        closed: Whether it was ended, or no more can be written in it.
    """

    req_id: str
    stream: str
    began: float | None = None
    beginning: asyncio.Future[None] | None = None
    expiring: asyncio.Future[None] | None = None
    closed: bool = False


def _markdown(text: str) -> dict[str, Any]:
    return {"msgtype": "markdown", "markdown": {"content": text}}


def _stream(asked: _Asked, text: str, *, finish: bool) -> dict[str, Any]:
    return {"msgtype": "stream", "stream": {"id": asked.stream, "finish": finish, "content": text}}


class Assistant:
    """Holds the conversation with the owner of the bot, in WeCom."""

    def __init__(self, ctx: Context, config: Config, agents: Agents, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions
        self._now: Callable[[], float] = time.time
        self._clock: Callable[[], float] = time.monotonic
        self._pause: Callable[[float], Awaitable[None]] = asyncio.sleep
        self._words = words_for(config.language or "zh")
        self._link: Link | None = None
        self._http: httpx.AsyncClient | None = None
        self._settings: store.Settings | None = None
        self._agent: Agent | None = None
        self._seen: _Seen | None = None
        self._pairing: _Pairing | None = None
        self._queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
        self._waiting: deque[_Asked] = deque()
        self._turns: dict[int, tuple[_Asked | None, int]] = {}
        self._doing: set[asyncio.Future[Any]] = set()
        self._state: store.State = "connecting"
        self._error: str | None = None

    # Starting and ending

    async def run(self) -> int:
        """Hold the conversation until the program is stopped, or WeCom refuses the bot.

        Returns:
            1 where WeCom refused the bot or the session could not be opened, and 2 where
            nothing that the assistant needs is there.
        """
        config = self._config
        if any(
            importlib.util.find_spec(package) is None for package in ("websockets", "cryptography")
        ):
            _tell(
                "the assistant in WeCom needs the packages websockets and cryptography, which come "
                "with 'pip install columbina[wecom]'"
            )
            return 2
        folder = config.home / store.FOLDER
        try:
            folder.mkdir(parents=True, exist_ok=True)
            held = acquire(folder / store.LOCK)
        except OSError as error:
            _tell(f"the directory of the assistant cannot be used: {error}")
            return 2
        if held is None:
            _tell("an assistant in WeCom runs on this machine already, and only one may run")
            return 2
        try:
            # The bot is read under the lock: it is set up or deleted only while nobody holds it.
            try:
                chosen = store.settings(config.home)
                place = None if chosen is None else address(os.environ)
                secret = None if place is None else store.secret(config.home, place)
            except WecomError as error:
                _tell(str(error))
                return 2
            if chosen is None or place is None or secret is None:
                _tell(
                    "no bot of WeCom is set up. Set one up in the settings of the desktop "
                    "application, or with 'columbina wecom --setup'"
                )
                return 2
            if config.writable and not chosen.writable:
                # Its owner took that back after the program chose what to load.
                _tell(
                    "the assistant was started to change files, which its owner does not let it "
                    "do. Start it again"
                )
                return 2
            if config.writable and within(config.cwd, config.home):
                _tell(
                    f"the directory of the assistant, {config.cwd}, lies in the home of the "
                    "program, where it may change no file. Name another with "
                    f"{store.ASSISTANT_HOME}"
                )
                return 2
            try:
                config.cwd.mkdir(parents=True, exist_ok=True)
            except OSError as error:
                _tell(f"the directory of the assistant cannot be used: {error}")
                return 2
            return await self._serve(chosen, place, secret)
        finally:
            held.release()

    async def _serve(self, chosen: store.Settings, place: str, secret: str) -> int:
        config = self._config
        self._settings = chosen
        self._seen = _Seen(config.home / store.FOLDER / SEEN)
        http = self._http = _downloads(os.environ)
        link = self._link = Link(
            place, chosen.bot_id, secret, heard=self._heard, told=self._told, env=os.environ
        )
        if chosen.owner is None:
            self._pair_anew()
        working = asyncio.ensure_future(self._work())
        try:
            try:
                self._agent = await self._opened(chosen.session_id)
            except (SessionError, AgentError, ValueError) as error:
                self._failed(f"the session of the assistant cannot be opened: {error}")
                return 1
            _tell(f"the assistant of the bot {chosen.bot_id} is starting. Ctrl+C stops it")
            try:
                await self._held(link)
            except WecomError as error:
                self._failed(str(error))
                return 1
            return 0
        finally:
            working.cancel()
            await asyncio.gather(working, return_exceptions=True)
            await self._end(http)

    async def _held(self, link: Link) -> None:
        """Keep the link until it fails, or until the program that runs the assistant stops it.

        Raises:
            WecomError: WeCom refused the bot, or another program signed in with it.
        """
        if not self._config.json_lines:
            return await link.run()
        linked = asyncio.ensure_future(link.run())
        closed = asyncio.ensure_future(when_the_input_closes().wait())
        try:
            await asyncio.wait({linked, closed}, return_when=asyncio.FIRST_COMPLETED)
            if closed.done() and not linked.done():
                # The answers being written say so, over the connection that is still open.
                await self._leave()
        finally:
            closed.cancel()
            if not linked.done():
                linked.cancel()
            await asyncio.gather(linked, closed, return_exceptions=True)
        if not linked.cancelled():
            linked.result()
        return None

    async def _leave(self) -> None:
        """Say in the answers that are being written that they will not be: the assistant stops."""
        left = [*(asked for asked, _ in self._turns.values() if asked), *self._waiting]
        if left:
            closing = [
                asyncio.ensure_future(self._closed(asked, self._words.interrupted))
                for asked in left
            ]
            await asyncio.wait(closing, timeout=_LAST_SECONDS)
            for doing in closing:
                doing.cancel()

    async def _end(self, http: httpx.AsyncClient) -> None:
        """End the agent and what is done on the side, and say that the assistant stopped."""
        agent, self._agent = self._agent, None
        if agent is not None:
            try:
                if not agent.at_rest:
                    agent.cancel("user", keep_inbox=True)
                await asyncio.shield(agent.close())
            except Exception as error:
                logger.error("the agent of the assistant stopped badly: %s", error)
        self._waiting.clear()
        self._turns.clear()
        # What is still being sent then is broken off: the connection is gone.
        for doing in list(self._doing):
            doing.cancel()
        if self._doing:
            await asyncio.wait(self._doing)
        self._link = self._http = None
        await http.aclose()
        if self._state != "failed":
            self._state = "stopped"
            self._status()

    def _failed(self, why: str) -> None:
        self._state, self._error = "failed", why
        self._status()
        _tell(why)

    async def _opened(self, session_id: str | None) -> Agent:
        """Open the session of the conversation, and take up what waited in it.

        Raises:
            SessionError: The stored session cannot be read or is open elsewhere.
            AgentError: The agent could not be made.
            ValueError: The id cannot name a session.
        """
        config = self._config
        spec = AgentSpec(cwd=config.cwd, model=config.model, system_prompt=config.system_prompt)
        agent = await open_agent(self._ctx, self._agents, self._sessions, spec, session_id)
        permissions = self._ctx.get(PERMISSIONS)
        if permissions is not None:
            # What its owner lets it do holds for the conversation that goes on as well, whatever
            # its log or the configuration of the program said.
            permissions.set_preset(
                agent.session, "workspace-write" if config.writable else "read-only"
            )
        agent.ctx.on(SESSION_EVENT, self._follower(agent))
        settings = self._settings
        if settings is not None and settings.session_id != agent.id:
            self._settings = settings.model_copy(update={"session_id": agent.id})
            self._save()
        # A message that was taken and not answered before the program ended is answered now.
        agent.wake()
        return agent

    def _save(self) -> str | None:
        """Write down what changed of the bot, and return why it was not, or ``None``."""
        settings = self._settings
        if settings is None:  # pragma: no cover - set before anything is saved
            return None
        home = self._config.home
        try:
            kept = store.settings(home)
            if kept is None or kept.bot_id != settings.bot_id:
                # The bot was deleted, or another one was set up, while the assistant ran. What
                # changed belongs to this bot, and would take the place of the other one.
                logger.warning("the bot changed while the assistant ran, so it is left as it is")
                return "the bot was changed while the assistant ran"
            store.save(home, settings)
        except WecomError as error:
            logger.error("what changed of the bot cannot be written down: %s", error)
            return str(error)
        return None

    # How the assistant is doing

    def _status(self) -> None:
        settings, pairing = self._settings, self._pairing
        paired = settings is not None and settings.owner is not None
        code = None if pairing is None else pairing.code
        store.tell(
            self._config.home, os.getpid(), self._state, paired=paired, code=code, error=self._error
        )
        if not self._config.json_lines:
            return
        line = json.dumps(
            {
                "state": self._state,
                "paired": paired,
                "code": code,
                "error": self._error,
                "bot_id": None if settings is None else settings.bot_id,
            },
            ensure_ascii=False,
        )
        # A program that went away reads nothing more, and the assistant stops all the same.
        try:
            print(line, flush=True)
        except (OSError, ValueError) as error:
            logger.info("how the assistant is doing cannot be said: %s", error)

    def _told(self, state: str, why: str | None) -> None:
        connected = state == "connected"
        if connected and self._state != "connected":
            _tell("connected to WeCom")
        self._state = "connected" if connected else "connecting"
        self._error = why
        self._status()

    def _pair_anew(self) -> None:
        minutes = self._config.pairing_minutes
        code = f"{secrets.randbelow(1_000_000):06d}"
        self._pairing = _Pairing(code, self._now() + minutes * 60, {})
        self._status()
        _tell(
            f"to be the one whom the assistant answers, send its bot this code in WeCom: {code}. "
            f"It holds for {minutes:g} minutes"
        )

    # What comes in

    def _heard(self, frame: dict[str, Any]) -> None:
        """Take a frame that WeCom sent: a message, or an event."""
        body = _object(frame.get("body"))
        req_id = _object(frame.get("headers")).get("req_id")
        if frame.get("cmd") == EVENT:
            event = _object(body.get("event"))
            settings = self._settings
            if (
                event.get("eventtype") == "enter_chat"
                and isinstance(req_id, str)
                and settings is not None
                and settings.owner is None
            ):
                # Somebody opened the chat while the assistant answers nobody yet.
                welcome = {"msgtype": "text", "text": {"content": self._words.pair_please}}
                self._later(self._sent(WELCOME, welcome, req_id, "welcome", tries=1))
            return
        message_id = body.get("msgid")
        seen = self._seen
        if not isinstance(message_id, str) or not message_id or seen is None:
            return
        if message_id in seen:
            logger.info("a message that came again is passed over")
            return
        if not seen.add(message_id):
            # It is taken all the same: a disk that takes nothing would leave the assistant deaf.
            self._error = _NOT_NOTED
            self._status()
        self._queue.put_nowait(frame)

    async def _work(self) -> None:
        """Take the messages one after another, in the order in which they came."""
        while True:
            frame = await self._queue.get()
            try:
                await self._take(frame)
            except Exception as error:
                logger.error("a message from WeCom could not be taken", exc_info=error)

    async def _take(self, frame: dict[str, Any]) -> None:
        body = _object(frame.get("body"))
        req_id = _object(frame.get("headers")).get("req_id")
        # A message in a group, or one that some other part of WeCom passes on, is no chat of two.
        if body.get("chattype") != "single" or "compat_info" in body:
            return
        who, message_id = _object(body.get("from")).get("userid"), body.get("msgid")
        if not isinstance(who, str) or not who or not isinstance(req_id, str) or not req_id:
            return
        said = messages.read(body)
        settings = self._settings
        if settings is None:  # pragma: no cover - set before the first message is taken
            return
        if settings.owner is None:
            await self._pair(who, req_id, said)
        elif who != settings.owner:
            logger.info("a message from someone other than the owner is passed over")
        elif isinstance(said, messages.Text):
            text = said.text.strip()
            if text.startswith("/"):
                await self._command(text, req_id)
            elif text:
                self._ask(said.asked(), req_id)
        elif isinstance(said, messages.File):
            await self._file(said, req_id, str(message_id))
        else:
            self._say(req_id, self._words.unread)

    async def _pair(self, who: str, req_id: str, said: messages.Said) -> None:
        pairing = self._pairing
        if pairing is None:
            return
        if pairing.until < self._now():
            self._pair_anew()
            pairing = self._pairing
            if pairing is None:  # pragma: no cover - set a moment ago
                return
        given = said.text.strip() if isinstance(said, messages.Text) else ""
        settings = self._settings
        if given == pairing.code and settings is not None:
            self._settings = settings.model_copy(update={"owner": who})
            why = self._save()
            if why is not None:
                # Whom the assistant answers would be forgotten at the next start.
                self._settings = settings
                self._say(req_id, self._words.pair_not_kept.format(reason=why))
                return
            self._pairing = None
            self._status()
            _tell("the assistant is paired, and answers the one who sent the code")
            self._say(req_id, f"{self._words.paired}\n\n{self._words.help}")
            return
        tries = pairing.tries[who] = pairing.tries.get(who, 0) + 1
        pairing.total += 1
        if tries >= _TRIES_EACH or pairing.total >= _TRIES_ALL:
            self._pairing = None
            self._error = (
                "the code of pairing was given up on after too many wrong tries. Start the "
                "assistant again for a new one"
            )
            self._status()
            _tell(self._error)
        elif tries == 1:
            self._say(req_id, self._words.pair_please)

    def _ask(self, text: str, req_id: str) -> None:
        """Give the agent a message, which it answers in a turn of its own."""
        agent = self._agent
        if agent is None:
            # A new session could not be opened: the owner is told how to try again.
            self._say(req_id, self._words.no_session)
            return
        asked = _Asked(req_id, request_id("stream"))
        self._waiting.append(asked)
        agent.followup(text)
        asked.beginning = self._later(self._begin(asked))

    # The commands

    async def _command(self, text: str, req_id: str) -> None:
        command, _, rest = text.partition(" ")
        command, rest = command.lower(), rest.strip()
        words = self._words
        if command == "/new":
            await self._new(req_id)
        elif command == "/stop":
            await self._stop(req_id)
        elif command == "/skills" and (rest == "rm" or rest.startswith("rm ")):
            await self._remove(rest[2:].strip(), req_id)
        elif command == "/skills":
            await self._list(req_id)
        elif command == "/help":
            self._say(req_id, words.help)
        else:
            self._say(req_id, words.unknown_command)

    def _undone(self) -> list[_Asked]:
        """Forget the messages that the agent is answering or will answer, and return them."""
        undone = [*(asked for asked, _ in self._turns.values() if asked), *self._waiting]
        self._turns.clear()
        self._waiting.clear()
        return undone

    async def _new(self, req_id: str) -> None:
        old = self._agent
        self._agent = None
        dropped = self._undone()
        words = self._words
        if old is not None:
            if not old.at_rest:
                old.cancel("user")
            await asyncio.shield(old.close())
        for asked in dropped:
            self._later(self._closed(asked, words.dropped_here))
        try:
            self._agent = await self._opened(None)
        except (SessionError, AgentError, ValueError) as error:
            # Without a session the assistant answers nothing: the owner is told, and may try again.
            logger.error("a new session of the assistant cannot be opened: %s", error)
            self._say(req_id, words.new_failed.format(reason=error))
            return
        said = words.new_dropped.format(count=len(dropped)) if dropped else words.new
        self._say(req_id, said)

    async def _stop(self, req_id: str) -> None:
        agent = self._agent
        words = self._words
        waiting = list(self._waiting)
        if agent is None or (agent.at_rest and not waiting):
            self._say(req_id, words.nothing_to_stop)
            return
        self._waiting.clear()
        # Input that waits is dropped with the turn. The answer of the turn says that it stopped.
        agent.cancel("user")
        for asked in waiting:
            self._later(self._closed(asked, words.dropped_here))
        said = words.stopped_dropped.format(count=len(waiting)) if waiting else words.stopped
        self._say(req_id, said)

    def _skills(self) -> Path:
        return self._config.cwd / SKILLS_DIRECTORY

    async def _list(self, req_id: str) -> None:
        skills, agent = self._ctx.get(SKILLS), self._agent
        found = (
            None
            if skills is None or agent is None
            else await skills.found(agent.session, self._config.cwd)
        )
        listed = [] if found is None else list(found.skills.values())
        words = self._words
        if not listed:
            self._say(req_id, words.no_skills)
            return
        lines = "\n".join(f"- {skill.name}: {one_line(skill.description, 160)}" for skill in listed)
        self._say(req_id, words.skills.format(lines=lines))

    async def _remove(self, named: str, req_id: str) -> None:
        words = self._words
        directory = self._skills() / named
        if _NAME.fullmatch(named) is None or not (directory / FILE).is_file():
            self._say(req_id, words.no_such_skill.format(name=named or "?"))
            return
        if [entry.name for entry in directory.iterdir()] != [FILE]:
            self._say(req_id, words.skill_not_removable.format(name=named, path=directory))
            return
        (directory / FILE).unlink()
        directory.rmdir()
        self._say(req_id, words.skill_removed.format(name=named))

    # Files and skills

    async def _file(self, sent: messages.File, req_id: str, message_id: str) -> None:
        words, http = self._words, self._http
        if http is None:  # pragma: no cover - while connected
            return
        try:
            got = await media.fetched(http, sent.url, most=MAX_BYTES)
            content = media.decrypted(got.content, sent.key)
        except WecomError as error:
            too_large = error.code == "TOO_LARGE"
            self._say(
                req_id,
                words.too_large.format(most="1 MB")
                if too_large
                else words.file_failed.format(reason=error),
            )
            return
        if len(content) > MAX_BYTES:
            self._say(req_id, words.too_large.format(most="1 MB"))
            return
        if got.name is None:
            # WeCom did not name it: a skill and a file of text are known by what they hold.
            if _a_skill(content):
                await self._install(content, req_id)
                return
            if not _text_like(content):
                self._say(req_id, words.unread)
                return
        named = _UNNAMED if got.name is None else _safe(got.name)
        suffix = Path(named).suffix.lower()
        if suffix not in _TEXT:
            self._say(req_id, words.unread)
            return
        if suffix in (".md", ".markdown") and (named == FILE or _a_skill(content)):
            await self._install(content, req_id)
            return
        place = self._config.cwd / UPLOADS / _safe(message_id) / named
        try:
            files.replaced(place, content)
        except OSError as error:
            self._say(req_id, words.file_failed.format(reason=error.strerror))
            return
        where = place.relative_to(self._config.cwd).as_posix()
        self._ask(
            f"[I sent you the file {where}, in your working directory. Read it with the tool "
            "read when you need it.]",
            req_id,
        )

    async def _install(self, content: bytes, req_id: str) -> None:
        words = self._words
        try:
            fields, _ = frontmatter.read(content, FILE)
            named = frontmatter.name(fields)
            parsed(content, named)
        except (frontmatter.FrontMatterError, SkillError) as error:
            self._say(req_id, words.not_a_skill.format(reason=error))
            return
        directory = self._skills() / named
        target = directory / FILE
        there = target.exists()
        others = sorted(entry.name for entry in directory.iterdir()) if directory.is_dir() else []
        try:
            files.replaced(target, content)
        except OSError as error:
            self._say(req_id, words.file_failed.format(reason=error.strerror))
            return
        said = (words.skill_replaced if there else words.skill_installed).format(name=named)
        if [other for other in others if other != FILE]:
            said = f"{said}\n\n{words.skill_left_alone.format(path=directory)}"
        self._say(req_id, said)

    # What goes out

    def _later(self, work: Coroutine[Any, Any, Any]) -> asyncio.Future[Any]:
        """Do something on the side, and have it ended with the assistant."""
        doing = asyncio.ensure_future(work)
        self._doing.add(doing)
        doing.add_done_callback(self._doing.discard)
        return doing

    def _follower(self, agent: Agent) -> Callable[[Committed], None]:
        """Return what follows the turns of an agent, while it is the one of the conversation."""

        def follow(committed: Committed) -> None:
            if agent is not self._agent or committed.session is not agent.session:
                return
            data = committed.event.data
            if isinstance(data, TurnStartEvent):
                asked = self._waiting.popleft() if self._waiting else None
                self._turns[data.turn] = (asked, len(agent.session.events))
            elif isinstance(data, TurnEndEvent):
                asked, began = self._turns.pop(data.turn, (None, len(agent.session.events)))
                said = replies(event.data for event in agent.session.events[began:])
                self._later(self._answered(asked, data, said[-1] if said else ""))

        return follow

    async def _answered(self, asked: _Asked | None, end: TurnEndEvent, text: str) -> None:
        """Send the answer of a turn, with how the turn ended where it did not end well."""
        words = self._words
        if end.reason == "aborted":
            # The turn was stopped, and whoever stopped it said so. Its answer says it too.
            if asked is not None:
                await self._closed(asked, words.stopped_here)
            return
        detail = end.detail or end.code or end.reason
        notice = {
            "error": words.failed.format(detail=detail),
            "blocked": words.blocked.format(detail=detail),
            "max-tokens": words.cut_off,
            "interrupted": words.interrupted,
        }.get(end.reason)
        said = "\n\n".join(part for part in (text.strip(), notice) if part) or words.empty
        pieces = parts(said, _MOST)
        if asked is not None and await self._closed(asked, pieces[0]):
            pieces = pieces[1:]
        for piece in pieces:
            await self._pushed(piece)

    async def _begin(self, asked: _Asked) -> None:
        """Begin the answer to a message: say in it that the agent is thinking."""
        asked.began = self._clock()
        thinking = _stream(asked, self._words.thinking, finish=False)
        if await self._sent(RESPOND, thinking, asked.req_id, "thinking"):
            asked.expiring = self._later(self._expiring(asked))

    async def _expiring(self, asked: _Asked) -> None:
        """End an answer that the agent takes longer for than it may be written."""
        await self._pause(_STREAM_SECONDS)
        asked.expiring = None
        await self._closed(asked, self._words.later)

    async def _closed(self, asked: _Asked, text: str) -> bool:
        """End the answer to a message with a text, once, and say whether WeCom took it.

        An answer that was ended, or that may be written no longer, takes nothing more.
        """
        if asked.closed:
            return False
        asked.closed = True
        # An answer is begun as soon as its message is given to the agent.
        if asked.beginning is not None:  # pragma: no branch
            await asyncio.gather(asked.beginning, return_exceptions=True)
        if asked.expiring is not None:
            asked.expiring.cancel()
        if asked.began is not None and self._clock() - asked.began >= _STREAM_SECONDS:
            return False
        return await self._sent(RESPOND, _stream(asked, text, finish=True), asked.req_id, "answer")

    async def _pushed(self, text: str) -> None:
        """Send the owner a message that answers no message of the moment."""
        settings = self._settings
        if settings is None or settings.owner is None:  # pragma: no cover - paired by then
            return
        body = {"chatid": settings.owner, "chat_type": 1, **_markdown(text)}
        await self._sent(SEND, body, None, "message")

    def _say(self, req_id: str, said: str) -> None:
        """Answer a message on the side, so that what comes after it is taken meanwhile."""
        self._later(self._said(req_id, said))

    async def _said(self, req_id: str, said: str) -> None:
        first, *rest = parts(said, _MOST)
        await self._sent(RESPOND, _markdown(first), req_id, "reply")
        for piece in rest:
            await self._pushed(piece)

    async def _sent(
        self,
        cmd: str,
        body: Mapping[str, Any],
        req_id: str | None,
        what: str,
        *,
        tries: int = len(_PAUSES) + 1,
    ) -> bool:
        """Send a frame, again after a pause where WeCom could not take it for now.

        Returns:
            Whether WeCom took it.
        """
        for attempt in range(tries):
            link = self._link
            if link is None:
                return False
            try:
                await link.call(cmd, body, req_id)
            except WecomError as error:
                passing = error.code == "UNREACHABLE" or error.number == _TOO_OFTEN
                if passing and attempt + 1 < tries:
                    await self._pause(_PAUSES[attempt])
                    continue
                logger.error("%s (%s): %s", _SEND_FAILED, what, error)
                self._error = f"{_SEND_FAILED}: {error}"
                self._status()
                return False
            # An answer went out, so the last one that could not is no trouble any more.
            if self._error is not None and self._error.startswith(_SEND_FAILED):
                self._error = None
                self._status()
            return True
        return False  # pragma: no cover - no try is made only where none is asked for


def _downloads(env: Mapping[str, str]) -> httpx.AsyncClient:
    """Return the client that the files that are sent to the bot are downloaded with."""
    return httpx.AsyncClient(
        timeout=httpx.Timeout(10.0, read=60.0),
        headers={"User-Agent": f"Columbina/{columbina.__version__}"},
        proxy=proxy_for("https://work.weixin.qq.com", env),
    )


def _a_skill(content: bytes) -> bool:
    """Tell whether a file of Markdown begins as a skill does: with a name and what it is for."""
    try:
        fields, _ = frontmatter.read(content, FILE)
        frontmatter.name(fields)
        return bool(frontmatter.text(fields, "description"))
    except frontmatter.FrontMatterError:
        return False


def apply(ctx: Context, config: Config) -> None:
    """Provide the assistant in WeCom."""
    ctx.provide(FRONTEND, Assistant(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS)))
