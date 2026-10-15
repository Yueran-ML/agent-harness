"""A GitHub on this machine, for the tests: the device flow of an app, its tokens, and the REST API.

One server stands for github.com and for api.github.com at once, on one port: ``/login/...``
and the pictures of users are the one, the rest is the API. A test lays out what it answers,
and reads what it was asked. The server asks nobody for the name of the machine.
"""

import json
import socket
import socketserver
import sys
import threading
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler
from typing import Any
from urllib.parse import parse_qs, urlsplit

import pytest

__all__ = ["CLIENT_ID", "PICTURE", "SLUG", "Asked", "FakeGitHub", "github", "serving"]

CLIENT_ID = "Iv23liFakeClient"
SLUG = "columbina-test"
PICTURE = b"\x89PNG\r\n\x1a\nnot really a picture"
_HOURS_8 = 28_800
_MONTHS_6 = 15_897_600


@dataclass(frozen=True)
class Asked:
    """One request: its method, its path with the query, its headers, and what it sent."""

    method: str
    path: str
    headers: dict[str, str]
    form: dict[str, str]
    body: Any


@dataclass
class FakeGitHub:
    """What the stand-in answers, and what it was asked.

    Attributes:
        address: Where it is, ``http://127.0.0.1:<port>``.
        asked: Every request, in the order in which it came.
        polls: What the device flow answers to each poll, in order. The last
            one is answered again. ``None`` stands for the tokens.
        access: The token for the API that is given next, and the ones that hold.
        renewal: The token that renews it that is given next, and the ones that hold.
        renewals: How many times the tokens were renewed.
        user: What ``/user`` answers.
        installations: The installations of the app that the user can reach, each
            with its repositories.
        repos: The repositories by ``owner/name``.
        issues: The issues and pull requests by ``owner/name#number``.
        comments: Their comments, by the same key.
        files: The files of a pull request, by the same key.
        checks: The check runs of a commit, by ``owner/name@ref``.
        statuses: The combined status of a commit, by ``owner/name@ref``.
        pulls_made: The pull requests that were opened.
        failing: What the next request is answered with instead: a status, a
            body and headers.
    """

    address: str = ""
    asked: list[Asked] = field(default_factory=list[Asked])
    device: dict[str, Any] = field(default_factory=dict[str, Any])
    polls: list[dict[str, Any] | None] = field(default_factory=lambda: [None])
    access: list[str] = field(default_factory=lambda: ["ghu_first0123456789"])
    renewal: list[str] = field(default_factory=lambda: ["ghr_first0123456789"])
    expires_in: int = _HOURS_8
    renewals: int = 0
    user: dict[str, Any] = field(default_factory=dict[str, Any])
    installations: list[dict[str, Any]] = field(default_factory=list[dict[str, Any]])
    repos: dict[str, dict[str, Any]] = field(default_factory=dict[str, dict[str, Any]])
    issues: dict[str, dict[str, Any]] = field(default_factory=dict[str, dict[str, Any]])
    comments: dict[str, list[dict[str, Any]]] = field(
        default_factory=dict[str, list[dict[str, Any]]]
    )
    files: dict[str, list[dict[str, Any]]] = field(default_factory=dict[str, list[dict[str, Any]]])
    checks: dict[str, list[dict[str, Any]]] = field(default_factory=dict[str, list[dict[str, Any]]])
    statuses: dict[str, dict[str, Any]] = field(default_factory=dict[str, dict[str, Any]])
    pulls_made: list[dict[str, Any]] = field(default_factory=list[dict[str, Any]])
    failing: tuple[int, Any, dict[str, str]] | None = None
    lock: threading.Lock = field(default_factory=threading.Lock)

    def fails_next(self, status: int, body: Any, headers: dict[str, str] | None = None) -> None:
        """Answer the next request with this, whatever it asks."""
        self.failing = (status, body, headers or {})

    def holding(self, token: str) -> bool:
        """Whether a token for the API is one that holds."""
        return token in self.access

    def answer(self, asked: Asked) -> tuple[int, Any, dict[str, str]]:
        """What a request is answered with."""
        with self.lock:
            self.asked.append(asked)
            if self.failing is not None:
                failing, self.failing = self.failing, None
                return failing
            path = urlsplit(asked.path).path
            if asked.method == "POST" and path == "/login/device/code":
                return 200, self.device or self._device(), {}
            if asked.method == "POST" and path == "/login/oauth/access_token":
                return 200, self._token(asked.form), {}
            if path.startswith("/avatars/"):
                return 200, PICTURE, {"Content-Type": "image/png"}
            token = asked.headers.get("authorization", "").removeprefix("Bearer ")
            if not self.holding(token):
                return 401, {"message": "Bad credentials"}, {}
            return self._api(asked.method, path, asked.body)

    def _device(self) -> dict[str, Any]:
        return {
            "device_code": "device-code-0123",
            "user_code": "WDJB-MJHT",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 899,
            "interval": 5,
        }

    def _token(self, form: dict[str, str]) -> dict[str, Any]:
        if form.get("grant_type") == "refresh_token":
            if form.get("refresh_token") != self.renewal[-1]:
                return {
                    "error": "bad_refresh_token",
                    "error_description": "The refresh token passed is incorrect or expired.",
                }
            self.renewals += 1
            self.access.append(f"ghu_renewed{self.renewals}0123456789")
            self.renewal.append(f"ghr_renewed{self.renewals}0123456789")
            return self._tokens()
        said = self.polls[0] if len(self.polls) == 1 else self.polls.pop(0)
        return self._tokens() if said is None else said

    def _tokens(self) -> dict[str, Any]:
        return {
            "access_token": self.access[-1],
            "expires_in": self.expires_in,
            "refresh_token": self.renewal[-1],
            "refresh_token_expires_in": _MONTHS_6,
            "token_type": "bearer",
            "scope": "",
        }

    def _api(self, method: str, path: str, body: Any) -> tuple[int, Any, dict[str, str]]:
        parts = path.strip("/").split("/")
        if parts == ["user"]:
            return 200, self.user, {}
        if parts == ["user", "installations"]:
            listed = [
                {key: value for key, value in one.items() if key != "repositories"}
                for one in self.installations
            ]
            return 200, {"total_count": len(listed), "installations": listed}, {}
        if (
            len(parts) == 4
            and parts[:2] == ["user", "installations"]
            and parts[3] == "repositories"
        ):
            found = next((one for one in self.installations if str(one["id"]) == parts[2]), None)
            if found is None:
                return 404, {"message": "Not Found"}, {}
            return (
                200,
                {"total_count": len(found["repositories"]), "repositories": found["repositories"]},
                {},
            )
        if len(parts) < 3 or parts[0] != "repos":
            return 404, {"message": "Not Found"}, {}
        repo, rest = f"{parts[1]}/{parts[2]}", parts[3:]
        if not rest:
            return (
                (200, self.repos[repo], {})
                if repo in self.repos
                else (404, {"message": "Not Found"}, {})
            )
        if rest[0] == "issues" and len(rest) == 2:
            key = f"{repo}#{rest[1]}"
            return (
                (200, self.issues[key], {})
                if key in self.issues
                else (404, {"message": "Not Found"}, {})
            )
        if rest[0] == "issues" and len(rest) == 3 and rest[2] == "comments":
            key = f"{repo}#{rest[1]}"
            if method == "POST":
                made = {
                    "id": 900 + len(self.comments.get(key, [])),
                    "body": body["body"],
                    "html_url": f"https://github.com/{repo}/issues/{rest[1]}#issuecomment-900",
                    "user": {"login": "octocat"},
                }
                self.comments.setdefault(key, []).append(made)
                return 201, made, {}
            return 200, self.comments.get(key, []), {}
        if rest[0] == "pulls" and len(rest) == 1 and method == "POST":
            number = 100 + len(self.pulls_made)
            made = {
                "number": number,
                "html_url": f"https://github.com/{repo}/pull/{number}",
                **body,
            }
            self.pulls_made.append(made)
            return 201, made, {}
        if rest[0] == "pulls" and len(rest) == 2:
            key = f"{repo}#{rest[1]}"
            return (
                (200, self.issues[key]["pull"], {})
                if key in self.issues
                else (404, {"message": "Not Found"}, {})
            )
        if rest[0] == "pulls" and len(rest) == 3 and rest[2] == "files":
            return 200, self.files.get(f"{repo}#{rest[1]}", []), {}
        if rest[0] == "commits" and len(rest) == 3 and rest[2] == "check-runs":
            runs = self.checks.get(f"{repo}@{rest[1]}", [])
            return 200, {"total_count": len(runs), "check_runs": runs}, {}
        if rest[0] == "commits" and len(rest) == 3 and rest[2] == "status":
            return (
                200,
                self.statuses.get(f"{repo}@{rest[1]}", {"state": "pending", "statuses": []}),
                {},
            )
        return 404, {"message": "Not Found"}, {}


