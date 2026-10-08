from collections.abc import AsyncIterator

import pytest

import tack.llm
from tack.kernel import Context
from tack.llm import LLM, Llm


@pytest.fixture
async def ctx() -> AsyncIterator[Context]:
    """A root context that is disposed when the test ends."""
    root = Context()
    yield root
    await root.fiber.dispose()


@pytest.fixture
async def llm(ctx: Context) -> Llm:
    """The model service, loaded as a plugin with a short idle timeout."""
    await ctx.plugin(tack.llm, {"idle_timeout_seconds": 0.05})
    return ctx.use(LLM)
