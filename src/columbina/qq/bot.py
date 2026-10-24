"""The assistant in QQ: a conversation with the owner of the bot, and one in each group.

This module is a plugin. It provides the frontend of ``columbina qq``.

The assistant keeps the gateway of the bot, :mod:`columbina.qq.link`, and hears what is
written to it. In a chat of two it answers one person, its owner: whoever sends the code of
pairing first becomes the owner. The code is shown on the computer that the assistant runs on,
holds for some minutes, and is given up after too many wrong tries. What anybody else writes to
the bot alone is passed over. A few things are done by the assistant itself: ``/new`` begins a
new conversation, ``/stop`` stops the answer, ``/help`` says what there is.

In a group the bot hears only what mentions it, and answers whoever wrote that, owner or not,
before anybody is paired too. Each group has a conversation of its own, with a session that is
taken up again after a new start, and a directory of its own in ``groups``. A group is for a
chat: its agent is told who wrote each message, has no tool at all, and is given nothing that
the owner keeps. The commands are the owner's, so in a group they are words like any others.

Each message is a turn of its own, and the answer of the turn answers that message. QQ takes
such answers only for a while after the message, five minutes in a group and an hour in a chat
of two, and only a few to each message. An answer too long for one message goes on in more, as
many as QQ takes, and what is left after them is cut off. An answer that comes too late is not
sent, and the status says so.

QQ may send an event again. The messages that were taken are written down, and one that comes
again is passed over, also after the assistant started again.

Another program, such as the desktop application, may run the assistant. It is then told how
the assistant is doing in lines of JSON, and stops it by closing the standard input.

The agent of the owner may read the files of the directory of the assistant, and change nothing
and run nothing: the profile gives it no such tool. No agent of the assistant is given the
instructions that the user keeps for their own work, nor their skills.
"""

import asyncio
import importlib.util
import json
import logging
import os
import secrets
import sys
import time
from collections import deque
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast

from pydantic import BaseModel, Field

from columbina.agent import AGENTS, Agent, AgentError, Agents, AgentSpec, replies
from columbina.apps import FRONTEND
from columbina.apps.common import open_agent, when_the_input_closes
from columbina.apps.terminal_text import harmless
from columbina.feishu.messages import parts
from columbina.kernel import Context
from columbina.llm import CallConfig
from columbina.permissions import PERMISSIONS
from columbina.qq import store
from columbina.qq.app import Api, QqError, token_url
from columbina.qq.link import GROUP, PRIVATE, Link
from columbina.qq.words import Language, words_for
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

__all__ = ["GROUP_PROMPT", "PROMPT", "Assistant", "Config", "apply", "inject", "name"]

name = "qq"
inject = (AGENTS, SESSIONS)

logger = logging.getLogger(__name__)

PROMPT = (
    "You are Columbina, a personal assistant. You talk with your user in a chat of QQ. Answer "
    "the way one does in a chat: short and to the point, and at length only where the user "
    "asks for it. What you write is shown as plain text, so write no Markdown."
)
"""The words that open the conversation with the owner."""

GROUP_PROMPT = (
    "You are Columbina, an assistant in a group chat of QQ. The people of the group write to "
    "you by mentioning you, and you see nothing else that is written in the group. Each "
    "message says who wrote it, by an id that QQ gives. This is a chat: answer briefly and in "
    "a friendly way, in a sentence or a few. What you write is shown as plain text, so write no "
    "Markdown."
)
"""The words that open the conversation in a group."""

GROUPS = "groups"
"""The directory, in the directory of the assistant, that holds one directory for each group."""

OWNER_TOOLS = frozenset({"read", "glob", "grep"})
"""The tools of the agent of the owner: those that read, in the directory of the assistant."""

SEEN = "seen.json"
"""The file that names the messages that were taken, in what the program keeps for the assistant."""

_MOST = 3000
"""How many bytes an answer in one message may take, as UTF-8."""

_ANSWERS = {True: 5, False: 4}
"""How many answers QQ takes to one message: in a group, and in a chat of two."""

_WINDOW = {True: 290.0, False: 3540.0}
"""For how many seconds after a message QQ takes answers to it, with a little left for the
last: in a group, and in a chat of two."""

_SEEN_MOST = 500
_LAST_SECONDS = 3.0
_TRIES_EACH = 5
_TRIES_ALL = 20

_SEND_FAILED = "an answer could not be sent to QQ"
_NOT_NOTED = (
    "which messages were taken cannot be written down, so one that QQ sends again after a new "
    "start may be answered twice"
)


