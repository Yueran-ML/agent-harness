"""A token for the API of GitHub that holds: ``columbina.github.session``."""

import asyncio
import os
import socket
from collections.abc import AsyncIterator
from pathlib import Path

import httpx
import pytest

from columbina.github import store
from columbina.github.app import App, GitHubError, app
from columbina.github.session import EARLY, access_token
from columbina.github.store import Tokens
from support.fake_github import FakeGitHub, github

__all__ = ["github"]

NOW = 1_000_000.0
FIRST = "ghu_first0123456789"
RENEWED = "ghu_renewed10123456789"
OTHER = Tokens("ghu_other0123456789", "ghr_other0123456789", NOW + 28_800, NOW + 1e6)
"""A pair that another program renewed."""


@pytest.fixture
async def http() -> AsyncIterator[httpx.AsyncClient]:
    async with httpx.AsyncClient() as client:
        yield client


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


@pytest.fixture
def here(github: FakeGitHub) -> App:
    return app(os.environ)


@pytest.fixture
async def first_read(monkeypatch: pytest.MonkeyPatch) -> asyncio.Event:
    """Set once the tokens were read, as a program reads them before it renews them."""
    read = asyncio.Event()
    loop = asyncio.get_running_loop()
    kept = store.tokens

    def reading(home: Path, here: App) -> Tokens | None:
        found = kept(home, here)
        loop.call_soon_threadsafe(read.set)
        return found

    monkeypatch.setattr(store, "tokens", reading)
    return read


def signed_in(
    home: Path, here: App, *, access_ends: float, renewal_ends: float = NOW + 1e6
) -> None:
    given = Tokens(FIRST, "ghr_first0123456789", access_ends, renewal_ends)
    store.keep(home, here, given, "octocat", "The Octocat")


def at_the_start() -> float:
    return NOW


async def test_a_token_that_holds_is_given_as_it_is(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW + EARLY + 1)

    assert await access_token(home, here, http, now=at_the_start) == FIRST
    assert github.asked == []


async def test_a_token_near_its_end_is_renewed_and_the_new_pair_is_kept(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW + EARLY)

    token = await access_token(home, here, http, now=at_the_start)

    assert token == RENEWED
    assert github.renewals == 1
    kept = store.tokens(home, here)
    assert kept is not None
    assert (kept.access, kept.renewal) == (RENEWED, "ghr_renewed10123456789")
    assert kept.access_expires_at == NOW + 28_800
    said = store.signed(home, here)
    assert said is not None
    assert (said.login, said.name) == ("octocat", "The Octocat")


async def test_a_token_that_github_turned_down_is_renewed_though_it_should_hold(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW + 3600)

    token = await access_token(home, here, http, now=at_the_start, turned_down=FIRST)

    assert token == RENEWED
    assert github.renewals == 1


async def test_a_token_turned_down_that_was_renewed_since_is_not_renewed_again(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW + 3600)
    store.keep(home, here, OTHER, "octocat", "The Octocat")

    token = await access_token(home, here, http, now=at_the_start, turned_down=FIRST)

    assert token == OTHER.access
    assert github.renewals == 0


async def test_two_that_ask_at_once_renew_once_and_get_the_same_token(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW)

    first, second = await asyncio.gather(
        access_token(home, here, http, now=at_the_start),
        access_token(home, here, http, now=at_the_start),
    )

    assert first == second == RENEWED
    assert github.renewals == 1


async def test_a_pair_that_another_program_renewed_while_this_one_waited_is_taken(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient, first_read: asyncio.Event
) -> None:
    signed_in(home, here, access_ends=NOW)

    async with store.renewing(home):
        asking = asyncio.create_task(access_token(home, here, http, now=at_the_start))
        await asyncio.wait_for(first_read.wait(), timeout=10)
        store.keep(home, here, OTHER, "octocat", "The Octocat")

    assert await asking == OTHER.access
    assert github.renewals == 0


async def test_a_sign_out_while_this_program_waited_is_no_sign_in(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient, first_read: asyncio.Event
) -> None:
    signed_in(home, here, access_ends=NOW)

    async with store.renewing(home):
        asking = asyncio.create_task(access_token(home, here, http, now=at_the_start))
        await asyncio.wait_for(first_read.wait(), timeout=10)
        store.forget(home)

    with pytest.raises(GitHubError) as refused:
        await asking
    assert refused.value.code == "SIGNED_OUT"
    assert github.renewals == 0


async def test_a_renewing_token_that_ran_out_ends_the_sign_in(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW - 1, renewal_ends=NOW)

    with pytest.raises(GitHubError) as ended:
        await access_token(home, here, http, now=at_the_start)

    assert ended.value.code == "SIGNED_OUT"
    assert str(ended.value) == "the sign-in to GitHub has run out. Sign in again"
    assert github.asked == []
    assert store.signed(home, here) is None


async def test_a_renewing_token_that_still_holds_renews(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW - 1, renewal_ends=NOW + 1)

    assert await access_token(home, here, http, now=at_the_start) == RENEWED


async def test_a_renewing_token_that_github_turns_down_ends_the_sign_in(
    github: FakeGitHub, home: Path, here: App, http: httpx.AsyncClient
) -> None:
    signed_in(home, here, access_ends=NOW)
    github.renewal.append("ghr_elsewhere0123456789")

    with pytest.raises(GitHubError) as ended:
        await access_token(home, here, http, now=at_the_start)

    assert ended.value.code == "SIGNED_OUT"
    assert store.signed(home, here) is None


async def test_github_that_cannot_be_reached_keeps_the_sign_in(
    home: Path, http: httpx.AsyncClient
) -> None:
    with socket.socket() as closed:
        closed.bind(("127.0.0.1", 0))
        port = closed.getsockname()[1]
    gone = App(
        client_id="Iv23liGone",
        web=f"http://127.0.0.1:{port}",
        api=f"http://127.0.0.1:{port}",
        slug="gone",
    )
    signed_in(home, gone, access_ends=NOW)

    with pytest.raises(GitHubError) as failed:
        await access_token(home, gone, http, now=at_the_start)

    assert failed.value.code == "UNREACHABLE"
    assert store.tokens(home, gone) is not None


async def test_nobody_signed_in_is_told_how_to_sign_in(
    home: Path, here: App, http: httpx.AsyncClient
) -> None:
    with pytest.raises(GitHubError) as refused:
        await access_token(home, here, http)

    assert refused.value.code == "SIGNED_OUT"
    assert "columbina login github" in str(refused.value)
