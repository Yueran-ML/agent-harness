"""The tools that write, into the working directory and to GitHub: ``columbina.github.tools``."""

import base64
import os
import time
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
import columbina.session
import columbina.tools
from columbina import process, programs
from columbina.fs import FS
from columbina.github import store
from columbina.github.app import app
from columbina.github.client import GitHub
from columbina.github.store import Tokens
from columbina.github.tools import write_tools
from columbina.kernel import Context
from columbina.permissions import PERMISSIONS
from columbina.sandbox import SANDBOX
from columbina.session import SESSIONS
from columbina.tools import TOOLS, Deny, Invocation, ToolRun
from support.desk import Desk
from support.fake_github import FakeGitHub, github
from support.git_repos import apart, git, needs_git
from support.links import link_directory, unlink_directory

__all__ = ["github"]

pytestmark = needs_git


@pytest.fixture
def upstreams(tmp_path: Path) -> Path:
    """Where the repositories of GitHub lie, for git: one bare repository for each."""
    return tmp_path / "upstreams"


def upstream(upstreams: Path, tmp_path: Path, owner: str = "octocat", repo: str = "hello") -> Path:
    """Make a repository of GitHub with one commit on main, and return where it is."""
    bare = upstreams / owner / f"{repo}.git"
    bare.mkdir(parents=True)
    git(bare, "init", "-q", "--bare")
    seed = tmp_path / f"seed-{owner}-{repo}"
    git(tmp_path, "clone", "-q", str(bare), str(seed))
    (seed / "README.md").write_text("Hello.\n", encoding="utf-8")
    git(seed, "add", "README.md")
    git(seed, "commit", "-q", "-m", "Begin")
    git(seed, "push", "-q", "origin", "main")
    return bare


async def writing_desk(
    ctx: Context,
    tmp_path: Path,
    upstreams: Path | None,
    permissions: dict[str, Any] | None = None,
) -> AsyncIterator[Desk]:
    """A desk with the tools that write, and git that finds GitHub in ``upstreams``.

    Without ``upstreams``, git asks GitHub itself, which is the stand-in of the tests.
    """
    work = tmp_path / "work"
    work.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    if permissions is not None:
        await ctx.plugin(columbina.approval)
        await ctx.plugin(columbina.permissions, permissions)
    client = GitHub(tmp_path / "home", os.environ)

    def remote(owner: str, repo: str) -> str:
        assert upstreams is not None
        return (upstreams / owner / f"{repo}.git").as_uri()

    tools = write_tools(
        ctx, client, client.app, ctx.use(FS), remote=None if upstreams is None else remote
    )
    ctx.use(TOOLS).add(ctx, *tools)
    session = ctx.use(SESSIONS).create(ctx, work)
    yield Desk(ctx, ctx.use(TOOLS), ctx.use(FS), ctx.use(SANDBOX), session, work)
    await client.aclose()


@pytest.fixture
async def desk(
    ctx: Context,
    tmp_path: Path,
    upstreams: Path,
    github: FakeGitHub,
    monkeypatch: pytest.MonkeyPatch,
) -> AsyncIterator[Desk]:
    apart(monkeypatch, tmp_path)
    signed_in(tmp_path / "home")
    github.repos["octocat/hello"] = {"full_name": "octocat/hello", "default_branch": "main"}
    async for made in writing_desk(ctx, tmp_path, upstreams):
        yield made


def signed_in(home: Path) -> None:
    given = Tokens("ghu_first0123456789", "ghr_first0123456789", time.time() + 3600, 2e9)
    store.keep(home, app(os.environ), given, "octocat", "The Octocat")


def target(desk: Desk, name: str, **arguments: Any) -> str:
    """What a call acts on, as the checks are told and the user is shown."""
    tool = desk.tools.registry.get(name)
    assert tool is not None
    assert tool.target is not None
    return tool.target(
        tool.parameters.model_validate(arguments), ToolRun("x", desk.work, desk.session)
    )


def a_clone_with_a_branch(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub, branch: str = "fix-parser"
) -> Path:
    """A clone of octocat/hello in the working directory, whose origin is on GitHub, and on it
    a branch with two commits that GitHub does not have."""
    bare = upstream(upstreams, tmp_path)
    work = desk.work / "hello"
    git(desk.work, "clone", "-q", str(bare), str(work))
    git(work, "remote", "set-url", "origin", f"{github.address}/octocat/hello.git")
    git(work, "switch", "-q", "-C", branch)
    for number in (1, 2):
        (work / f"part{number}.py").write_text(f"part = {number}\n", encoding="utf-8")
        git(work, "add", f"part{number}.py")
        git(work, "commit", "-q", "-m", f"Add part {number}")
    return work


