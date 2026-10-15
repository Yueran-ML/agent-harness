"""The REST API of GitHub, asked in the name of the user who signed in.

Every request goes with a token that holds, see :func:`columbina.github.session.access_token`.
GitHub may turn down a token that should hold, where another program renewed it in the
meantime: the token is renewed then, or the renewed one is taken, and the request is made once
more. The token goes to the API of GitHub and nowhere else: a request that would lead
elsewhere, such as a page of a list that another place is said to hold, is not made.

What goes wrong is said in a sentence and a code, :class:`columbina.github.app.GitHubError`,
and never with a token in it. Where no variable of the environment names a proxy, GitHub is
asked through the proxy of the system, see :mod:`columbina.net`.
"""

import math
import re
import time
from collections.abc import Callable, Mapping
from http import HTTPStatus
from pathlib import Path
from typing import Any, cast

import httpx

import columbina
from columbina.github.app import App, GitHubError, app
from columbina.github.session import access_token
from columbina.llm.credentials import origin_of, spellings
from columbina.net import proxy_for

__all__ = ["GitHub"]

_VERSION = "2022-11-28"
"""The version of the REST API that the requests are written for."""
_PAGE = 100
"""How many items GitHub gives in a page at most."""
_NEXT = re.compile(r'<([^>]*)>\s*;\s*rel="next"')
_UNREACHABLE = "If GitHub is reached through a proxy, see that the proxy is on"
_MINUTE = 60.0


class GitHub:
    """Asks the API of GitHub, in the name of the user who signed in.

    A program without a GitHub App to sign in with has one all the same: it
    says so wherever GitHub is to be asked.

    Args:
        home: The home of the program, where the sign-in is kept.
        env: The environment: which app, where GitHub is, and the proxy.
        transport: The transport of the HTTP client, for tests. A request
            that goes to it goes through no proxy.
        now: The time, in seconds since 1970.
    """

    def __init__(
        self,
        home: Path,
        env: Mapping[str, str],
        *,
        transport: httpx.AsyncBaseTransport | None = None,
        now: Callable[[], float] = time.time,
    ) -> None:
        self._home = home
        self._now = now
        try:
            self._app: App | GitHubError = app(env)
        except GitHubError as error:
            self._app = error
        api = self._app.api if isinstance(self._app, App) else "https://api.github.com"
        self.http = httpx.AsyncClient(
            timeout=httpx.Timeout(10.0, read=30.0),
            headers={"User-Agent": f"Columbina/{columbina.__version__}"},
            transport=transport,
            proxy=None if transport is not None else proxy_for(api, env),
        )

    @property
    def app(self) -> App:
        """The app that the user signs in with.

        Raises:
            GitHubError: There is none for this build of Columbina.
        """
        if isinstance(self._app, GitHubError):
            raise self._app
        return self._app

    async def aclose(self) -> None:
        """Close the connections to GitHub."""
        await self.http.aclose()

    async def token(self) -> str:
        """Return a token for the API that holds for a few minutes at least, for git.

        Raises:
            GitHubError: Nobody is signed in, or the token could not be renewed.
        """
        return await access_token(self._home, self.app, self.http, now=self._now)

    async def get(
        self, path: str, params: Mapping[str, str] | None = None, *, token: str | None = None
    ) -> Any:
        """Return what the API answers to a request for ``path``, read from its JSON.

        Args:
            path: Where to ask, such as ``/user``.
            params: The parameters of the query.
            token: A token to ask with, in the place of the one of the sign-in:
                the one of a sign-in that is not kept yet.

        Raises:
            GitHubError: GitHub did not answer it.
        """
        return _read(await self._asked("GET", path, params=params, token=token))

    async def post(self, path: str, body: Mapping[str, object]) -> Any:
        """Send ``body`` to ``path`` and return what the API answers, read from its JSON.

        Raises:
            GitHubError: GitHub did not take it.
        """
        return _read(await self._asked("POST", path, body=body))

    async def pages(self, path: str, *, key: str | None = None, most: int = 300) -> list[Any]:
        """Return the items of a list that comes in pages, at most ``most`` of them.

        Args:
            path: Where the list is.
            key: The name under which a page holds its items, for a list that
                comes as an object, such as ``repositories``.
            most: How many items are read at most.

        Raises:
            GitHubError: GitHub did not answer a page, or a page holds no list.
        """
        found: list[Any] = []
        url: str | None = path
        params: Mapping[str, str] | None = {"per_page": str(min(_PAGE, most))}
        while url is not None and len(found) < most:
            response = await self._asked("GET", url, params=params)
            page: object = _read(response)
            items = _field(page, key) if key is not None else page
            if not isinstance(items, list):
                raise GitHubError(
                    f"GitHub answered {path} with something that is no list", code="UNKNOWN"
                )
            found.extend(cast("list[Any]", items))
            later = _NEXT.search(response.headers.get("link", ""))
            # The address of the next page says what it asks for already.
            url, params = (later.group(1), None) if later else (None, None)
        return found[:most]

    async def _asked(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, str] | None = None,
        body: Mapping[str, object] | None = None,
        token: str | None = None,
    ) -> httpx.Response:
        """Make a request with a token that holds, and once more with a renewed one.

        Raises:
            GitHubError: The request leads away from the API, or GitHub did
                not answer it with success.
        """
        here = self.app
        url = path if "://" in path else f"{here.api}{path}"
        if origin_of(url) != origin_of(here.api):
            raise GitHubError(
                f"GitHub led to {origin_of(url)}, and the sign-in is not sent there",
                code="UNKNOWN",
            )
        if token is not None:
            response = await self._sent(method, url, token, params, body)
        else:
            token = await access_token(self._home, here, self.http, now=self._now)
            response = await self._sent(method, url, token, params, body)
            if response.status_code == HTTPStatus.UNAUTHORIZED:
                token = await access_token(
                    self._home, here, self.http, now=self._now, turned_down=token
                )
                response = await self._sent(method, url, token, params, body)
        if response.status_code == HTTPStatus.UNAUTHORIZED:
            raise GitHubError(
                "GitHub does not take the sign-in any more. Sign in again", code="SIGNED_OUT"
            )
        if response.status_code >= HTTPStatus.BAD_REQUEST:
            raise _refusal(response, token, here, self._now())
        return response

    async def _sent(
        self,
        method: str,
        url: str,
        token: str,
        params: Mapping[str, str] | None,
        body: Mapping[str, object] | None,
    ) -> httpx.Response:
        """Make one request with a token.

        Raises:
            GitHubError: GitHub could not be reached, or did not answer in time.
        """
        headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": _VERSION,
        }
        try:
            return await self.http.request(method, url, params=params, json=body, headers=headers)
        except httpx.TimeoutException:
            raise GitHubError(
                f"GitHub did not answer in time. {_UNREACHABLE}", code="TIMEOUT"
            ) from None
        except httpx.RequestError as error:
            said = _cleaned(str(error), token)
            raise GitHubError(
                f"GitHub could not be reached: {said}. {_UNREACHABLE}", code="UNREACHABLE"
            ) from None


