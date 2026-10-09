"""The sessions that are stored for a working directory, as a list shows them."""

import logging
import os
from pathlib import Path
from typing import Any

import pytest

import columbina.session
from columbina.kernel import Context
from columbina.llm import TextBlock, UserMessage
from columbina.session import SESSIONS, Listed, Sessions, TurnStartEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE

SECOND = 1_000_000_000


def said(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    return ctx.use(SESSIONS)


async def keep(
    sessions: Sessions, ctx: Context, work: Path, session_id: str, *events: object, at: int
) -> None:
    """Store a session that holds these events and was last written at second ``at``."""
    session = sessions.create(ctx, work, session_id=session_id)
    for event in events:
        session.append(event)  # pyright: ignore[reportArgumentType]
    await session.flush()
    session.close()
    file = sessions.directory(work, session_id) / SESSION_FILE
    os.utime(file, ns=(at * SECOND, at * SECOND))


async def test_the_sessions_of_a_working_directory_are_listed_the_last_used_first(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work, other = tmp_path / "work", tmp_path / "other"
    await keep(sessions, ctx, work, "older", said("Fix the parser."), at=1000)
    await keep(sessions, ctx, work, "newer", said("Write the notes."), at=3000)
    await keep(sessions, ctx, work, "middle", said("Run the checks."), at=2000)
    await keep(sessions, ctx, other, "elsewhere", said("Not of this directory."), at=4000)

    assert sessions.stored(work) == [
        Listed("newer", 3_000_000, "Write the notes."),
        Listed("middle", 2_000_000, "Run the checks."),
        Listed("older", 1_000_000, "Fix the parser."),
    ]


async def test_sessions_that_were_written_in_the_same_moment_come_in_the_order_of_their_ids(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "a", said("One."), at=1000)
    await keep(sessions, ctx, work, "c", said("Three."), at=1000)
    await keep(sessions, ctx, work, "b", said("Two."), at=1000)

    assert [listed.id for listed in sessions.stored(work)] == ["c", "b", "a"]


async def test_a_session_is_shown_with_what_the_user_said_first(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(
        sessions,
        ctx,
        work,
        "s-1",
        TurnStartEvent(turn=1),
        said("<context>The rules of the project.</context>", source="context/instructions"),
        said("  Fix the parser.\nIt fails on empty input.  "),
        said("And then the tests."),
        at=1000,
    )

    assert sessions.stored(work) == [Listed("s-1", 1_000_000, "Fix the parser.")]


async def test_a_long_first_input_is_cut_short(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", said("word " * 40), at=1000)

    shown = sessions.stored(work)[0].first_input

    assert shown is not None
    assert shown == ("word " * 40)[:79] + "..."
    assert len(shown) == 82


async def test_a_session_in_which_the_user_said_nothing_is_listed_without_words(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "begun", TurnStartEvent(turn=1), at=1000)
    await keep(sessions, ctx, work, "told", said("A fact.", source="context/runtime"), at=2000)

    assert sessions.stored(work) == [
        Listed("told", 2_000_000, None),
        Listed("begun", 1_000_000, None),
    ]


async def test_a_directory_that_holds_no_session_is_passed_over(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", said("One."), at=1000)
    beside = sessions.directory(work, "s-1").parent
    (beside / "not-a-session").mkdir()
    (beside / "a-file").write_text("x", encoding="utf-8")

    assert [listed.id for listed in sessions.stored(work)] == ["s-1"]


async def test_a_session_that_cannot_be_read_is_left_out_and_the_rest_is_listed(
    sessions: Sessions, ctx: Context, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "good", said("One."), at=2000)
    await keep(sessions, ctx, work, "torn", said("Two."), at=1000)
    await keep(sessions, ctx, work, "other-format", said("Three."), at=500)
    torn = sessions.directory(work, "torn") / SESSION_FILE
    torn.write_bytes(b"not a session\n")
    other = sessions.directory(work, "other-format") / SESSION_FILE
    lines = other.read_bytes().split(b"\n")
    other.write_bytes(b"\n".join([lines[0], b"{ not json", *lines[2:]]))

    with caplog.at_level(logging.WARNING, logger="columbina.session"):
        listed = sessions.stored(work)

    assert [one.id for one in listed] == ["good"]
    assert sorted(record.getMessage() for record in caplog.records) == [
        f"the session in {other.parent} is left out of the list: line 2 is not an event",
        f"the session in {torn.parent} is left out of the list: "
        "the first line is not a session header",
    ]


@pytest.mark.parametrize(
    "line",
    [
        b"{ not json",
        b"[1, 2]",
        b'{"type":"user/message","seq":0,"time":1,"data":{"source":"user"}}',
    ],
    ids=["no-json", "no-object", "no-message"],
)
async def test_a_line_that_is_no_event_makes_a_session_unreadable(
    sessions: Sessions,
    ctx: Context,
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
    line: bytes,
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", TurnStartEvent(turn=1), said("One."), at=1000)
    file = sessions.directory(work, "s-1") / SESSION_FILE
    lines = file.read_bytes().split(b"\n")
    file.write_bytes(b"\n".join([lines[0], lines[1], line, *lines[3:]]))

    with caplog.at_level(logging.WARNING, logger="columbina.session"):
        listed = sessions.stored(work)

    assert listed == []
    assert [record.getMessage() for record in caplog.records] == [
        f"the session in {file.parent} is left out of the list: line 3 is not an event"
    ]


async def test_a_session_file_that_cannot_be_opened_is_left_out(
    sessions: Sessions,
    ctx: Context,
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", said("One."), at=1000)
    really_open: Any = Path.open

    def locked(path: Path, *args: Any, **options: Any) -> Any:
        if path.name == SESSION_FILE:
            raise PermissionError(13, "Permission denied")
        return really_open(path, *args, **options)

    monkeypatch.setattr(Path, "open", locked)

    with caplog.at_level(logging.WARNING, logger="columbina.session"):
        listed = sessions.stored(work)

    assert listed == []
    assert "is left out of the list: [Errno 13] Permission denied" in caplog.text


async def test_a_session_whose_last_line_was_never_finished_and_holds_no_input_is_listed(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", TurnStartEvent(turn=1), at=1000)
    file = sessions.directory(work, "s-1") / SESSION_FILE
    with file.open("ab") as log:
        log.write(b'{"type":"user/mess')
    os.utime(file, ns=(1000 * SECOND, 1000 * SECOND))

    assert sessions.stored(work) == [Listed("s-1", 1_000_000, None)]


async def test_a_last_line_that_was_never_finished_does_not_hide_a_session(
    sessions: Sessions, ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    await keep(sessions, ctx, work, "s-1", said("One."), at=1000)
    file = sessions.directory(work, "s-1") / SESSION_FILE
    with file.open("ab") as log:
        log.write(b'{"type":"user/mess')
    os.utime(file, ns=(1000 * SECOND, 1000 * SECOND))

    assert sessions.stored(work) == [Listed("s-1", 1_000_000, "One.")]


async def test_a_working_directory_without_sessions_has_none(
    sessions: Sessions, tmp_path: Path
) -> None:
    assert sessions.stored(tmp_path / "work") == []


async def test_sessions_that_are_kept_in_memory_are_not_listed(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session)
    sessions = ctx.use(SESSIONS)
    session = sessions.create(ctx, tmp_path, session_id="s-1")
    session.append(said("One."))

    assert sessions.stored(tmp_path) == []
