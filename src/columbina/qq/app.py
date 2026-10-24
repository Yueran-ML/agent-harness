"""The open platform of QQ, as the assistant talks to it over HTTP: token, gateway and answers.

A bot is known by its AppID and its AppSecret. The two are exchanged for an access token that
holds for some time, and every request carries it as ``QQBot <token>``. The gateway, the address
of the WebSocket on which the bot hears what is written to it, is asked for with the token.

An answer to a message goes out as a request of its own, which names the message that it
answers: QQ takes such answers for a while after the message, a few to each message, each with
its own number. A message of the bot that answers none is not counted on: QQ may refuse it.

What is written about a request never holds the AppSecret or the token.
"""

import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from typing import Any, cast

import httpx

import columbina
from columbina.llm.credentials import spellings
from columbina.net import proxy_for

__all__ = ["API", "API_ENV", "TOKEN_ENV", "TOKEN_URL", "Api", "QqError", "token_url"]

TOKEN_URL = "https://bots.qq.com/app/getAppAccessToken"
"""Where the AppID and the AppSecret are exchanged for an access token."""

API = "https://api.sgroup.qq.com"
"""The address of the API of the bots."""

TOKEN_ENV = "COLUMBINA_QQ_TOKEN_URL"
API_ENV = "COLUMBINA_QQ_API"
"""The variables of the environment that name other addresses, as for a test."""

_EARLY = 60.0
"""How long before a token runs out another one is asked for, in seconds."""

_ANSWER_SECONDS = 15.0
"""How long QQ is given to answer a request."""


class QqError(Exception):
    """QQ could not be asked, or it refused.

    Attributes:
        code: What went wrong, for a program: ``REFUSED`` where QQ said no, ``UNREACHABLE`` where
            it could not be asked, ``MALFORMED`` where its answer cannot be read, ``STORE``
            where what is kept cannot be read or written.
        number: The code of the error that QQ gave, where it gave one.
    """

    def __init__(self, message: str, *, code: str, number: int | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.number = number


def token_url(env: Mapping[str, str]) -> str:
    """Return where the token is asked for: the address that the environment names, or QQ's."""
    return env.get(TOKEN_ENV) or TOKEN_URL


def _object(value: object) -> dict[str, Any]:
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


@dataclass(slots=True)
class _Token:
    value: str
    until: float


class Api:
    """The requests of one bot to QQ.

    Args:
        app_id: The AppID of the bot.
        secret: Its AppSecret.
        env: The environment, which may name a proxy and other addresses.
        transport: What sends the requests, in place of the network, for a test.
        clock: The time in seconds since 1970.
    """

    def __init__(
        self,
        app_id: str,
        secret: str,
        env: Mapping[str, str],
        *,
        transport: httpx.AsyncBaseTransport | None = None,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.app_id = app_id
        self._secret = secret
        self._token_url = token_url(env)
        self._api = (env.get(API_ENV) or API).rstrip("/")
        self._clock = clock
        self._token: _Token | None = None
        self._client = httpx.AsyncClient(
            timeout=httpx.Timeout(10.0, read=_ANSWER_SECONDS),
            headers={"User-Agent": f"Columbina/{columbina.__version__}"},
            transport=transport,
            proxy=None if transport is not None else proxy_for(self._api, env),
        )

    async def aclose(self) -> None:
        """Close the connections to QQ."""
        await self._client.aclose()

    async def token(self) -> str:
        """Return an access token of the bot, a new one where the last runs out soon.

        Raises:
            QqError: QQ does not take the AppID with the AppSecret, or could not be asked.
        """
        held = self._token
        if held is not None and self._clock() < held.until - _EARLY:
            return held.value
        body = {"appId": self.app_id, "clientSecret": self._secret}
        answer = await self._sent("POST", self._token_url, body, signed=False)
        value, lasts = answer.get("access_token"), answer.get("expires_in")
        try:
            seconds = float(lasts)  # pyright: ignore[reportArgumentType]
        except (TypeError, ValueError):
            seconds = -1.0
        if not isinstance(value, str) or not value or seconds <= 0:
            raise QqError(
                "QQ does not take this AppID with this AppSecret: it gave no access token. See "
                "that both are those that the page of the bot on the open platform shows",
                code="REFUSED",
            )
        self._token = _Token(value, self._clock() + seconds)
        return value

    def forget_token(self) -> None:
        """Ask for another token next time: QQ no longer takes the one that is held."""
        self._token = None

    async def gateway(self) -> str:
        """Return the address of the WebSocket on which the bot hears what is written to it.

        Raises:
            QqError: QQ could not be asked, refused, or gave no address.
        """
        answer = await self._sent("GET", f"{self._api}/gateway", None)
        url = answer.get("url")
        if not isinstance(url, str) or not url:
            raise QqError("QQ gave no address of its gateway", code="MALFORMED")
        return url

    async def answer(
        self, text: str, *, message_id: str, number: int, group: str | None, user: str | None
    ) -> None:
        """Answer a message, in the group or the chat of two that it came from.

        Args:
            text: What is said.
            message_id: The message that is answered.
            number: Which answer to that message this is, from 1.
            group: The group that the message came from, or ``None`` for a chat of two.
            user: The person who wrote the message in a chat of two.

        Raises:
            QqError: QQ could not be asked, or it refused.
        """
        where = f"groups/{group}" if group is not None else f"users/{user}"
        body = {"content": text, "msg_type": 0, "msg_id": message_id, "msg_seq": number}
        await self._sent("POST", f"{self._api}/v2/{where}/messages", body)

    async def _sent(
        self, method: str, url: str, body: Mapping[str, Any] | None, *, signed: bool = True
    ) -> dict[str, Any]:
        headers: dict[str, str] = {}
        if signed:
            headers["Authorization"] = f"QQBot {await self.token()}"
            headers["X-Union-Appid"] = self.app_id
        try:
            response = await self._client.request(method, url, json=body, headers=headers)
        except httpx.HTTPError as error:
            raise QqError(
                f"QQ could not be reached: {self._said(str(error) or type(error).__name__)}",
                code="UNREACHABLE",
            ) from None
        try:
            answer = _object(response.json())
        except ValueError:
            answer = {}
        if response.status_code >= 300:
            said = answer.get("message") or answer.get("msg") or response.reason_phrase
            number = answer.get("code") if isinstance(answer.get("code"), int) else None
            if response.status_code == 401 and signed:
                # A token that QQ no longer takes is asked for anew next time.
                self._token = None
            raise QqError(
                f"QQ refused it: {self._said(str(said))} (HTTP {response.status_code}"
                + (f", code {number})" if number is not None else ")"),
                code="UNREACHABLE" if response.status_code >= 500 else "REFUSED",
                number=number,
            )
        return answer

    def _said(self, text: str) -> str:
        """Take the AppSecret and the token out of what is said, wherever they are spelled."""
        held = [self._secret, *([] if self._token is None else [self._token.value])]
        for kept in held:
            for spelling in spellings(kept):
                text = text.replace(spelling, "[secret]")
        return text
