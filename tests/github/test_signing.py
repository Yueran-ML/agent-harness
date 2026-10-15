"""Who signed in to GitHub, and what Columbina may reach: ``columbina.github.signing``."""

import base64
import os
import time
from collections.abc import AsyncIterator, Callable
from pathlib import Path

import httpx
import pytest

from columbina import files
from columbina.github import signing, store
from columbina.github.app import GitHubError, app
from columbina.github.client import GitHub
from columbina.github.store import Tokens
from support.fake_github import FakeGitHub, github

__all__ = ["github"]

FIRST = "ghu_first0123456789"
PNG = b"\x89PNG\r\n\x1a\n" + b"\x00" * 64
JPEG = b"\xff\xd8\xff\xe0" + b"\x00" * 64
PICTURE_AT = "https://avatars.githubusercontent.com/u/583231?v=4"
ENV = {"COLUMBINA_GITHUB_CLIENT_ID": "Iv23liTest", "COLUMBINA_GITHUB_SLUG": "test"}
USER = {"login": "octocat", "name": "The Octocat", "avatar_url": PICTURE_AT}

Answer = Callable[[httpx.Request], httpx.Response]


def tokens() -> Tokens:
    return Tokens(FIRST, "ghr_first0123456789", time.time() + 3600, time.time() + 1e6)


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


class Asked:
    """A GitHub of a test: who the user is, the picture, and every request."""

    def __init__(self, user: object = USER, picture: httpx.Response | None = None) -> None:
        self.user = user
        self.picture = picture or httpx.Response(200, content=PNG)
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if request.url.host == "api.github.com" and request.url.path == "/user":
            return httpx.Response(200, json=self.user)
        if request.url.host == "avatars.githubusercontent.com":
            return self.picture
        return httpx.Response(404, json={"message": "Not Found"})

    def to(self, host: str) -> list[httpx.Request]:
        return [request for request in self.requests if request.url.host == host]


async def kept(home: Path, asked: Asked | Answer) -> signing.Who:
    """Keep a sign-in with a GitHub of a test, and return who it is."""
    client = GitHub(home, ENV, transport=httpx.MockTransport(asked))
    try:
        return await signing.keep(client, home, tokens())
    finally:
        await client.aclose()


async def test_a_sign_in_is_kept_with_who_signed_in_and_the_picture(home: Path) -> None:
    asked = Asked()

    found = await kept(home, asked)

    assert found == signing.Who("octocat", "The Octocat")
    said = store.signed(home, app(ENV))
    assert said is not None
    assert (said.login, said.name) == ("octocat", "The Octocat")
    assert (home / store.AVATAR).read_bytes() == PNG
    assert asked.to("api.github.com")[0].headers["authorization"] == f"Bearer {FIRST}"


async def test_the_picture_is_asked_for_small_and_without_the_token(home: Path) -> None:
    asked = Asked()

    await kept(home, asked)

    [picture] = asked.to("avatars.githubusercontent.com")
    assert "authorization" not in picture.headers
    assert picture.url.params["s"] == "128"
    assert picture.url.params["v"] == "4"


async def test_a_user_without_a_name_is_kept_without_one(home: Path) -> None:
    found = await kept(home, Asked(user={"login": "octocat", "name": None}))

    assert found == signing.Who("octocat", None)
    assert not (home / store.AVATAR).exists()


@pytest.mark.parametrize(
    "user", [{}, {"login": ""}, {"login": 7}, ["octocat"]], ids=["none", "empty", "number", "list"]
)
async def test_a_user_that_github_does_not_name_is_no_sign_in(home: Path, user: object) -> None:
    with pytest.raises(GitHubError) as refused:
        await kept(home, Asked(user=user))

    assert refused.value.code == "UNKNOWN"
    assert store.signed(home, app(ENV)) is None


@pytest.mark.parametrize(
    "picture",
    [
        pytest.param(httpx.Response(404), id="not-found"),
        pytest.param(httpx.Response(200, content=b"<html>no picture</html>"), id="no-picture"),
        pytest.param(httpx.Response(200, content=PNG + b"\x00" * (1024 * 1024)), id="too-large"),
    ],
)
async def test_a_picture_that_cannot_be_had_is_not_kept_and_the_sign_in_is(
    home: Path, picture: httpx.Response
) -> None:
    found = await kept(home, Asked(picture=picture))

    assert found.login == "octocat"
    assert not (home / store.AVATAR).exists()


async def test_a_picture_as_large_as_may_be_is_kept(home: Path) -> None:
    content = PNG + b"\x00" * (1024 * 1024 - len(PNG))

    await kept(home, Asked(picture=httpx.Response(200, content=content)))

    assert (home / store.AVATAR).read_bytes() == content


