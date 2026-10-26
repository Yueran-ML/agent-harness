"""The proxy that lets the commands of a container reach some hosts of the internet and no others.

A container whose network is ``allowed`` stands in a network of Docker that
has no way out. Beside it stands one more container, which is in that
network and in the usual one of Docker, and runs this program: a proxy of
HTTP that the commands are told of in ``HTTP_PROXY`` and ``HTTPS_PROXY``.
What they ask of a host that the config names is passed on, and what they ask
of any other host is refused with the words of :data:`REFUSED`.

HTTPS goes through the proxy as a tunnel, which is asked for with
``CONNECT``: the proxy sees which host is meant, and nothing of what is said.
Plain HTTP is passed on request by request. A host is named in the config as
it is, ``pypi.org``, or with a star for every name below one,
``*.pythonhosted.org``. A host that is named by an address of the internet,
and not by a name, is refused unless the config names that address.

This file is run in the container as it is, by the Python of the image, so it
uses nothing but what comes with Python.
"""

import asyncio
import contextlib
import sys
from collections.abc import Sequence

__all__ = ["PORT", "REFUSED", "allows", "main", "serve"]

PORT = 3128
"""Where the proxy listens."""

REFUSED = "The sandbox of Columbina does not let commands reach {host}."

_LINE_MOST = 64 * 1024
"""How long a line of the head of a request may be."""

_PIECE = 64 * 1024


def allows(host: str, patterns: Sequence[str]) -> bool:
    """Tell whether a host is one that the patterns name.

    ``pypi.org`` names that host, and ``*.pythonhosted.org`` every host below
    ``pythonhosted.org``, but not that name itself. Case does not count, and
    neither does a dot at the end.
    """
    host = host.lower().rstrip(".")
    for pattern in patterns:
        named = pattern.lower().rstrip(".")
        if named.startswith("*."):
            if host.endswith(named[1:]) and len(host) > len(named) - 1:
                return True
        elif host == named:
            return True
    return False


def _split(where: str, default: int) -> tuple[str, int]:
    """Return the host and the port of ``host:port``, or of ``[address]:port``."""
    if where.startswith("["):
        host, _, rest = where[1:].partition("]")
        port = rest.removeprefix(":")
    else:
        host, colon, port = where.rpartition(":")
        if not colon:
            host, port = where, ""
    return host, int(port) if port else default


async def _pipe(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    """Pass on what one side says to the other, and that it has done, once it has.

    The other side may still be answering, so it is only told that nothing
    more is coming: an HTTP client that has sent its request waits for the
    answer with its writing half closed.
    """
    with contextlib.suppress(ConnectionError, OSError):
        while piece := await reader.read(_PIECE):
            writer.write(piece)
            await writer.drain()
    with contextlib.suppress(ConnectionError, OSError, RuntimeError):
        writer.write_eof()


async def _refuse(writer: asyncio.StreamWriter, status: str, said: str) -> None:
    body = (said + "\n").encode("utf-8")
    head = (
        f"HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    )
    writer.write(head.encode("ascii") + body)
    with contextlib.suppress(ConnectionError, OSError):
        await writer.drain()
    writer.close()


async def _handle(
    reader: asyncio.StreamReader, writer: asyncio.StreamWriter, patterns: Sequence[str]
) -> None:
    try:
        first = await reader.readuntil(b"\r\n")
        if len(first) > _LINE_MOST:
            await _refuse(writer, "400 Bad Request", "The request is too long.")
            return
        method, target, version = first.decode("latin-1").strip().split(" ", 2)
        head: list[bytes] = []
        while (line := await reader.readuntil(b"\r\n")) != b"\r\n":
            head.append(line)
    except (asyncio.IncompleteReadError, asyncio.LimitOverrunError, ValueError):
        await _refuse(writer, "400 Bad Request", "That is no request of HTTP.")
        return
    if method.upper() == "CONNECT":
        host, port = _split(target, 443)
        opening = b""
    elif target.startswith("http://"):
        where, _, path = target.removeprefix("http://").partition("/")
        host, port = _split(where, 80)
        opening = f"{method} /{path} {version}\r\n".encode("latin-1") + b"".join(head) + b"\r\n"
    else:
        await _refuse(writer, "400 Bad Request", "Only a proxy is listening here.")
        return
    if not allows(host, patterns):
        await _refuse(writer, "403 Forbidden", REFUSED.format(host=host))
        return
    try:
        upstream_reader, upstream_writer = await asyncio.open_connection(host, port)
    except OSError as error:
        await _refuse(writer, "502 Bad Gateway", f"{host} cannot be reached: {error}")
        return
    if opening:
        upstream_writer.write(opening)
    else:
        writer.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
    await asyncio.gather(_pipe(reader, upstream_writer), _pipe(upstream_reader, writer))
    for each in (upstream_writer, writer):
        each.close()


async def serve(host: str, port: int, patterns: Sequence[str]) -> asyncio.Server:
    """Start the proxy, and return the server that it is."""

    async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        await _handle(reader, writer, patterns)

    return await asyncio.start_server(handle, host, port, limit=_LINE_MOST * 2)


def main(argv: Sequence[str]) -> None:  # pragma: no cover - it runs in a container
    """Serve on every address of the container, for the hosts that the arguments name."""

    async def forever() -> None:
        server = await serve("0.0.0.0", PORT, argv)
        async with server:
            await server.serve_forever()

    asyncio.run(forever())


if __name__ == "__main__":  # pragma: no cover - it runs in a container
    main(sys.argv[1:])
