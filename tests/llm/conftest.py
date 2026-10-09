import pytest

import columbina.llm
from columbina.kernel import Context
from columbina.llm import LLM, Llm


@pytest.fixture
async def llm(ctx: Context) -> Llm:
    """The model service, loaded as a plugin with a short idle timeout."""
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 0.05})
    return ctx.use(LLM)
