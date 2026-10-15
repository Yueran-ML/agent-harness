"""Where the sign-in to GitHub is kept: its tokens apart from the keys of models, and who it is.

The two tokens of a sign-in go into the keyring of the system through
:mod:`columbina.llm.credentials`, filed under a file of their own,
``github-credentials.json``. That is also where they lie where no keyring takes them. So they
are apart from the keys of the services of models, and a service of the user's own that is
called ``github`` has nothing to do with them. The token for the API is handed out only for
requests to the API of GitHub, and the one that renews it only for GitHub itself.

``github.json`` holds what is no secret: with which app the user signed in, who that is, and
until when the tokens hold. Tokens without that file are no sign-in.

Each renewal of the tokens makes the last renewing token useless. So whoever renews them takes
``github.lock`` first, and two programs that renew at once do not spoil each other's tokens,
see :func:`renewing`.
"""

import asyncio
import time
from collections.abc import AsyncGenerator, Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, ValidationError

from columbina import files
from columbina.github.app import App, GitHubError
from columbina.llm import credentials
from columbina.llm.credentials import CredentialsError, Kept, origin_of
from columbina.session.lock import acquire

__all__ = [
    "AVATAR",
    "CREDENTIALS",
    "LOCK",
    "SIGNED",
    "Signed",
    "Tokens",
    "forget",
    "keep",
    "renewing",
    "signed",
    "tokens",
]

CREDENTIALS = "github-credentials.json"
"""The file in the home of the program under which the tokens are kept."""

SIGNED = "github.json"
"""The file in the home of the program that says who signed in, and until when."""

AVATAR = "github-avatar.png"
"""The picture of the user who signed in, as GitHub gave it."""

LOCK = "github.lock"
"""The file that whoever renews the tokens holds a lock on."""

_ACCESS = "access"
_RENEWING = "refresh"


class Signed(BaseModel):
    """What is known of a sign-in, and is no secret: the content of ``github.json``.

    Attributes:
        client_id: The client ID of the app that the user signed in with.
        login: The name of the user on GitHub.
        name: What the user is called there, or ``None`` where nothing is said.
        access_expires_at: When the token for the API stops holding, in seconds
            since 1970.
        renewal_expires_at: When the token that renews it stops holding.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    version: Literal[1] = 1
    client_id: str
    login: str
    name: str | None = None
    access_expires_at: float
    renewal_expires_at: float


@dataclass(frozen=True, slots=True)
class Tokens:
    """The tokens of a sign-in.

    Attributes:
        access: The token that requests to the API are made with.
        renewal: The token that gets a new pair of tokens.
        access_expires_at: When the first stops holding, in seconds since 1970.
        renewal_expires_at: When the second stops holding.
    """

    access: str = field(repr=False)
    renewal: str = field(repr=False)
    access_expires_at: float
    renewal_expires_at: float


def _unusable(error: CredentialsError) -> GitHubError:
    return GitHubError(f"the tokens of GitHub cannot be used: {error}", code="STORE")


def keep(home: Path, app: App, tokens: Tokens, login: str, name: str | None) -> Kept:
    """Keep the tokens of a sign-in, and who signed in.

    Returns:
        Where the token for the API was kept. The other one goes the same way.

    Raises:
        GitHubError: The tokens or the file cannot be written.
    """
    path = home / CREDENTIALS
    try:
        kept = credentials.keep(path, _ACCESS, tokens.access, origin_of(app.api))
        credentials.keep(path, _RENEWING, tokens.renewal, origin_of(app.web))
    except CredentialsError as error:
        raise _unusable(error) from None
    said = Signed(
        client_id=app.client_id,
        login=login,
        name=name,
        access_expires_at=tokens.access_expires_at,
        renewal_expires_at=tokens.renewal_expires_at,
    )
    try:
        files.replaced(home / SIGNED, said.model_dump_json(indent=2) + "\n", private=True)
    except OSError as error:
        raise GitHubError(
            f"{home / SIGNED} cannot be written: {error.strerror}", code="STORE"
        ) from None
    return kept


def signed(home: Path, app: App) -> Signed | None:
    """Return who signed in with this app, or ``None`` if nobody did.

    A file that this program did not write, or one of another app, is no
    sign-in.

    Raises:
        GitHubError: The file is there and cannot be read.
    """
    try:
        text = (home / SIGNED).read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError) as error:
        why = error.strerror if isinstance(error, OSError) else "it is not text"
        raise GitHubError(f"{home / SIGNED} cannot be read: {why}", code="STORE") from None
    try:
        said = Signed.model_validate_json(text)
    except ValidationError:
        return None
    return said if said.client_id == app.client_id else None


def tokens(home: Path, app: App) -> Tokens | None:
    """Return the tokens of the sign-in with this app, or ``None`` if there is none.

    Raises:
        GitHubError: What is kept cannot be read.
    """
    said = signed(home, app)
    if said is None:
        return None
    path = home / CREDENTIALS
    try:
        access = credentials.key_for(path, _ACCESS, origin_of(app.api))
        renewal = credentials.key_for(path, _RENEWING, origin_of(app.web))
    except CredentialsError as error:
        raise _unusable(error) from None
    if access is None or renewal is None:
        return None
    return Tokens(access, renewal, said.access_expires_at, said.renewal_expires_at)


def forget(home: Path) -> bool:
    """Delete the sign-in: the tokens, who signed in, and the picture.

    Returns:
        Whether there was anything to delete.

    Raises:
        GitHubError: A token cannot be taken out of the keyring of the system.
            Nothing else was deleted then.
    """
    path = home / CREDENTIALS
    there = False
    for token in (_ACCESS, _RENEWING):
        try:
            there = bool(credentials.forget(path, token)) or there
        except CredentialsError as error:
            raise _unusable(error) from None
    for name in (SIGNED, AVATAR):
        try:
            (home / name).unlink()
        except FileNotFoundError:
            continue
        there = True
    return there


@asynccontextmanager
async def renewing(
    home: Path,
    *,
    seconds: float = 30.0,
    pause: float = 0.1,
    clock: Callable[[], float] = time.monotonic,
) -> AsyncGenerator[None]:
    """Hold the lock of whoever renews the tokens, for as long as the block lasts.

    Another program, or another task of this one, that holds it is waited
    for. A lock that the system holds for a process ends with the process.

    Raises:
        GitHubError: The lock was held all the time that it was waited for.
    """
    await asyncio.to_thread(home.mkdir, parents=True, exist_ok=True)
    deadline = clock() + seconds
    while (held := acquire(home / LOCK)) is None:
        if clock() >= deadline:
            raise GitHubError(
                f"another Columbina has been renewing the tokens of GitHub for {seconds:g} "
                "seconds and is not done. Try again in a moment",
                code="BUSY",
            )
        await asyncio.sleep(pause)
    try:
        yield
    finally:
        held.release()
