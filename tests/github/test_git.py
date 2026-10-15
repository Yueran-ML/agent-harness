"""git as the tools of GitHub run it: ``columbina.github.git``."""

import base64
import os
from pathlib import Path

import pytest

from columbina.github.git import Push, described, git_env, remote_of
from support.git_repos import GIT, apart, git, needs_git

TOKEN = "ghu_token0123456789"
WEB = "https://github.com"


def test_the_token_goes_in_as_a_header_for_github_alone() -> None:
    env = git_env({"PATH": "/bin"}, TOKEN, WEB, None)

    header = "Basic " + base64.b64encode(f"x-access-token:{TOKEN}".encode()).decode()
    assert env["GIT_CONFIG_COUNT"] == "2"
    assert env["GIT_CONFIG_KEY_0"] == "http.https://github.com/.extraheader"
    assert env["GIT_CONFIG_VALUE_0"] == f"Authorization: {header}"
    assert (env["GIT_CONFIG_KEY_1"], env["GIT_CONFIG_VALUE_1"]) == ("credential.helper", "")
    assert env["GIT_TERMINAL_PROMPT"] == "0"
    assert env["GCM_INTERACTIVE"] == "never"
    assert env["PATH"] == "/bin"
    assert TOKEN not in "".join(env.values())


def test_nothing_that_asks_for_a_password_is_kept() -> None:
    env = git_env({"GIT_ASKPASS": "/x", "SSH_ASKPASS": "/y", "HOME": "/h"}, TOKEN, WEB, None)

    assert "GIT_ASKPASS" not in env
    assert "SSH_ASKPASS" not in env
    assert env["HOME"] == "/h"


def test_settings_of_git_in_the_environment_give_way_to_those_of_the_run() -> None:
    # A key of these was taken out with the variables that sound like secrets.
    base = {
        "GIT_CONFIG_COUNT": "2",
        "GIT_CONFIG_VALUE_0": "x",
        "GIT_CONFIG_KEY_1": "y",
        "GIT_CONFIG_VALUE_1": "z",
        "GIT_CONFIG_GLOBAL": "/settings",
        "GIT_CONFIG_NOSYSTEM": "1",
    }

    env = git_env(base, TOKEN, WEB, None)

    assert env["GIT_CONFIG_COUNT"] == "2"
    assert env["GIT_CONFIG_KEY_1"] == "credential.helper"
    assert env["GIT_CONFIG_VALUE_1"] == ""
    assert (env["GIT_CONFIG_GLOBAL"], env["GIT_CONFIG_NOSYSTEM"]) == ("/settings", "1")


def test_the_proxy_of_the_system_is_given_where_none_is() -> None:
    assert git_env({}, TOKEN, WEB, "http://127.0.0.1:7890")["HTTPS_PROXY"] == (
        "http://127.0.0.1:7890"
    )
    assert git_env({"HTTPS_PROXY": "http://p:1"}, TOKEN, WEB, "http://q:2")["HTTPS_PROXY"] == (
        "http://p:1"
    )
    assert "HTTPS_PROXY" not in git_env({}, TOKEN, WEB, None)


@pytest.mark.parametrize(
    ("url", "found"),
    [
        ("https://github.com/octocat/hello.git", ("octocat", "hello")),
        ("https://github.com/octocat/hello", ("octocat", "hello")),
        ("https://github.com/octocat/hello/", ("octocat", "hello")),
        ("https://someone@github.com/octocat/hello.git", ("octocat", "hello")),
        ("git@github.com:octocat/hello.git", ("octocat", "hello")),
        ("ssh://git@github.com/octocat/my.repo", ("octocat", "my.repo")),
        ("  https://GitHub.com/octocat/hello.git\n", ("octocat", "hello")),
        ("https://gitlab.com/octocat/hello.git", None),
        ("git@gitlab.com:octocat/hello.git", None),
        ("http://github.com/octocat/hello.git", None),
        ("https://github.com:8443/octocat/hello.git", None),
        ("https://github.com/octocat", None),
        ("https://github.com/octocat/hello/tree/main", None),
        ("file:///srv/octocat/hello.git", None),
        ("/srv/octocat/hello.git", None),
    ],
)
def test_a_remote_is_on_github_or_elsewhere(url: str, found: tuple[str, str] | None) -> None:
    assert remote_of(url, WEB) == found