# Cloning.


async def test_a_repository_is_cloned_into_the_working_directory(
    desk: Desk, upstreams: Path, tmp_path: Path
) -> None:
    upstream(upstreams, tmp_path)

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.error is None, outcome.text
    assert outcome.text == "Cloned octocat/hello into hello."
    assert (desk.work / "hello" / "README.md").read_text(encoding="utf-8") == "Hello.\n"


async def test_a_clone_goes_where_it_is_told(desk: Desk, upstreams: Path, tmp_path: Path) -> None:
    upstream(upstreams, tmp_path)
    (desk.work / "empty").mkdir()

    outcome = await desk.call("github_clone", repo="octocat/hello", path="empty")

    assert outcome.error is None, outcome.text
    assert (desk.work / "empty" / "README.md").exists()


async def test_a_clone_is_checked_as_writing_its_place_is(desk: Desk) -> None:
    tool = desk.tools.registry.get("github_clone")
    assert tool is not None
    assert tool.effect == "write"

    assert target(desk, "github_clone", repo="octocat/hello", path="deep/place") == "deep/place"
    assert target(desk, "github_clone", repo="octocat/hello") == "hello"


@pytest.mark.parametrize("there", ["file", "full directory"])
async def test_nothing_is_cloned_over_what_is_there(
    desk: Desk, upstreams: Path, tmp_path: Path, there: str
) -> None:
    upstream(upstreams, tmp_path)
    if there == "file":
        (desk.work / "hello").write_text("mine", encoding="utf-8")
    else:
        (desk.work / "hello").mkdir()
        (desk.work / "hello" / "mine.txt").write_text("mine", encoding="utf-8")

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_EMPTY"


async def test_nothing_is_cloned_outside_the_working_directory(
    desk: Desk, upstreams: Path, tmp_path: Path
) -> None:
    upstream(upstreams, tmp_path)

    outcome = await desk.call("github_clone", repo="octocat/hello", path=str(tmp_path / "out"))

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"
    assert not (tmp_path / "out").exists()


async def test_nothing_is_cloned_in_the_mode_that_writes_nothing(
    desk: Desk, upstreams: Path, tmp_path: Path
) -> None:
    upstream(upstreams, tmp_path)
    desk.sandbox.set_mode(desk.session, "read-only")

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.error is not None
    assert outcome.error.code == "READ_ONLY"


async def test_a_repository_that_cannot_be_cloned_says_why(desk: Desk) -> None:
    outcome = await desk.call("github_clone", repo="octocat/nowhere")

    assert outcome.error is not None
    assert outcome.error.code == "GIT_FAILED"
    assert outcome.text.startswith("git could not clone octocat/nowhere: ")
    assert "ghu_" not in outcome.text


@pytest.mark.parametrize("repo", ["hello", "octocat/", "-x/hello", "octocat/hello/x"])
async def test_what_names_no_repository_is_not_cloned(desk: Desk, repo: str) -> None:
    outcome = await desk.call("github_clone", repo=repo)

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"


