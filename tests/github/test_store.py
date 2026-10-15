"""Where the sign-in to GitHub is kept: ``columbina.github.store``."""

import asyncio
import subprocess
import sys
from pathlib import Path

import pytest

from columbina.github.app import App, GitHubError
from columbina.github.store import (
    AVATAR,
    CREDENTIALS,
    LOCK,
    SIGNED,
    Signed,
    Tokens,
    forget,
    keep,
    renewing,
    signed,
    tokens,
)
from columbina.llm import credentials
from columbina.llm.credentials import key_for, origin_of
from support.processes import python
from support.vault import Memory

POSIX = sys.platform != "win32"
APP = App(
    client_id="Iv23liTest", web="https://github.com", api="https://api.github.com", slug="test"
)
OTHER_APP = App(
    client_id="Iv23liOther", web="https://github.com", api="https://api.github.com", slug="other"
)
TOKENS = Tokens("ghu_access0123456789", "ghr_renewal0123456789", 1000.0, 2000.0)


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def signed_in(home: Path, app: App = APP, given: Tokens = TOKENS) -> None:
    keep(home, app, given, "octocat", "The Octocat")


def nowhere_in_the_files_of(home: Path) -> bool:
    """Whether no token lies in any file of the home."""
    return not any(
        token.encode() in found.read_bytes()
        for found in home.iterdir()
        if found.is_file()
        for token in (TOKENS.access, TOKENS.renewal)
    )


def test_a_sign_in_is_kept_and_read_again(home: Path) -> None:
    signed_in(home)

    assert tokens(home, APP) == TOKENS
    assert signed(home, APP) == Signed(
        client_id="Iv23liTest",
        login="octocat",
        name="The Octocat",
        access_expires_at=1000.0,
        renewal_expires_at=2000.0,
    )


def test_the_tokens_lie_in_the_keyring_under_a_file_of_their_own_and_in_no_file(
    home: Path,
) -> None:
    vault = Memory().install()

    signed_in(home)

    filed_under = f"columbina {(home / CREDENTIALS).resolve()}"
    assert sorted(vault.held) == [(filed_under, "access"), (filed_under, "refresh")]
    assert nowhere_in_the_files_of(home)
    assert credentials.stored(home / CREDENTIALS) == {
        "access": "https://api.github.com",
        "refresh": "https://github.com",
    }


def test_where_no_keyring_takes_them_the_tokens_lie_in_their_own_file(home: Path) -> None:
    signed_in(home)

    assert TOKENS.access.encode() in (home / CREDENTIALS).read_bytes()
    assert TOKENS.access.encode() not in (home / SIGNED).read_bytes()
    assert not (home / "credentials.json").exists()
    if POSIX:
        assert (home / CREDENTIALS).stat().st_mode & 0o777 == 0o600
        assert (home / SIGNED).stat().st_mode & 0o777 == 0o600


def test_each_token_is_handed_out_only_for_its_own_place(home: Path) -> None:
    signed_in(home)

    assert key_for(home / CREDENTIALS, "access", origin_of(APP.api)) == TOKENS.access
    with pytest.raises(credentials.CredentialsError):
        key_for(home / CREDENTIALS, "access", origin_of(APP.web))


def test_the_tokens_of_github_are_apart_from_the_keys_of_models(home: Path) -> None:
    credentials.keep(home / "credentials.json", "github", "sk-model-0123", "https://models.test")

    signed_in(home)
    credentials.forget(home / "credentials.json", "github")

    assert tokens(home, APP) == TOKENS
    assert credentials.stored(home / "credentials.json") == {}


def test_a_sign_in_with_another_app_is_none(home: Path) -> None:
    signed_in(home)

    assert signed(home, OTHER_APP) is None
    assert tokens(home, OTHER_APP) is None


def test_nobody_has_signed_in_where_nothing_is_kept(home: Path) -> None:
    assert signed(home, APP) is None
    assert tokens(home, APP) is None


@pytest.mark.parametrize("text", ["", "{}", "[]", '{"version": 2}', "not JSON"])
def test_a_file_that_this_program_did_not_write_is_no_sign_in(home: Path, text: str) -> None:
    signed_in(home)
    (home / SIGNED).write_text(text, encoding="utf-8")

    assert signed(home, APP) is None
    assert tokens(home, APP) is None


def test_tokens_without_the_file_that_says_who_signed_in_are_no_sign_in(home: Path) -> None:
    signed_in(home)
    (home / SIGNED).unlink()

    assert tokens(home, APP) is None


def test_a_token_that_was_taken_away_ends_the_sign_in(home: Path) -> None:
    signed_in(home)
    credentials.forget(home / CREDENTIALS, "refresh")

    assert tokens(home, APP) is None


