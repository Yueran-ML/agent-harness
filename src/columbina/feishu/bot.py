"""The assistant in Feishu: a conversation with the person who owns the app, in a chat of Feishu.

This module is a plugin. It provides the frontend of ``columbina feishu``, and the tool that
reads documents of Feishu, :mod:`columbina.feishu.documents`.

The assistant keeps the long connection of the app, :mod:`columbina.feishu.link`, and hears
every message that is written to the bot. It answers one person, its owner, in a chat of
the two. Whoever sends the code of pairing first becomes the owner. The code is shown on the
computer that the assistant runs on, holds for some minutes, and is given up after too many
wrong tries. What anybody else writes is passed over.

What the owner writes goes to an agent, in a session of its own that is taken up again when
the assistant starts again. Each message is a turn of its own, and the answer of the turn is
a reply to that message. While the agent works on it, the message shows that the bot is
writing. A few things are done by the assistant itself: ``/new`` begins a new session,
``/stop`` stops the answer, ``/skills`` lists the skills and deletes one, ``/help`` says what
there is. A file of text that the owner sends is kept in the directory ``uploads`` and the
agent is told of it. A ``SKILL.md`` is a skill, which goes into the directory of skills of the
assistant.

Feishu sends a message again where it did not hear that it was taken, and a message that was
taken may still come twice. The messages that were taken are written down before Feishu is
told so, and one that comes again is passed over, also after the assistant started again.

Another program, such as the desktop application, may run the assistant. It is then told
how the assistant is doing in lines of JSON, and stops it by closing the standard input.

The agent may read, and not change anything or run anything: the profile gives it no such
tool, and nobody is there to say yes to one.
"""

import asyncio
import hashlib
import importlib.util
import json
import logging
import os
import re
import secrets
import sys
import time
from collections import deque
from collections.abc import Callable, Coroutine
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from pydantic import BaseModel, Field

from columbina import files, frontmatter
from columbina.agent import AGENTS, Agent, AgentError, Agents, AgentSpec, replies
from columbina.apps import FRONTEND
from columbina.apps.common import open_agent, when_the_input_closes
from columbina.apps.terminal_text import harmless
from columbina.feishu import messages, store
from columbina.feishu.app import FeishuError, Host, host
from columbina.feishu.client import AGAIN, Feishu, field
from columbina.feishu.documents import read_tool
from columbina.feishu.link import Link
from columbina.feishu.words import Language, words_for
from columbina.kernel import Context
from columbina.llm import CallConfig
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
from columbina.tools import TOOLS

__all__ = ["PROMPT", "Assistant", "Config", "apply", "inject", "name"]

name = "feishu"
inject = (AGENTS, SESSIONS, TOOLS)

logger = logging.getLogger(__name__)

PROMPT = (
    "You are Columbina, a personal assistant. You talk with your user in a chat of Feishu, or "
    "of Lark, which is Feishu outside China. Answer the way one does in a chat: to the point, "
    "and at length only where the user asks for it. What you write is shown as Markdown. The "
    "user can give you links to documents of Feishu, and you read them with the tool "
    "feishu_read. A file that the user sends you lies in the directory uploads, and you are "
    "told where."
)
"""The words that open a conversation of the assistant."""

SEEN = "seen.json"
"""The file in the directory of the assistant that names the messages that were taken."""

UPLOADS = "uploads"
"""The directory, in the directory of the assistant, of the files that the owner sent."""

SKILLS_DIRECTORY = "skills"
"""The directory of skills of the assistant, in its directory."""

_SEEN_MOST = 500
"""How many messages that were taken are remembered."""

_LAST_SECONDS = 3.0
"""How long the assistant takes at most, when it stops, to stop showing that it writes."""

_TRIES = 3
"""How many times an answer is sent at most, where Feishu asked for a pause or was not reached."""

_TRIES_EACH = 5
_TRIES_ALL = 20
"""How many wrong codes of pairing one sender may send, and all senders together. The last of
them gives the code up until the assistant starts again."""

_SEND_FAILED = "an answer could not be sent to Feishu"
"""How what went wrong begins where an answer could not be sent. It holds until one is."""

