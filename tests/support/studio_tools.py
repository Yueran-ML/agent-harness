"""A session whose video tools edit the same Studio project as the desktop."""

import json
import sys
from pathlib import Path
from typing import Any

import columbina.fs
import columbina.sandbox
import columbina.session
import columbina.tools
import columbina.video
from columbina.builtin_tools import edit, read
from columbina.fs import FS
from columbina.kernel import Context
from columbina.sandbox import SANDBOX
from columbina.session import SESSIONS
from columbina.tools import TOOLS
from support.desk import Desk


async def desk_of(ctx: Context, tmp_path: Path, **config: object) -> Desk:
    """Give the session real file permissions and the configured engine connection."""
    work = tmp_path / "work"
    work.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(read)
    await ctx.plugin(edit)
    await ctx.plugin(columbina.video, {"enabled": True, "engine": sys.executable, **config})
    session = ctx.use(SESSIONS).create(ctx, work)
    return Desk(ctx, ctx.use(TOOLS), ctx.use(FS), ctx.use(SANDBOX), session, work)


def result(outcome: Any) -> dict[str, Any]:
    """Read the structured receipt, failing with the tool's explanation on an error."""
    assert outcome.error is None, outcome.text
    return json.loads(outcome.text)
