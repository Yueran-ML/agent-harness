"""What is kept for the assistant in Feishu: the app, its secret, its owner, and how it is doing.

The App Secret goes into the keyring of the system through :mod:`columbina.llm.credentials`,
filed under a file of its own, ``feishu-credentials.json``, apart from the keys of models. It
is handed out only for the host of the app.

``feishu.json`` holds what is no secret: which app it is and where, whom the assistant
answers, and which session the conversation with that person is in. The directory
``feishu`` holds the rest: the working directory of the assistant, with its instruction file,
its skills and the files that it was sent; the lock that one running assistant holds; and
``status.json``, which says how the running assistant is doing, for whoever shows it.
"""

import contextlib
import time
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, ValidationError

from columbina import files
from columbina.feishu.app import Domain, FeishuError, Host
from columbina.llm import credentials
from columbina.llm.credentials import CredentialsError, Kept, origin_of

__all__ = [
    "CREDENTIALS",
    "FOLDER",
    "LOCK",
    "SETTINGS",
    "STATUS",
    "Settings",
    "State",
    "Status",
    "forget",
    "keep",
    "save",
    "secret",
    "settings",
    "status",
    "tell",
]

CREDENTIALS = "feishu-credentials.json"
"""The file in the home of the program under which the App Secret is kept."""

SETTINGS = "feishu.json"
"""The file in the home of the program that names the app and the person it answers."""

FOLDER = "feishu"
"""The directory of the assistant in the home of the program."""

STATUS = "status.json"
"""The file in that directory that says how the running assistant is doing."""

LOCK = "bot.lock"
"""The file in that directory that the running assistant holds a lock on."""

_SECRET = "app-secret"


class Settings(BaseModel):
    """What is known of the app, and is no secret: the content of ``feishu.json``.

    Attributes:
        domain: Whether the app is in Feishu or in Lark.
        app_id: The App ID of the app.
        owner: The open ID of the person whom the assistant answers, once that person has
            given the code of pairing; ``None`` before.
        session_id: The session of the conversation with that person, once there is one.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Literal[1] = 1
    domain: Domain
    app_id: str
    owner: str | None = None
    session_id: str | None = None


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


def _unusable(error: CredentialsError) -> FeishuError:
    return FeishuError(f"the App Secret cannot be used: {error}", code="STORE")


def _written(path: Path, model: BaseModel, *, private: bool) -> None:
    try:
        files.replaced(path, model.model_dump_json(indent=2) + "\n", private=private)
    except OSError as error:
        raise FeishuError(f"{path} cannot be written: {error.strerror}", code="STORE") from None


def keep(home: Path, chosen: Settings, secret: str, host: Host) -> Kept:
    """Keep the App Secret of an app, and which app it is.

    Whom the assistant answered, and in which session, belongs to the app that it answered
    for: another app begins without either. Where the app cannot be written down, the secret
    that was kept before is put back, so that the secret and the app never belong to two apps.

    Returns:
        Where the secret was kept.

    Raises:
        FeishuError: The secret or the file cannot be written, or the file that is there
            cannot be read.
    """
    before = settings(home)
    path = home / CREDENTIALS
    try:
        held = credentials.stored(path).get(_SECRET)
        old = None if held is None else credentials.key_for(path, _SECRET, held)
        kept = credentials.keep(path, _SECRET, secret, origin_of(host.api))
    except CredentialsError as error:
        raise _unusable(error) from None
    if before is not None and (before.app_id, before.domain) == (chosen.app_id, chosen.domain):
        chosen = chosen.model_copy(update={"owner": before.owner, "session_id": before.session_id})
    try:
        _written(home / SETTINGS, chosen, private=True)
    except FeishuError:
        with contextlib.suppress(CredentialsError):
            if old is None or held is None:
                credentials.forget(path, _SECRET)
            else:
                credentials.keep(path, _SECRET, old, held)
        raise
    return kept


def settings(home: Path) -> Settings | None:
    """Return which app the assistant is for, or ``None`` if none was set up.

    A file that this program did not write is no app.

    Raises:
        FeishuError: The file is there and cannot be read.
    """
    path = home / SETTINGS
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError) as error:
        why = error.strerror if isinstance(error, OSError) else "it is not text"
        raise FeishuError(f"{path} cannot be read: {why}", code="STORE") from None
    try:
        return Settings.model_validate_json(text)
    except ValidationError:
        return None


def save(home: Path, chosen: Settings) -> None:
    """Write down what changed of the app: whom it answers, and in which session.

    Raises:
        FeishuError: The file cannot be written.
    """
    _written(home / SETTINGS, chosen, private=True)


def secret(home: Path, host: Host) -> str | None:
    """Return the App Secret that was kept for the host, or ``None`` if there is none.

    Raises:
        FeishuError: What is kept cannot be read.
    """
    try:
        return credentials.key_for(home / CREDENTIALS, _SECRET, origin_of(host.api))
    except CredentialsError as error:
        raise _unusable(error) from None


def forget(home: Path) -> bool:
    """Delete the app: its secret, which app it is, and whom it answered.

    The directory of the assistant is left as it is: its skills and files are the user's.

    Returns:
        Whether there was anything to delete.

    Raises:
        FeishuError: The secret cannot be taken out of the keyring of the system. Nothing
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
    with contextlib.suppress(FeishuError):
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