async def test_a_picture_that_cannot_be_reached_is_no_failure(home: Path) -> None:
    asked = Asked()

    def answer(request: httpx.Request) -> httpx.Response:
        if request.url.host == "avatars.githubusercontent.com":
            raise httpx.ConnectError("no way there", request=request)
        return asked(request)

    assert (await kept(home, answer)).login == "octocat"
    assert not (home / store.AVATAR).exists()


async def test_a_picture_at_no_address_is_no_failure(home: Path) -> None:
    found = await kept(home, Asked(user={**USER, "avatar_url": "http://[::1"}))

    assert found.login == "octocat"
    assert not (home / store.AVATAR).exists()


async def test_the_picture_of_whoever_signed_in_before_goes(home: Path) -> None:
    await kept(home, Asked())

    await kept(home, Asked(user={"login": "hubot"}))

    assert not (home / store.AVATAR).exists()


async def test_a_picture_that_cannot_be_written_is_no_failure(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written = files.replaced

    def refused(path: Path, content: str | bytes, *, private: bool = False) -> None:
        if path.name == store.AVATAR:
            raise PermissionError(13, "Permission denied")
        written(path, content, private=private)

    monkeypatch.setattr(files, "replaced", refused)

    assert (await kept(home, Asked())).login == "octocat"
    assert not (home / store.AVATAR).exists()


@pytest.fixture
async def client(github: FakeGitHub, home: Path) -> AsyncIterator[GitHub]:
    made = GitHub(home, os.environ)
    store.keep(home, made.app, tokens(), "octocat", "The Octocat")
    yield made
    await made.aclose()


async def test_who_is_asked_of_github_with_the_tokens_that_are_kept(
    github: FakeGitHub, client: GitHub
) -> None:
    github.user = {"login": "octocat", "name": "Mona"}

    assert await signing.who(client) == signing.Who("octocat", "Mona")


async def test_the_repositories_of_every_installation_are_reached(
    github: FakeGitHub, client: GitHub
) -> None:
    github.installations = [
        {"id": 1, "repositories": [{"full_name": "octocat/hello"}, {"full_name": "octocat/x"}]},
        {"id": 2, "repositories": [{"full_name": "acme/tools"}]},
        {"id": 3, "repositories": []},
    ]

    assert await signing.reach(client) == ["octocat/hello", "octocat/x", "acme/tools"]


async def test_no_installation_reaches_no_repository(github: FakeGitHub, client: GitHub) -> None:
    assert await signing.reach(client) == []


NONE: list[dict[str, object]] = []


@pytest.mark.parametrize(
    "installations",
    [
        pytest.param([{"account": "octocat", "repositories": NONE}], id="no-number"),
        pytest.param([{"id": True, "repositories": NONE}], id="a-truth-for-a-number"),
        pytest.param([{"id": 1, "repositories": [{"name": "hello"}]}], id="no-full-name"),
        pytest.param([{"id": 1, "repositories": ["octocat/hello"]}], id="no-object"),
    ],
)
async def test_what_is_no_list_of_repositories_is_unknown(
    github: FakeGitHub, client: GitHub, installations: list[dict[str, object]]
) -> None:
    github.installations = installations

    with pytest.raises(GitHubError) as refused:
        await signing.reach(client)

    assert refused.value.code == "UNKNOWN"


@pytest.mark.parametrize(
    ("content", "kind"), [(PNG, "image/png"), (JPEG, "image/jpeg")], ids=["png", "jpeg"]
)
def test_the_picture_is_given_as_an_address_of_data(home: Path, content: bytes, kind: str) -> None:
    home.mkdir()
    (home / store.AVATAR).write_bytes(content)

    assert signing.avatar(home) == f"data:{kind};base64,{base64.b64encode(content).decode()}"


@pytest.mark.parametrize(
    "content",
    [b"<svg onload=alert(1)>", PNG + b"\x00" * (1024 * 1024)],
    ids=["no-picture", "too-large"],
)
def test_a_file_that_is_no_picture_that_was_kept_is_not_given(home: Path, content: bytes) -> None:
    home.mkdir()
    (home / store.AVATAR).write_bytes(content)

    assert signing.avatar(home) is None


def test_without_a_picture_none_is_given(home: Path) -> None:
    assert signing.avatar(home) is None


def test_the_right_of_the_app_is_taken_back_on_the_github_where_it_was_given() -> None:
    assert signing.revoke_url("https://github.com") == (
        "https://github.com/settings/apps/authorizations"
    )
    assert signing.revoke_url("https://git.example.com") == (
        "https://git.example.com/settings/apps/authorizations"
    )
