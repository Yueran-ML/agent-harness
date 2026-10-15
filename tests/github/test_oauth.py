"""The tokens of GitHub, from the device flow and renewed: ``columbina.github.oauth``."""

import os
import socket
from collections.abc import AsyncIterator
from typing import Any

import httpx
import pytest

from columbina.github.app import App, GitHubError, app
from columbina.github.oauth import Code, ask_for_a_code, renewed, wait_for_the_user
from columbina.github.store import Tokens
from support.fake_github import CLIENT_ID, FakeGitHub, github

__all__ = ["github"]

NOW = 1_000_000.0
PENDING = {"error": "authorization_pending"}


@pytest.fixture
async def http() -> AsyncIterator[httpx.AsyncClient]:
    async with httpx.AsyncClient() as client:
        yield client


@pytest.fixture
def here(github: FakeGitHub) -> App:
    return app(os.environ)


class Waits:
    """Stands in for the sleep between two asks, and writes down how long each was."""

    def __init__(self) -> None:
        self.seconds: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.seconds.append(seconds)


def code(**changed: Any) -> Code:
    given: dict[str, Any] = {
        "user_code": "WDJB-MJHT",
        "verification_uri": "https://github.com/login/device",
        "expires_at": NOW + 900,
        "interval": 5.0,
        "device_code": "device-code-0123",
    }
    return Code(**(given | changed))


async def test_a_code_is_asked_for_with_the_client_id_and_for_a_token_that_renews(
    github: FakeGitHub, here: App, http: httpx.AsyncClient
) -> None:
    given = await ask_for_a_code(http, here, now=lambda: NOW)

    assert given == code(expires_at=NOW + 899)
    asked = github.asked[0]
    assert (asked.method, asked.path) == ("POST", "/login/device/code")
    assert asked.form == {"client_id": CLIENT_ID, "scope": "offline_access"}
    assert asked.headers["accept"] == "application/json"


async def test_the_user_is_waited_for_and_more_slowly_when_github_asks_for_it(
    github: FakeGitHub, here: App, http: httpx.AsyncClient
) -> None:
    github.polls = [
        PENDING,
        {"error": "slow_down", "interval": 10},
        PENDING,
        {"error": "slow_down"},
        None,
    ]
    waits = Waits()

    tokens = await wait_for_the_user(http, here, code(), sleep=waits, now=lambda: NOW)

    assert waits.seconds == [5.0, 5.0, 10.0, 10.0, 15.0]
    assert tokens == Tokens(
        access="ghu_first0123456789",
        renewal="ghr_first0123456789",
        access_expires_at=NOW + 28_800,
        renewal_expires_at=NOW + 15_897_600,
    )
    polled = [asked.form for asked in github.asked]
    assert (
        polled
        == [
            {
                "client_id": CLIENT_ID,
                "device_code": "device-code-0123",
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }
        ]
        * 5
    )


@pytest.mark.parametrize(
    ("said", "kind"),
    [
        ({"error": "expired_token"}, "EXPIRED"),
        ({"error": "access_denied"}, "DENIED"),
        (
            {"error": "device_flow_disabled", "error_description": "Device Flow must be enabled"},
            "MISCONFIGURED",
        ),
        ({"error": "incorrect_client_credentials"}, "MISCONFIGURED"),
        ({"error": "incorrect_device_code"}, "MISCONFIGURED"),
        ({"error": "unsupported_grant_type"}, "MISCONFIGURED"),
        ({"error": "something_new", "error_description": "Who knows"}, "UNKNOWN"),
    ],
)
async def test_what_github_says_ends_the_wait_as_what_it_means(
    github: FakeGitHub, here: App, http: httpx.AsyncClient, said: dict[str, Any], kind: str
) -> None:
    github.polls = [said]

    with pytest.raises(GitHubError) as ended:
        await wait_for_the_user(http, here, code(), sleep=Waits(), now=lambda: NOW)

    assert ended.value.code == kind
    if "error_description" in said:
        assert said["error_description"] in str(ended.value)


async def test_a_code_that_has_run_out_by_the_clock_is_asked_with_no_more(
    github: FakeGitHub, here: App, http: httpx.AsyncClient
) -> None:
    github.polls = [PENDING]
    moments = iter([NOW, NOW + 600, NOW + 900])

    with pytest.raises(GitHubError) as ended:
        await wait_for_the_user(http, here, code(), sleep=Waits(), now=lambda: next(moments))

    assert ended.value.code == "EXPIRED"
    assert len(github.asked) == 2


@pytest.mark.parametrize("renewal", [{}, {"refresh_token": ""}], ids=["none", "empty"])
async def test_without_a_token_that_renews_the_app_is_set_up_wrong(
    github: FakeGitHub, here: App, http: httpx.AsyncClient, renewal: dict[str, str]
) -> None:
    github.polls = [
        {"access_token": "ghu_x", "expires_in": 28_800, "token_type": "bearer", **renewal}
    ]

    with pytest.raises(GitHubError) as ended:
        await wait_for_the_user(http, here, code(), sleep=Waits(), now=lambda: NOW)

    assert ended.value.code == "MISCONFIGURED"
    assert "expire" in str(ended.value)


