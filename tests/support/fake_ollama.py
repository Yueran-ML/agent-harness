"""A stand-in for ``ollama serve``: it answers on the port that ``OLLAMA_HOST`` names.

``/api/version`` answers with the address, the directory of models and the
length of context that the environment gave it, so a test can see what the
server was started with.
"""

import http.server
import json
import os
import socketserver

HOST, PORT = os.environ["OLLAMA_HOST"].rsplit(":", 1)


class Answer(http.server.BaseHTTPRequestHandler):
    """Answers every request with what the server was started with."""

    def do_GET(self) -> None:
        body = json.dumps(
            {
                "version": "0.0.0",
                "host": os.environ.get("OLLAMA_HOST"),
                "models": os.environ.get("OLLAMA_MODELS"),
                "context": os.environ.get("OLLAMA_CONTEXT_LENGTH"),
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        """Say nothing."""


# socketserver and not http.server.HTTPServer: that asks for the name of the machine first.
with socketserver.TCPServer((HOST, int(PORT)), Answer) as server:
    server.serve_forever()
