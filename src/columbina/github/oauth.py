"""Getting the tokens of GitHub: the device flow of the app, and their renewal.

The device flow lets a user sign in without a page of the program: the program asks GitHub
for a code, the user enters the code at ``github.com/login/device`` in a browser in which the
user is signed in to GitHub, and the program asks until GitHub hands it the tokens. No secret
of the app is needed for any of it, and none for renewing the tokens that came this way:
GitHub says of its client secret, "Required unless the user access token was generated using
the device flow."

The token for the API holds for eight hours, and the one that renews it for six months.
Every renewal gives a new pair and makes the old renewing token useless.
"""

import asyncio
import math
import time
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, field
from typing import Any, cast
from urllib.parse import urlsplit

import httpx

from columbina.github.app import App, GitHubError
from columbina.github.store import Tokens

__all__ = ["Code", "ask_for_a_code", "renewed", "wait_for_the_user"]

_DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
_SLOWER = 5.0
"""How many seconds GitHub adds to the wait between two asks when it is asked too often."""
_UNREACHABLE = "If GitHub is reached through a proxy, see that the proxy is on"
_MISCONFIGURED = {
    "device_flow_disabled": "the GitHub App of this Columbina does not allow the device flow",
    "incorrect_client_credentials": "GitHub does not know the client ID of this Columbina",
    "incorrect_device_code": "GitHub does not know the code that this Columbina asked with",
    "unsupported_grant_type": "GitHub does not take the way in which this Columbina asks",
}
"""What GitHub answers where the app or this program is set up wrong, said for whoever made it."""


@dataclass(frozen=True, slots=True)
class Code:
    """What the user enters on GitHub, and what the program asks for the tokens with.

    Attributes:
        user_code: The code that the user enters, such as ``WDJB-MJHT``.
        verification_uri: The page on which the user enters it.
        expires_at: When the code stops holding, in seconds since 1970.
        interval: How many seconds to wait between two asks for the tokens.
        device_code: The code that the program asks with. It is for nobody's eyes.
    """

    user_code: str
    verification_uri: str
    expires_at: float
    interval: float
    device_code: str = field(repr=False)


async def _posted(http: httpx.AsyncClient, url: str, form: Mapping[str, str]) -> dict[str, Any]:
    """Post a form to GitHub and return the object of JSON that it answers with.

    Raises:
        GitHubError: GitHub could not be reached, did not answer in time, or
            answered with something that is no object of JSON.
    """
    try:
        response = await http.post(url, data=dict(form), headers={"Accept": "application/json"})
    except httpx.TimeoutException:
        raise GitHubError(
            f"GitHub did not answer in time. {_UNREACHABLE}", code="TIMEOUT"
        ) from None
    except httpx.RequestError as error:
        raise GitHubError(
            f"GitHub could not be reached: {error}. {_UNREACHABLE}", code="UNREACHABLE"
        ) from None
    if response.status_code >= 500:
        raise GitHubError(
            f"GitHub answered with HTTP {response.status_code}. Try again in a moment", code="BUSY"
        )
    try:
        said: object = response.json()
    except ValueError:
        said = None
    if not isinstance(said, dict):
        raise GitHubError(
            f"GitHub answered with HTTP {response.status_code} and nothing that can be read",
            code="UNKNOWN",
        )
    return cast("dict[str, Any]", said)


def _seconds(said: Mapping[str, Any], name: str) -> float:
    """Return a number of seconds that GitHub gave.

    Raises:
        GitHubError: There is none, or it is no number of seconds.
    """
    found = said.get(name)
    if isinstance(found, bool) or not isinstance(found, int | float) or not math.isfinite(found):
        raise GitHubError(f"GitHub answered without {name!r}", code="UNKNOWN")
    return float(found)


def _text(said: Mapping[str, Any], name: str) -> str:
    """Return a text that GitHub gave.

    Raises:
        GitHubError: There is none.
    """
    found = said.get(name)
    if not isinstance(found, str) or not found:
        raise GitHubError(f"GitHub answered without {name!r}", code="UNKNOWN")
    return found


