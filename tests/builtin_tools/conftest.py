"""What the tests of the tools that work on files share."""

from pathlib import Path

import pytest

import columbina.fs
import columbina.sandbox
import columbina.session
import columbina.tools
from columbina.builtin_tools import edit, glob, read, write
from columbina.fs import FS
from columbina.kernel import Context
from columbina.sandbox import SANDBOX
from columbina.session import SESSIONS
from columbina.tools import TOOLS
from support.desk import Desk


@pytest.fixture
async def desk(ctx: Context, tmp_path: Path) -> Desk:
    """A working directory with a session and the tools that read and write files."""
    work = tmp_path / "work"
    work.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(read)
    await ctx.plugin(write)
    await ctx.plugin(edit)
    await ctx.plugin(glob)
    session = ctx.use(SESSIONS).create(ctx, work)
    return Desk(ctx, ctx.use(TOOLS), ctx.use(FS), ctx.use(SANDBOX), session, work)
