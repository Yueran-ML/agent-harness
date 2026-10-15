"""Who signed in to GitHub, and what Columbina may reach in that name.

``columbina login github`` and the desktop application sign in the same way. The program asks
GitHub for a code (:func:`columbina.github.oauth.ask_for_a_code`), the user enters it on
GitHub, and the program waits for that (:func:`columbina.github.oauth.wait_for_the_user`).
Then :func:`keep` asks GitHub who the user is, keeps the tokens with that, and fetches the
user's picture, and :func:`reach` asks which repositories the user chose for the app. What is
said of it to a person, or written for a program, is for the command line to say.
"""

import asyncio
import base64
from dataclasses import dataclass
from http import HTTPStatus
from pathlib import Path
from typing import cast

import httpx

from columbina import files
from columbina.github import store
from columbina.github.app import GitHubError
from columbina.github.client import GitHub, field
from columbina.github.store import Tokens

__all__ = ["Who", "avatar", "keep", "reach", "repositories", "revoke_url", "who"]

_PICTURE_BYTES = 1024 * 1024
"""How many bytes the picture of a user may have."""
_PICTURE_SIZE = "128"
"""How many pixels wide the picture of a user is asked for."""
_PICTURES = ((b"\x89PNG\r\n\x1a\n", "image/png"), (b"\xff\xd8\xff", "image/jpeg"))
"""How a picture that is kept begins, and what kind of picture that is."""


@dataclass(frozen=True, slots=True)
class Who:
    """Who signed in.

    Attributes:
        login: The name of the user on GitHub.
        name: What the user is called there, or ``None`` where nothing is said.
    """

    login: str
    name: str | None


def revoke_url(web: str) -> str:
    """The page on which a user takes back the right that was given to the app."""
    return f"{web}/settings/apps/authorizations"


def _who(user: object) -> tuple[Who, str | None]:
    """Return who a user of GitHub is, and the address of the picture, as ``/user`` says.

    Raises:
        GitHubError: GitHub did not say who it is.
    """
    login = field(user, "login")
    if not isinstance(login, str) or not login:
        raise GitHubError("GitHub did not say who signed in", code="UNKNOWN")
    name = field(user, "name")
    picture = field(user, "avatar_url")
    return (
        Who(login, name if isinstance(name, str) and name else None),
        picture if isinstance(picture, str) and picture else None,
    )


async def keep(github: GitHub, home: Path, tokens: Tokens) -> Who:
    """Keep the tokens of a sign-in with who signed in, and fetch the picture of the user.

    A picture that cannot be had is no failure of the sign-in: there is none
    then, and not the one of whoever signed in before.

    Raises:
        GitHubError: GitHub did not say who signed in, or the sign-in cannot
            be kept.
    """
    found, picture = _who(await github.get("/user", token=tokens.access))
    await asyncio.to_thread(store.keep, home, github.app, tokens, found.login, found.name)
    await asyncio.to_thread((home / store.AVATAR).unlink, missing_ok=True)
    if picture is not None:
        await _fetch_picture(github.http, home, picture)
    return found


async def who(github: GitHub) -> Who:
    """Ask GitHub who is signed in, with the tokens that are kept.

    Raises:
        GitHubError: Nobody is signed in, or GitHub did not answer.
    """
    found, _ = _who(await github.get("/user"))
    return found


async def repositories(github: GitHub) -> list[dict[str, object]]:
    """Return the repositories that the user chose for the app, as GitHub describes them.

    Each one has its name as ``owner/name`` under ``full_name``.

    Raises:
        GitHubError: GitHub did not answer, or answered what is no list of them.
    """
    found: list[dict[str, object]] = []
    for installation in await github.pages("/user/installations", key="installations"):
        number = field(installation, "id")
        if not isinstance(number, int) or isinstance(number, bool):
            raise GitHubError(
                "GitHub answered with an installation without its number", code="UNKNOWN"
            )
        path = f"/user/installations/{number}/repositories"
        for repository in await github.pages(path, key="repositories"):
            name = field(repository, "full_name")
            if not isinstance(name, str) or not name:
                raise GitHubError(
                    "GitHub answered with a repository without its name", code="UNKNOWN"
                )
            found.append(cast("dict[str, object]", repository))
    return found


async def reach(github: GitHub) -> list[str]:
    """Return the repositories that the user chose for the app, as ``owner/name``.

    Raises:
        GitHubError: GitHub did not answer, or answered what is no list of them.
    """
    return [str(repository["full_name"]) for repository in await repositories(github)]


async def _fetch_picture(http: httpx.AsyncClient, home: Path, address: str) -> bool:
    """Fetch the picture of the user and keep it beside the sign-in. Whether that was done.

    The picture is asked for without a token: GitHub's pictures want none, and they lie
    elsewhere than its API.
    """
    content = bytearray()
    try:
        # Set on the address, and not given as parameters: those would take the place of
        # the ones that the address has, such as the version of the picture.
        small = httpx.URL(address).copy_set_param("s", _PICTURE_SIZE)
        async with http.stream("GET", small) as response:
            if response.status_code != HTTPStatus.OK:
                return False
            async for chunk in response.aiter_bytes():
                content += chunk
                if len(content) > _PICTURE_BYTES:
                    return False
    except (httpx.HTTPError, httpx.InvalidURL):
        return False
    if _kind(bytes(content)) is None:
        return False
    try:
        await asyncio.to_thread(files.replaced, home / store.AVATAR, bytes(content), private=True)
    except OSError:
        return False
    return True


def _kind(content: bytes) -> str | None:
    """Return what kind of picture it is, or ``None`` for one that is not kept."""
    return next((kind for start, kind in _PICTURES if content.startswith(start)), None)


def avatar(home: Path) -> str | None:
    """Return the picture of the user who signed in as an address of data, or ``None``."""
    try:
        content = (home / store.AVATAR).read_bytes()
    except OSError:
        return None
    kind = _kind(content)
    if kind is None or len(content) > _PICTURE_BYTES:
        return None
    return f"data:{kind};base64,{base64.b64encode(content).decode()}"
