"""What is kept for the assistant in WeCom: the bot, its secret, its owner, and how it is doing.

The Secret of the bot goes into the keyring of the system where there is one that takes it,
and into ``wecom-credentials.json`` in the home of the program otherwise, as the keys of the
model services do. It is handed out only for the address that it was kept for.

``wecom.json`` holds what is no secret: which bot it is, whom the assistant answers, in which
session the conversation with that person is, and whether its agent may change the files of
its directory. The directory ``wecom`` in the home of the program holds what the program keeps
for the assistant: the lock that one running assistant holds, and ``status.json``, which says
how the running assistant is doing, for whoever shows it.

The directory of the assistant itself, the working directory of its agent with its
instruction file, its skills and the files that it was sent, lies apart from the home of the
program, see :func:`directory`. An agent that may change the files of its directory reaches
nothing of the program there.
"""

import contextlib
import time
from collections.abc import Mapping
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from columbina import files
from columbina.apps.thinking import Effort
from columbina.llm import credentials
from columbina.llm.credentials import CredentialsError, Kept, origin_of
from columbina.wecom.app import WecomError

__all__ = [
    "ASSISTANT_HOME",
    "CREDENTIALS",
    "FOLDER",
    "LOCK",
    "SETTINGS",
    "STATUS",
    "Settings",
    "State",
    "Status",
    "directory",
    "forget",
    "keep",
    "save",
    "secret",
    "settings",
    "status",
    "tell",
]

CREDENTIALS = "wecom-credentials.json"
"""The file in the home of the program under which the Secret of the bot is kept."""

SETTINGS = "wecom.json"
"""The file in the home of the program that names the bot and the person it answers."""

FOLDER = "wecom"
"""The directory in the home of the program that the program keeps for the assistant."""

ASSISTANT_HOME = "COLUMBINA_WECOM_HOME"
"""The variable of the environment that names the directory of the assistant."""

STATUS = "status.json"
"""The file in that directory that says how the running assistant is doing."""

LOCK = "bot.lock"
"""The file in that directory that the running assistant holds a lock on."""

_SECRET = "bot-secret"


class Settings(BaseModel):
    """What is known of the bot, and is no secret: the content of ``wecom.json``.

    Attributes:
        bot_id: The Bot ID of the bot.
        owner: The user ID of the person whom the assistant answers, once that person has
            given the code of pairing; ``None`` before.
        session_id: The session of the conversation with that person, once there is one.
        writable: Whether the agent may change the files of the directory of the assistant.
        groups: The session of the conversation in each group that the bot was asked in, by
            the id of the group.
        model: The model that the assistant asks, or ``None`` for the one that holds for all.
        effort: How hard that model thinks, or ``None`` for what holds for all.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Literal[1] = 1
    bot_id: str
    owner: str | None = None
    session_id: str | None = None
    writable: bool = False
    groups: dict[str, str] = Field(default_factory=dict[str, str])
    model: str | None = None
    effort: Effort | None = None


def directory(home: Path, env: Mapping[str, str]) -> Path:
    """Return the directory of the assistant: the working directory of its agent.

    It is the one that :data:`ASSISTANT_HOME` names, and else ``Columbina/wecom`` beside the
    home of the program, so ``~/Columbina/wecom`` where the home is ``~/.columbina``.
    """
    named = env.get(ASSISTANT_HOME)
    return Path(named) if named else home.parent / "Columbina" / "wecom"


State = Literal["connecting", "connected", "stopped", "failed"]
"""How the connection of a running assistant stands, or that it has ended."""


class Status(BaseModel):
    """How the running assistant is doing: the content of ``status.json``.

    Attributes:
        pid: The process of the assistant.
        state: How its connection stands.
        paired: Whether it knows whom to answer.
        code: The code that a person sends the bot to be the one it answers, while nobody is.
        error: What went wrong last, in words for a person, where something did.
        at: When this was written, in seconds since 1970.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Literal[1] = 1
    pid: int
    state: State
    paired: bool
    code: str | None = None
    error: str | None = None
    at: float


def _unusable(error: CredentialsError) -> WecomError:
    return WecomError(f"the Secret of the bot cannot be used: {error}", code="STORE")


def _written(path: Path, model: BaseModel, *, private: bool) -> None:
    try:
        files.replaced(path, model.model_dump_json(indent=2) + "\n", private=private)
    except OSError as error:
        raise WecomError(f"{path} cannot be written: {error.strerror}", code="STORE") from None


