import asyncio
import os
import webbrowser
from collections.abc import AsyncIterator, Iterator
from pathlib import Path

import keyring
import pytest
from keyring.backends import null

import columbina.agent
import columbina.agent_loop
import columbina.llm
import columbina.session
import columbina.tools
from columbina.agent import AGENTS
from columbina.kernel import Context
from columbina.llm import LLM
from columbina.llm.adapters.scripted import Scripted
from columbina.session import SESSIONS
from columbina.tools import TOOLS
from support import cases
from support.loop import Harness

# What every test goes by that draws its cases.
cases.register()

# How long the end of a test may take. Ending takes a moment at most.
_END_SECONDS = 20

# No test asks the keyring of the system for anything or puts anything into it: what a test
# stores there would lie among the passwords of whoever runs the tests. This is for the
# programs that a test starts. The fixture below is for the tests themselves.
os.environ["PYTHON_KEYRING_BACKEND"] = "keyring.backends.null.Keyring"


@pytest.fixture(autouse=True)
def no_browser_of_the_system(monkeypatch: pytest.MonkeyPatch) -> None:
    """Open no page in a browser of this machine: a test that signs in would open GitHub there.

    A test that looks at what was opened puts a stand-in of its own in.
    """

    def opened(url: str, new: int = 0, autoraise: bool = True) -> bool:
        return False

    monkeypatch.setattr(webbrowser, "open", opened)


@pytest.fixture(autouse=True)
def no_keyring_of_the_system() -> Iterator[None]:
    """Give every test a keyring that keeps nothing, in the place of the one of the system.

    A test of what a keyring does puts one of its own in, see :mod:`support.vault`, and
    it is taken out again here.
    """
    keyring.set_keyring(null.Keyring())
    yield
    keyring.set_keyring(null.Keyring())


@pytest.fixture
async def ctx() -> AsyncIterator[Context]:
    """A root context that is disposed when the test ends.

    A test that has failed may leave something behind that never lets go, and the limit
    that every test has stops counting once a test has failed. So the end has a limit of
    its own: such a test is reported, and does not hold the whole run up.
    """
    root = Context()
    yield root
    await asyncio.wait_for(root.fiber.dispose(), timeout=_END_SECONDS)


@pytest.fixture
async def harness(ctx: Context, tmp_path: Path) -> Harness:
    """The loop with its services, a scripted model, and sessions stored in a temporary place."""
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 5})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.agent)
    await ctx.plugin(columbina.agent_loop)
    model = Scripted()
    ctx.use(LLM).adapters.add(ctx, "scripted", model)
    work = tmp_path / "work"
    work.mkdir()
    return Harness(
        ctx=ctx,
        agents=ctx.use(AGENTS),
        sessions=ctx.use(SESSIONS),
        tools=ctx.use(TOOLS),
        model=model,
        cwd=work,
    )
