"""Searching the web."""

import json
from collections.abc import Callable

import httpx
import pytest

from columbina.tools import ToolFailure
from columbina.web.search import Engine, Result, search
from support.web import DUCKDUCKGO_PAGE, responding


def served(answer: Callable[[httpx.Request], httpx.Response]) -> httpx.MockTransport:
    return httpx.MockTransport(answer)


async def searched(
    engine: Engine,
    answer: Callable[[httpx.Request], httpx.Response],
    *,
    most: int = 5,
    key: str | None = None,
) -> list[Result]:
    return await search(engine, "python docs", most, key, {}, transport=served(answer))


async def failure_of(
    engine: Engine, answer: Callable[[httpx.Request], httpx.Response]
) -> ToolFailure:
    with pytest.raises(ToolFailure) as caught:
        await searched(engine, answer, key="k")
    return caught.value


async def test_duckduckgo_is_asked_for_its_page_and_its_results_are_read() -> None:
    seen: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, text=DUCKDUCKGO_PAGE)

    results = await searched("duckduckgo", answer)

    assert seen[0].method == "POST"
    assert str(seen[0].url) == "https://html.duckduckgo.com/html/"
    assert seen[0].content == b"q=python+docs"
    assert results == [
        Result("Python & docs", "https://docs.python.org/3/", "The official docs."),
        Result("Round", "https://example.org/a?b=1", ""),
        Result("Third", "https://third.example/", ""),
    ]


async def test_no_more_results_are_given_than_asked_for() -> None:
    answer = responding(200, text=DUCKDUCKGO_PAGE)

    assert len(await searched("duckduckgo", answer, most=1)) == 1


async def test_tavily_is_asked_with_its_key() -> None:
    seen: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(
            200,
            json={
                "results": [
                    {"title": " A ", "url": "https://a.example", "content": "about a"},
                    {"title": "No address", "url": None},
                    "not a result",
                    {"title": "B", "url": "https://b.example", "content": 3},
                ]
            },
        )

    results = await searched("tavily", answer, most=3, key="tvly-1")

    assert str(seen[0].url) == "https://api.tavily.com/search"
    assert seen[0].headers["authorization"] == "Bearer tvly-1"
    assert json.loads(seen[0].content) == {"query": "python docs", "max_results": 3}
    assert results == [
        Result("A", "https://a.example", "about a"),
        Result("B", "https://b.example", ""),
    ]


async def test_brave_is_asked_with_its_key() -> None:
    seen: list[httpx.Request] = []

    def answer(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(
            200,
            json={
                "web": {
                    "results": [
                        {"title": "A", "url": "https://a.example", "description": "<b>a</b>"}
                    ]
                }
            },
        )

    results = await searched("brave", answer, most=2, key="b-1")

    assert seen[0].url.path == "/res/v1/web/search"
    assert dict(seen[0].url.params) == {"q": "python docs", "count": "2"}
    assert seen[0].headers["x-subscription-token"] == "b-1"
    assert results == [Result("A", "https://a.example", "a")]


@pytest.mark.parametrize("said", [{}, {"web": []}, {"web": {"results": {}}}, []])
async def test_an_answer_without_results_has_none(said: object) -> None:
    answer = responding(200, json=said)

    assert await searched("brave", answer, key="k") == []


async def test_an_answer_that_is_no_json_is_said() -> None:
    failure = await failure_of("tavily", responding(200, text="<html>"))

    assert failure.code == "HTTP_ERROR"
    assert str(failure) == "Tavily did not answer in JSON."


@pytest.mark.parametrize("status", [401, 403])
async def test_a_key_that_is_not_taken_is_said(status: int) -> None:
    failure = await failure_of("brave", responding(status))

    assert failure.code == "UNAUTHORIZED"
    assert str(failure) == "Brave did not take the key of the search."


@pytest.mark.parametrize(("engine", "status"), [("tavily", 429), ("duckduckgo", 202)])
async def test_too_many_searches_are_said(engine: Engine, status: int) -> None:
    failure = await failure_of(engine, responding(status))

    assert failure.code == "RATE_LIMITED"


async def test_another_error_of_http_is_said() -> None:
    failure = await failure_of("duckduckgo", responding(403))

    assert failure.code == "HTTP_ERROR"
    assert str(failure) == "DuckDuckGo answered 403 Forbidden."


async def test_a_search_that_cannot_reach_its_service_is_said() -> None:
    def answer(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectTimeout("too slow")

    failure = await failure_of("tavily", answer)

    assert failure.code == "UNREACHABLE"
    assert str(failure) == "The search could not be made: ConnectTimeout: too slow"
