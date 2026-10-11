"""What the tests of the service process share: the process, running until the test is over."""

import asyncio
from collections.abc import AsyncIterator

import pytest

from support.loop import Harness
from support.served import Served, loaded
from support.wires import Far


@pytest.fixture
async def served(harness: Harness) -> AsyncIterator[Served]:
    """A service process that runs until the test is over."""
    far = Far()
    running = await loaded(harness, far)
    yield Served(harness, far, running)
    if not running.done():
        far.hang_up()
    await asyncio.wait_for(running, 10)
    # Whatever a test was about: no request was answered twice, and no answer was answered.
    assert far.answered_too_often() == []