def _refused(said: Mapping[str, Any]) -> GitHubError:
    """Return what GitHub's answer of an error stands for."""
    error = str(said.get("error"))
    told = said.get("error_description")
    because = f": {told}" if isinstance(told, str) and told else ""
    if error in _MISCONFIGURED:
        return GitHubError(f"{_MISCONFIGURED[error]}{because}", code="MISCONFIGURED")
    if error == "expired_token":
        return GitHubError("the code is no longer good. Ask for a new one", code="EXPIRED")
    if error == "access_denied":
        return GitHubError("the sign-in was turned down on GitHub", code="DENIED")
    return GitHubError(f"GitHub answered {error}{because}", code="UNKNOWN")


def _tokens(said: Mapping[str, Any], at: float) -> Tokens:
    """Return the tokens of an answer that holds them, as of the moment it came.

    Raises:
        GitHubError: The answer is an error, or holds no token that renews.
    """
    if "error" in said:
        raise _refused(said)
    access = _text(said, "access_token")
    renewal = said.get("refresh_token")
    if not isinstance(renewal, str) or not renewal:
        raise GitHubError(
            "GitHub gave no token to renew the sign-in with. The GitHub App of this Columbina "
            "is to let its tokens expire, which is one of its optional features",
            code="MISCONFIGURED",
        )
    return Tokens(
        access=access,
        renewal=renewal,
        access_expires_at=at + _seconds(said, "expires_in"),
        renewal_expires_at=at + _seconds(said, "refresh_token_expires_in"),
    )


async def ask_for_a_code(
    http: httpx.AsyncClient, app: App, *, now: Callable[[], float] = time.time
) -> Code:
    """Ask GitHub for a code that the user enters.

    The page on which the code is entered is opened in a browser of this
    machine, so it has to be one of the web: a ``file:`` address would open
    a program instead.

    Raises:
        GitHubError: GitHub gave no code, or no page of the web to enter it on.
    """
    form = {"client_id": app.client_id, "scope": "offline_access"}
    said = await _posted(http, f"{app.web}/login/device/code", form)
    if "error" in said:
        raise _refused(said)
    page = _text(said, "verification_uri")
    if urlsplit(page).scheme != "https":
        raise GitHubError(
            f"GitHub named {page!r} as the page to enter the code on, and that page does not "
            "begin with https://",
            code="UNKNOWN",
        )
    return Code(
        user_code=_text(said, "user_code"),
        verification_uri=page,
        expires_at=now() + _seconds(said, "expires_in"),
        interval=_seconds(said, "interval"),
        device_code=_text(said, "device_code"),
    )


async def wait_for_the_user(
    http: httpx.AsyncClient,
    app: App,
    code: Code,
    *,
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
    now: Callable[[], float] = time.time,
) -> Tokens:
    """Ask GitHub for the tokens until the user has entered the code, and return them.

    Raises:
        GitHubError: The code stopped holding (``EXPIRED``), the user turned
            the sign-in down (``DENIED``), the app or this program is set up
            wrong (``MISCONFIGURED``), or GitHub could not be asked.
    """
    form = {
        "client_id": app.client_id,
        "device_code": code.device_code,
        "grant_type": _DEVICE_GRANT,
    }
    interval = code.interval
    while True:
        if now() >= code.expires_at:
            raise GitHubError("the code is no longer good. Ask for a new one", code="EXPIRED")
        await sleep(interval)
        said = await _posted(http, f"{app.web}/login/oauth/access_token", form)
        error = said.get("error")
        if error == "authorization_pending":
            continue
        if error == "slow_down":
            given = said.get("interval")
            if isinstance(given, int | float) and not isinstance(given, bool):
                interval = float(given)
            else:
                interval += _SLOWER
            continue
        return _tokens(said, now())


async def renewed(
    http: httpx.AsyncClient, app: App, renewal: str, *, now: Callable[[], float] = time.time
) -> Tokens:
    """Renew the tokens with the token that renews them, and return the new pair.

    Raises:
        GitHubError: GitHub does not renew them (``SIGNED_OUT``: the token is
            no longer good, and the user signs in again), or could not be asked.
    """
    form = {"client_id": app.client_id, "grant_type": "refresh_token", "refresh_token": renewal}
    said = await _posted(http, f"{app.web}/login/oauth/access_token", form)
    if said.get("error") == "bad_refresh_token":
        raise GitHubError("the sign-in to GitHub has run out. Sign in again", code="SIGNED_OUT")
    return _tokens(said, now())
