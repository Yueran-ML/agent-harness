"""The REST API of GitHub, asked in the name of the user: ``columbina.github.client``."""

import os
import socket
import time
from collections.abc import AsyncIterator, Callable
from pathlib import Path
from typing import Any

import httpx
import pytest

import columbina
from columbina.github import client as github_client
from columbina.github import store
from columbina.github.app import App, GitHubError
from columbina.github.client import GitHub
from columbina.github.store import Tokens
from support.fake_github import CLIENT_ID, SLUG, FakeGitHub, github

__all__ = ["github"]

FIRST = "ghu_first0123456789"
RENEWED = "ghu_renewed10123456789"
NOW = 1_000_000.0
APP_ENV = {"COLUMBINA_GITHUB_CLIENT_ID": CLIENT_ID, "COLUMBINA_GITHUB_SLUG": SLUG}
"""An app on the GitHub of the Internet, which a transport of a test stands in for."""


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def signed_in(home: Path, here: App, *, access_ends: float | None = None) -> None:
    ends = time.time() + 3600 if access_ends is None else access_ends
    given = Tokens(FIRST, "ghr_first0123456789", ends, time.time() + 1e6)
    store.keep(home, here, given, "octocat", "The Octocat")


@pytest.fixture
async def client(github: FakeGitHub, home: Path) -> AsyncIterator[GitHub]:
    made = GitHub(home, os.environ)
    signed_in(home, made.app)
    yield made
    await made.aclose()


def to_the_api(github: FakeGitHub) -> list[str]:
    """The paths that the API was asked for, without the sign-in."""
    return [asked.path for asked in github.asked if not asked.path.startswith("/login/")]


async def test_a_request_goes_with_the_token_and_the_version_of_the_api(
    github: FakeGitHub, client: GitHub
) -> None:
    assert await client.get("/user") == github.user

    headers = github.asked[-1].headers
    assert headers["authorization"] == f"Bearer {FIRST}"
    assert headers["x-github-api-version"] == "2022-11-28"
    assert headers["accept"] == "application/vnd.github+json"
    assert headers["user-agent"] == f"Columbina/{columbina.__version__}"


async def test_a_token_near_its_end_is_renewed_before_the_request(
    github: FakeGitHub, home: Path, client: GitHub
) -> None:
    signed_in(home, client.app, access_ends=time.time())

    await client.get("/user")

    assert github.renewals == 1
    assert github.asked[-1].headers["authorization"] == f"Bearer {RENEWED}"


async def test_a_token_turned_down_is_renewed_and_the_request_made_once_more(
    github: FakeGitHub, client: GitHub
) -> None:
    github.access.remove(FIRST)

    assert await client.get("/user") == github.user

    assert github.renewals == 1
    assert to_the_api(github) == ["/user", "/user"]
    assert github.asked[-1].headers["authorization"] == f"Bearer {RENEWED}"


async def test_a_token_turned_down_twice_is_a_sign_in_that_github_does_not_take(
    github: FakeGitHub, client: GitHub, monkeypatch: pytest.MonkeyPatch
) -> None:
    def refusing(token: str) -> bool:
        return False

    monkeypatch.setattr(github, "holding", refusing)

    with pytest.raises(GitHubError) as refused:
        await client.get("/user")

    assert refused.value.code == "SIGNED_OUT"
    assert "Sign in again" in str(refused.value)
    assert github.renewals == 1
    assert to_the_api(github) == ["/user", "/user"]


async def test_a_token_that_is_given_is_asked_with_and_never_renewed(
    github: FakeGitHub, home: Path
) -> None:
    made = GitHub(home, os.environ)
    try:
        github.access.append("ghu_given0123456789")
        assert await made.get("/user", token="ghu_given0123456789") == github.user
        with pytest.raises(GitHubError) as refused:
            await made.get("/user", token="ghu_unknown0123456789")
    finally:
        await made.aclose()

    assert refused.value.code == "SIGNED_OUT"
    assert github.renewals == 0
    assert store.signed(home, made.app) is None


async def test_the_token_for_git_holds(github: FakeGitHub, home: Path, client: GitHub) -> None:
    assert await client.token() == FIRST

    signed_in(home, client.app, access_ends=time.time())
    assert await client.token() == RENEWED


