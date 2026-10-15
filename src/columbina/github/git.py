"""git, as the tools of GitHub run it: with the token of the sign-in, and for that run alone.

A token reaches one run of git and nothing more. It goes in as a header of the requests to
GitHub, through the configuration that git takes from its environment (``GIT_CONFIG_COUNT``
and its pairs): not on the command line, where other programs can read it from the list of
processes, and not into a file of the repository, where it would stay. git asks nobody for
anything: there is nobody at a terminal to answer, and a helper for credentials that the
user set up would open a window of its own.
"""

import base64
import re
import subprocess
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

__all__ = ["Push", "described", "git_env", "remote_of"]

_ASKING = ("GIT_ASKPASS", "SSH_ASKPASS")
"""The variables that name a program for git to ask a password of."""
_SECONDS = 5.0
"""How long one run of git that says what a push would do may take."""
_LISTED = 5
"""How many of the commits of a push are named."""
_PATH = re.compile(
    r"^/*(?P<owner>[A-Za-z0-9][A-Za-z0-9-]{0,38})/(?P<repo>[A-Za-z0-9._-]+?)(?:\.git)?/*$"
)


def git_env(base: Mapping[str, str], token: str, web: str, proxy: str | None) -> dict[str, str]:
    """Return the environment of one run of git that reaches GitHub with a token.

    The settings that the environment gave git, ``GIT_CONFIG_COUNT`` and its
    pairs, give way to these, which are counted from the first pair. Those of
    the environment are whole no more anyway once the variables whose names
    sound like secrets are taken out, ``GIT_CONFIG_KEY_0`` among them.

    Args:
        base: The environment to begin with, without secrets.
        token: The token for the API of the sign-in.
        web: Where GitHub is, such as ``https://github.com``.
        proxy: The proxy of the system for GitHub, or ``None``.
    """
    pair = base64.b64encode(f"x-access-token:{token}".encode()).decode()
    env = {name: value for name, value in base.items() if name not in _ASKING}
    settings = (
        (f"http.{web}/.extraheader", f"Authorization: Basic {pair}"),
        # No helper of the user is asked for a password that the header did not give.
        ("credential.helper", ""),
    )
    for number, (key, value) in enumerate(settings):
        env[f"GIT_CONFIG_KEY_{number}"] = key
        env[f"GIT_CONFIG_VALUE_{number}"] = value
    env["GIT_CONFIG_COUNT"] = str(len(settings))
    env["GIT_TERMINAL_PROMPT"] = "0"
    env["GCM_INTERACTIVE"] = "never"
    if proxy is not None:
        env.setdefault("HTTPS_PROXY", proxy)
    return env


def remote_of(url: str, web: str) -> tuple[str, str] | None:
    """Return the owner and the name of the repository on GitHub that a remote leads to.

    Args:
        url: The address of the remote, as ``git remote get-url`` says it:
            ``https://github.com/owner/name.git``, ``git@github.com:owner/name.git``
            or ``ssh://git@github.com/owner/name``.
        web: Where GitHub is. A remote on another machine leads elsewhere.

    Returns:
        The owner and the name, or ``None`` for a remote elsewhere.
    """
    url = url.strip()
    host = urlsplit(web).hostname
    if host is None:
        return None
    scp = re.fullmatch(r"[^@/:\s]+@(?P<host>[^:/\s]+):(?P<path>[^\s]+)", url)
    if scp is not None:
        there, path = scp["host"], scp["path"]
    else:
        parts = urlsplit(url)
        if parts.scheme not in ("https", "http", "ssh") or parts.hostname is None:
            return None
        there, path = parts.hostname, parts.path
        if parts.scheme in ("https", "http") and (parts.scheme, parts.port) != (
            urlsplit(web).scheme,
            urlsplit(web).port,
        ):
            return None
    if there.lower() != host.lower():
        return None
    found = _PATH.match(path)
    return None if found is None else (found["owner"], found["repo"])


@dataclass(frozen=True, slots=True)
class Push:
    """What a push of a branch would do, as far as the repository can tell before it is made.

    Attributes:
        repository: ``owner/name`` on GitHub that ``origin`` leads to, or
            ``None`` where it leads elsewhere or there is none.
        branch: The branch to push, or ``None`` where none is checked out.
        commits: The commits that ``origin`` does not have yet as far as the
            repository knows, the newest first, each as its short hash and
            the first line of its message. At most five are named.
        more: How many more commits there are than are named, or ``None``
            where they could not be counted.
    """

    repository: str | None
    branch: str | None
    commits: tuple[str, ...]
    more: int | None

    def said(self) -> str:
        """Say what the push would do, in one line: what the user is asked about."""
        where = self.repository or "a repository that is not on GitHub"
        if self.branch is None:
            return f"{where}: push no branch, since none is checked out"
        if self.more is None:
            return f"{where}: push {self.branch} (its commits could not be listed)"
        total = len(self.commits) + self.more
        if total == 0:
            return f"{where}: push {self.branch} (no commit that origin does not have)"
        named = "; ".join(self.commits) + ("; ..." if self.more else "")
        counted = "1 commit" if total == 1 else f"{total} commits"
        return f"{where}: push {self.branch} ({counted}: {named})"


def _git(git: str, where: Path, arguments: Sequence[str], env: Mapping[str, str]) -> str | None:
    """Run git in a repository and return what it wrote, or ``None`` if it failed."""
    try:
        done = subprocess.run(
            [git, *arguments],
            cwd=where,
            env=dict(env),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=_SECONDS,
            check=False,
            stdin=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return done.stdout.strip() if done.returncode == 0 else None


def described(git: str, where: Path, branch: str | None, web: str, env: Mapping[str, str]) -> Push:
    """Say what a push of a branch would do. This runs git, and blocks while it does.

    Args:
        git: The program git.
        where: The repository.
        branch: The branch, or ``None`` for the one that is checked out.
        web: Where GitHub is.
        env: The environment of git, without the token.
    """
    url = _git(git, where, ["remote", "get-url", "origin"], env)
    remote = None if url is None else remote_of(url, web)
    repository = None if remote is None else f"{remote[0]}/{remote[1]}"
    if branch is None:
        branch = _git(git, where, ["symbolic-ref", "--quiet", "--short", "HEAD"], env)
    if branch is None:
        return Push(repository, None, (), None)
    ahead = [f"refs/heads/{branch}", "--not", "--remotes=origin"]
    counted = _git(git, where, ["rev-list", "--count", *ahead], env)
    listed = _git(git, where, ["log", f"-n{_LISTED}", "--format=%h %s", *ahead], env)
    if counted is None or not counted.isdigit() or listed is None:
        return Push(repository, branch, (), None)
    commits = tuple(line for line in listed.splitlines() if line)
    return Push(repository, branch, commits, int(counted) - len(commits))
