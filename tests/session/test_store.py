"""Session files: the log of one session as JSON Lines, written by one process."""

import logging
import os
import sys
from pathlib import Path

import pytest

from columbina.llm import TextBlock, UserMessage
from columbina.session import (
    CORE_EVENTS,
    EventData,
    SessionEvent,
    SessionHeader,
    SessionLockedError,
    SessionStoreError,
    TurnEndEvent,
    TurnStartEvent,
    UnreadableSessionError,
    UserMessageEvent,
    encode_event,
    encode_header,
)
from columbina.session.store import (
    SESSION_FILE,
    SessionFile,
    check_session_id,
    new_session_id,
    read_session,
    workspace_key,
)

KINDS: dict[str, type[EventData]] = {kind.event_type: kind for kind in CORE_EVENTS}
HEADER = SessionHeader(id="s-1", created_at=1000, cwd="/work")


def said(text: str) -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)))


EVENTS: list[SessionEvent[EventData]] = [
    SessionEvent(seq=0, time=1000, data=TurnStartEvent(turn=1)),
    SessionEvent(seq=1, time=1001, data=said("你好"), on_surface=True),
    SessionEvent(seq=2, time=1002, data=said("how are you"), on_surface=True),
    SessionEvent(seq=3, time=1003, data=said("a greeting"), on_surface=True, replaces=(1, 2)),
    SessionEvent(seq=4, time=1004, data=TurnEndEvent(turn=1, reason="completed")),
]


def stored(directory: Path, events: list[SessionEvent[EventData]] = EVENTS) -> Path:
    """Write a session to ``directory`` and close it. Return the path of its file."""
    file = SessionFile.create(directory, HEADER)
    for event in events:
        file.write(event)
    file.close()
    return directory / SESSION_FILE


def lines(*events: SessionEvent[EventData]) -> str:
    return "".join(encode_event(event) + "\n" for event in events)


# Writing


async def test_nothing_is_written_before_the_first_event(tmp_path: Path) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)

    await file.flush()
    file.close()

    assert not (tmp_path / "s-1").exists()


def test_the_file_holds_the_header_and_then_one_line_for_each_event(tmp_path: Path) -> None:
    path = stored(tmp_path / "deep" / "s-1")

    assert path.name == "session.v1.jsonl"
    assert path.read_bytes() == (encode_header(HEADER) + "\n" + lines(*EVENTS)).encode()


def test_an_event_is_in_the_file_as_soon_as_it_is_written(tmp_path: Path) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)

    file.write(EVENTS[0])

    text = (tmp_path / "s-1" / SESSION_FILE).read_text(encoding="utf-8")
    assert text == encode_header(HEADER) + "\n" + lines(EVENTS[0])
    file.close()


async def test_a_barrier_syncs_the_file_to_the_disk(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    synced: list[int] = []
    real = os.fsync

    def fsync(descriptor: int) -> None:
        synced.append(descriptor)
        real(descriptor)

    monkeypatch.setattr(os, "fsync", fsync)
    file = SessionFile.create(tmp_path / "s-1", HEADER)
    file.write(EVENTS[0])

    await file.flush()

    assert len(synced) == 1
    file.close()


def test_closing_twice_is_harmless(tmp_path: Path) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)
    file.write(EVENTS[0])

    file.close()
    file.close()

    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS[:1]


# Reading


def test_a_session_reads_back_as_it_was_written(tmp_path: Path) -> None:
    stored(tmp_path / "s-1")

    content = read_session(tmp_path / "s-1", KINDS)

    assert content.header == HEADER
    assert content.events == EVENTS


def test_a_session_can_be_opened_again_to_go_on_writing(tmp_path: Path) -> None:
    stored(tmp_path / "s-1", EVENTS[:2])

    file, events = SessionFile.open(tmp_path / "s-1", KINDS)
    for event in EVENTS[2:]:
        file.write(event)
    file.close()

    assert file.header == HEADER
    assert events == EVENTS[:2]
    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS


def test_a_session_that_is_not_there_cannot_be_read(tmp_path: Path) -> None:
    with pytest.raises(UnreadableSessionError, match="there is no session file at"):
        read_session(tmp_path / "s-1", KINDS)

    with pytest.raises(UnreadableSessionError, match="there is no session file at"):
        SessionFile.open(tmp_path / "s-1", KINDS)

    assert not (tmp_path / "s-1").exists()


def test_an_empty_file_is_no_session(tmp_path: Path) -> None:
    (tmp_path / "s-1").mkdir()
    (tmp_path / "s-1" / SESSION_FILE).write_bytes(b"")

    with pytest.raises(UnreadableSessionError, match="line 1: the first line is not a session"):
        read_session(tmp_path / "s-1", KINDS)