@pytest.mark.parametrize(
    ("status", "body", "headers", "code", "words"),
    [
        pytest.param(
            403,
            {"message": "API rate limit exceeded"},
            {"x-ratelimit-remaining": "0", "x-ratelimit-reset": str(int(NOW) + 150)},
            "RATE_LIMITED",
            "GitHub asks for a pause: API rate limit exceeded. Try again in 3 minutes",
            id="limit-with-reset",
        ),
        pytest.param(
            429,
            {"message": "secondary rate limit"},
            {"retry-after": "30"},
            "RATE_LIMITED",
            "GitHub asks for a pause: secondary rate limit. Try again in a minute",
            id="retry-after",
        ),
        pytest.param(
            429,
            {"message": "slow down"},
            {"retry-after": "60"},
            "RATE_LIMITED",
            "Try again in a minute",
            id="retry-after-a-minute",
        ),
        pytest.param(
            429,
            {},
            {"retry-after": "61"},
            "RATE_LIMITED",
            "GitHub asks for a pause: HTTP 429. Try again in 2 minutes",
            id="retry-after-past-a-minute",
        ),
        pytest.param(
            429,
            {"message": "slow down"},
            {},
            "RATE_LIMITED",
            "Try again in a few minutes",
            id="limit-without-a-time",
        ),
        pytest.param(
            403,
            {"message": "Resource not accessible by integration"},
            {"x-ratelimit-remaining": "4999"},
            "FORBIDDEN",
            "GitHub does not allow it: Resource not accessible by integration",
            id="forbidden",
        ),
        pytest.param(
            404,
            {"message": "Not Found"},
            {},
            "NOT_FOUND",
            "GitHub has nothing there that Columbina may see: Not Found. A repository of yours "
            "may not be chosen for Columbina yet: ",
            id="not-found",
        ),
        pytest.param(
            422,
            {"message": "Validation Failed", "errors": [{"field": "head", "code": "invalid"}]},
            {},
            "INVALID",
            "GitHub does not take it: Validation Failed. [{'field': 'head', 'code': 'invalid'}]",
            id="invalid",
        ),
        pytest.param(
            422,
            {"message": "Validation Failed"},
            {},
            "INVALID",
            "GitHub does not take it: Validation Failed.",
            id="invalid-without-errors",
        ),
        pytest.param(
            502,
            b"<html>Bad Gateway</html>",
            {"Content-Type": "text/html"},
            "BUSY",
            "GitHub answered with HTTP 502. Try again in a moment",
            id="busy",
        ),
        pytest.param(
            418,
            {"message": "I'm a teapot"},
            {},
            "UNKNOWN",
            "GitHub answered with HTTP 418: I'm a teapot",
            id="other",
        ),
        pytest.param(
            409,
            b"",
            {},
            "UNKNOWN",
            "GitHub answered with HTTP 409: HTTP 409",
            id="no-message",
        ),
    ],
)
async def test_what_github_refuses_is_said_with_a_code(
    github: FakeGitHub,
    home: Path,
    status: int,
    body: Any,
    headers: dict[str, str],
    code: str,
    words: str,
) -> None:
    made = GitHub(home, os.environ, now=lambda: NOW)
    signed_in(home, made.app)
    github.fails_next(status, body, headers)
    try:
        with pytest.raises(GitHubError) as refused:
            await made.get("/repos/octocat/hello")
    finally:
        await made.aclose()

    assert refused.value.code == code
    assert words in str(refused.value)
    if code == "NOT_FOUND":
        assert str(refused.value).endswith(made.app.install_url)


async def test_no_token_is_said_in_an_error(github: FakeGitHub, client: GitHub) -> None:
    github.fails_next(422, {"message": f"bad {FIRST}", "errors": [{"value": f'"{FIRST}"'}]})

    with pytest.raises(GitHubError) as refused:
        await client.post("/repos/octocat/hello/pulls", {"title": "x"})

    assert FIRST not in str(refused.value)
    assert "[token]" in str(refused.value)


async def test_an_answer_that_is_no_json_is_unknown(github: FakeGitHub, client: GitHub) -> None:
    github.fails_next(200, b"<html>a page of a proxy</html>", {"Content-Type": "text/html"})

    with pytest.raises(GitHubError) as refused:
        await client.get("/user")

    assert refused.value.code == "UNKNOWN"
    assert "HTTP 200" in str(refused.value)


async def test_an_answer_without_content_is_none(github: FakeGitHub, client: GitHub) -> None:
    github.fails_next(204, b"", {})

    assert await client.get("/user") is None


async def test_github_that_cannot_be_reached_is_said_to_be_so(home: Path) -> None:
    with socket.socket() as closed:
        closed.bind(("127.0.0.1", 0))
        port = closed.getsockname()[1]
    env = {
        **APP_ENV,
        "COLUMBINA_GITHUB_WEB": f"http://127.0.0.1:{port}",
        "COLUMBINA_GITHUB_API": f"http://127.0.0.1:{port}",
    }
    made = GitHub(home, env)
    signed_in(home, made.app)
    try:
        with pytest.raises(GitHubError) as refused:
            await made.get("/user")
    finally:
        await made.aclose()

    assert refused.value.code == "UNREACHABLE"
    assert str(refused.value).endswith(
        "If GitHub is reached through a proxy, see that the proxy is on"
    )