def _read(response: httpx.Response) -> Any:
    """Return the JSON of an answer, or ``None`` for an answer without content.

    Raises:
        GitHubError: The answer is no JSON.
    """
    if not response.content:
        return None
    try:
        return response.json()
    except ValueError:
        raise GitHubError(
            f"GitHub answered with HTTP {response.status_code} and nothing that can be read",
            code="UNKNOWN",
        ) from None


def _cleaned(text: str, token: str) -> str:
    """Return a text with the token taken out, however it is spelled there."""
    for spelling in spellings(token):
        text = text.replace(spelling, "[token]")
    return text


def _field(said: object, name: str) -> object:
    """Return what an object of JSON holds under a name, or ``None``."""
    return cast("dict[str, object]", said).get(name) if isinstance(said, dict) else None


def _refusal(response: httpx.Response, token: str, here: App, now: float) -> GitHubError:
    """Return what an answer of an error stands for, in words that hold no token."""
    status = response.status_code
    try:
        said: object = response.json()
    except ValueError:
        said = None
    message = _field(said, "message")
    told = _cleaned(message, token) if isinstance(message, str) and message else f"HTTP {status}"
    limited = response.headers.get("x-ratelimit-remaining") == "0"
    if status == HTTPStatus.TOO_MANY_REQUESTS or (status == HTTPStatus.FORBIDDEN and limited):
        return GitHubError(
            f"GitHub asks for a pause: {told}. {_until(response, now)}", code="RATE_LIMITED"
        )
    if status == HTTPStatus.FORBIDDEN:
        return GitHubError(f"GitHub does not allow it: {told}", code="FORBIDDEN")
    if status == HTTPStatus.NOT_FOUND:
        return GitHubError(
            f"GitHub has nothing there that Columbina may see: {told}. A repository of yours "
            f"may not be chosen for Columbina yet: {here.install_url}",
            code="NOT_FOUND",
        )
    if status == HTTPStatus.UNPROCESSABLE_ENTITY:
        errors = _field(said, "errors")
        more = f" {_cleaned(str(errors), token)}" if errors else ""
        return GitHubError(f"GitHub does not take it: {told}.{more}", code="INVALID")
    if status >= HTTPStatus.INTERNAL_SERVER_ERROR:
        return GitHubError(
            f"GitHub answered with HTTP {status}. Try again in a moment", code="BUSY"
        )
    return GitHubError(f"GitHub answered with HTTP {status}: {told}", code="UNKNOWN")


def _until(response: httpx.Response, now: float) -> str:
    """Say when GitHub takes requests again, as far as it said."""
    after = response.headers.get("retry-after", "").strip()
    reset = response.headers.get("x-ratelimit-reset", "").strip()
    if after.isdigit():
        seconds = float(after)
    elif reset.isdigit():
        seconds = float(reset) - now
    else:
        return "Try again in a few minutes"
    if seconds <= _MINUTE:
        return "Try again in a minute"
    return f"Try again in {math.ceil(seconds / _MINUTE)} minutes"