async def test_without_git_nothing_is_cloned(
    desk: Desk, upstreams: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    upstream(upstreams, tmp_path)
    monkeypatch.setenv("PATH", str(tmp_path / "nothing-here"))

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.error is not None
    assert outcome.error.code == "GIT_NOT_FOUND"


# Pushing.


async def test_a_branch_is_pushed_after_the_user_was_told_what_goes(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    work = a_clone_with_a_branch(desk, upstreams, tmp_path, github)

    said = target(desk, "github_push", path="hello")
    outcome = await desk.call("github_push", path="hello")

    assert said.startswith("octocat/hello: push fix-parser (2 commits: ")
    assert " Add part 2; " in said
    assert said.endswith(" Add part 1)")

    assert outcome.error is None, outcome.text
    assert outcome.text.startswith("Pushed fix-parser to octocat/hello.")
    bare = upstreams / "octocat" / "hello.git"
    assert git(bare, "rev-parse", "fix-parser") == git(work, "rev-parse", "fix-parser")


async def test_a_branch_that_is_named_is_pushed(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    work = a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    git(work, "switch", "-q", "main")

    outcome = await desk.call("github_push", path="hello", branch="fix-parser")

    assert outcome.error is None, outcome.text


async def test_what_changed_after_the_check_is_not_pushed(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    work = a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    pending = desk.begin("github_push", path="hello")
    (work / "late.py").write_text("late = True\n", encoding="utf-8")
    git(work, "add", "late.py")
    git(work, "commit", "-q", "-m", "Late")

    outcome = await pending.execute()

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert "fix-parser" not in git(upstreams / "octocat" / "hello.git", "branch", "--list")


async def test_the_default_branch_is_not_pushed_to(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    a_clone_with_a_branch(desk, upstreams, tmp_path, github, branch="main")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "PUSH_TO_DEFAULT"


async def test_a_branch_that_github_has_moved_on_is_not_forced(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    other = tmp_path / "other"
    git(tmp_path, "clone", "-q", str(upstreams / "octocat" / "hello.git"), str(other))
    git(other, "switch", "-q", "-c", "fix-parser")
    (other / "theirs.py").write_text("theirs = True\n", encoding="utf-8")
    git(other, "add", "theirs.py")
    git(other, "commit", "-q", "-m", "Theirs")
    git(other, "push", "-q", "origin", "fix-parser")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_FAST_FORWARD"


async def test_no_branch_is_not_pushed(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    work = a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    git(work, "switch", "-q", "--detach")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "DETACHED"


async def test_a_repository_whose_origin_is_elsewhere_is_not_pushed(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    work = a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    git(work, "remote", "set-url", "origin", "https://gitlab.com/octocat/hello.git")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "NOT_ON_GITHUB"


async def test_nothing_is_pushed_in_the_mode_that_writes_nothing(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    desk.sandbox.set_mode(desk.session, "read-only")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "READ_ONLY"


async def test_nothing_is_pushed_from_outside_the_working_directory(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    bare = upstream(upstreams, tmp_path)
    outside = tmp_path / "outside"
    git(tmp_path, "clone", "-q", str(bare), str(outside))

    outcome = await desk.call("github_push", path=str(outside))

    assert outcome.error is not None
    assert outcome.error.code == "OUTSIDE_WORKING_DIRECTORY"


@pytest.mark.parametrize("branch", ["-x", "a..b", "a b", "x.lock", "x/", ""])
async def test_what_cannot_be_a_branch_is_not_pushed(desk: Desk, branch: str) -> None:
    outcome = await desk.call("github_push", path=".", branch=branch)

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"


async def test_without_git_nothing_is_asked_about(
    desk: Desk, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("PATH", str(tmp_path / "nothing-here"))

    outcome = await desk.call("github_push", path=".")

    assert outcome.error is not None
    assert outcome.error.code == "GIT_NOT_FOUND"


# Pull requests and comments.


async def test_a_pull_request_is_opened_as_the_user_was_shown(
    desk: Desk, github: FakeGitHub
) -> None:
    arguments: dict[str, Any] = {
        "repo": "octocat/hello",
        "head": "fix-parser",
        "title": "Fix the parser",
        "body": "It dropped the last line.",
        "draft": True,
    }

    assert target(desk, "github_open_pull_request", **arguments) == (
        "octocat/hello: open a pull request as a draft from fix-parser into the default "
        "branch\nTitle: Fix the parser\n\nIt dropped the last line."
    )

    outcome = await desk.call("github_open_pull_request", **arguments)

    assert outcome.error is None, outcome.text
    assert outcome.text == "Opened octocat/hello#100: https://github.com/octocat/hello/pull/100"
    [made] = github.pulls_made
    assert {key: made[key] for key in ("title", "head", "base", "body", "draft")} == {
        "title": "Fix the parser",
        "head": "fix-parser",
        "base": "main",
        "body": "It dropped the last line.",
        "draft": True,
    }


async def test_a_pull_request_goes_into_the_branch_that_is_named(
    desk: Desk, github: FakeGitHub
) -> None:
    arguments = {"repo": "octocat/hello", "head": "x", "title": "T", "base": "release"}

    assert target(desk, "github_open_pull_request", **arguments) == (
        "octocat/hello: open a pull request from x into release\nTitle: T"
    )
    outcome = await desk.call("github_open_pull_request", **arguments)

    assert outcome.error is None, outcome.text
    assert github.pulls_made[0]["base"] == "release"
    assert [asked.path for asked in github.asked if asked.path == "/repos/octocat/hello"] == []


async def test_what_github_does_not_take_is_said_with_its_reason(
    desk: Desk, github: FakeGitHub
) -> None:
    github.fails_next(
        422, {"message": "Validation Failed", "errors": [{"message": "No commits between"}]}
    )

    outcome = await desk.call(
        "github_open_pull_request", repo="octocat/hello", head="x", title="T", base="main"
    )

    assert outcome.error is not None
    assert outcome.error.code == "GITHUB_INVALID"
    assert "No commits between" in outcome.text


async def test_a_comment_is_written_as_the_user_was_shown(desk: Desk, github: FakeGitHub) -> None:
    arguments = {"ref": "octocat/hello#12", "body": "It works now."}

    assert target(desk, "github_comment", **arguments) == (
        "octocat/hello#12: comment\n\nIt works now."
    )

    outcome = await desk.call("github_comment", **arguments)

    assert outcome.error is None, outcome.text
    assert outcome.text == (
        "Commented on octocat/hello#12: https://github.com/octocat/hello/issues/12#issuecomment-900"
    )
    assert github.comments["octocat/hello#12"][0]["body"] == "It works now."


@pytest.mark.parametrize("body", ["", "  \n"])
async def test_a_comment_without_text_is_not_written(
    desk: Desk, github: FakeGitHub, body: str
) -> None:
    outcome = await desk.call("github_comment", ref="octocat/hello#12", body=body)

    assert outcome.error is not None
    assert outcome.error.code == "INVALID_ARGS"
    assert github.comments == {}


@pytest.mark.parametrize(
    ("name", "arguments"),
    [
        ("github_comment", {"ref": "octocat/hello#12", "body": "Hi."}),
        ("github_open_pull_request", {"repo": "octocat/hello", "head": "x", "title": "T"}),
    ],
)
async def test_nothing_is_written_to_github_in_the_mode_that_writes_nothing(
    desk: Desk, github: FakeGitHub, name: str, arguments: dict[str, Any]
) -> None:
    desk.sandbox.set_mode(desk.session, "read-only")

    outcome = await desk.call(name, **arguments)

    assert outcome.error is not None
    assert outcome.error.code == "READ_ONLY"
    assert github.asked == []


# Asking.


@pytest.fixture
async def asking(
    ctx: Context,
    tmp_path: Path,
    upstreams: Path,
    github: FakeGitHub,
    monkeypatch: pytest.MonkeyPatch,
    request: pytest.FixtureRequest,
) -> AsyncIterator[Desk]:
    apart(monkeypatch, tmp_path)
    signed_in(tmp_path / "home")
    permissions = {"preset": "workspace-write", **getattr(request, "param", {})}
    async for made in writing_desk(ctx, tmp_path, upstreams, permissions):
        yield made


async def test_a_comment_needs_a_yes_and_is_not_written_where_nobody_can_give_one(
    asking: Desk, github: FakeGitHub
) -> None:
    outcome = await asking.call("github_comment", ref="octocat/hello#12", body="Hi.")

    assert outcome.error is not None
    assert outcome.error.code == "APPROVAL_UNAVAILABLE"
    assert github.comments == {}


async def test_no_yes_counts_for_later_writes(asking: Desk) -> None:
    permissions = asking.ctx.use(PERMISSIONS)
    target = "octocat/hello#12: comment\n\nHi."

    for name in ("github_comment", "github_push", "github_open_pull_request"):
        assert permissions.standing(asking.session, name, target) is None


async def test_a_session_with_full_access_writes_without_asking(
    asking: Desk, github: FakeGitHub
) -> None:
    asking.sandbox.set_mode(asking.session, "danger-full-access")

    outcome = await asking.call("github_comment", ref="octocat/hello#12", body="Hi.")

    assert outcome.error is None, outcome.text
    assert github.comments["octocat/hello#12"][0]["body"] == "Hi."


@pytest.mark.parametrize("asking", [{"allow": ["github_comment"]}], indirect=True)
async def test_a_rule_that_names_the_tool_lets_it_write(asking: Desk, github: FakeGitHub) -> None:
    outcome = await asking.call("github_comment", ref="octocat/hello#12", body="Hi.")

    assert outcome.error is None, outcome.text


# How git is run.


@pytest.fixture
async def on_github(
    ctx: Context, tmp_path: Path, github: FakeGitHub, monkeypatch: pytest.MonkeyPatch
) -> AsyncIterator[Desk]:
    """A desk whose git asks GitHub itself, which is the stand-in of the tests."""
    apart(monkeypatch, tmp_path)
    signed_in(tmp_path / "home")
    async for made in writing_desk(ctx, tmp_path, None):
        yield made


async def test_git_asks_github_with_the_token_in_a_header_and_not_on_its_command_line(
    on_github: Desk, github: FakeGitHub
) -> None:
    outcome = await on_github.call("github_clone", repo="octocat/hello")

    # The stand-in speaks no git, so the clone fails after the first request.
    assert outcome.error is not None
    assert outcome.error.code == "GIT_FAILED"
    [asked] = [one for one in github.asked if one.path.startswith("/octocat/hello.git/")]
    pair = base64.b64encode(b"x-access-token:ghu_first0123456789").decode()
    assert asked.headers["authorization"] == f"Basic {pair}"
    assert "ghu_" not in outcome.text


def a_finished_run(*, exit_code: int | None, timed_out: bool = False) -> process.Completed:
    return process.Completed(exit_code, timed_out, "", 0, 0, 0)


async def test_git_that_cannot_be_started_is_said_to_be_so(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def refused(*args: object, **kwargs: object) -> process.Completed:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(process, "run", refused)

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.error is not None
    assert outcome.error.code == "GIT_FAILED"
    assert outcome.text.startswith("git could not be started: ")


async def test_git_that_takes_too_long_is_stopped_and_said_to_be(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub, monkeypatch: pytest.MonkeyPatch
) -> None:
    a_clone_with_a_branch(desk, upstreams, tmp_path, github)

    async def slow(*args: object, **kwargs: object) -> process.Completed:
        return a_finished_run(exit_code=None, timed_out=True)

    monkeypatch.setattr(process, "run", slow)

    for name, arguments in (
        ("github_clone", {"repo": "octocat/other"}),
        ("github_push", {"path": "hello"}),
    ):
        outcome = await desk.call(name, **arguments)

        assert outcome.error is not None
        assert outcome.error.code == "TIMEOUT"
        assert outcome.text.startswith("git took longer than ")


async def test_git_that_fails_without_a_word_is_said_to_have_failed(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def failed(*args: object, **kwargs: object) -> process.Completed:
        return a_finished_run(exit_code=128)

    monkeypatch.setattr(process, "run", failed)

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.text == "git could not clone octocat/hello: git ended with 128"


async def test_a_push_that_git_cannot_make_says_why(
    desk: Desk, upstreams: Path, tmp_path: Path, github: FakeGitHub
) -> None:
    a_clone_with_a_branch(desk, upstreams, tmp_path, github)
    (upstreams / "octocat" / "hello.git").rename(upstreams / "octocat" / "gone.git")

    outcome = await desk.call("github_push", path="hello")

    assert outcome.error is not None
    assert outcome.error.code == "GIT_FAILED"
    assert outcome.text.startswith("git could not push fix-parser to octocat/hello: ")


async def test_a_place_that_moved_after_the_check_is_not_cloned_into(
    desk: Desk, upstreams: Path, tmp_path: Path
) -> None:
    upstream(upstreams, tmp_path)
    (desk.work / "first").mkdir()
    (desk.work / "second").mkdir()
    link_directory(desk.work / "door", desk.work / "first")

    async def turning(context: Context, invocation: Invocation) -> Deny | None:
        unlink_directory(desk.work / "door")
        link_directory(desk.work / "door", desk.work / "second")
        return None

    desk.tools.guards.add(desk.ctx, "turning", turning)

    outcome = await desk.call("github_clone", repo="octocat/hello", path="door/hello")

    assert outcome.error is not None
    assert outcome.error.code == "MOVED"
    assert desk.names("second") == []


async def test_git_is_not_looked_for_in_the_working_directory(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    looked: list[tuple[str, list[Path]]] = []

    def found(name: str, search: str, outside: list[Path]) -> None:
        looked.append((name, list(outside)))

    monkeypatch.setattr(programs, "found", found)

    await desk.call("github_clone", repo="octocat/hello")

    assert looked == [("git", [desk.work])]


async def test_what_git_says_is_told_without_the_token(
    desk: Desk, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def telling(*args: object, **kwargs: object) -> process.Completed:
        said = "fatal: Authorization: token ghu_first0123456789 was refused"
        return process.Completed(1, False, said, len(said), 0, 0)

    monkeypatch.setattr(process, "run", telling)

    outcome = await desk.call("github_clone", repo="octocat/hello")

    assert outcome.text == (
        "git could not clone octocat/hello: fatal: Authorization: token [token] was refused"
    )
