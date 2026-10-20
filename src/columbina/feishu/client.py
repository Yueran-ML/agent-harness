"""The REST API of Feishu, asked in the name of the app.

An app asks with a token of its own, which it gets for its App ID and App Secret and which
holds for two hours. The token is kept for as long as it holds, less ten minutes as Feishu's
own client does, and got anew after that, or at once when Feishu says that it holds no more.

Feishu answers every request with an object of JSON that has a code, 0 where the request was
done, and a message. What went wrong is said in a sentence with that message and its code,
:class:`columbina.feishu.app.FeishuError`, and never with a token or the App Secret in it.

Feishu limits how often an app may ask, and asks for a pause when it asks too often. A request
that may be made again, one that reads or one that Feishu tells apart from a repetition by its
``uuid``, is made again after the pause, a few times at most. So is one that Feishu was too
busy for, or that did not reach it. Where no variable of the environment names a proxy, Feishu
is asked through the proxy of the system, see :mod:`columbina.net`.
"""

import asyncio
import json
import time
from collections.abc import Awaitable, Callable, Mapping
from http import HTTPStatus
from typing import Any, cast

import httpx

import columbina
from columbina.feishu.app import FeishuError, Host
from columbina.llm.credentials import spellings
from columbina.net import proxy_for

__all__ = ["AGAIN", "TOKEN", "Feishu", "field"]

TOKEN = "/open-apis/auth/v3/tenant_access_token/internal"
"""Where the app gets the token that it asks with."""

AGAIN = ("RATE_LIMITED", "BUSY", "TIMEOUT", "UNREACHABLE")
"""The codes of what may pass, after which a request that may be made again is made again."""

_MARGIN = 600.0
"""How many seconds before a token stops holding a new one is got."""

_TOKEN_GONE = (99991661, 99991663, 99991664, 99991665)
"""The codes with which Feishu says that the token of the app is missing or holds no more."""

_LIMITED = (99991400, 230020)
"""The codes with which Feishu says that too many requests came: for the API, and for the
messages to one person or one group."""

_LONGEST_PAUSE = 30.0
"""How long a pause between two tries may be at most, whatever Feishu asks for."""

_UNREACHABLE = "If Feishu is reached through a proxy, see that the proxy is on"


def field(said: object, *path: str) -> object:
    """Return what an object of JSON holds along a path of names, or ``None``."""
    for name in path:
        if not isinstance(said, dict):
            return None
        said = cast("dict[str, object]", said).get(name)
    return said


def _pause(response: httpx.Response) -> float | None:
    """Return how many seconds Feishu asked to be left alone, where it said so."""
    said = response.headers.get("x-ogw-ratelimit-reset", "").strip()
    try:
        return max(0.0, float(said))
    except ValueError:
        return None