def keep(home: Path, chosen: Settings, secret: str, place: str) -> Kept:
    """Keep the Secret of a bot, and which bot it is.

    Whom the assistant answered, in which session, and whether its agent may change files,
    belongs to the bot that it answered for: another bot begins without any of it. Where the
    bot cannot be written down, the secret that was kept before is put back, so that the
    secret and the bot never belong to two bots.

    Args:
        home: The home of the program.
        chosen: The bot.
        secret: Its Secret.
        place: Where the long connection is opened, which is the only place that the Secret
            is handed out for.

    Returns:
        Where the secret was kept.

    Raises:
        WecomError: The secret or the file cannot be written, or the file that is there
            cannot be read.
    """
    before = settings(home)
    path = home / CREDENTIALS
    try:
        held = credentials.stored(path).get(_SECRET)
        old = None if held is None else credentials.key_for(path, _SECRET, held)
        kept = credentials.keep(path, _SECRET, secret, origin_of(place))
    except CredentialsError as error:
        raise _unusable(error) from None
    if before is not None and before.bot_id == chosen.bot_id:
        kept_of_it = {
            "owner": before.owner,
            "session_id": before.session_id,
            "writable": before.writable,
            "groups": before.groups,
            "model": before.model,
            "effort": before.effort,
        }
        chosen = chosen.model_copy(update=kept_of_it)
    try:
        _written(home / SETTINGS, chosen, private=True)
    except WecomError:
        with contextlib.suppress(CredentialsError):
            if old is None or held is None:
                credentials.forget(path, _SECRET)
            else:
                credentials.keep(path, _SECRET, old, held)
        raise
    return kept


def settings(home: Path) -> Settings | None:
    """Return which bot the assistant is for, or ``None`` if none was set up.

    A file that this program did not write is no bot.

    Raises:
        WecomError: The file is there and cannot be read.
    """
    path = home / SETTINGS
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError) as error:
        why = error.strerror if isinstance(error, OSError) else "it is not text"
        raise WecomError(f"{path} cannot be read: {why}", code="STORE") from None
    try:
        return Settings.model_validate_json(text)
    except ValidationError:
        return None


def save(home: Path, chosen: Settings) -> None:
    """Write down what changed of the bot: whom it answers, and in which session.

    Raises:
        WecomError: The file cannot be written.
    """
    _written(home / SETTINGS, chosen, private=True)


def secret(home: Path, place: str) -> str | None:
    """Return the Secret that was kept for the place of the connection, or ``None`` if none is.

    Raises:
        WecomError: What is kept cannot be read, or was kept for another place.
    """
    try:
        return credentials.key_for(home / CREDENTIALS, _SECRET, origin_of(place))
    except CredentialsError as error:
        raise _unusable(error) from None


def forget(home: Path) -> bool:
    """Delete the bot: its secret, which bot it is, and whom it answered.

    The directory of the assistant is left as it is: its skills and files are the user's.

    Returns:
        Whether there was anything to delete.

    Raises:
        WecomError: The secret cannot be taken out of the keyring of the system. Nothing
            else was deleted then.
    """
    try:
        there = bool(credentials.forget(home / CREDENTIALS, _SECRET))
    except CredentialsError as error:
        raise _unusable(error) from None
    try:
        (home / SETTINGS).unlink()
    except FileNotFoundError:
        return there
    return True


def tell(
    home: Path,
    pid: int,
    state: State,
    *,
    paired: bool,
    code: str | None = None,
    error: str | None = None,
    now: float | None = None,
) -> Status:
    """Write down how the running assistant is doing, and return what was written.

    A status that cannot be written is let go: it is for showing, and nothing depends on it.
    """
    said = Status(
        pid=pid,
        state=state,
        paired=paired,
        code=code,
        error=error,
        at=time.time() if now is None else now,
    )
    with contextlib.suppress(WecomError):
        _written(home / FOLDER / STATUS, said, private=True)
    return said


def status(home: Path) -> Status | None:
    """Return what the running assistant wrote last of how it is doing, or ``None``.

    A file that cannot be read, or that this program did not write, says nothing.
    """
    try:
        return Status.model_validate_json((home / FOLDER / STATUS).read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValidationError):
        return None
