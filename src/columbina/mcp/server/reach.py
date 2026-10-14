"""How a server is reached: by starting a program, or at an address.

A program that is started
    It is talked to over its standard input and output. Its environment is
    the one that the SDK gives every server, a few variables such as
    ``PATH``, with what the config of the server adds. A key of the program
    that starts it does not reach it. What the server writes to its standard
    error goes into a file, and never to the terminal, where it would run
    through whatever is typed there.

A server at an address
    It is talked to over HTTP, with the headers that the config of the
    server names in every request. A server that speaks the protocol of
    before 2026 gives the connection a session, and every request names it.
    Such a server may end the session when it likes, and answers a request
    that still names it with the status 404. The protocol has a client begin
    a new session then. So every answer is looked at here, and a call that
    was answered like this is told so, see :func:`_answered`: the connection
    is made again for it. Leaving the old connection, the SDK tries to end
    the session that was gone, and logs that it could not. That line is kept
    out of the log: it is no failure, and nobody could do anything about it.

What is kept out of a log
    An address and a header may hold what lets somebody in. Such texts are
    handed to the connection, which keeps them out of every reason for a
    failure that it logs or tells of, see :func:`kept_out`.
"""

import asyncio
import json
import logging
import os
from collections.abc import AsyncGenerator, Mapping, Sequence
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, TextIO, cast
from urllib.parse import parse_qsl, urlsplit

import httpx2
from mcp import StdioServerParameters, stdio_client
from mcp.client.streamable_http import MCP_SESSION_ID, streamable_http_client

from columbina.mcp.server.connection import Target, turned_away

__all__ = ["kept_out", "reached", "started"]

_CALL = "tools/call"
"""The method of the protocol that calls a tool."""

_UNKNOWN = 404
"""The status with which a server answers a request that names a session it knows nothing of."""

_SDK_LOG = "mcp.client.streamable_http"
"""The log of the part of the SDK that talks to a server at an address."""

_GONE_ALREADY = "Session termination failed: 404"
"""What the SDK logs where it ends a session that the server had ended before."""

_LOG_KEEPS = 1024 * 1024
"""How many bytes of what a server wrote are kept when it is started again."""

_SECRET_FROM = 6
"""How long a text is at least for it to be something that lets somebody in."""

_CONNECT_SECONDS = 30.0
_READ_SECONDS = 300.0


def _opened(log: Path | None) -> TextIO:
    """Open the file for what a server writes to its standard error. Its caller closes it."""
    if log is None:
        return Path(os.devnull).open("w", encoding="utf-8")
    log.parent.mkdir(parents=True, exist_ok=True)
    # What an earlier run wrote stays, until it is more than anybody reads.
    grown = log.is_file() and log.stat().st_size > _LOG_KEEPS
    return log.open("w" if grown else "a", encoding="utf-8")


@asynccontextmanager
async def _started(parameters: StdioServerParameters, log: Path | None) -> AsyncGenerator[Any]:
    errlog = await asyncio.to_thread(_opened, log)
    try:
        async with stdio_client(parameters, errlog=errlog) as streams:
            yield streams
    finally:
        errlog.close()


def started(
    command: str,
    args: Sequence[str],
    env: Mapping[str, str],
    cwd: Path | None,
    log: Path | None,
) -> Target:
    """Return the way to a server that is a program to start.

    Args:
        command: The program.
        args: Its arguments.
        env: The variables that it gets beside the few that every server
            gets.
        cwd: The directory that it is started in, or ``None`` for the one of
            this program.
        log: The file for what it writes to its standard error, or ``None``
            if that is thrown away.
    """
    parameters = StdioServerParameters(
        command=command, args=list(args), env=dict(env) or None, cwd=cwd
    )
    return _started(parameters, log)


async def _asks(request: httpx2.Request) -> object:
    """Return the method of the protocol that a request over HTTP asks for, or ``None``."""
    try:
        # Read, and not only looked up. A server may send a request on to another path,
        # and the request that goes there is made from the first one: what it holds is
        # there to be read, and is not kept with it as it is with the first.
        said = json.loads(await request.aread())
    except (ValueError, httpx2.StreamError):
        # No request of the protocol, or one that went to the server piece by piece.
        return None
    return cast("dict[str, object]", said).get("method") if isinstance(said, dict) else None


async def _answered(response: httpx2.Response) -> None:
    """Tell a call that the server knew nothing of its session, where the answer says so.

    Only a call is told. The SDK asks for other things in the name of a
    call, the list of the tools for one, after the call was made. An answer
    like this to one of those says nothing of whether the call was taken.
    """
    request = response.request
    if (
        response.status_code == _UNKNOWN
        and MCP_SESSION_ID in request.headers
        and await _asks(request) == _CALL
    ):
        turned_away()


class _Unsaid(logging.Filter):
    """Keeps out of the log that the SDK could not end a session that was gone already."""

    def filter(self, record: logging.LogRecord) -> bool:
        # The message as the SDK writes it, which is put together already. Putting a record
        # together here could fail, and would fail whoever logs it.
        return record.msg != _GONE_ALREADY


@asynccontextmanager
async def _reached(url: str, headers: Mapping[str, str]) -> AsyncGenerator[Any]:
    limits = httpx2.Timeout(_CONNECT_SECONDS, read=_READ_SECONDS)
    looking = {"response": [_answered]}
    # One of its own for every server, so that leaving one leaves the others as they are.
    unsaid = _Unsaid()
    sdk = logging.getLogger(_SDK_LOG)
    sdk.addFilter(unsaid)
    try:
        async with (
            httpx2.AsyncClient(headers=dict(headers), timeout=limits, event_hooks=looking) as http,
            streamable_http_client(url, http_client=http) as streams,
        ):
            yield streams
    finally:
        sdk.removeFilter(unsaid)


def reached(url: str, headers: Mapping[str, str]) -> Target:
    """Return the way to a server at an address.

    Args:
        url: The address.
        headers: What every request to it carries beside what the protocol
            asks for.
    """
    return _reached(url, headers)


def _spellings(text: str) -> set[str]:
    """Return a text in the forms in which a failure may repeat it.

    As it is, without the blanks around it, and with one blank where it has
    several: a failure is told on one line. And each of these as it is
    written where a sign that cannot be printed, a line break for one, is
    shown as an escape.
    """
    plain = {text, text.strip(), " ".join(text.split())}
    escaped = {repr(form)[1:-1] for form in plain}
    escaped |= {repr(form.encode("utf-8", "surrogatepass"))[2:-1] for form in plain}
    return plain | escaped


def kept_out(url: str | None, headers: Mapping[str, str]) -> tuple[str, ...]:
    """Return the texts of an address and of headers that are for no log.

    They are what the address holds before the name of its machine, what it
    holds after its question mark, and the values of the headers, each in
    the forms in which a failure may repeat it. A text of a few characters is
    none that lets anybody in, and it is left alone: it would be blotted out
    of every word that holds it.
    """
    found = list(headers.values())
    if url is not None:
        try:
            parts = urlsplit(url)
            found += [parts.username or "", parts.password or "", parts.query]
            found += [value for _, value in parse_qsl(parts.query)]
        except ValueError:
            # An address that cannot be taken apart is kept out as a whole.
            found.append(url)
    spelled = {form for text in found for form in _spellings(text)}
    return tuple(sorted(form for form in spelled if len(form) >= _SECRET_FROM))
