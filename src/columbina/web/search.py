"""Searching the web: by DuckDuckGo with no key, or by Tavily or Brave with one.

DuckDuckGo has a page of results without scripts for browsers that run
none, and that page is read here. It needs no key and no account, and it is
no API: the page may change, and a search that is made too often is asked to
wait. Tavily and Brave are services with an API for programs, which want a
key and give some searches a month for nothing. Where a key for one of them
is kept, see :mod:`.keys`, it is asked in the place of DuckDuckGo.

A result is a title, an address and a few lines from the page. The model
reads the pages that matter with the tool that fetches one.
"""

import re
from collections.abc import Mapping
from dataclasses import dataclass
from html import unescape
from typing import Any, Literal, cast
from urllib.parse import parse_qs, urlsplit

import httpx

import columbina
from columbina.net import proxy_for
from columbina.tools import ToolFailure

__all__ = ["DUCKDUCKGO", "ENGINES", "Engine", "Result", "search"]

Engine = Literal["duckduckgo", "tavily", "brave"]
"""Who searches."""

ENGINES: Mapping[Engine, str] = {
    "duckduckgo": "https://html.duckduckgo.com/html/",
    "tavily": "https://api.tavily.com/search",
    "brave": "https://api.search.brave.com/res/v1/web/search",
}
"""Where each one is asked."""

DUCKDUCKGO: Engine = "duckduckgo"

_ANCHOR = re.compile(r'<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>', re.DOTALL)
_SNIPPET = re.compile(r'<a[^>]*class="result__snippet"[^>]*>(.*?)</a>', re.DOTALL)
_RESULT = re.compile(r'<div[^>]*class="[^"]*\bresult\b[^"]*"', re.IGNORECASE)
_TAGS = re.compile(r"<[^>]+>")
_SPACES = re.compile(r"\s+")


@dataclass(frozen=True, slots=True)
class Result:
    """One result of a search.

    Attributes:
        title: The title of the page.
        address: Where it is.
        snippet: A few lines from it.
    """

    title: str
    address: str
    snippet: str


def _plain(html: str) -> str:
    return _SPACES.sub(" ", unescape(_TAGS.sub("", html))).strip()


def _address(href: str) -> str:
    """Return where a link of DuckDuckGo leads: some go round by a page of its own."""
    href = unescape(href)
    if href.startswith("//"):
        href = "https:" + href
    parts = urlsplit(href)
    if parts.path == "/l/" and parts.hostname is not None and "duckduckgo" in parts.hostname:
        return parse_qs(parts.query).get("uddg", [href])[0]
    return href


def _of_duckduckgo(page: str, most: int) -> list[Result]:
    found: list[Result] = []
    for block in _RESULT.split(page)[1:]:
        if "result--ad" in block[:200] or "/y.js?" in block:
            continue
        anchor = _ANCHOR.search(block)
        if anchor is None:
            continue
        snippet = _SNIPPET.search(block)
        found.append(
            Result(
                _plain(anchor.group(2)),
                _address(anchor.group(1)),
                "" if snippet is None else _plain(snippet.group(1)),
            )
        )
        if len(found) == most:
            break
    return found


def _field(item: Mapping[str, Any], name: str) -> str:
    value = item.get(name)
    return value.strip() if isinstance(value, str) else ""


def _items(said: object, *path: str) -> list[Mapping[str, Any]]:
    """Return the list that a document of JSON holds where ``path`` leads, or none."""
    for name in path:
        if not isinstance(said, dict):
            return []
        said = cast("dict[str, object]", said).get(name)
    if not isinstance(said, list):
        return []
    return [
        cast("Mapping[str, Any]", item)
        for item in cast("list[object]", said)
        if isinstance(item, dict)
    ]


async def search(
    engine: Engine,
    query: str,
    most: int,
    key: str | None,
    env: Mapping[str, str],
    *,
    seconds: float = 30.0,
    transport: httpx.AsyncBaseTransport | None = None,
) -> list[Result]:
    """Search the web, and return at most ``most`` results.

    Args:
        engine: Who searches.
        query: What for.
        most: How many results at most.
        key: The key of the engine, for one that wants a key.
        env: The environment of the program, for its proxy.
        seconds: How long the search may take.
        transport: The transport of the client, for tests.

    Raises:
        ToolFailure: The search could not be made, and the text says why.
    """
    address = ENGINES[engine]
    async with httpx.AsyncClient(
        timeout=seconds,
        headers={"User-Agent": f"Mozilla/5.0 (compatible; Columbina/{columbina.__version__})"},
        transport=transport,
        proxy=None if transport is not None else proxy_for(address, env),
    ) as client:
        try:
            if engine == "duckduckgo":
                response = await client.post(address, data={"q": query})
            elif engine == "tavily":
                response = await client.post(
                    address,
                    json={"query": query, "max_results": most},
                    headers={"Authorization": f"Bearer {key}"},
                )
            else:
                response = await client.get(
                    address,
                    params={"q": query, "count": str(most)},
                    headers={"X-Subscription-Token": key or "", "Accept": "application/json"},
                )
        except httpx.HTTPError as error:
            raise ToolFailure(
                f"The search could not be made: {type(error).__name__}: {error}",
                code="UNREACHABLE",
            ) from None
    title = engine.capitalize() if engine != "duckduckgo" else "DuckDuckGo"
    if response.status_code in (401, 403) and engine != "duckduckgo":
        raise ToolFailure(f"{title} did not take the key of the search.", code="UNAUTHORIZED")
    if response.status_code == 429 or (engine == "duckduckgo" and response.status_code == 202):
        raise ToolFailure(
            f"{title} has had too many searches for a while. Wait before searching again.",
            code="RATE_LIMITED",
        )
    if response.status_code >= 400:
        raise ToolFailure(
            f"{title} answered {response.status_code} {response.reason_phrase}.",
            code="HTTP_ERROR",
        )
    if engine == "duckduckgo":
        return _of_duckduckgo(response.text, most)
    try:
        said: object = response.json()
    except ValueError:
        raise ToolFailure(f"{title} did not answer in JSON.", code="HTTP_ERROR") from None
    if engine == "tavily":
        items = _items(said, "results")
        found = [Result(_field(i, "title"), _field(i, "url"), _field(i, "content")) for i in items]
    else:
        items = _items(said, "web", "results")
        found = [
            Result(_field(i, "title"), _field(i, "url"), _plain(_field(i, "description")))
            for i in items
        ]
    return [one for one in found if one.address][:most]
