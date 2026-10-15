"""Signing in to GitHub at the command line: ``columbina login`` and ``columbina logout``."""

import asyncio
import json
import os
import signal
import subprocess
import sys
import threading
import time
import webbrowser
from collections.abc import Iterator
from pathlib import Path
from typing import Any, NoReturn

import pytest

from columbina.apps import launcher
from columbina.apps.launcher import main
from columbina.github import oauth, signing, store
from columbina.github.app import GitHubError, app
from columbina.github.store import Tokens
from support.fake_github import PICTURE, FakeGitHub, github

__all__ = ["github"]

DEVICE = {
    "device_code": "device-code-0123",
    "user_code": "WDJB-MJHT",
    "verification_uri": "https://github.com/login/device",
    "expires_in": 899,
    "interval": 0,
}
"""What GitHub gives for a code, with no wait between two asks for the tokens."""
GITHUB_VARIABLES = (
    "COLUMBINA_GITHUB_CLIENT_ID",
    "COLUMBINA_GITHUB_SLUG",
    "COLUMBINA_GITHUB_WEB",
    "COLUMBINA_GITHUB_API",
)


@pytest.fixture(autouse=True)
def no_github_of_ones_own(monkeypatch: pytest.MonkeyPatch) -> None:
    """Leave out the app and the GitHub that whoever runs the tests may have named.

    A test that signs in uses the GitHub of the tests, and names it again.
    """
    for variable in GITHUB_VARIABLES:
        monkeypatch.delenv(variable, raising=False)


@pytest.fixture
def signing_in(github: FakeGitHub, home: Path) -> FakeGitHub:
    """A GitHub that hands out the tokens as soon as it is asked for them."""
    github.device = dict(DEVICE)
    github.installations = [
        {"id": 1, "repositories": [{"full_name": "octocat/hello"}, {"full_name": "acme/tools"}]}
    ]
    return github


