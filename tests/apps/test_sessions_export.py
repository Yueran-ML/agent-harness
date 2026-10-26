"""``columbina sessions export``: a stored session written as a page."""

import asyncio
from pathlib import Path

import pytest

import columbina.agent
import columbina.session
from columbina.apps.launcher import main
from columbina.kernel import Context
from columbina.session import SESSIONS
from support.told_sessions import HAPPENED


def stored_one(home: Path, work: Path) -> str:
    async def write() -> str:
        ctx = Context()
        try:
            await ctx.plugin(columbina.session, {"root": home / "sessions"})
            await ctx.plugin(columbina.agent)
            session = ctx.use(SESSIONS).create(ctx, work, session_id="s-1")
            for data in HAPPENED:
                session.append(data)
            return session.id
        finally:
            await ctx.fiber.dispose()

    return asyncio.run(write())


def test_a_stored_session_is_written_out_from_the_command_line(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    work = tmp_path / "work"
    work.mkdir()
    stored_one(home, work)
    out = tmp_path / "page.md"

    assert main(["sessions", "export", "s-1", "--cwd", str(work)]) == 0
    assert capsys.readouterr().out.startswith("<!doctype html>")
    assert (
        main(["sessions", "export", "s-1", "--cwd", str(work), "--format", "md", "--out", str(out)])
        == 0
    )
    assert capsys.readouterr().out == f"Written to {out}\n"
    assert out.read_text(encoding="utf-8").startswith("# Session s-1")
    assert main(["sessions", "export", "none", "--cwd", str(work)]) == 2
    assert main(["sessions", "export", "s-1", "--cwd", str(work), "--out", str(tmp_path)]) == 2