class Feishu:
    """Asks the REST API of Feishu in the name of an app.

    Args:
        host: Where the app is.
        app_id: Its App ID.
        secret: Its App Secret.
        env: The environment, for the proxy.
        transport: The transport of the HTTP client, for tests. A request that goes to it
            goes through no proxy.
        now: The time, in seconds since 1970.
        sleep: Waits, for the pauses between tries.
    """

    def __init__(
        self,
        host: Host,
        app_id: str,
        secret: str,
        env: Mapping[str, str],
        *,
        transport: httpx.AsyncBaseTransport | None = None,
        now: Callable[[], float] = time.time,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
    ) -> None:
        self.host = host
        self.app_id = app_id
        self._secret = secret
        self._now = now
        self._sleep = sleep
        self._token: tuple[str, float] | None = None
        self.http = httpx.AsyncClient(
            timeout=httpx.Timeout(10.0, read=60.0),
            headers={"User-Agent": f"Columbina/{columbina.__version__}"},
            transport=transport,
            proxy=None if transport is not None else proxy_for(host.api, env),
        )

    async def aclose(self) -> None:
        """Close the connections to Feishu."""
        await self.http.aclose()

    def _said(self, text: str, token: str | None = None) -> str:
        """Return a text with the App Secret and the tokens taken out, however they are spelled."""
        hidden = [self._secret, token, None if self._token is None else self._token[0]]
        for secret in hidden:
            for spelling in spellings(secret) if secret else []:
                text = text.replace(spelling, "[secret]")
        return text

    async def token(self, *, turned_down: str | None = None) -> str:
        """Return a token of the app that holds for ten minutes at least.

        Args:
            turned_down: A token that Feishu said holds no more. A new one is got then.

        Raises:
            FeishuError: Feishu gives no token for the App ID and the App Secret.
        """
        held = self._token
        if held is not None and held[0] != turned_down and held[1] - _MARGIN > self._now():
            return held[0]
        response = await self._sent(
            "POST", TOKEN, body={"app_id": self.app_id, "app_secret": self._secret}
        )
        said = self._read(response, TOKEN)
        token, expire = field(said, "tenant_access_token"), field(said, "expire")
        if not isinstance(token, str) or not token:
            raise FeishuError("Feishu gave no token for the app", code="UNKNOWN")
        lasts = float(expire) if isinstance(expire, int | float) else 0.0
        self._token = (token, self._now() + lasts)
        return token

    async def call(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, str] | None = None,
        body: Mapping[str, object] | None = None,
        tries: int = 1,
    ) -> dict[str, Any]:
        """Make a request in the name of the app, and return the data that Feishu answered with.

        A token that Feishu says holds no more is got anew, and the request made once more.

        Args:
            method: The method of HTTP.
            path: Where to ask, from ``/open-apis``.
            params: The parameters of the query.
            body: What to send, as JSON.
            tries: How many times the request is made at most, where what went wrong may pass.
                Only a request that may be made again should be made more than once.

        Raises:
            FeishuError: Feishu did not do it.
        """
        for left in range(tries - 1, 0, -1):
            try:
                return await self._called(method, path, params, body)
            except FeishuError as error:
                if error.code not in AGAIN:
                    raise
                await self._sleep(min(error.after or 2.0 ** (tries - left), _LONGEST_PAUSE))
        return await self._called(method, path, params, body)

    async def _called(
        self,
        method: str,
        path: str,
        params: Mapping[str, str] | None,
        body: Mapping[str, object] | None,
    ) -> dict[str, Any]:
        token = await self.token()
        response = await self._sent(method, path, params=params, body=body, token=token)
        if field(_json(response), "code") in _TOKEN_GONE:
            token = await self.token(turned_down=token)
            response = await self._sent(method, path, params=params, body=body, token=token)
        data = field(self._read(response, path, token), "data")
        return cast("dict[str, Any]", data) if isinstance(data, dict) else {}

    async def bytes_of(self, path: str, params: Mapping[str, str], *, most: int) -> bytes:
        """Download what lies at a path, at most ``most`` bytes of it.

        A token that Feishu says holds no more is got anew, and the download tried once more.

        Raises:
            FeishuError: Feishu did not give it, or it is larger than ``most``.
        """
        token = await self.token()
        try:
            return await self._downloaded(path, params, most, token)
        except FeishuError as error:
            if error.number not in _TOKEN_GONE:
                raise
        token = await self.token(turned_down=token)
        return await self._downloaded(path, params, most, token)

    async def _downloaded(
        self, path: str, params: Mapping[str, str], most: int, token: str
    ) -> bytes:
        headers = {"Authorization": f"Bearer {token}"}
        url = f"{self.host.api}{path}"
        try:
            async with self.http.stream("GET", url, params=params, headers=headers) as response:
                if response.status_code >= HTTPStatus.BAD_REQUEST:
                    await response.aread()
                    self._read(response, path, token)
                kept = bytearray()
                async for chunk in response.aiter_bytes():
                    kept.extend(chunk)
                    if len(kept) > most:
                        raise FeishuError(
                            f"the file is larger than {most} bytes, more than is taken",
                            code="TOO_LARGE",
                        )
                return bytes(kept)
        except httpx.TimeoutException:
            raise FeishuError(
                f"Feishu did not answer in time. {_UNREACHABLE}", code="TIMEOUT"
            ) from None
        except httpx.RequestError as error:
            raise FeishuError(
                f"Feishu could not be reached: {self._said(str(error), token)}. {_UNREACHABLE}",
                code="UNREACHABLE",
            ) from None

    async def _sent(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, str] | None = None,
        body: Mapping[str, object] | None = None,
        token: str | None = None,
    ) -> httpx.Response:
        """Make one request.

        Raises:
            FeishuError: Feishu could not be reached, or did not answer in time.
        """
        headers = {"Content-Type": "application/json; charset=utf-8"}
        if token is not None:
            headers["Authorization"] = f"Bearer {token}"
        try:
            return await self.http.request(
                method, f"{self.host.api}{path}", params=params, json=body, headers=headers
            )
        except httpx.TimeoutException:
            raise FeishuError(
                f"Feishu did not answer in time. {_UNREACHABLE}", code="TIMEOUT"
            ) from None
        except httpx.RequestError as error:
            raise FeishuError(
                f"Feishu could not be reached: {self._said(str(error), token)}. {_UNREACHABLE}",
                code="UNREACHABLE",
            ) from None

    def _read(self, response: httpx.Response, path: str, token: str | None = None) -> object:
        """Return the JSON of an answer in which Feishu did what was asked.

        Raises:
            FeishuError: Feishu did not do it, and says why or answers with an HTTP error.
        """
        said = _json(response)
        code = field(said, "code")
        status = response.status_code
        if code == 0 and status < HTTPStatus.BAD_REQUEST:
            return said
        message = field(said, "msg")
        told = self._said(message, token) if isinstance(message, str) and message else None
        number = code if isinstance(code, int) and not isinstance(code, bool) else None
        if code in _LIMITED or status == HTTPStatus.TOO_MANY_REQUESTS:
            raise FeishuError(
                f"Feishu asks for a pause: {told or status}",
                code="RATE_LIMITED",
                number=number,
                after=_pause(response),
            )
        if number is None:
            if status >= HTTPStatus.INTERNAL_SERVER_ERROR:
                raise FeishuError(
                    f"Feishu answered with HTTP {status}. Try again in a moment", code="BUSY"
                )
            raise FeishuError(f"Feishu answered {path} with HTTP {status}", code="UNKNOWN")
        why = f"{told or 'it gave no reason'} (code {number})"
        if status == HTTPStatus.FORBIDDEN:
            raise FeishuError(f"Feishu does not allow it: {why}", code="FORBIDDEN", number=number)
        if status == HTTPStatus.NOT_FOUND:
            raise FeishuError(f"Feishu has nothing there: {why}", code="NOT_FOUND", number=number)
        raise FeishuError(f"Feishu did not do it: {why}", code="REFUSED", number=number)


def _json(response: httpx.Response) -> object:
    """Return the JSON of an answer, or ``None`` where it holds none."""
    try:
        return json.loads(response.content) if response.content else None
    except ValueError:
        return None