def test_a_file_of_a_newer_format_is_refused(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1")
    path.write_text(
        path.read_text(encoding="utf-8").replace('"version":1', '"version":2'), encoding="utf-8"
    )

    with pytest.raises(UnreadableSessionError, match="line 1: the session was written in format"):
        read_session(tmp_path / "s-1", KINDS)


def test_a_complete_line_that_is_malformed_makes_the_session_unreadable(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:2])
    with path.open("ab") as file:
        file.write(b'{"type":"turn/start"\n' + lines(EVENTS[2]).encode())

    with pytest.raises(UnreadableSessionError, match=r"session\.v1\.jsonl, line 4: the line is"):
        read_session(tmp_path / "s-1", KINDS)


def test_an_event_of_an_unknown_type_makes_the_session_unreadable(tmp_path: Path) -> None:
    stored(tmp_path / "s-1")
    without_turns = {name: kind for name, kind in KINDS.items() if name != "turn/end"}

    with pytest.raises(
        UnreadableSessionError, match="line 6: the event type 'turn/end' is not known"
    ):
        read_session(tmp_path / "s-1", without_turns)


def test_a_gap_in_the_numbering_makes_the_session_unreadable(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:1])
    with path.open("ab") as file:
        file.write(lines(EVENTS[2]).encode())

    with pytest.raises(UnreadableSessionError, match="line 3: the event has seq 2 where 1 belongs"):
        read_session(tmp_path / "s-1", KINDS)


def test_a_surface_that_does_not_add_up_makes_the_session_unreadable(tmp_path: Path) -> None:
    misplaced = SessionEvent(seq=1, time=1, data=said("summary"), on_surface=True, replaces=(0,))
    stored(tmp_path / "s-1", [EVENTS[0], misplaced])

    with pytest.raises(
        UnreadableSessionError, match="line 3: event 0 is not on the message surface"
    ):
        read_session(tmp_path / "s-1", KINDS)


# An unfinished last line


def test_an_unfinished_last_line_is_not_read(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:2])
    whole = path.read_bytes()
    path.write_bytes(whole + lines(EVENTS[2]).encode()[:-10])

    content = read_session(tmp_path / "s-1", KINDS)

    assert content.events == EVENTS[:2]
    assert content.valid_length == len(whole)
    assert path.stat().st_size > len(whole)


def test_a_last_line_that_only_lacks_its_line_break_is_not_read(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:2])
    path.write_bytes(path.read_bytes() + encode_event(EVENTS[2]).encode())

    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS[:2]


def test_a_header_without_its_line_break_is_no_session(tmp_path: Path) -> None:
    (tmp_path / "s-1").mkdir()
    (tmp_path / "s-1" / SESSION_FILE).write_bytes(encode_header(HEADER).encode())

    with pytest.raises(UnreadableSessionError, match="line 1: the first line is not a session"):
        read_session(tmp_path / "s-1", KINDS)


def test_opening_for_writing_cuts_an_unfinished_last_line_off(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:2])
    whole = path.read_bytes()
    path.write_bytes(whole + b'{"type":"user/mess')

    with caplog.at_level(logging.WARNING, logger="columbina.session"):
        file, events = SessionFile.open(tmp_path / "s-1", KINDS)
    file.write(EVENTS[2])
    file.close()

    assert events == EVENTS[:2]
    assert "an unfinished line of 18 bytes" in caplog.text
    assert path.read_bytes() == whole + lines(EVENTS[2]).encode()


# One writer


def test_a_session_that_is_being_written_cannot_be_opened_for_writing(tmp_path: Path) -> None:
    writer = SessionFile.create(tmp_path / "s-1", HEADER)
    writer.write(EVENTS[0])

    with pytest.raises(SessionLockedError, match="the session 's-1' is open for writing"):
        SessionFile.open(tmp_path / "s-1", KINDS)

    writer.close()
    file, events = SessionFile.open(tmp_path / "s-1", KINDS)
    file.close()
    assert events == EVENTS[:1]


def test_a_session_that_is_being_written_can_still_be_read(tmp_path: Path) -> None:
    writer = SessionFile.create(tmp_path / "s-1", HEADER)
    writer.write(EVENTS[0])

    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS[:1]

    writer.close()