async def test_github_that_does_not_answer_in_time_is_said_to_be_so(home: Path) -> None:
    def slow(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("no answer", request=request)

    made = GitHub(home, APP_ENV, transport=httpx.MockTransport(slow))
    signed_in(home, made.app)
    try:
        with pytest.raises(GitHubError) as refused:
            await made.get("/user")
    finally:
        await made.aclose()

    assert refused.value.code == "TIMEOUT"


async def test_a_program_without_an_app_says_so_where_github_is_asked(
    unregistered: None, home: Path
) -> None:
    made = GitHub(home, {})
    try:
        with pytest.raises(GitHubError) as refused:
            await made.get("/user")
    finally:
        await made.aclose()

    assert refused.value.code == "NOT_REGISTERED"


Pages = Callable[[httpx.Request], httpx.Response]


def paged(total: int, *, size: int = 100, elsewhere: bool = False) -> tuple[Pages, list[str]]:
    """A list of ``total`` repositories in pages, and the addresses that were asked for."""
    asked: list[str] = []

    def answer(request: httpx.Request) -> httpx.Response:
        asked.append(str(request.url))
        page = int(request.url.params.get("page", "1"))
        items = [{"id": n} for n in range((page - 1) * size, min(page * size, total))]
        headers: dict[str, str] = {}
        if page * size < total:
            where = "https://elsewhere.example" if elsewhere else "https://api.github.com"
            headers["link"] = (
                f'<{where}/user/repos?per_page={size}&page={page + 1}>; rel="next", '
                f'<{where}/user/repos?per_page={size}&page=9>; rel="last"'
            )
        return httpx.Response(200, json={"repositories": items}, headers=headers)

    return answer, asked


async def test_the_pages_of_a_list_are_followed_up_to_the_most_that_is_asked_for(
    home: Path,
) -> None:
    answer, asked = paged(1000)
    made = GitHub(home, APP_ENV, transport=httpx.MockTransport(answer))
    signed_in(home, made.app)
    try:
        found = await made.pages("/user/repos", key="repositories", most=150)
    finally:
        await made.aclose()

    assert [item["id"] for item in found] == list(range(150))
    assert asked == [
        "https://api.github.com/user/repos?per_page=100",
        "https://api.github.com/user/repos?per_page=100&page=2",
    ]


async def test_a_list_shorter_than_a_page_is_read_in_one(home: Path) -> None:
    answer, asked = paged(7, size=5)
    made = GitHub(home, APP_ENV, transport=httpx.MockTransport(answer))
    signed_in(home, made.app)
    try:
        found = await made.pages("/user/repos", key="repositories", most=5)
    finally:
        await made.aclose()

    assert len(found) == 5
    assert asked == ["https://api.github.com/user/repos?per_page=5"]


async def test_a_list_that_comes_as_it_is_is_read(github: FakeGitHub, client: GitHub) -> None:
    github.comments["octocat/hello#1"] = [{"id": 1}, {"id": 2}]

    assert await client.pages("/repos/octocat/hello/issues/1/comments") == [{"id": 1}, {"id": 2}]


async def test_a_page_elsewhere_is_not_asked_for(home: Path) -> None:
    answer, asked = paged(300, elsewhere=True)
    made = GitHub(home, APP_ENV, transport=httpx.MockTransport(answer))
    signed_in(home, made.app)
    try:
        with pytest.raises(GitHubError) as refused:
            await made.pages("/user/repos", key="repositories")
    finally:
        await made.aclose()

    assert refused.value.code == "UNKNOWN"
    assert "https://elsewhere.example" in str(refused.value)
    assert asked == ["https://api.github.com/user/repos?per_page=100"]


async def test_a_page_without_its_list_is_unknown(github: FakeGitHub, client: GitHub) -> None:
    with pytest.raises(GitHubError) as refused:
        await client.pages("/user", key="repositories")

    assert refused.value.code == "UNKNOWN"


async def test_the_requests_go_through_the_proxy_of_the_system(
    monkeypatch: pytest.MonkeyPatch, home: Path
) -> None:
    asked: list[str] = []

    def proxy_for(url: str, env: object) -> str:
        asked.append(url)
        return "http://127.0.0.1:7890"

    made: list[dict[str, Any]] = []
    real = httpx.AsyncClient

    def http(**given: Any) -> httpx.AsyncClient:
        made.append(given)
        return real(**given)

    monkeypatch.setattr(github_client, "proxy_for", proxy_for)
    monkeypatch.setattr(httpx, "AsyncClient", http)

    await GitHub(home, APP_ENV).aclose()
    await GitHub(home, APP_ENV, transport=httpx.MockTransport(paged(0)[0])).aclose()

    assert asked == ["https://api.github.com"]
    assert [given["proxy"] for given in made] == ["http://127.0.0.1:7890", None]