class Config(BaseModel):
    """The config of the assistant in QQ.

    Attributes:
        model: The model that the agents talk to, and how.
        cwd: The directory of the assistant: the working directory of the agent of the owner. It
            lies apart from the home of the program.
        home: The home of the program, where the bot is kept.
        system_prompt: The instructions that open the conversation with the owner.
        language: The language of what the assistant says by itself, Chinese where none is said.
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


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def _text(value: object) -> str:
    return value if isinstance(value, str) else ""


def _safe(name: str) -> str:
    """Return an id of QQ as the name of a directory."""
    return "".join(c if c.isalnum() or c in "-_" else "_" for c in name)[-120:] or "group"


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
            self._path.parent.mkdir(parents=True, exist_ok=True)
            self._path.write_text(json.dumps(list(self._ids)), encoding="utf-8")
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
    """A message that is answered: where it came from, when, and how many answers it had.

    Attributes:
        message_id: The message, as QQ names it.
        group: The group that it came from, or ``None`` for a chat of two.
        user: Who wrote it in a chat of two.
        at: When it was taken, on the clock of the assistant.
        number: The number of the next answer to it.
        closed: Whether its answer was given, or will not be.
    """

    message_id: str
    group: str | None
    user: str | None
    at: float
    number: int = 1
    closed: bool = False


@dataclass(slots=True)
class _Talk:
    """A conversation that the assistant holds: with its owner, or in a group.

    Attributes:
        group: The group, as QQ names it, or ``None`` for the conversation with the owner.
        agent: The agent of the conversation, once it is open.
        waiting: The messages that the agent was given, and has not begun to answer.
        turns: The turns under way, by their number: the message that each answers, and how
            long the log was when it began.
    """

    group: str | None = None
    agent: Agent | None = None
    waiting: deque[_Asked] = field(default_factory=deque[_Asked])
    turns: dict[int, tuple[_Asked | None, int]] = field(
        default_factory=dict[int, tuple[_Asked | None, int]]
    )

    def undone(self) -> list[_Asked]:
        """Return the messages that the agent is answering or will answer."""
        return [*(asked for asked, _ in self.turns.values() if asked), *self.waiting]


class Assistant:
    """Holds the conversations of the bot in QQ."""

    def __init__(self, ctx: Context, config: Config, agents: Agents, sessions: Sessions) -> None:
        self._ctx = ctx
        self._config = config
        self._agents = agents
        self._sessions = sessions
        self._now: Callable[[], float] = time.time
        self._clock: Callable[[], float] = time.monotonic
        self._words = words_for(config.language or "zh")
        self._api: Api | None = None
        self._link: Link | None = None
        self._settings: store.Settings | None = None
        self._owner = _Talk()
        self._groups: dict[str, _Talk] = {}
        self._seen: _Seen | None = None
        self._pairing: _Pairing | None = None
        self._queue: asyncio.Queue[tuple[str, dict[str, Any]]] = asyncio.Queue()
        self._doing: set[asyncio.Future[Any]] = set()
        self._state: store.State = "connecting"
        self._error: str | None = None

    # Starting and ending

    async def run(self) -> int:
        """Hold the conversations until the program is stopped, or QQ refuses the bot.

        Returns:
            1 where QQ refused the bot or the session could not be opened, and 2 where nothing
            that the assistant needs is there.
        """
        config = self._config
        if importlib.util.find_spec("websockets") is None:
            _tell(
                "the assistant in QQ needs the package websockets, which comes with "
                "'pip install columbina[qq]'"
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
            _tell("an assistant in QQ runs on this machine already, and only one may run")
            return 2
        try:
            # The bot is read under the lock: it is set up or deleted only while nobody holds it.
            try:
                chosen = store.settings(config.home)
                secret = (
                    None if chosen is None else store.secret(config.home, token_url(os.environ))
                )
            except QqError as error:
                _tell(str(error))
                return 2
            if chosen is None or secret is None:
                _tell(
                    "no bot of QQ is set up. Set one up in the settings of the desktop "
                    "application, or with 'columbina qq --setup'"
                )
                return 2
            try:
                config.cwd.mkdir(parents=True, exist_ok=True)
            except OSError as error:
                _tell(f"the directory of the assistant cannot be used: {error}")
                return 2
            return await self._serve(chosen, secret)
        finally:
            held.release()

    async def _serve(self, chosen: store.Settings, secret: str) -> int:
        config = self._config
        self._settings = chosen
        self._seen = _Seen(config.home / store.FOLDER / SEEN)
        api = self._api = Api(chosen.app_id, secret, os.environ)
        link = self._link = Link(api, heard=self._heard, told=self._told)
        if chosen.owner is None:
            self._pair_anew()
        working = asyncio.ensure_future(self._work())
        try:
            try:
                self._owner.agent = await self._opened(self._owner, chosen.session_id)
            except (SessionError, AgentError, ValueError) as error:
                self._failed(f"the session of the assistant cannot be opened: {error}")
                return 1
            _tell(f"the assistant of the bot {chosen.app_id} is starting. Ctrl+C stops it")
            try:
                await self._held(link)
            except QqError as error:
                self._failed(str(error))
                return 1
            return 0
        finally:
            working.cancel()
            await asyncio.gather(working, return_exceptions=True)
            await self._end(api)

    async def _held(self, link: Link) -> None:
        """Keep the link until it fails, or until the program that runs the assistant stops it.

        Raises:
            QqError: QQ refused the bot.
        """
        if not self._config.json_lines:
            return await link.run()
        linked = asyncio.ensure_future(link.run())
        closed = asyncio.ensure_future(when_the_input_closes().wait())
        try:
            await asyncio.wait({linked, closed}, return_when=asyncio.FIRST_COMPLETED)
            if closed.done() and not linked.done():
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
        left = [asked for talk in self._talks() for asked in talk.undone()]
        if left:
            closing = [
                asyncio.ensure_future(self._closed(asked, self._words.interrupted))
                for asked in left
            ]
            await asyncio.wait(closing, timeout=_LAST_SECONDS)
            for doing in closing:
                doing.cancel()

    async def _end(self, api: Api) -> None:
        """End the agents and what is done on the side, and say that the assistant stopped."""
        talks = self._talks()
        self._groups.clear()
        for talk in talks:
            agent, talk.agent = talk.agent, None
            talk.waiting.clear()
            talk.turns.clear()
            if agent is None:
                continue
            try:
                if not agent.at_rest:
                    agent.cancel("user", keep_inbox=True)
                await asyncio.shield(agent.close())
            except Exception as error:
                logger.error("the agent of the assistant stopped badly: %s", error)
        for doing in list(self._doing):
            doing.cancel()
        if self._doing:
            await asyncio.wait(self._doing)
        self._link = self._api = None
        await api.aclose()
        if self._state != "failed":
            self._state = "stopped"
            self._status()

    def _failed(self, why: str) -> None:
        self._state, self._error = "failed", why
        self._status()
        _tell(why)

    def _talks(self) -> list[_Talk]:
        """Return the conversations: the one with the owner first, then those in groups."""
        return [self._owner, *self._groups.values()]

    async def _opened(self, talk: _Talk, session_id: str | None) -> Agent:
        """Open the session of a conversation, and take up what waited in it.

        Raises:
            SessionError: The stored session cannot be read or is open elsewhere.
            AgentError: The agent could not be made.
            ValueError: The id cannot name a session.
            OSError: The directory of a group cannot be made.
        """
        config = self._config
        group = talk.group
        if group is None:
            spec = AgentSpec(
                cwd=config.cwd,
                model=config.model,
                system_prompt=config.system_prompt,
                tools=OWNER_TOOLS,
                strangers=True,
            )
        else:
            cwd = config.cwd / GROUPS / _safe(group)
            cwd.mkdir(parents=True, exist_ok=True)
            spec = AgentSpec(
                cwd=cwd,
                model=config.model,
                system_prompt=GROUP_PROMPT,
                tools=frozenset(),
                strangers=True,
            )
        agent = await open_agent(self._ctx, self._agents, self._sessions, spec, session_id)
        permissions = self._ctx.get(PERMISSIONS)
        if permissions is not None:
            permissions.set_preset(agent.session, "read-only")
        agent.ctx.on(SESSION_EVENT, self._follower(talk, agent))
        settings = self._settings
        if settings is not None and group is None and settings.session_id != agent.id:
            self._settings = settings.model_copy(update={"session_id": agent.id})
            self._save()
        elif settings is not None and group is not None and settings.groups.get(group) != agent.id:
            groups = {**settings.groups, group: agent.id}
            self._settings = settings.model_copy(update={"groups": groups})
            self._save()
        # A message that was taken and not answered before the program ended is answered now,
        # where QQ still takes an answer to it.
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
            if kept is None or kept.app_id != settings.app_id:
                logger.warning("the bot changed while the assistant ran, so it is left as it is")
                return "the bot was changed while the assistant ran"
            store.save(home, settings)
        except QqError as error:
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
                "app_id": None if settings is None else settings.app_id,
            },
            ensure_ascii=False,
        )
        try:
            print(line, flush=True)
        except (OSError, ValueError) as error:
            logger.info("how the assistant is doing cannot be said: %s", error)

    def _told(self, state: str, why: str | None) -> None:
        connected = state == "connected"
        if connected and self._state != "connected":
            _tell("connected to QQ")
        self._state = "connected" if connected else "connecting"
        self._error = why
        self._status()

    def _pair_anew(self) -> None:
        minutes = self._config.pairing_minutes
        code = f"{secrets.randbelow(1_000_000):06d}"
        self._pairing = _Pairing(code, self._now() + minutes * 60, {})
        self._status()
        _tell(
            f"to be the one whom the assistant answers, send its bot this code in QQ: {code}. "
            f"It holds for {minutes:g} minutes"
        )

    # What comes in

    def _heard(self, kind: str, data: dict[str, Any], event_id: str | None) -> None:
        """Take an event that QQ sent: a message to the bot, alone or in a group."""
        if kind not in (GROUP, PRIVATE):
            return
        message_id = data.get("id")
        seen = self._seen
        if not isinstance(message_id, str) or not message_id or seen is None:
            return
        if message_id in seen:
            logger.info("a message that came again is passed over")
            return
        if not seen.add(message_id):
            self._error = _NOT_NOTED
            self._status()
        self._queue.put_nowait((kind, data))

    async def _work(self) -> None:
        """Take the messages one after another, in the order in which they came."""
        while True:
            kind, data = await self._queue.get()
            try:
                await self._take(kind, data)
            except Exception as error:
                logger.error("a message from QQ could not be taken", exc_info=error)

    async def _take(self, kind: str, data: dict[str, Any]) -> None:
        author = _object(data.get("author"))
        text = _text(data.get("content")).strip()
        message_id = _text(data.get("id"))
        if kind == GROUP:
            group, who = _text(data.get("group_openid")), _text(author.get("member_openid"))
            if group and who:
                await self._in_group(_Asked(message_id, group, None, self._clock()), who, text)
            return
        who = _text(author.get("user_openid"))
        settings = self._settings
        if not who or settings is None:
            return
        asked = _Asked(message_id, None, who, self._clock())
        if settings.owner is None:
            await self._pair(asked, text)
        elif who != settings.owner:
            logger.info("a message from someone other than the owner is passed over")
        elif not text:
            self._say(asked, self._words.unread)
        elif text.startswith("/"):
            await self._command(text, asked)
        else:
            self._ask(self._owner, text, asked)

    async def _in_group(self, asked: _Asked, who: str, text: str) -> None:
        """Answer a message in a group, whoever wrote it: they mentioned the bot.

        The conversation of a group is opened when somebody first writes in it after a start.
        """
        group = cast("str", asked.group)
        words = self._words
        if not text:
            self._say(asked, words.unread)
            return
        talk = self._groups.get(group)
        if talk is None:
            talk = self._groups[group] = _Talk(group)
        if talk.agent is None:
            known = None if self._settings is None else self._settings.groups.get(group)
            try:
                talk.agent = await self._opened(talk, known)
            except (SessionError, AgentError, ValueError, OSError) as error:
                logger.error("the conversation of a group cannot be opened: %s", error)
                self._say(asked, words.group_failed.format(reason=error))
                return
        self._ask(talk, f"[{who} wrote:]\n{text}", asked)

    async def _pair(self, asked: _Asked, given: str) -> None:
        pairing = self._pairing
        who = cast("str", asked.user)
        if pairing is None:
            return
        if pairing.until < self._now():
            self._pair_anew()
            pairing = self._pairing
            if pairing is None:  # pragma: no cover - set a moment ago
                return
        settings = self._settings
        if given == pairing.code and settings is not None:
            self._settings = settings.model_copy(update={"owner": who})
            why = self._save()
            if why is not None:
                self._settings = settings
                self._say(asked, self._words.pair_not_kept.format(reason=why))
                return
            self._pairing = None
            self._status()
            _tell("the assistant is paired, and answers the one who sent the code")
            self._say(asked, f"{self._words.paired}\n\n{self._words.help}")
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
            self._say(asked, self._words.pair_please)

    def _ask(self, talk: _Talk, text: str, asked: _Asked) -> None:
        """Give the agent of a conversation a message, which it answers in a turn of its own."""
        agent = talk.agent
        if agent is None:
            self._say(asked, self._words.no_session)
            return
        talk.waiting.append(asked)
        agent.followup(text)

    # The commands

    async def _command(self, text: str, asked: _Asked) -> None:
        command = text.partition(" ")[0].lower()
        words = self._words
        if command == "/new":
            await self._new(asked)
        elif command == "/stop":
            await self._stop(asked)
        elif command == "/help":
            self._say(asked, words.help)
        else:
            self._say(asked, words.unknown_command)

    async def _new(self, asked: _Asked) -> None:
        owner = self._owner
        old, owner.agent = owner.agent, None
        dropped = owner.undone()
        owner.turns.clear()
        owner.waiting.clear()
        words = self._words
        if old is not None:
            if not old.at_rest:
                old.cancel("user")
            await asyncio.shield(old.close())
        for waited in dropped:
            self._later(self._closed(waited, words.dropped_here))
        try:
            owner.agent = await self._opened(owner, None)
        except (SessionError, AgentError, ValueError) as error:
            logger.error("a new session of the assistant cannot be opened: %s", error)
            self._say(asked, words.new_failed.format(reason=error))
            return
        said = words.new_dropped.format(count=len(dropped)) if dropped else words.new
        self._say(asked, said)

    async def _stop(self, asked: _Asked) -> None:
        owner = self._owner
        agent = owner.agent
        words = self._words
        waiting = list(owner.waiting)
        if agent is None or (agent.at_rest and not waiting):
            self._say(asked, words.nothing_to_stop)
            return
        owner.waiting.clear()
        agent.cancel("user")
        for waited in waiting:
            self._later(self._closed(waited, words.dropped_here))
        said = words.stopped_dropped.format(count=len(waiting)) if waiting else words.stopped
        self._say(asked, said)

    # What goes out

    def _later(self, work: Coroutine[Any, Any, Any]) -> asyncio.Future[Any]:
        """Do something on the side, and have it ended with the assistant."""
        doing = asyncio.ensure_future(work)
        self._doing.add(doing)
        doing.add_done_callback(self._doing.discard)
        return doing

    def _follower(self, talk: _Talk, agent: Agent) -> Callable[[Committed], None]:
        """Return what follows the turns of an agent, while it is the one of its conversation."""

        def follow(committed: Committed) -> None:
            if agent is not talk.agent or committed.session is not agent.session:
                return
            data = committed.event.data
            if isinstance(data, TurnStartEvent):
                asked = talk.waiting.popleft() if talk.waiting else None
                talk.turns[data.turn] = (asked, len(agent.session.events))
            elif isinstance(data, TurnEndEvent):
                asked, began = talk.turns.pop(data.turn, (None, len(agent.session.events)))
                said = replies(event.data for event in agent.session.events[began:])
                if asked is not None:
                    self._later(self._answered(asked, data, said[-1] if said else ""))

        return follow

    async def _answered(self, asked: _Asked, end: TurnEndEvent, text: str) -> None:
        """Send the answer of a turn, with how the turn ended where it did not end well."""
        words = self._words
        if end.reason == "aborted":
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
        await self._closed(asked, said)

    async def _closed(self, asked: _Asked, text: str) -> None:
        """Answer a message, once, in as many messages as QQ takes."""
        if asked.closed:
            return
        asked.closed = True
        pieces = parts(text, _MOST)
        most = _ANSWERS[asked.group is not None]
        if len(pieces) > most:
            pieces = [*pieces[: most - 1], f"{pieces[most - 1]}\n{self._words.cut_off}"]
        for piece in pieces:
            if not await self._sent(asked, piece):
                return

    def _say(self, asked: _Asked, said: str) -> None:
        """Answer a message on the side, so that what comes after it is taken meanwhile."""
        self._later(self._closed(asked, said))

    async def _sent(self, asked: _Asked, text: str) -> bool:
        """Send one answer to a message, and say whether QQ took it."""
        api = self._api
        if api is None:
            return False
        in_group = asked.group is not None
        if self._clock() - asked.at > _WINDOW[in_group]:
            self._error = f"{_SEND_FAILED}: QQ takes no answer to a message as old as this one"
            self._status()
            logger.error(self._error)
            return False
        number, asked.number = asked.number, asked.number + 1
        try:
            await api.answer(
                text,
                message_id=asked.message_id,
                number=number,
                group=asked.group,
                user=asked.user,
            )
        except QqError as error:
            logger.error("%s: %s", _SEND_FAILED, error)
            self._error = f"{_SEND_FAILED}: {error}"
            self._status()
            return False
        if self._error is not None and self._error.startswith(_SEND_FAILED):
            self._error = None
            self._status()
        return True


def apply(ctx: Context, config: Config) -> None:
    """Provide the assistant in QQ."""
    ctx.provide(FRONTEND, Assistant(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS)))
