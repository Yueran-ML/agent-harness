"""A service of models on this machine, which answers the list of its models as a test says.

The doctor asks a service for that list, and so does ``columbina setup --check``.
"""

import json
import socket
import socketserver
import sys
import threading
import time
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler
from typing import Any

__all__ = ["Service", "listing", "serving"]


def listing(*ids: str) -> dict[str, Any]:
    """The list of models that a service answers with, as the OpenAI protocol writes it."""
    return {"object": "list", "data": [{"id": id, "object": "model"} for id in ids]}


class _Models(BaseHTTPRequestHandler):
    """Stands in for a service: answers each request as the test said, and keeps it."""

    said: dict[str, Any]
    asked: list[tuple[str, str | None]]
    heard: list[dict[str, str]]

    def do_GET(self) -> None:
        self.asked.append((self.path, self.headers["Authorization"]))
        self.heard.append({name.lower(): value for name, value in self.headers.items()})
        time.sleep(self.said.get("delay", 0.0))
        body = self.said["body"]
        content = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(self.said["status"])
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def log_message(self, format: str, *args: object) -> None:
        """Say nothing of a request on the screen of the test."""


class _OnThisMachine(socketserver.ThreadingTCPServer):
    """The server of the stand-in. It asks nobody for the name of the machine that it runs on.

    An answer that is still being waited out is not waited for when the server closes. A client
    that went away before its answer was written is no error of the stand-in: a test of a
    timeout gives up on purpose, and the answer is written while the next test runs. So that
    test is not told of it on its screen.
    """

    daemon_threads = True
    block_on_close = False

    def handle_error(
        self, request: socket.socket | tuple[bytes, socket.socket], client_address: Any
    ) -> None:
        if not isinstance(sys.exc_info()[1], ConnectionError):
            super().handle_error(request, client_address)


@dataclass
class Service:
    """A service on this machine: where it is, and what it was asked.

    Attributes:
        address: Its address, as a config of the adapter has it.
        asked: The path and the header of the key of each request.
        said: What it answers: a status and a body, after a delay.
        heard: The headers of each request, by their names in lower case.
    """

    address: str
    asked: list[tuple[str, str | None]]
    said: dict[str, Any]
    heard: list[dict[str, str]]

    def answers(self, status: int, body: object) -> None:
        self.said.update(status=status, body=body)

    def waits(self, seconds: float) -> None:
        """Answer each request after so many seconds."""
        self.said.update(delay=seconds)


@contextmanager
def serving(*models: str) -> Generator[Service]:
    """Serve a service on this machine that lists these models, until the block is left."""
    asked: list[tuple[str, str | None]] = []
    heard: list[dict[str, str]] = []
    said: dict[str, Any] = {"status": 200, "body": listing(*models)}
    handler = type("Handler", (_Models,), {"asked": asked, "said": said, "heard": heard})
    with _OnThisMachine(("127.0.0.1", 0), handler) as server:
        thread = threading.Thread(target=server.serve_forever, args=(0.05,), daemon=True)
        thread.start()
        try:
            yield Service(f"http://127.0.0.1:{server.server_address[1]}/v1", asked, said, heard)
        finally:
            server.shutdown()
            thread.join(timeout=5)
