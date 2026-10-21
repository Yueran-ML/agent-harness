"""What the adapters that ask a model service over HTTP share: its key, its failures, its stream.

The API key is looked for when a request is made: first in an environment
variable, then among the keys that the user stored, under the name of the
provider. The file of keys says which there are, and a key lies in the
keyring of the system or in that file, see :mod:`columbina.llm.credentials`.
A stored key is one that was typed in for an address, and it is used only for
requests to the place that this address leads to: the address of an adapter
may come from a variable of the environment, and a key does not follow it
there.

The key is cut out of the text of every failure that leaves an adapter,
whether the service or the connection wrote that text. It is looked for as it
is and in the spellings in which a text commonly repeats it. Then the text is
cut to a length, in this order, so that no part of the key is left at the end
of it.

A reply is streamed as server-sent events. Only the data of each event is
read: the services that are spoken to here say in the data what kind of event
it is.
"""

import asyncio
import math
import os
import time
from collections.abc import AsyncIterator
from datetime import UTC
from email.utils import parsedate_to_datetime
from pathlib import Path
from typing import Protocol

import httpx

from columbina.llm.credentials import (
    CredentialsError,
    key_for,
    origin_of,
    sendable,
    spellings,
    trimmed,
)
from columbina.llm.runtime import LlmError
from columbina.llm.types import LlmFailure

__all__ = [
    "ID_CHARACTERS",
    "MESSAGE_CHARACTERS",
    "Keyed",
    "describe",
    "events",
    "failure_of",
    "retry_after_ms",
    "secret_of",
]

MESSAGE_CHARACTERS = 500
"""How much of the text of a failure is kept."""
ID_CHARACTERS = 200
"""How much of the id of a request is kept."""


class Keyed(Protocol):
    """The part of the config of an adapter that says where its key is."""

    @property
    def provider(self) -> str:
        """The name under which the adapter is registered, and its key is stored."""
        ...

    @property
    def base_url(self) -> str:
        """The address of the service."""
        ...

    @property
    def api_key_env(self) -> str | None:
        """The environment variable that holds the key, or ``None`` if no variable does."""
        ...

    @property
    def api_key_file(self) -> Path | None:
        """The file of the keys that the user stored, or ``None`` if there is none."""
        ...

    @property
    def api_key_required(self) -> bool:
        """Whether a request is made only with a key."""
        ...


async def events(lines: AsyncIterator[str]) -> AsyncIterator[str]:
    """Yield the data of each server-sent event of a stream of lines."""
    data: list[str] = []
    async for line in lines:
        if not line:
            if data:
                yield "\n".join(data)
                data = []
            continue
        field, _, value = line.partition(":")
        if field == "data":
            data.append(value.removeprefix(" "))
    if data:
        yield "\n".join(data)


def describe(error: BaseException) -> str:
    """Say what an error is, by its kind and its words."""
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


def retry_after_ms(response: httpx.Response) -> int | None:
    """Return how long the service asks to wait: it names seconds, or a point in time."""
    value = response.headers.get("retry-after", "")
    try:
        seconds = float(value)
    except ValueError:
        try:
            moment = parsedate_to_datetime(value)
        except ValueError:
            return None
        if moment.tzinfo is None:
            # A date of HTTP is in universal time also where it does not say so.
            moment = moment.replace(tzinfo=UTC)
        return max(round((moment.timestamp() - time.time()) * 1000), 0)
    return round(seconds * 1000) if math.isfinite(seconds) and seconds >= 0 else None


def failure_of(error: httpx.RequestError | LlmError, secret: str | None) -> LlmFailure:
    """Return the failure that an error stands for, fit to be shown and to be stored.

    Every failure of an adapter passes through here. The key is cut out of
    every text that came from outside, which is the message and the id of the
    request. Then the text is cut to a length. In this order no part of the key
    is left at the end of it.
    """
    if isinstance(error, LlmError):
        failure = error.failure
    else:
        code = "TIMEOUT" if isinstance(error, httpx.TimeoutException) else "TRANSPORT"
        failure = LlmFailure(message=describe(error), code=code)
    cleaned: dict[str, str] = {"message": _cleaned(failure.message, secret, MESSAGE_CHARACTERS)}
    if failure.request_id is not None:
        cleaned["request_id"] = _cleaned(failure.request_id, secret, ID_CHARACTERS)
    return failure.model_copy(update=cleaned)


def _cleaned(text: str, secret: str | None, limit: int) -> str:
    if secret:
        for spelling in spellings(secret):
            text = text.replace(spelling, "[key]")
    return text[:limit]


def _missing(config: Keyed) -> str | None:
    """Return what to say where no key is found, or ``None`` if a request is made without one."""
    variable, file = config.api_key_env, config.api_key_file
    if not config.api_key_required or (variable is None and file is None):
        return None
    if file is None:
        return f"the environment variable {variable} is not set"
    if variable is None:
        return f"no API key is stored for {config.provider}. Enter one with 'columbina setup'"
    return (
        f"there is no API key for {config.provider}: the environment variable {variable} is "
        "not set, and no key is stored. Enter one with 'columbina setup', or set the variable"
    )


async def secret_of(config: Keyed, given: str | None) -> str | None:
    """Return the key to send, or ``None`` for a service that is sent none.

    Args:
        config: Where the key of the service is.
        given: A key to send instead of the one that would be looked for: one
            that is checked before it is stored.

    Raises:
        LlmError: A key is needed and none is found, the key that is found
            cannot be sent, or the stored keys cannot be used.
    """
    variable, file = config.api_key_env, config.api_key_file
    if given is not None:
        secret, where, again = given, "that was given", "Give it again"
    else:
        secret = "" if variable is None else trimmed(os.environ.get(variable, ""))
        where = f"in the environment variable {variable}"
        again = "Set it again"
    if not secret and file is not None:
        try:
            # A stored key is for the place that it was typed in for, and for no other.
            place = origin_of(config.base_url)
            stored = await asyncio.to_thread(key_for, file, config.provider, place)
            secret = trimmed(stored or "")
        except CredentialsError as error:
            # The error says what is wrong with the stored keys, and holds none of them.
            failure = LlmFailure(message=str(error), code="AUTH")
            raise LlmError(failure) from None
        where = f"that is stored for {config.provider}"
        again = "Enter it again with 'columbina setup'"
    if not secret:
        missing = _missing(config)
        if missing is None:
            return None
        raise LlmError(LlmFailure(message=missing, code="AUTH"))
    if not sendable(secret):
        raise LlmError(
            LlmFailure(
                message=f"the API key {where} cannot be sent: there is a character in "
                "it that no key has, such as a blank, a line break or a quotation mark "
                f"of a word processor. {again}",
                code="AUTH",
            )
        )
    return secret