@pytest.fixture
def opened(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """The pages that were to be opened in a browser of this machine, which has a desktop."""
    pages: list[str] = []

    def open_(url: str, new: int = 0, autoraise: bool = True) -> bool:
        pages.append(url)
        return True

    monkeypatch.setattr(webbrowser, "open", open_)
    # Linux has a desktop where this names a display. Windows and macOS have one anyway.
    monkeypatch.setenv("DISPLAY", ":0")
    return pages


def signed_in(home: Path, *, renewal_ends: float | None = None) -> None:
    ends = time.time() + 1e6 if renewal_ends is None else renewal_ends
    given = Tokens("ghu_first0123456789", "ghr_first0123456789", time.time() + 3600, ends)
    store.keep(home, app(os.environ), given, "octocat", "The Octocat")


def lines(out: str) -> list[dict[str, Any]]:
    """Read what the command wrote for a program: a document of JSON on each line."""
    assert out.endswith("\n")
    return [json.loads(line) for line in out.splitlines()]


def no_token_in(text: str) -> bool:
    return "ghu_" not in text and "ghr_" not in text and "device-code" not in text


async def ctrl_c(*args: object, **kwargs: object) -> NoReturn:
    """Stand in for a wait that the user ends with Ctrl+C, by the signal that the system sends.

    The signal reaches the program as it would at a terminal: the loop cancels its task, and
    the interrupt comes out of ``asyncio.run``.
    """
    signal.raise_signal(signal.SIGINT)
    await asyncio.sleep(20)
    raise AssertionError("the interrupt did not end the wait")


@pytest.fixture
def offline(monkeypatch: pytest.MonkeyPatch) -> None:
    """Have the code and the tokens of a sign-in come without a request.

    The tests of Ctrl+C make none before the interrupt: on Python 3.11, coverage loses the
    lines that run after a task that used httpx is cancelled, and would count them as never
    run. The interrupt is the same either way.
    """

    async def code(*args: object, **kwargs: object) -> oauth.Code:
        return oauth.Code("WDJB-MJHT", "https://github.com/login/device", time.time() + 899, 0, "d")

    async def tokens(*args: object, **kwargs: object) -> Tokens:
        return Tokens("ghu_first0123456789", "ghr_first0123456789", time.time() + 3600, 2e9)

    async def kept(github: object, home: Path, given: Tokens) -> signing.Who:
        store.keep(home, app(os.environ), given, "octocat", "The Octocat")
        return signing.Who("octocat", "The Octocat")

    monkeypatch.setattr(oauth, "ask_for_a_code", code)
    monkeypatch.setattr(oauth, "wait_for_the_user", tokens)
    monkeypatch.setattr(signing, "keep", kept)


# At a terminal.


def test_a_sign_in_says_the_code_and_where_to_enter_it_and_who_signed_in(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login", "github"]) == 0

    out, err = capsys.readouterr()
    assert err == ""
    assert out == (
        "Enter this code on GitHub to sign in:\n"
        "\n"
        "    WDJB-MJHT\n"
        "\n"
        "The page to enter it on is https://github.com/login/device, and it was opened in "
        "your browser.\n"
        "The code holds for 15 minutes. Ctrl+C stops the waiting.\n"
        "Signed in to GitHub as @octocat (The Octocat).\n"
        "Columbina may reach these repositories:\n"
        "  octocat/hello\n"
        "  acme/tools\n"
        f"Choose others on GitHub: {signing_in.address}/apps/columbina-test/installations/new\n"
    )
    assert opened == ["https://github.com/login/device"]
    said = store.signed(home, app(os.environ))
    assert said is not None
    assert said.login == "octocat"
    assert (home / store.AVATAR).read_bytes() == PICTURE


def test_a_page_that_no_browser_opened_is_named_all_the_same(
    signing_in: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    def nowhere(url: str, env: object) -> bool:
        return False

    monkeypatch.setattr(launcher, "_opened", nowhere)

    assert main(["login", "github"]) == 0

    assert "The page to enter it on is https://github.com/login/device.\nThe code holds" in (
        capsys.readouterr().out
    )


def test_a_user_without_a_name_is_named_by_the_login(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    signing_in.user = {"login": "octocat"}

    assert main(["login", "github"]) == 0

    assert "Signed in to GitHub as @octocat.\n" in capsys.readouterr().out


def test_a_sign_in_that_reaches_no_repository_says_where_to_choose_them(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    signing_in.installations = []

    assert main(["login", "github"]) == 0

    assert capsys.readouterr().out.endswith(
        "Signed in to GitHub as @octocat (The Octocat).\n"
        "Columbina may reach no repository of yours yet. Choose them on GitHub: "
        f"{signing_in.address}/apps/columbina-test/installations/new\n"
    )


def test_more_repositories_than_are_named_are_counted(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    names = [{"full_name": f"octocat/r{n}"} for n in range(23)]
    signing_in.installations = [{"id": 1, "repositories": names}]

    assert main(["login", "github"]) == 0

    out = capsys.readouterr().out
    assert "  octocat/r19\n  and 3 more\n" in out
    assert "octocat/r20" not in out


def test_as_many_repositories_as_are_named_are_named_all(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    names = [{"full_name": f"octocat/r{n}"} for n in range(20)]
    signing_in.installations = [{"id": 1, "repositories": names}]

    assert main(["login", "github"]) == 0

    out = capsys.readouterr().out
    assert "  octocat/r19\nChoose others on GitHub: " in out
    assert "more" not in out


def test_repositories_that_cannot_be_listed_leave_the_sign_in(
    signing_in: FakeGitHub,
    home: Path,
    opened: list[str],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def unreachable(github: object) -> list[str]:
        raise GitHubError("GitHub did not answer in time", code="TIMEOUT")

    monkeypatch.setattr(signing, "reach", unreachable)

    assert main(["login", "github"]) == 0

    out, err = capsys.readouterr()
    assert out.endswith("Signed in to GitHub as @octocat (The Octocat).\n")
    assert err == (
        "columbina: GitHub could not be asked which repositories Columbina may reach: "
        "GitHub did not answer in time\n"
    )


def test_a_new_sign_in_says_that_it_takes_the_place_of_the_last(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home)

    assert main(["login", "github"]) == 0

    assert capsys.readouterr().out.startswith(
        "You are signed in to GitHub as @octocat. A new sign-in takes its place.\n"
        "Enter this code on GitHub to sign in:\n"
    )


@pytest.mark.parametrize(
    ("poll", "said"),
    [
        ({"error": "access_denied"}, "the sign-in was turned down on GitHub"),
        ({"error": "expired_token"}, "the code is no longer good. Ask for a new one"),
    ],
    ids=["denied", "expired"],
)
def test_a_sign_in_that_github_does_not_give_ends_with_2(
    signing_in: FakeGitHub,
    home: Path,
    opened: list[str],
    capsys: pytest.CaptureFixture[str],
    poll: dict[str, str],
    said: str,
) -> None:
    signing_in.polls = [poll]

    assert main(["login", "github"]) == 2

    assert capsys.readouterr().err == f"columbina: {said}\n"
    assert store.signed(home, app(os.environ)) is None


def test_a_build_without_an_app_says_that_it_cannot_sign_in(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login", "github"]) == 2

    out, err = capsys.readouterr()
    assert out == ""
    assert err.startswith("columbina: this build of Columbina has no GitHub App to sign in with")


def test_an_interrupted_wait_keeps_nothing(
    signing_in: FakeGitHub,
    home: Path,
    opened: list[str],
    offline: None,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(oauth, "wait_for_the_user", ctrl_c)

    assert main(["login", "github"]) == 130

    out, err = capsys.readouterr()
    assert out.endswith("Ctrl+C stops the waiting.\n\n")
    assert err == "columbina: the sign-in was stopped\n"
    assert store.signed(home, app(os.environ)) is None


def test_an_interrupted_listing_says_that_the_sign_in_was_kept(
    signing_in: FakeGitHub,
    home: Path,
    opened: list[str],
    offline: None,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(signing, "reach", ctrl_c)

    assert main(["login", "github"]) == 130

    assert capsys.readouterr().err == (
        "columbina: signed in to GitHub as @octocat. The repositories were not listed\n"
    )
    assert store.signed(home, app(os.environ)) is not None


def test_what_github_says_is_made_harmless_for_the_terminal(
    signing_in: FakeGitHub, home: Path, opened: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    signing_in.user = {"login": "octocat", "name": "The \x1b[2JOctocat"}

    assert main(["login", "github"]) == 0

    assert "\x1b" not in capsys.readouterr().out


@pytest.mark.parametrize(
    ("platform", "env", "opens"),
    [
        ("win32", {}, True),
        ("darwin", {}, True),
        ("linux", {"DISPLAY": ":0"}, True),
        ("linux", {"WAYLAND_DISPLAY": "wayland-0"}, True),
        ("linux", {}, False),
        ("linux", {"DISPLAY": ""}, False),
    ],
)
def test_a_browser_is_opened_where_there_is_a_desktop(
    opened: list[str], platform: str, env: dict[str, str], opens: bool
) -> None:
    assert launcher._opened("https://github.com/login/device", env, platform) is opens  # pyright: ignore[reportPrivateUsage]

    assert opened == (["https://github.com/login/device"] if opens else [])


def test_a_browser_that_cannot_be_opened_is_no_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    def refused(url: str, new: int = 0, autoraise: bool = True) -> bool:
        raise webbrowser.Error("no browser")

    monkeypatch.setattr(webbrowser, "open", refused)

    assert launcher._opened("https://github.com/login/device", {}, "win32") is False  # pyright: ignore[reportPrivateUsage]


def test_login_without_a_service_is_told_what_to_say(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login"]) == 2

    assert "columbina login github" in capsys.readouterr().err


@pytest.mark.parametrize("command", [["login", "gitlab"], ["logout"], ["logout", "gitlab"]])
def test_no_other_service_is_signed_in_to(
    home: Path, capsys: pytest.CaptureFixture[str], command: list[str]
) -> None:
    with pytest.raises(SystemExit) as ended:
        main(command)

    assert ended.value.code == 2


# For another program.


@pytest.fixture
def no_input(monkeypatch: pytest.MonkeyPatch) -> Iterator[int]:
    """A standard input that stays open, as a program that waits for the sign-in keeps it.

    Yields:
        The end of the pipe that the program writes to.
    """
    read, write = os.pipe()

    class Open:
        def fileno(self) -> int:
            return read

    monkeypatch.setattr(sys, "stdin", Open())
    yield write
    os.close(write)
    # The command watches the input until its end, which has come now.
    for thread in threading.enumerate():
        if thread.name == "columbina-input":
            thread.join(timeout=5)
    os.close(read)


def test_what_a_program_writes_to_the_standard_input_does_not_stop_a_sign_in(
    signing_in: FakeGitHub, home: Path, no_input: int, capsys: pytest.CaptureFixture[str]
) -> None:
    signing_in.device["interval"] = 0.05
    pending = {"error": "authorization_pending"}
    signing_in.polls = [pending, pending, None]
    os.write(no_input, b"something\n")

    assert main(["login", "github", "--json"]) == 0

    assert lines(capsys.readouterr().out)[-1]["signed_in"]["login"] == "octocat"


def test_a_sign_in_for_a_program_says_the_code_and_then_who_signed_in(
    signing_in: FakeGitHub,
    home: Path,
    opened: list[str],
    no_input: int,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert main(["login", "github", "--json"]) == 0

    out, err = capsys.readouterr()
    assert err == ""
    code, done = lines(out)
    assert code == {
        "code": "WDJB-MJHT",
        "url": "https://github.com/login/device",
        "expires_in": code["expires_in"],
    }
    assert 897 <= code["expires_in"] <= 899
    assert done == {
        "signed_in": {
            "login": "octocat",
            "name": "The Octocat",
            "avatar": signing.avatar(home),
        },
        "install_url": f"{signing_in.address}/apps/columbina-test/installations/new",
        "repositories": ["octocat/hello", "acme/tools"],
    }
    assert str(done["signed_in"]["avatar"]).startswith("data:image/png;base64,")
    assert no_token_in(out)
    assert opened == []


def test_closing_the_standard_input_stops_a_sign_in(
    signing_in: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    signing_in.device["interval"] = 0.05
    signing_in.polls = [{"error": "authorization_pending"}]
    read, write = os.pipe()

    class Closed:
        def fileno(self) -> int:
            return read

    monkeypatch.setattr(sys, "stdin", Closed())
    os.close(write)
    try:
        assert main(["login", "github", "--json"]) == 130
    finally:
        os.close(read)

    code, done = lines(capsys.readouterr().out)
    assert code["code"] == "WDJB-MJHT"
    assert done == {"error": "the sign-in was stopped", "code": "STOPPED"}
    assert store.signed(home, app(os.environ)) is None


def test_a_program_reads_the_code_at_once_and_stops_the_sign_in_by_closing_the_input(
    signing_in: FakeGitHub, home: Path
) -> None:
    signing_in.device["interval"] = 0.05
    signing_in.polls = [{"error": "authorization_pending"}]
    child = subprocess.Popen(
        [sys.executable, "-m", "columbina", "login", "github", "--json"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert child.stdout is not None
    try:
        # Read while the command waits: the line is not kept back in a buffer.
        first = json.loads(child.stdout.readline())
        # This closes the standard input first, as the program that stops a sign-in does.
        out, err = child.communicate(timeout=20)
    except BaseException:
        child.kill()
        child.communicate()
        raise

    assert first["code"] == "WDJB-MJHT"
    assert child.returncode == 130
    assert lines(out.decode()) == [{"error": "the sign-in was stopped", "code": "STOPPED"}]
    assert err == b""


def test_a_standard_input_that_cannot_be_watched_leaves_the_wait(
    signing_in: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The standard input of a test is none that can be read.
    assert main(["login", "github", "--json"]) == 0

    assert "signed_in" in lines(capsys.readouterr().out)[-1]


def test_a_sign_in_that_github_turns_down_is_said_for_a_program(
    signing_in: FakeGitHub, home: Path, no_input: int, capsys: pytest.CaptureFixture[str]
) -> None:
    signing_in.polls = [{"error": "access_denied"}]

    assert main(["login", "github", "--json"]) == 2

    code, done = lines(capsys.readouterr().out)
    assert code["code"] == "WDJB-MJHT"
    assert done == {"error": "the sign-in was turned down on GitHub", "code": "DENIED"}


def test_a_build_without_an_app_says_so_to_a_program(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login", "github", "--json"]) == 2

    [refused] = lines(capsys.readouterr().out)
    assert refused["code"] == "NOT_REGISTERED"


def test_an_interrupt_is_said_to_a_program(
    signing_in: FakeGitHub,
    home: Path,
    no_input: int,
    offline: None,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(oauth, "wait_for_the_user", ctrl_c)

    assert main(["login", "github", "--json"]) == 130

    code, stopped = lines(capsys.readouterr().out)
    assert code["code"] == "WDJB-MJHT"
    assert stopped == {"error": "the sign-in was stopped", "code": "STOPPED"}


def test_repositories_that_cannot_be_listed_are_said_to_a_program(
    signing_in: FakeGitHub,
    home: Path,
    no_input: int,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def unreachable(github: object) -> list[str]:
        raise GitHubError("GitHub did not answer in time", code="TIMEOUT")

    monkeypatch.setattr(signing, "reach", unreachable)

    assert main(["login", "github", "--json"]) == 0

    done = lines(capsys.readouterr().out)[-1]
    assert done["offline"] == "GitHub did not answer in time"
    assert done["signed_in"]["login"] == "octocat"
    assert "repositories" not in done


# Who is signed in.


def test_nobody_signed_in_is_said_to_be_nobody(
    github: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login", "--status"]) == 0
    assert capsys.readouterr().out == (
        "Nobody is signed in to GitHub. Sign in with 'columbina login github'.\n"
    )

    assert main(["login", "--status", "--json"]) == 0
    assert lines(capsys.readouterr().out) == [
        {
            "signed_in": None,
            "install_url": f"{github.address}/apps/columbina-test/installations/new",
        }
    ]
    assert github.asked == []


def test_who_is_signed_in_is_asked_of_github_with_what_may_be_reached(
    signing_in: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home)
    (home / store.AVATAR).write_bytes(PICTURE)
    signing_in.user = {"login": "octocat", "name": "Mona"}

    assert main(["login", "--status", "--json"]) == 0

    [said] = lines(capsys.readouterr().out)
    assert said == {
        "signed_in": {"login": "octocat", "name": "Mona", "avatar": signing.avatar(home)},
        "install_url": f"{signing_in.address}/apps/columbina-test/installations/new",
        "repositories": ["octocat/hello", "acme/tools"],
    }
    assert said["signed_in"]["avatar"] is not None
    assert [asked.path.split("?")[0] for asked in signing_in.asked] == [
        "/user",
        "/user/installations",
        "/user/installations/1/repositories",
    ]


def test_who_is_signed_in_is_said_to_a_person(
    signing_in: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    # What GitHub answers is the business of the test above. This one asks nothing of it: on
    # Python 3.11 coverage loses the lines that run after asyncio.run has used httpx.
    signed_in(home)

    async def who(github: object) -> signing.Who:
        return signing.Who("octocat", "The Octocat")

    async def reach(github: object) -> list[str]:
        return ["octocat/hello", "acme/tools"]

    monkeypatch.setattr(signing, "who", who)
    monkeypatch.setattr(signing, "reach", reach)

    assert main(["login", "github", "--status"]) == 0

    assert capsys.readouterr().out == (
        "Signed in to GitHub as @octocat (The Octocat).\n"
        "Columbina may reach these repositories:\n"
        "  octocat/hello\n"
        "  acme/tools\n"
        f"Choose others on GitHub: {signing_in.address}/apps/columbina-test/installations/new\n"
    )


def test_github_that_cannot_be_asked_leaves_who_signed_in(
    signing_in: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    signed_in(home)

    async def unreachable(github: object) -> signing.Who:
        raise GitHubError("GitHub could not be reached: refused", code="UNREACHABLE")

    monkeypatch.setattr(signing, "who", unreachable)

    assert main(["login", "--status"]) == 0
    assert capsys.readouterr().out == (
        "Signed in to GitHub as @octocat (The Octocat). GitHub could not be asked which "
        "repositories Columbina may reach: GitHub could not be reached: refused\n"
    )

    assert main(["login", "--status", "--json"]) == 0
    [said] = lines(capsys.readouterr().out)
    assert said["signed_in"]["login"] == "octocat"
    assert said["offline"] == "GitHub could not be reached: refused"


def test_a_sign_in_that_ran_out_is_said_to_be_over(
    signing_in: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home, renewal_ends=time.time() - 1)
    store.keep(
        home,
        app(os.environ),
        Tokens("ghu_first0123456789", "ghr_first0123456789", time.time() - 1, time.time() - 1),
        "octocat",
        "The Octocat",
    )

    assert main(["login", "--status", "--json"]) == 0

    [said] = lines(capsys.readouterr().out)
    assert said["signed_in"] is None
    assert said["ended"] == "the sign-in to GitHub has run out. Sign in again"


def test_a_sign_in_that_ran_out_is_said_to_be_over_at_a_terminal(
    signing_in: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    store.keep(
        home,
        app(os.environ),
        Tokens("ghu_first0123456789", "ghr_first0123456789", time.time() - 1, time.time() - 1),
        "octocat",
        "The Octocat",
    )

    assert main(["login", "--status"]) == 0

    assert capsys.readouterr().out == (
        "The sign-in to GitHub has run out. Sign in again with 'columbina login github'.\n"
    )


def test_a_build_without_an_app_says_so_when_asked_who_is_signed_in(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["login", "--status", "--json"]) == 0
    [said] = lines(capsys.readouterr().out)
    assert said["signed_in"] is None
    assert said["registered"] is False

    assert main(["login", "--status"]) == 0
    assert capsys.readouterr().out.startswith(
        "This build of Columbina has no GitHub App to sign in with."
    )


def test_a_sign_in_that_cannot_be_read_is_an_error(
    github: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (home / store.SIGNED).mkdir(parents=True)

    assert main(["login", "--status", "--json"]) == 2
    [refused] = lines(capsys.readouterr().out)
    assert refused["code"] == "STORE"

    assert main(["login", "--status"]) == 2
    assert capsys.readouterr().err.startswith("columbina: ")


def test_tokens_that_cannot_be_read_are_an_error_and_no_sign_out(
    github: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home)
    (home / store.CREDENTIALS).write_text("not what is written there", encoding="utf-8")

    assert main(["login", "--status", "--json"]) == 2

    [refused] = lines(capsys.readouterr().out)
    assert refused["code"] == "STORE"
    assert store.signed(home, app(os.environ)) is not None
    assert github.asked == []


def test_a_github_that_cannot_be_one_is_an_error(
    github: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setenv("COLUMBINA_GITHUB_WEB", "ftp://github.example")

    assert main(["login", "--status", "--json"]) == 2

    [refused] = lines(capsys.readouterr().out)
    assert refused["code"] == "MISCONFIGURED"
    assert "COLUMBINA_GITHUB_WEB" in refused["error"]


# Signing out.


def test_signing_out_deletes_the_sign_in_and_says_where_to_take_the_right_back(
    github: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home)
    (home / store.AVATAR).write_bytes(PICTURE)

    assert main(["logout", "github"]) == 0

    assert capsys.readouterr().out == (
        "Signed out of GitHub on this machine. The app keeps the right that you gave it until "
        f"you take it back on GitHub: {github.address}/settings/apps/authorizations\n"
    )
    assert store.signed(home, app(os.environ)) is None
    assert not (home / store.AVATAR).exists()

    assert main(["logout", "github"]) == 0
    assert capsys.readouterr().out == "Nobody was signed in to GitHub.\n"


def test_signing_out_for_a_program_says_whether_anybody_was_signed_in(
    github: FakeGitHub, home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    signed_in(home)
    revoke = f"{github.address}/settings/apps/authorizations"

    assert main(["logout", "github", "--json"]) == 0
    assert lines(capsys.readouterr().out) == [{"signed_out": True, "revoke": revoke}]

    assert main(["logout", "github", "--json"]) == 0
    assert lines(capsys.readouterr().out) == [{"signed_out": False, "revoke": revoke}]


def test_signing_out_needs_no_app_and_no_good_address(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("COLUMBINA_GITHUB_WEB", "ftp://nowhere")

    assert main(["logout", "github", "--json"]) == 0

    assert lines(capsys.readouterr().out) == [
        {"signed_out": False, "revoke": "https://github.com/settings/apps/authorizations"}
    ]


def test_a_sign_in_that_cannot_be_deleted_is_an_error(
    github: FakeGitHub,
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    def refused(home: Path) -> bool:
        raise GitHubError("the tokens of GitHub cannot be used: locked", code="STORE")

    monkeypatch.setattr(store, "forget", refused)

    assert main(["logout", "github", "--json"]) == 2
    assert lines(capsys.readouterr().out) == [
        {"error": "the tokens of GitHub cannot be used: locked", "code": "STORE"}
    ]

    assert main(["logout", "github"]) == 2
    assert capsys.readouterr().err == "columbina: the tokens of GitHub cannot be used: locked\n"
