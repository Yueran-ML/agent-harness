"""Taking a page from the web: where it may come from, how much of it, and what it says.

A page is asked for with GET and taken as text: a page of HTML as what a
reader sees of it (see :mod:`.page`), and plain text, JSON, XML, Markdown and
the like as they stand. Anything else, a picture or a PDF, is not taken, and
the model is told what it was.

**Where a page may come from.** A model asks for an address that it read
somewhere, and what it read may have been written to lead it astray. The
machine it runs on reaches places that the web does not: the services of the
machine itself and of the network it stands in, and the address that a
cloud machine is told its secrets at. So an address is looked up before it
is asked, and one that leads to such a place is refused, unless the config
lets it. So is every address that a page redirects to on the way, which is
why redirects are followed here and not by the client. Between the looking
up and the asking a name could be made to lead elsewhere, and this does not
stand against that. A proxy asks for the page in its own name, and what it
reaches is up to it.

**How much.** At most so many bytes are taken, and a page that is longer is
cut there. A page takes so many seconds at most, and so many redirects.
"""

import asyncio
import codecs
import ipaddress
import re
import socket
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from urllib.parse import urljoin, urlsplit

import httpx

import columbina
from columbina.net import proxy_for
from columbina.tools import ToolFailure
from columbina.web.page import text_of

__all__ = ["Fetched", "Lookup", "fetch", "lookup"]

Lookup = Callable[[str, int], Awaitable[list[str]]]
"""Says which addresses of the internet a host name leads to."""

_REDIRECTS = 5
_TEXTUAL = ("text/", "application/json", "application/xml", "application/xhtml+xml")
_TEXTUAL_ENDINGS = ("+json", "+xml")
_HTML = ("text/html", "application/xhtml+xml")
_CHARSET = re.compile(rb"""<meta[^>]+charset=["']?([A-Za-z0-9_-]+)""", re.IGNORECASE)


@dataclass(frozen=True, slots=True)
class Fetched:
    """A page as text.

    Attributes:
        address: Where the page was found in the end, after the redirects.
        title: Its title, or an empty text.
        text: What it says.
        cut: Whether the page was longer than what was taken of it.
    """

    address: str
    title: str
    text: str
    cut: bool


async def lookup(host: str, port: int) -> list[str]:
    """Return the addresses of the internet that a host name leads to."""
    found = await asyncio.get_running_loop().getaddrinfo(host, port, type=socket.SOCK_STREAM)
    return [str(entry[4][0]) for entry in found]


def _inside(address: str) -> bool:
    """Tell whether an address of the internet is of this machine or of a network of its own."""
    ip = ipaddress.ip_address(address.split("%", 1)[0])
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped
    return not ip.is_global or ip.is_multicast


async def _checked(address: str, look_up: Lookup, *, inside: bool) -> None:
    """Refuse an address that the tool does not ask, and say why."""
    parts = urlsplit(address)
    if parts.scheme not in ("http", "https"):
        raise ToolFailure(
            f"{address} is no address of the web: it has to begin with http:// or https://.",
            code="INVALID_ARGS",
        )
    host = parts.hostname
    if not host:
        raise ToolFailure(f"{address} names no host.", code="INVALID_ARGS")
    if inside:
        return
    port = parts.port or (443 if parts.scheme == "https" else 80)
    try:
        addresses = await look_up(host, port)
    except OSError as error:
        raise ToolFailure(f"{host} could not be found: {error}", code="NOT_FOUND") from None
    if not addresses or any(_inside(one) for one in addresses):
        raise ToolFailure(
            f"{host} leads to this machine or to a network of its own, which this tool does "
            "not ask.",
            code="DENIED",
        )


def _kind(response: httpx.Response) -> str:
    return response.headers.get("content-type", "").split(";", 1)[0].strip().lower()


def _encoding(response: httpx.Response, body: bytes) -> str:
    """Return the encoding of a page: the one that its header names, or the one that it names."""
    if "charset=" in response.headers.get("content-type", "").lower():
        return response.charset_encoding or "utf-8"
    named = _CHARSET.search(body[:2048])
    if named is not None:
        try:
            return codecs.lookup(named.group(1).decode("ascii")).name
        except LookupError:
            pass
    return "utf-8"


def _textual(kind: str) -> bool:
    return kind == "" or kind.startswith(_TEXTUAL) or kind.endswith(_TEXTUAL_ENDINGS)


async def fetch(
    address: str,
    env: Mapping[str, str],
    *,
    most_bytes: int,
    seconds: float,
    inside: bool = False,
    look_up: Lookup = lookup,
    transport: httpx.AsyncBaseTransport | None = None,
) -> Fetched:
    """Take a page from the web, as text.

    Args:
        address: Where the page is.
        env: The environment of the program, for its proxy.
        most_bytes: How many bytes of the page are taken at most.
        seconds: How long it may take.
        inside: Whether places of this machine and of its own networks may
            be asked.
        look_up: Says where a host name leads.
        transport: The transport of the client, for tests. A request that
            goes to it goes through no proxy.

    Raises:
        ToolFailure: The page was not taken, and the text says why.
    """
    try:
        async with asyncio.timeout(seconds):
            return await _fetched(address, env, most_bytes, inside, look_up, transport)
    except TimeoutError:
        raise ToolFailure(
            f"{address} did not come within {seconds:g} seconds.", code="TIMEOUT"
        ) from None
    except httpx.HTTPError as error:
        raise ToolFailure(
            f"{address} could not be reached: {type(error).__name__}: {error}",
            code="UNREACHABLE",
        ) from None


async def _fetched(
    address: str,
    env: Mapping[str, str],
    most_bytes: int,
    inside: bool,
    look_up: Lookup,
    transport: httpx.AsyncBaseTransport | None,
) -> Fetched:
    for _ in range(_REDIRECTS + 1):
        await _checked(address, look_up, inside=inside)
        async with (
            httpx.AsyncClient(
                headers={
                    "User-Agent": f"Mozilla/5.0 (compatible; Columbina/{columbina.__version__})",
                    "Accept": "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5",
                },
                transport=transport,
                proxy=None if transport is not None else proxy_for(address, env),
                follow_redirects=False,
            ) as client,
            client.stream("GET", address) as response,
        ):
            if response.is_redirect and "location" in response.headers:
                address = urljoin(address, response.headers["location"])
                continue
            if response.status_code >= 400:
                raise ToolFailure(
                    f"{address} answered {response.status_code} {response.reason_phrase}.",
                    code="HTTP_ERROR",
                )
            kind = _kind(response)
            if not _textual(kind):
                raise ToolFailure(
                    f"{address} is {kind}, and this tool takes only text and pages of HTML.",
                    code="NOT_TEXT",
                )
            body = bytearray()
            cut = False
            async for chunk in response.aiter_bytes():
                body += chunk
                if len(body) > most_bytes:
                    del body[most_bytes:]
                    cut = True
                    break
            text = bytes(body).decode(_encoding(response, bytes(body)), errors="replace")
            if kind in _HTML or (kind == "" and "<html" in text[:1000].lower()):
                page = text_of(text, address)
                return Fetched(address, page.title, page.text, cut)
            return Fetched(address, "", text, cut)
    raise ToolFailure(
        f"The page redirected more than {_REDIRECTS} times, and was not followed further.",
        code="TOO_MANY_REDIRECTS",
    )
