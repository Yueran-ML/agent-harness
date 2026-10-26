"""A language server for tests, run as a program of its own: ``python fake_lsp.py MODE``.

It speaks enough of the protocol to be asked what a real one is asked.

- A file is wrong where a line holds ``ERROR``, and may be where one holds
  ``WARN``. What it says of a file it says twice, the second time a moment
  later, as a server says more as it looks further. It names the file in
  the way of a server of Windows where the path has a drive: the drive in
  small letters, its colon written ``%3A``.
- It asks for its configuration and to register a capability, and goes on
  whatever it is answered.
- A name is defined in the first line of the file, used in its second, and
  is ``a thing``. The file defines ``f``, with ``x`` in it.

The mode changes that: ``silent`` says nothing of files, ``crash`` ends after
it was greeted, ``die`` ends when it is asked something, ``long`` and
``headless`` write a header that is too long or has no length once greeted,
``garbage`` writes what is no message when a file is shown,
``refuse`` answers every question with an error, ``mute`` does not answer the
greeting, and ``nothing`` answers every question with nothing.
"""

import json
import re
import sys
import time
from typing import Any

MODE = sys.argv[1] if len(sys.argv) > 1 else "normal"


def send(message: object) -> None:
    body = json.dumps(message).encode("utf-8")
    head = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n"
    sys.stdout.buffer.write(head + b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def receive() -> dict[str, Any] | None:
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if not line.strip():
            break
        name, _, value = line.decode("ascii").partition(":")
        if name.strip().lower() == "content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))


def windows_like(uri: str) -> str:
    return re.sub(r"^file:///([A-Za-z]):", lambda m: f"file:///{m.group(1).lower()}%3A", uri)


def diagnostics(uri: str, text: str) -> None:
    found: list[dict[str, Any]] = []
    for number, line in enumerate(text.splitlines()):
        for word, severity in (("ERROR", 1), ("WARN", 2)):
            at = line.find(word)
            if at >= 0:
                found.append(
                    {
                        "range": {"start": {"line": number, "character": at}, "end": {}},
                        "severity": severity,
                        "message": f"{word.lower()} here\nmore",
                        "source": "fake",
                    }
                )
    for _ in range(2):
        send(
            {
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {"uri": windows_like(uri), "diagnostics": found},
            }
        )
        time.sleep(0.05)


def answer(said: dict[str, Any]) -> object:
    method = said["method"]
    uri = said.get("params", {}).get("textDocument", {}).get("uri", "")
    if method == "textDocument/definition":
        return {"uri": uri, "range": {"start": {"line": 0, "character": 4}}}
    if method == "textDocument/references":
        return [
            {"targetUri": uri, "targetSelectionRange": {"start": {"line": 1, "character": 0}}},
            {"uri": "file:///elsewhere/other.py"},
            {"range": {"start": {"line": 3}}},
            "junk",
        ]
    if method == "textDocument/hover":
        return {"contents": [{"value": "a thing"}, "more", 3, {"value": 3}]}
    if method == "textDocument/documentSymbol":
        return [
            {
                "name": "f",
                "kind": 12,
                "selectionRange": {"start": {"line": 0}},
                "children": [
                    {"name": "x", "kind": 13, "location": {"range": {"start": {"line": 1}}}}
                ],
            },
            {"name": "odd", "kind": 99},
            "junk",
        ]
    return None


def main() -> None:
    while True:
        said = receive()
        if said is None:
            return
        method = said.get("method")
        if "id" in said and method is None:
            continue
        if method == "initialize":
            if MODE == "mute":
                continue
            send({"jsonrpc": "2.0", "id": said["id"], "result": {"capabilities": {}}})
            send(
                {
                    "jsonrpc": "2.0",
                    "id": 900,
                    "method": "workspace/configuration",
                    "params": {"items": [{"section": "a"}, {"section": "b"}]},
                }
            )
            send({"jsonrpc": "2.0", "id": 901, "method": "client/registerCapability", "params": {}})
            send(["not", "a", "message", "of", "this", "kind"])
            send({"jsonrpc": "2.0", "id": 12345, "result": "an answer that nobody asked for"})
            send({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"message": "hi"}})
            send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {}})
            if MODE == "crash":
                return
            if MODE == "long":
                sys.stdout.buffer.write(b"X" * 5000 + b"\r\n")
                sys.stdout.buffer.flush()
            if MODE == "headless":
                sys.stdout.buffer.write(b"Content-Type: x\r\n\r\n")
                sys.stdout.buffer.flush()
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            params = said["params"]
            uri = params["textDocument"]["uri"]
            text = params["textDocument"].get("text") or params["contentChanges"][0]["text"]
            if MODE == "garbage":
                sys.stdout.buffer.write(b"Content-Length: nonsense\r\n\r\n")
                sys.stdout.buffer.flush()
            elif MODE != "silent":
                diagnostics(uri, text)
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": said["id"], "result": None})
        elif method == "exit":
            return
        elif "id" in said:
            if MODE == "die":
                return
            if MODE == "refuse":
                send({"jsonrpc": "2.0", "id": said["id"], "error": {"code": -1, "message": "no"}})
            else:
                result = None if MODE == "nothing" else answer(said)
                send({"jsonrpc": "2.0", "id": said["id"], "result": result})


if __name__ == "__main__":
    main()
