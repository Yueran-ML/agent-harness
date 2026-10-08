from collections.abc import AsyncIterator

import pytest

from tack.kernel import Context


@pytest.fixture
async def ctx() -> AsyncIterator[Context]:
    """A root context that is disposed when the test ends."""
    root = Context()
    yield root
    await root.fiber.dispose()