class _Answering(BaseHTTPRequestHandler):
    """Hands each request to the stand-in, and writes back what it answers."""

    github: FakeGitHub

    def _answer(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        kind = self.headers.get("Content-Type", "")
        form = (
            {key: values[-1] for key, values in parse_qs(raw.decode()).items()}
            if "form" in kind
            else {}
        )
        body = json.loads(raw) if raw and "json" in kind else None
        headers = {key.lower(): value for key, value in self.headers.items()}
        status, said, more = self.github.answer(Asked(self.command, self.path, headers, form, body))
        content = said if isinstance(said, bytes) else json.dumps(said).encode()
        self.send_response(status)
        self.send_header("Content-Type", more.pop("Content-Type", "application/json"))
        for name, value in more.items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def do_GET(self) -> None:
        self._answer()

    def do_POST(self) -> None:
        self._answer()

    def log_message(self, format: str, *args: object) -> None:
        """Say nothing of a request on the screen of the test."""


class _OnThisMachine(socketserver.ThreadingTCPServer):
    """The server of the stand-in. A client that went away before its answer is no error of it.

    The answer may be written while the next test runs, and that test is not told of it.
    """

    daemon_threads = True
    block_on_close = False

    def handle_error(
        self, request: socket.socket | tuple[bytes, socket.socket], client_address: Any
    ) -> None:
        if not isinstance(sys.exc_info()[1], ConnectionError):
            super().handle_error(request, client_address)


@contextmanager
def serving() -> Generator[FakeGitHub]:
    """Serve a GitHub on this machine until the block is left."""
    fake = FakeGitHub()
    handler = type("Handler", (_Answering,), {"github": fake})
    with _OnThisMachine(("127.0.0.1", 0), handler) as server:
        fake.address = f"http://127.0.0.1:{server.server_address[1]}"
        fake.user = {
            "login": "octocat",
            "name": "The Octocat",
            "avatar_url": f"{fake.address}/avatars/u/583231?v=4",
        }
        thread = threading.Thread(target=server.serve_forever, args=(0.05,), daemon=True)
        thread.start()
        try:
            yield fake
        finally:
            server.shutdown()
            thread.join(timeout=5)


@pytest.fixture
def github(monkeypatch: pytest.MonkeyPatch) -> Generator[FakeGitHub]:
    """A GitHub on this machine, and Columbina pointed at it, with an app of the tests."""
    with serving() as fake:
        monkeypatch.setenv("COLUMBINA_GITHUB_CLIENT_ID", CLIENT_ID)
        monkeypatch.setenv("COLUMBINA_GITHUB_SLUG", SLUG)
        monkeypatch.setenv("COLUMBINA_GITHUB_WEB", fake.address)
        monkeypatch.setenv("COLUMBINA_GITHUB_API", fake.address)
        yield fake
