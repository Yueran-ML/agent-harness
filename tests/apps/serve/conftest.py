"""What the tests of the service process share: the process, running until the test is over."""

import asyncio
import contextlib
from collections.abc import AsyncGenerator, AsyncIterator
from typing import Any

import pytest

from support.loop import MODEL, Harness
from support.served import WINDOW, Served, loaded
from support.wires import Far


@contextlib.asynccontextmanager
async def serving(harness: Harness, **config: Any) -> AsyncGenerator[Served]:
    """Run a service process until the test is over, with what a test says of its config."""
    far = Far()
    running = await loaded(harness, far, **config)
    yield Served(harness, far, running)
    if not running.done():
        far.hang_up()
    await asyncio.wait_for(running, 10)
    # Whatever a test was about: no request was answered twice, and no answer was answered.
    assert far.answered_too_often() == []


@pytest.fixture
async def served(harness: Harness) -> AsyncIterator[Served]:
    """A service process that runs until the test is over."""
    async with serving(harness) as process:
        yield process


@pytest.fixture
async def served_with_a_window(harness: Harness) -> AsyncIterator[Served]:
    """The same, with a model whose window is known: it takes WINDOW tokens."""
    async with serving(harness, model={**MODEL.model_dump(), "context_window": WINDOW}) as process:
        yield process