def test_a_remote_on_another_github_is_found_there() -> None:
    assert remote_of("http://127.0.0.1:5000/octocat/hello.git", "http://127.0.0.1:5000") == (
        "octocat",
        "hello",
    )
    assert remote_of("http://127.0.0.1:5001/octocat/hello.git", "http://127.0.0.1:5000") is None
    assert remote_of("https://github.com/octocat/hello.git", "not an address") is None


@pytest.mark.parametrize(
    ("push", "said"),
    [
        (Push(None, "x", (), 0), "a repository that is not on GitHub: push x (no commit that"),
        (Push("o/r", None, (), None), "o/r: push no branch, since none is checked out"),
        (Push("o/r", "x", (), None), "o/r: push x (its commits could not be listed)"),
        (Push("o/r", "x", ("abc Fix",), 0), "o/r: push x (1 commit: abc Fix)"),
        (
            Push("o/r", "x", ("abc Fix", "def Add"), 3),
            "o/r: push x (5 commits: abc Fix; def Add; ...)",
        ),
    ],
)
def test_a_push_is_said_in_one_line(push: Push, said: str) -> None:
    assert push.said().startswith(said)


@pytest.fixture
def repository(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """A clone of a repository with one commit on main, whose origin is on GitHub."""
    apart(monkeypatch, tmp_path)
    upstream = tmp_path / "upstream"
    upstream.mkdir()
    git(upstream, "init", "-q")
    (upstream / "README.md").write_text("Hello.\n", encoding="utf-8")
    git(upstream, "add", "README.md")
    git(upstream, "commit", "-q", "-m", "Begin")
    work = tmp_path / "work"
    git(tmp_path, "clone", "-q", str(upstream), str(work))
    git(work, "remote", "set-url", "origin", "https://github.com/octocat/hello.git")
    return work


def commit(work: Path, name: str, message: str) -> None:
    (work / name).write_text(f"{message}\n", encoding="utf-8")
    git(work, "add", name)
    git(work, "commit", "-q", "-m", message)


@needs_git
def test_the_commits_that_origin_does_not_have_are_described(repository: Path) -> None:
    assert GIT is not None
    git(repository, "switch", "-q", "-c", "fix-parser")
    for number in range(7):
        commit(repository, f"f{number}.txt", f"Change {number}")

    push = described(GIT, repository, None, WEB, dict(os.environ))

    assert push.repository == "octocat/hello"
    assert push.branch == "fix-parser"
    assert [line.split(" ", 1)[1] for line in push.commits] == [
        "Change 6",
        "Change 5",
        "Change 4",
        "Change 3",
        "Change 2",
    ]
    assert push.more == 2


@needs_git
def test_a_branch_that_is_named_is_described_and_not_the_one_checked_out(
    repository: Path,
) -> None:
    assert GIT is not None
    git(repository, "switch", "-q", "-c", "other")
    commit(repository, "other.txt", "Other")
    git(repository, "switch", "-q", "main")

    push = described(GIT, repository, "other", WEB, dict(os.environ))

    assert (push.branch, len(push.commits), push.more) == ("other", 1, 0)


@needs_git
def test_a_branch_without_new_commits_has_none(repository: Path) -> None:
    assert GIT is not None

    push = described(GIT, repository, None, WEB, dict(os.environ))

    assert (push.branch, push.commits, push.more) == ("main", (), 0)


@needs_git
def test_no_branch_is_none(repository: Path) -> None:
    assert GIT is not None
    git(repository, "switch", "-q", "--detach")

    push = described(GIT, repository, None, WEB, dict(os.environ))

    assert push == Push("octocat/hello", None, (), None)


@needs_git
def test_a_branch_that_is_not_there_has_commits_that_cannot_be_listed(repository: Path) -> None:
    assert GIT is not None

    push = described(GIT, repository, "nowhere", WEB, dict(os.environ))

    assert push == Push("octocat/hello", "nowhere", (), None)


@needs_git
def test_a_remote_elsewhere_or_none_is_no_repository_on_github(repository: Path) -> None:
    assert GIT is not None
    env = dict(os.environ)
    git(repository, "remote", "set-url", "origin", "https://gitlab.com/octocat/hello.git")
    assert described(GIT, repository, None, WEB, env).repository is None

    git(repository, "remote", "remove", "origin")
    assert described(GIT, repository, None, WEB, env).repository is None


def test_a_git_that_cannot_be_run_says_nothing(tmp_path: Path) -> None:
    push = described(str(tmp_path / "no-git"), tmp_path, None, WEB, {})

    assert push == Push(None, None, (), None)
