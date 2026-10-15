"""An Ollama of the benchmark's own: ``bench.ollama``."""

import socket
import sys
from pathlib import Path

import pytest

from bench.ollama import Ollama
from support.ollama import FAKE, asked, free_port


async def test_the_server_is_started_with_its_models_its_port_and_its_context(
    tmp_path: Path,
) -> None:
    port = free_port()
    server = Ollama(tmp_path, port=port, context=8192, command=FAKE)

    async with server:
        assert await asked(port) == {
            "version": "0.0.0",
            "host": f"127.0.0.1:{port}",
            "models": str(tmp_path),
            "context": "8192",
            "parallel": "1",
            "loaded": "1",
        }
        assert server.base_url == f"http://127.0.0.1:{port}/v1"

    assert await asked(port) is None


async def test_a_server_that_does_not_answer_is_stopped_and_said_to_be_so(tmp_path: Path) -> None:
    port = free_port()
    # It takes what is sent to its port, and says nothing.
    listens = (
        "import os, socket, time; "
        "host, port = os.environ['OLLAMA_HOST'].rsplit(':', 1); "
        "held = socket.create_server((host, int(port))); "
        "time.sleep(60)"
    )
    server = Ollama(tmp_path, port=port, command=(sys.executable, "-c", listens), seconds=1.0)

    with pytest.raises(
        RuntimeError, match=f"ollama did not answer on port {port} within 1 seconds"
    ):
        async with server:
            pass

    with pytest.raises(ConnectionRefusedError):
        socket.create_connection(("127.0.0.1", port), timeout=10).close()


async def test_a_server_that_does_not_run_is_let_go_without_a_word(tmp_path: Path) -> None:
    server = Ollama(tmp_path, port=free_port(), command=FAKE)

    await server.__aexit__(None, None, None)
    async with server:
        pass
    await server.__aexit__(None, None, None)


async def test_a_server_that_ended_by_itself_is_not_stopped_again(tmp_path: Path) -> None:
    port = free_port()
    server = Ollama(tmp_path, port=port, command=(sys.executable, "-c", "pass"), seconds=1.0)

    with pytest.raises(RuntimeError, match="did not answer"):
        async with server:
            pass
