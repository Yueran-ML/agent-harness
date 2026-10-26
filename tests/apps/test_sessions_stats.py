"""``columbina sessions stats``: how the tasks of the stored sessions went."""

import asyncio
import json
import logging
from pathlib import Path

import pytest

import columbina.agent
import columbina.session
from columbina import verdicts
from columbina.apps.launcher import main
from columbina.apps.sessions import counted_sessions
from columbina.kernel import Context
from columbina.session import (
    SESSIONS,
    EventData,
    SessionError,
    TurnEndEvent,
    TurnStartEvent,
)
from columbina.session.store import SESSION_FILE, workspace_key
from columbina.verdicts import VerdictEvent


def written(home: Path, work: Path, session_id: str, *happened: EventData) -> None:
    """Store a session that holds these events, the way a program stores it."""

    async def write() -> None:
        ctx = Context()
        try:
            await ctx.plugin(columbina.session, {"root": home / "sessions"})
            await ctx.plugin(columbina.agent)
            await ctx.plugin(verdicts)
            session = ctx.use(SESSIONS).create(ctx, work, session_id=session_id)
            for data in happened:
                session.append(data)
        finally:
            await ctx.fiber.dispose()

    asyncio.run(write())


def a_turn(turn: int, reason: str = "completed") -> tuple[EventData, EventData]:
    return TurnStartEvent(turn=turn), TurnEndEvent(turn=turn, reason=reason)  # type: ignore[arg-type]


@pytest.fixture
def work(tmp_path: Path) -> Path:
    place = tmp_path / "work"
    place.mkdir()
    return place


def counted_by_main(*arguments: str, capsys: pytest.CaptureFixture[str]) -> tuple[int, str, str]:
    capsys.readouterr()
    code = main(["sessions", "stats", *arguments])
    captured = capsys.readouterr()
    return code, captured.out, captured.err


def test_the_sessions_of_a_directory_are_counted(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "s-1", *a_turn(1), VerdictEvent(turn=1, right=True), *a_turn(2, "error"))

    code, out, err = counted_by_main("--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    assert out.split("\n")[:2] == [
        "Conversations: 1. Turns: 2 (1 done, 1 failed).",
        "Said to be right: 1 of the 1 turns that were judged (100%).",
    ]


def test_every_directory_is_counted_with_all_and_a_program_reads_json(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    other = tmp_path / "other"
    other.mkdir()
    written(home, work, "s-1", *a_turn(1))
    written(home, other, "s-2", *a_turn(1))

    code, out, _ = counted_by_main("--cwd", str(work), "--json", capsys=capsys)
    assert code == 0
    assert json.loads(out)["sessions"] == 1
    code, out, _ = counted_by_main("--cwd", str(work), "--all", "--json", capsys=capsys)
    assert code == 0
    assert json.loads(out)["sessions"] == 2


def test_turns_older_than_the_days_asked_about_are_not_counted(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "s-1", *a_turn(1))

    code, out, _ = counted_by_main("--cwd", str(work), "--days", "0", "--json", capsys=capsys)

    assert code == 0
    assert json.loads(out)["sessions"] == 0


def test_what_cannot_be_read_is_passed_over(
    home: Path, work: Path, caplog: pytest.LogCaptureFixture, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "s-1", *a_turn(1))
    store = home / "sessions" / workspace_key(work)
    (store / "broken").mkdir()
    (store / "broken" / SESSION_FILE).write_text("not a session\n", encoding="utf-8")
    (store / "empty").mkdir()
    (store / "a file").write_text("", encoding="utf-8")
    (home / "sessions" / "not a directory").write_text("", encoding="utf-8")

    with caplog.at_level(logging.WARNING, "columbina.session"):
        code, out, _ = counted_by_main("--cwd", str(work), "--all", "--json", capsys=capsys)

    assert code == 0
    assert json.loads(out)["sessions"] == 1
    assert "is not counted" in caplog.text


def test_a_store_that_is_not_there_counts_nothing(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    for wider in (("--all",), ()):
        code, out, _ = counted_by_main("--cwd", str(work), *wider, "--json", capsys=capsys)
        assert code == 0
        assert json.loads(out)["sessions"] == 0


async def test_sessions_in_memory_cannot_be_counted(ctx: Context, tmp_path: Path) -> None:
    await ctx.plugin(columbina.session)

    with pytest.raises(SessionError, match="sessions are kept in memory"):
        counted_sessions(ctx, tmp_path, everywhere=False, days=1, now_ms=0)


async def test_without_the_plugin_of_the_prices_the_known_ones_count(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})

    tally = counted_sessions(ctx, tmp_path, everywhere=True, days=1, now_ms=0)

    assert tally.sessions == 0