def test_a_file_that_cannot_be_read_is_said_to_be_so(home: Path) -> None:
    (home / SIGNED).mkdir(parents=True)

    with pytest.raises(GitHubError) as refused:
        signed(home, APP)

    assert refused.value.code == "STORE"
    assert str(home / SIGNED) in str(refused.value)


def test_a_file_that_is_no_text_is_said_to_be_so(home: Path) -> None:
    home.mkdir()
    (home / SIGNED).write_bytes(b"\xff\xfe\x00")

    with pytest.raises(GitHubError, match="it is not text"):
        signed(home, APP)


def test_tokens_that_cannot_be_read_are_said_to_be_unusable(home: Path) -> None:
    signed_in(home)
    (home / CREDENTIALS).write_text("not what is written there", encoding="utf-8")

    with pytest.raises(GitHubError) as refused:
        tokens(home, APP)

    assert refused.value.code == "STORE"
    assert str(refused.value).startswith("the tokens of GitHub cannot be used: ")


def test_a_token_that_cannot_be_one_is_not_kept(home: Path) -> None:
    with pytest.raises(GitHubError) as refused:
        keep(home, APP, Tokens("two halves", "ghr_x", 1.0, 2.0), "octocat", None)

    assert refused.value.code == "STORE"
    assert "halves" not in str(refused.value)
    assert not (home / SIGNED).exists()


def test_who_signed_in_that_cannot_be_written_down_is_said_to_be_so(home: Path) -> None:
    (home / SIGNED / "inside").mkdir(parents=True)

    with pytest.raises(GitHubError) as refused:
        signed_in(home)

    assert refused.value.code == "STORE"
    assert str(home / SIGNED) in str(refused.value)


def test_forgetting_deletes_the_tokens_who_signed_in_and_the_picture(home: Path) -> None:
    vault = Memory().install()
    signed_in(home)
    (home / AVATAR).write_bytes(b"\x89PNG")

    assert forget(home) is True

    assert vault.held == {}
    assert sorted(found.name for found in home.iterdir()) == [CREDENTIALS]
    assert tokens(home, APP) is None
    assert forget(home) is False


def test_forgetting_tokens_alone_is_forgetting_something(home: Path) -> None:
    vault = Memory().install()
    signed_in(home)
    (home / SIGNED).unlink()

    assert forget(home) is True

    assert vault.held == {}


def test_forgetting_a_picture_alone_is_forgetting_something(home: Path) -> None:
    home.mkdir()
    (home / AVATAR).write_bytes(b"\x89PNG")

    assert forget(home) is True


def test_a_token_that_its_keyring_does_not_give_up_is_said_to_be_so(home: Path) -> None:
    vault = Memory().install()
    signed_in(home)
    vault.locked = True

    with pytest.raises(GitHubError) as refused:
        forget(home)

    assert refused.value.code == "STORE"
    assert (home / SIGNED).exists()


# Renewing


async def test_whoever_renews_holds_the_lock_and_another_waits_for_it(home: Path) -> None:
    order: list[str] = []

    async def first() -> None:
        async with renewing(home):
            order.append("first in")
            await asyncio.sleep(0.2)
            order.append("first out")

    async def second() -> None:
        await asyncio.sleep(0.05)
        async with renewing(home, pause=0.01):
            order.append("second in")

    await asyncio.gather(first(), second())

    assert order == ["first in", "first out", "second in"]
    assert (home / LOCK).exists()


async def test_a_lock_that_is_held_too_long_is_given_up_on(home: Path) -> None:
    async with renewing(home):
        with pytest.raises(GitHubError) as refused:
            async with renewing(home, seconds=0.1, pause=0.01):
                pass

    assert refused.value.code == "BUSY"
    assert "0.1 seconds" in str(refused.value)


def holding(lock: Path) -> "subprocess.Popen[str]":
    """Start another program that takes the lock and keeps it, and wait until it has it."""
    holds = (
        "import sys, time\n"
        "from pathlib import Path\n"
        "from columbina.session.lock import acquire\n"
        "held = acquire(Path(sys.argv[1]))\n"
        "print('held' if held else 'not held', flush=True)\n"
        "time.sleep(60)\n"
    )
    other = subprocess.Popen(python(holds, str(lock)), stdout=subprocess.PIPE, text=True)
    assert other.stdout is not None
    assert other.stdout.readline().strip() == "held"
    return other


async def test_the_lock_of_another_program_is_waited_for_and_gone_when_it_ends(
    home: Path,
) -> None:
    home.mkdir()
    other = holding(home / LOCK)
    try:
        with pytest.raises(GitHubError):
            async with renewing(home, seconds=0.2, pause=0.02):
                pass
    finally:
        other.kill()
        other.wait(timeout=30)
        if other.stdout is not None:
            other.stdout.close()

    async with renewing(home, seconds=5):
        pass
