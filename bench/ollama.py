"""An Ollama of its own, for the local models of the benchmark.

The models are kept in ``D:/ollama-models``, beside an Ollama that serves
other work and whose settings are not to be changed. So the benchmark starts
another server: the same program, the models of that directory, a port of
its own, and the length of context that the attempts need. It is stopped,
with all that it started, when the attempts are done. The Ollama that was
there before is not touched: it is another process, on another port.
"""

import asyncio
import os
import signal
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from types import TracebackType
from typing import Any

import httpx

__all__ = ["MODELS", "Ollama"]

MODELS = Path("D:/ollama-models")
"""Where the local models are kept."""


class Ollama:
    """``ollama serve`` with a directory of models, a port and a length of context of its own.

    Attributes:
        models: The directory of the models.
        port: The port that it listens on, on this machine alone.
        context: How many tokens of context a model is given.
        command: How the server is started.
        seconds: How long it may take to answer once it was started.
    """

    def __init__(
        self,
        models: Path = MODELS,
        *,
        port: int = 11435,
        context: int = 16384,
        command: Sequence[str] = ("ollama", "serve"),
        seconds: float = 60.0,
    ) -> None:
        self.models = models
        self.port = port
        self.context = context
        self.command = tuple(command)
        self.seconds = seconds
        self._process: asyncio.subprocess.Process | None = None

    @property
    def base_url(self) -> str:
        """The address that Columbina talks to it at, as a service of the OpenAI protocol."""
        return f"http://127.0.0.1:{self.port}/v1"

    async def __aenter__(self) -> "Ollama":
        env = {
            **os.environ,
            "OLLAMA_MODELS": str(self.models),
            "OLLAMA_HOST": f"127.0.0.1:{self.port}",
            "OLLAMA_CONTEXT_LENGTH": str(self.context),
        }
        options: dict[str, Any] = (
            {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}  # pragma: windows only
            if sys.platform == "win32"
            else {"start_new_session": True}  # pragma: posix only
        )
        self._process = await asyncio.create_subprocess_exec(
            *self.command,
            env=env,
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.DEVNULL,
            **options,
        )
        loop = asyncio.get_running_loop()
        until = loop.time() + self.seconds
        async with httpx.AsyncClient(timeout=2.0) as client:
            while loop.time() < until:
                try:
                    answer = await client.get(f"http://127.0.0.1:{self.port}/api/version")
                except httpx.HTTPError:
                    answer = None
                if answer is not None and answer.status_code == 200:
                    return self
                await asyncio.sleep(0.2)
        await self.__aexit__(None, None, None)
        raise RuntimeError(
            f"ollama did not answer on port {self.port} within {self.seconds:g} seconds"
        )

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        trace: TracebackType | None,
    ) -> None:
        process, self._process = self._process, None
        if process is None:
            return
        if process.returncode is None:
            if sys.platform == "win32":  # pragma: windows only
                # The server starts a runner for each model; the whole tree goes.
                stopping = await asyncio.create_subprocess_exec(
                    "taskkill",
                    "/PID",
                    str(process.pid),
                    "/T",
                    "/F",
                    stdout=asyncio.subprocess.DEVNULL,
                    stderr=asyncio.subprocess.DEVNULL,
                )
                await stopping.wait()
            else:  # pragma: posix only
                os.killpg(process.pid, signal.SIGTERM)
        await process.wait()
