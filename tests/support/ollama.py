"""A stand-in for ``ollama serve`` that a test starts, and what it says of itself."""

import socket
import sys
from pathlib import Path

import httpx

__all__ = ["FAKE", "asked", "free_port"]

FAKE = (sys.executable, str(Path(__file__).resolve().parent / "fake_ollama.py"))
"""How the stand-in is started."""


def free_port() -> int:
    """Return a port of this machine that nothing listened on a moment ago."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


async def asked(port: int) -> dict[str, object] | None:
    """Return what the server on a port says of itself, or ``None`` if nothing answers there."""
    async with httpx.AsyncClient(timeout=5) as client:
        try:
            answer = await client.get(f"http://127.0.0.1:{port}/api/version")
        except httpx.HTTPError:
            return None
    told: dict[str, object] = answer.json()
    return told