@pytest.mark.parametrize(
    "said",
    [
        {"refresh_token": "ghr_x", "expires_in": 1, "refresh_token_expires_in": 1},
        {
            "access_token": "",
            "refresh_token": "ghr_x",
            "expires_in": 1,
            "refresh_token_expires_in": 1,
        },
        {
            "access_token": "ghu_x",
            "refresh_token": "ghr_x",
            "expires_in": "soon",
            "refresh_token_expires_in": 1,
        },
        {
            "access_token": "ghu_x",
            "refresh_token": "ghr_x",
            "expires_in": True,
            "refresh_token_expires_in": 1,
        },
        {"access_token": "ghu_x", "refresh_token": "ghr_x", "expires_in": 1},
    ],
)
async def test_tokens_without_what_belongs_to_them_are_none(
    github: FakeGitHub, here: App, http: httpx.AsyncClient, said: dict[str, Any]
) -> None:
    github.polls = [said]

    with pytest.raises(GitHubError) as ended:
        await wait_for_the_user(http, here, code(), sleep=Waits(), now=lambda: NOW)

    assert ended.value.code == "UNKNOWN"


@pytest.mark.parametrize(
    "page",
    [
        "file:///C:/Windows/System32/calc.exe",
        "javascript:alert(1)",
        "http://github.com/login/device",
        "github.com/login/device",
    ],
)
async def test_a_page_to_enter_the_code_on_that_is_no_page_of_the_web_is_refused(
    github: FakeGitHub, here: App, http: httpx.AsyncClient, page: str
) -> None:
    github.device = {
        "device_code": "d",
        "user_code": "WDJB-MJHT",
        "verification_uri": page,
        "expires_in": 899,
        "interval": 5,
    }

    with pytest.raises(GitHubError) as refused:
        await ask_for_a_code(http, here)

    assert refused.value.code == "UNKNOWN"
    assert "https://" in str(refused.value)


@pytest.mark.parametrize(
    "said",
    [
        {"error": "Not Found"},
        {
            "device_code": "d",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 9,
        },
    ],
)
async def test_a_code_that_github_does_not_give_is_none(
    github: FakeGitHub, here: App, http: httpx.AsyncClient, said: dict[str, Any]
) -> None:
    github.device = said

    with pytest.raises(GitHubError) as refused:
        await ask_for_a_code(http, here)

    assert refused.value.code == "UNKNOWN"


async def test_the_tokens_are_renewed_and_the_old_renewing_token_is_good_no_more(
    github: FakeGitHub, here: App, http: httpx.AsyncClient
) -> None:
    first = await renewed(http, here, "ghr_first0123456789", now=lambda: NOW)

    assert first == Tokens(
        access="ghu_renewed10123456789",
        renewal="ghr_renewed10123456789",
        access_expires_at=NOW + 28_800,
        renewal_expires_at=NOW + 15_897_600,
    )
    assert github.asked[-1].form == {
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": "ghr_first0123456789",
    }
    with pytest.raises(GitHubError) as ended:
        await renewed(http, here, "ghr_first0123456789")
    assert ended.value.code == "SIGNED_OUT"


async def test_no_secret_of_the_app_is_ever_sent(
    github: FakeGitHub, here: App, http: httpx.AsyncClient
) -> None:
    given = await ask_for_a_code(http, here)
    await wait_for_the_user(http, here, given, sleep=Waits())
    await renewed(http, here, "ghr_first0123456789")

    assert all("client_secret" not in asked.form for asked in github.asked)


async def test_github_that_cannot_be_reached_is_said_to_be_so_with_a_word_on_proxies(
    http: httpx.AsyncClient,
) -> None:
    with socket.socket() as closed:
        closed.bind(("127.0.0.1", 0))
        port = closed.getsockname()[1]
    gone = App(
        client_id="x", web=f"http://127.0.0.1:{port}", api=f"http://127.0.0.1:{port}", slug="x"
    )

    with pytest.raises(GitHubError) as refused:
        await ask_for_a_code(http, gone)

    assert refused.value.code == "UNREACHABLE"
    assert str(refused.value).endswith(
        "If GitHub is reached through a proxy, see that the proxy is on"
    )


def answering(handler: Any) -> httpx.AsyncClient:
    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


async def test_github_that_does_not_answer_in_time_is_said_to_be_so(here: App) -> None:
    def slow(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("no answer", request=request)

    async with answering(slow) as http:
        with pytest.raises(GitHubError) as refused:
            await ask_for_a_code(http, here)

    assert refused.value.code == "TIMEOUT"


@pytest.mark.parametrize(
    ("status", "content", "kind"),
    [
        (502, b"Bad Gateway", "BUSY"),
        (500, b"{}", "BUSY"),
        (200, b"<html>not JSON</html>", "UNKNOWN"),
        (200, b"[]", "UNKNOWN"),
    ],
)
async def test_an_answer_that_cannot_be_read_is_said_to_be_so(
    here: App, status: int, content: bytes, kind: str
) -> None:
    def answer(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status, content=content)

    async with answering(answer) as http:
        with pytest.raises(GitHubError) as refused:
            await ask_for_a_code(http, here)

    assert refused.value.code == kind