_NOT_NOTED = (
    "which messages were taken cannot be written down, so one that Feishu sends again after a "
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

_UNSAFE = re.compile(r'[\x00-\x1f\x7f<>:"/\\|?*]')
"""What a name of a file may not hold on one system or another."""

_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
"""What the name of a skill is made of."""


class Config(BaseModel):
    """The config of the assistant in Feishu.

    Attributes:
        model: The model that the agent talks to, and how.
        cwd: The directory of the assistant: the working directory of the agent, with its
            instruction file, its skills and the files that it was sent.
        home: The home of the program, where the app is kept.
        system_prompt: The instructions that open a conversation.
        language: The language of what the assistant says by itself. Without one, Chinese
            for an app in Feishu and English for one in Lark.
        pairing_minutes: How long a code of pairing holds.
        json_lines: Whether another program runs the assistant, such as the desktop
            application. It is then told how the assistant is doing in lines of JSON on the
            standard output, and stops it by closing the standard input.
    """

    model: CallConfig
    cwd: Path
    home: Path
    system_prompt: str = PROMPT
    language: Language | None = None
    pairing_minutes: float = Field(default=10.0, gt=0)
    json_lines: bool = False


def _tell(message: str) -> None:
    """Say something to whoever started the assistant."""
    print(harmless(f"columbina: {message}"), file=sys.stderr)


def _uuid(*parts: str) -> str:
    """Return the key by which Feishu tells a message apart from a repetition of it."""
    return hashlib.sha256("\x00".join(parts).encode()).hexdigest()[:32]


def _safe(name: str) -> str:
    """Return the name of a file that was sent as one that may be written on any system."""
    last = name.replace("\\", "/").rsplit("/", 1)[-1]
    cleaned = _UNSAFE.sub("_", last).strip(" .")
    return (cleaned or "file")[-120:]


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
        message_id: The message.
        reacting: The work of showing on it that the bot is writing.
        reaction: What shows it, once it is shown.
    """

    message_id: str
    reacting: asyncio.Future[None] | None = None
    reaction: str | None = None


class Assistant:
    """Holds the conversation with the owner of the app, in Feishu."""

    def __init__(self, ctx: Context, config: Config, agents: Agents, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions
        self._now: Callable[[], float] = time.time
        self._words = words_for(config.language or "zh")
        self._client: Feishu | None = None
        self._settings: store.Settings | None = None
        self._agent: Agent | None = None
        self._seen: _Seen | None = None
        self._pairing: _Pairing | None = None
        self._queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
        self._waiting: deque[_Asked] = deque()
        self._turns: dict[int, tuple[_Asked | None, int]] = {}
        self._doing: set[asyncio.Future[None]] = set()
        self._state: store.State = "connecting"
        self._error: str | None = None
        self._reacting = True

    def client(self) -> Feishu | None:
        """The client of the app while the assistant is connected, for the tool that reads."""
        return self._client

    # Starting and ending

    async def run(self) -> int:
        """Hold the conversation until the program is stopped, or Feishu refuses the app.

        Returns:
            1 where Feishu refused the app or the session could not be opened, and 2 where
            nothing that the assistant needs is there.
        """
        config = self._config
        if importlib.util.find_spec("websockets") is None:
            _tell(
                "the assistant in Feishu needs the package websockets, which comes with "
                "'pip install columbina[feishu]'"
            )
            return 2
        try:
            config.cwd.mkdir(parents=True, exist_ok=True)
            folder = config.home / store.FOLDER
            folder.mkdir(parents=True, exist_ok=True)
            held = acquire(folder / store.LOCK)
        except OSError as error:
            _tell(f"the directory of the assistant cannot be used: {error}")
            return 2
        if held is None:
            _tell("an assistant in Feishu runs on this machine already, and only one may run")
            return 2
        try:
            # The app is read under the lock: it is set up or deleted only while nobody holds it.
            try:
                chosen = store.settings(config.home)
                where = None if chosen is None else host(chosen.domain, os.environ)
                secret = None if where is None else store.secret(config.home, where)
            except FeishuError as error:
                _tell(str(error))
                return 2
            if chosen is None or where is None or secret is None:
                _tell(
                    "no app of Feishu is set up. Set one up in the settings of the desktop "
                    "application, or with 'columbina feishu --setup'"
                )
                return 2
            return await self._serve(chosen, where, secret)
        finally:
            held.release()

    async def _serve(self, chosen: store.Settings, where: Host, secret: str) -> int:
        config = self._config
        self._settings = chosen
        self._words = words_for(config.language or ("zh" if chosen.domain == "feishu" else "en"))
        self._seen = _Seen(config.cwd / SEEN)
        client = self._client = Feishu(where, chosen.app_id, secret, os.environ)
        if chosen.owner is None:
            self._pair_anew()
        working = asyncio.ensure_future(self._work())
        try:
            try:
                self._agent = await self._opened(chosen.session_id)
            except (SessionError, AgentError, ValueError) as error:
                self._failed(f"the session of the assistant cannot be opened: {error}")
                return 1
            link = Link(
                where,
                chosen.app_id,
                secret,
                client.http,
                heard=self._heard,
                told=self._told,
                env=os.environ,
            )
            _tell(f"the assistant of the app {chosen.app_id} is starting. Ctrl+C stops it")
            try:
                await self._held(link)
            except FeishuError as error:
                self._failed(str(error))
                return 1
            return 0
        finally:
            working.cancel()
            await asyncio.gather(working, return_exceptions=True)
            await self._end(client)

    async def _held(self, link: Link) -> None:
        """Keep the link until it fails, or until the program that runs the assistant stops it.

        Raises:
            FeishuError: Feishu refused the app.
        """
        if not self._config.json_lines:
            return await link.run()
        linked = asyncio.ensure_future(link.run())
        closed = asyncio.ensure_future(when_the_input_closes().wait())
        try:
            await asyncio.wait({linked, closed}, return_when=asyncio.FIRST_COMPLETED)
        finally:
            closed.cancel()
            if not linked.done():
                linked.cancel()
            await asyncio.gather(linked, closed, return_exceptions=True)
        if not linked.cancelled():
            linked.result()
        return None

    async def _end(self, client: Feishu) -> None:
        """End the agent and what is done on the side, and say that the assistant stopped."""
        agent, self._agent = self._agent, None
        if agent is not None:
            try:
                if not agent.at_rest:
                    agent.cancel("user", keep_inbox=True)
                await asyncio.shield(agent.close())
            except Exception as error:
                logger.error("the agent of the assistant stopped badly: %s", error)
        left = [*self._waiting, *(asked for asked, _ in self._turns.values() if asked)]
        self._waiting.clear()
        self._turns.clear()
        if left:
            # What the agent was working on will not be answered now: the bot stops showing it.
            await asyncio.wait(
                [asyncio.ensure_future(self._unreact(asked)) for asked in left],
                timeout=_LAST_SECONDS,
            )
        # What is still being sent then is broken off.
        for doing in list(self._doing):
            doing.cancel()
        if self._doing:
            await asyncio.wait(self._doing)
        self._client = None
        await client.aclose()
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
        agent.ctx.on(SESSION_EVENT, self._follower(agent))
        settings = self._settings
        if settings is not None and settings.session_id != agent.id:
            self._settings = settings.model_copy(update={"session_id": agent.id})
            self._save()
        # A message that was taken and not answered before the program ended is answered now.
        agent.wake()
        return agent

    def _save(self) -> str | None:
        """Write down what changed of the app, and return why it was not, or ``None``."""
        settings = self._settings
        if settings is None:  # pragma: no cover - set before anything is saved
            return None
        home = self._config.home
        try:
            kept = store.settings(home)
            if kept is None or (kept.domain, kept.app_id) != (settings.domain, settings.app_id):
                # The app was deleted, or another one was set up, while the assistant ran. What
                # changed belongs to this app, and would take the place of the other one.
                logger.warning("the app changed while the assistant ran, so it is left as it is")
                return "the app was changed while the assistant ran"
            store.save(home, settings)
        except FeishuError as error:
            logger.error("what changed of the app cannot be written down: %s", error)
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
                "domain": None if settings is None else settings.domain,
                "app_id": None if settings is None else settings.app_id,
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
            _tell("connected to Feishu")
        self._state = "connected" if connected else "connecting"
        self._error = why
        self._status()

    def _pair_anew(self) -> None:
        minutes = self._config.pairing_minutes
        code = f"{secrets.randbelow(1_000_000):06d}"
        self._pairing = _Pairing(code, self._now() + minutes * 60, {})
        self._status()
        _tell(
            f"to be the one whom the assistant answers, send its bot this code in Feishu: {code}. "
            f"It holds for {minutes:g} minutes"
        )

    # What comes in

    def _heard(self, event: dict[str, Any]) -> None:
        """Take an event that Feishu sent, before Feishu is told that it was taken."""
        if field(event, "header", "event_type") != "im.message.receive_v1":
            return
        message_id = field(event, "event", "message", "message_id")
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
        self._queue.put_nowait(event)

    async def _work(self) -> None:
        """Take the messages one after another, in the order in which they came."""
        while True:
            event = await self._queue.get()
            try:
                await self._take(event)
            except Exception as error:
                logger.error("a message from Feishu could not be taken", exc_info=error)

    async def _take(self, event: dict[str, Any]) -> None:
        message, sender = field(event, "event", "message"), field(event, "event", "sender")
        if field(message, "chat_type") != "p2p" or field(sender, "sender_type") != "user":
            return
        who, message_id = field(sender, "sender_id", "open_id"), field(message, "message_id")
        kind, content = field(message, "message_type"), field(message, "content")
        if not isinstance(who, str) or not who or not isinstance(message_id, str):
            return
        said = messages.read(
            kind if isinstance(kind, str) else "", content if isinstance(content, str) else ""
        )
        settings = self._settings
        if settings is None:  # pragma: no cover - set before the first message is taken
            return
        if settings.owner is None:
            await self._pair(who, message_id, said)
        elif who != settings.owner:
            logger.info("a message from someone other than the owner is passed over")
        elif isinstance(said, messages.Text):
            text = said.text.strip()
            if text.startswith("/"):
                await self._command(text, message_id)
            elif text:
                self._ask(text, message_id)
        elif isinstance(said, messages.File):
            await self._file(said, message_id)
        else:
            self._say(message_id, self._words.unread)

    async def _pair(self, who: str, message_id: str, said: messages.Said) -> None:
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
                self._say(message_id, self._words.pair_not_kept.format(reason=why))
                return
            self._pairing = None
            self._status()
            _tell("the assistant is paired, and answers the one who sent the code")
            self._say(message_id, f"{self._words.paired}\n\n{self._words.help}")
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
            self._say(message_id, self._words.pair_please)

    def _ask(self, text: str, message_id: str) -> None:
        """Give the agent a message, which it answers in a turn of its own."""
        agent = self._agent
        if agent is None:
            # A new session could not be opened: the owner is told how to try again.
            self._say(message_id, self._words.no_session)
            return
        asked = _Asked(message_id)
        self._waiting.append(asked)
        agent.followup(text)
        asked.reacting = self._later(self._react(asked))

    # The commands

    async def _command(self, text: str, message_id: str) -> None:
        command, _, rest = text.partition(" ")
        command, rest = command.lower(), rest.strip()
        words = self._words
        if command == "/new":
            await self._new(message_id)
        elif command == "/stop":
            await self._stop(message_id)
        elif command == "/skills" and (rest == "rm" or rest.startswith("rm ")):
            await self._remove(rest[2:].strip(), message_id)
        elif command == "/skills":
            await self._list(message_id)
        elif command == "/help":
            self._say(message_id, words.help)
        else:
            self._say(message_id, words.unknown_command)

    def _undone(self) -> list[_Asked]:
        """Forget the messages that the agent is answering or will answer, and return them."""
        undone = [*(asked for asked, _ in self._turns.values() if asked), *self._waiting]
        self._turns.clear()
        self._waiting.clear()
        return undone

    async def _new(self, message_id: str) -> None:
        old = self._agent
        self._agent = None
        dropped = self._undone()
        words = self._words
        if old is not None:
            if not old.at_rest:
                old.cancel("user")
            await asyncio.shield(old.close())
        for asked in dropped:
            self._later(self._unreact(asked))
        try:
            self._agent = await self._opened(None)
        except (SessionError, AgentError, ValueError) as error:
            # Without a session the assistant answers nothing: the owner is told, and may try again.
            logger.error("a new session of the assistant cannot be opened: %s", error)
            self._say(message_id, words.new_failed.format(reason=error))
            return
        said = words.new_dropped.format(count=len(dropped)) if dropped else words.new
        self._say(message_id, said)

    async def _stop(self, message_id: str) -> None:
        agent = self._agent
        words = self._words
        waiting = list(self._waiting)
        if agent is None or (agent.at_rest and not waiting):
            self._say(message_id, words.nothing_to_stop)
            return
        self._waiting.clear()
        # Input that waits is dropped with the turn. The turn ends as stopped, which says no more.
        agent.cancel("user")
        for asked in waiting:
            self._later(self._unreact(asked))
        said = words.stopped_dropped.format(count=len(waiting)) if waiting else words.stopped
        self._say(message_id, said)

    def _skills(self) -> Path:
        return self._config.cwd / SKILLS_DIRECTORY

    async def _list(self, message_id: str) -> None:
        skills, agent = self._ctx.get(SKILLS), self._agent
        found = (
            None
            if skills is None or agent is None
            else await skills.found(agent.session, self._config.cwd)
        )
        listed = [] if found is None else list(found.skills.values())
        words = self._words
        if not listed:
            self._say(message_id, words.no_skills)
            return
        lines = "\n".join(f"- {skill.name}: {one_line(skill.description, 160)}" for skill in listed)
        self._say(message_id, words.skills.format(lines=lines))

    async def _remove(self, named: str, message_id: str) -> None:
        words = self._words
        directory = self._skills() / named
        if _NAME.fullmatch(named) is None or not (directory / FILE).is_file():
            self._say(message_id, words.no_such_skill.format(name=named or "?"))
            return
        if [entry.name for entry in directory.iterdir()] != [FILE]:
            self._say(message_id, words.skill_not_removable.format(name=named, path=directory))
            return
        (directory / FILE).unlink()
        directory.rmdir()
        self._say(message_id, words.skill_removed.format(name=named))

    # Files and skills

    async def _file(self, sent: messages.File, message_id: str) -> None:
        words, client = self._words, self._client
        named = _safe(sent.name)
        suffix = Path(named).suffix.lower()
        if suffix not in _TEXT or client is None:
            self._say(message_id, words.unread)
            return
        try:
            content = await client.bytes_of(
                f"/open-apis/im/v1/messages/{message_id}/resources/{sent.key}",
                {"type": "file"},
                most=MAX_BYTES,
            )
        except FeishuError as error:
            said = (
                words.too_large.format(most="1 MB")
                if error.code == "TOO_LARGE"
                else words.file_failed.format(reason=error)
            )
            self._say(message_id, said)
            return
        if suffix in (".md", ".markdown") and (named == FILE or _a_skill(content)):
            await self._install(content, message_id)
            return
        place = self._config.cwd / UPLOADS / message_id / named
        try:
            files.replaced(place, content)
        except OSError as error:
            self._say(message_id, words.file_failed.format(reason=error.strerror))
            return
        where = place.relative_to(self._config.cwd).as_posix()
        self._ask(
            f"[I sent you the file {where}, in your working directory. Read it with the tool "
            "read when you need it.]",
            message_id,
        )

    async def _install(self, content: bytes, message_id: str) -> None:
        words = self._words
        try:
            fields, _ = frontmatter.read(content, FILE)
            named = frontmatter.name(fields)
            parsed(content, named)
        except (frontmatter.FrontMatterError, SkillError) as error:
            self._say(message_id, words.not_a_skill.format(reason=error))
            return
        directory = self._skills() / named
        target = directory / FILE
        there = target.exists()
        others = sorted(entry.name for entry in directory.iterdir()) if directory.is_dir() else []
        try:
            files.replaced(target, content)
        except OSError as error:
            self._say(message_id, words.file_failed.format(reason=error.strerror))
            return
        said = (words.skill_replaced if there else words.skill_installed).format(name=named)
        if [other for other in others if other != FILE]:
            said = f"{said}\n\n{words.skill_left_alone.format(path=directory)}"
        self._say(message_id, said)

    # What goes out

    def _later(self, work: Coroutine[Any, Any, None]) -> asyncio.Future[None]:
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
        try:
            if end.reason == "aborted":
                # The turn was stopped, and whoever stopped it said so.
                return
            words = self._words
            detail = end.detail or end.code or end.reason
            notice = {
                "error": words.failed.format(detail=detail),
                "blocked": words.blocked.format(detail=detail),
                "max-tokens": words.cut_off,
                "interrupted": words.interrupted,
            }.get(end.reason)
            said = "\n\n".join(part for part in (text.strip(), notice) if part) or words.empty
            await self._sent(said, asked.message_id if asked else None, f"turn {end.turn}")
        finally:
            if asked is not None:
                await self._unreact(asked)

    async def _reply(self, message_id: str, said: str) -> None:
        await self._sent(said, message_id, "reply")

    def _say(self, message_id: str, said: str) -> None:
        """Answer a message on the side, so that what comes after it is taken meanwhile.

        A stop that comes while an answer waits for Feishu, which may ask for a pause, is then
        taken at once.
        """
        self._later(self._reply(message_id, said))

    async def _sent(self, markdown: str, message_id: str | None, what: str) -> None:
        """Send Markdown in as many messages as it takes: as replies to a message, or to the owner.

        Each part has a key of its own that stays the same when it is sent again, so Feishu
        does not show a part twice that it took the first time.
        """
        client, settings = self._client, self._settings
        if client is None or settings is None:  # pragma: no cover - while connected
            return
        session = settings.session_id or ""
        for index, part in enumerate(messages.parts(markdown)):
            body: dict[str, object] = {
                "content": messages.posted(part, self._words.post),
                "msg_type": "post",
                "uuid": _uuid(message_id or session, what, str(index)),
            }
            try:
                if message_id is not None:
                    path = f"/open-apis/im/v1/messages/{message_id}/reply"
                    await client.call("POST", path, body=body, tries=_TRIES)
                else:
                    body["receive_id"] = settings.owner
                    await client.call(
                        "POST",
                        "/open-apis/im/v1/messages",
                        params={"receive_id_type": "open_id"},
                        body=body,
                        tries=_TRIES,
                    )
            except FeishuError as error:
                logger.error("%s: %s", _SEND_FAILED, error)
                self._error = f"{_SEND_FAILED}: {error}"
                self._status()
                return
        # An answer went out, so the last one that could not is no trouble any more.
        if self._error is not None and self._error.startswith(_SEND_FAILED):
            self._error = None
            self._status()

    async def _react(self, asked: _Asked) -> None:
        """Show on a message that the bot is writing an answer to it."""
        client = self._client
        if not self._reacting or client is None:
            return
        path = f"/open-apis/im/v1/messages/{asked.message_id}/reactions"
        try:
            data = await client.call("POST", path, body={"reaction_type": {"emoji_type": "Typing"}})
        except FeishuError as error:
            if error.code not in AGAIN:
                # The app may not do it. The answers come all the same, without that.
                self._reacting = False
            logger.warning("the bot cannot show that it is writing: %s", error)
            return
        reaction = data.get("reaction_id")
        asked.reaction = reaction if isinstance(reaction, str) else None

    async def _unreact(self, asked: _Asked) -> None:
        """Stop showing on a message that the bot is writing an answer to it."""
        # A message is shown to be answered from the moment that it is given to the agent.
        if asked.reacting is not None:  # pragma: no branch
            await asyncio.gather(asked.reacting, return_exceptions=True)
        reaction, asked.reaction = asked.reaction, None
        client = self._client
        if reaction is None or client is None:
            return
        path = f"/open-apis/im/v1/messages/{asked.message_id}/reactions/{reaction}"
        try:
            await client.call("DELETE", path)
        except FeishuError as error:
            logger.warning("the bot cannot stop showing that it is writing: %s", error)


def _a_skill(content: bytes) -> bool:
    """Tell whether a file of Markdown begins as a skill does: with a name and what it is for."""
    try:
        fields, _ = frontmatter.read(content, FILE)
        frontmatter.name(fields)
        return bool(frontmatter.text(fields, "description"))
    except frontmatter.FrontMatterError:
        return False


def apply(ctx: Context, config: Config) -> None:
    """Provide the assistant in Feishu, and the tool with which its agent reads documents."""
    assistant = Assistant(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS))
    ctx.use(TOOLS).add(ctx, read_tool(assistant.client))
    ctx.provide(FRONTEND, assistant)