def test_a_new_session_cannot_take_the_place_of_a_stored_one(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1")
    before = path.read_bytes()
    newcomer = SessionFile.create(tmp_path / "s-1", HEADER)

    with pytest.raises(SessionStoreError, match="a session 's-1' is stored already"):
        newcomer.write(EVENTS[0])

    newcomer.close()
    assert path.read_bytes() == before


def test_of_two_new_sessions_with_one_id_only_the_first_is_written(tmp_path: Path) -> None:
    first = SessionFile.create(tmp_path / "s-1", HEADER)
    second = SessionFile.create(tmp_path / "s-1", HEADER)
    first.write(EVENTS[0])

    with pytest.raises(SessionLockedError, match="the session 's-1' is open for writing"):
        second.write(EVENTS[0])

    first.write(EVENTS[1])
    first.close()
    second.close()
    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS[:2]


def test_the_lock_is_given_up_when_a_session_cannot_be_read(tmp_path: Path) -> None:
    path = stored(tmp_path / "s-1", EVENTS[:1])
    whole = path.read_bytes()
    path.write_bytes(whole + b"garbage\n")

    with pytest.raises(UnreadableSessionError, match="line 3"):
        SessionFile.open(tmp_path / "s-1", KINDS)

    path.write_bytes(whole)
    file, events = SessionFile.open(tmp_path / "s-1", KINDS)
    file.close()
    assert events == EVENTS[:1]


# Failures


async def test_a_write_that_fails_spoils_the_file_for_good(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)
    file.write(EVENTS[0])

    def full(event: SessionEvent[EventData]) -> str:
        raise OSError(28, "No space left on device")

    monkeypatch.setattr("columbina.session.store.encode_event", full)
    with pytest.raises(SessionStoreError, match=r"cannot be written: .*No space left on device"):
        file.write(EVENTS[1])
    monkeypatch.undo()

    with pytest.raises(SessionStoreError, match=r"can no longer be written: .*No space left"):
        file.write(EVENTS[1])
    with pytest.raises(SessionStoreError, match="can no longer be written"):
        await file.flush()
    file.close()
    assert read_session(tmp_path / "s-1", KINDS).events == EVENTS[:1]


async def test_a_barrier_that_fails_spoils_the_file_for_good(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)
    file.write(EVENTS[0])

    def broken(descriptor: int) -> None:
        raise OSError(5, "Input/output error")

    monkeypatch.setattr(os, "fsync", broken)
    with pytest.raises(SessionStoreError, match=r"cannot be synced to the disk: .*Input/output"):
        await file.flush()
    monkeypatch.undo()

    with pytest.raises(SessionStoreError, match="can no longer be written"):
        file.write(EVENTS[1])
    file.close()


def test_a_closed_file_takes_no_more_events(tmp_path: Path) -> None:
    file = SessionFile.create(tmp_path / "s-1", HEADER)
    file.write(EVENTS[0])
    file.close()

    with pytest.raises(SessionStoreError, match="the file of the session 's-1' is closed"):
        file.write(EVENTS[1])


# Names


def test_a_workspace_has_a_readable_and_stable_key(tmp_path: Path) -> None:
    project = tmp_path / "My Project"

    key = workspace_key(project)

    assert key == workspace_key(str(project))
    assert key == workspace_key(project / "sub" / "..")
    assert key[:-9].endswith("My-Project") or key[:-9].endswith("my-project")
    assert key[-9] == "-"
    assert all(character in "0123456789abcdef" for character in key[-8:])


def test_two_workspaces_have_two_keys(tmp_path: Path) -> None:
    assert workspace_key(tmp_path / "a b") != workspace_key(tmp_path / "a-b")


@pytest.mark.skipif(sys.platform != "win32", reason="paths ignore case on Windows only")
def test_on_windows_the_case_of_a_path_does_not_change_its_key(tmp_path: Path) -> None:
    assert workspace_key(str(tmp_path).upper()) == workspace_key(str(tmp_path).lower())


def test_the_key_of_a_long_path_keeps_its_end(tmp_path: Path) -> None:
    deep = tmp_path.joinpath(*["a-rather-long-directory-name"] * 6, "the-project")

    key = workspace_key(deep)

    assert len(key) <= 80
    assert key[:-9].endswith("the-project")


@pytest.mark.parametrize("session_id", ["s-1", "20261009T075512-9f2c1a7b", "A", "a.b_c-d"])
def test_an_id_that_can_name_a_directory_is_accepted(session_id: str) -> None:
    assert check_session_id(session_id) == session_id


@pytest.mark.parametrize(
    "session_id", ["", ".", "..", "../s-1", "a/b", "a\\b", "s 1", "-s", "s.", "x" * 101, "会话"]
)
def test_an_id_that_cannot_name_a_directory_is_refused(session_id: str) -> None:
    with pytest.raises(ValueError, match="is not a usable session id"):
        check_session_id(session_id)


def test_a_new_id_starts_with_its_time_and_is_usable() -> None:
    first = new_session_id(1_791_234_567_890)
    second = new_session_id(1_791_234_567_890)

    assert first.startswith("20261005T")
    assert len(first) == len("20261005T000000-00000000")
    assert first != second
    assert check_session_id(first) == first
    assert new_session_id(1_791_234_567_890 + 86_400_000) > first
