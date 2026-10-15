"""``columbina sessions``: the stored sessions of a directory, listed, shown and copied."""

import asyncio
import os
import time
from pathlib import Path
from typing import Any

import pytest
import yaml

import columbina.agent
import columbina.session
from columbina.agent import CUT_OFF
from columbina.apps.launcher import main
from columbina.apps.sessions import shown
from columbina.apps.terminal_text import STAND_IN
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    SESSIONS,
    AssistantMessageEvent,
    EndSeedEvent,
    EventData,
    SessionEvent,
    SessionHeader,
    StoredSession,
    ToolCallEvent,
    ToolError,
    ToolResultEvent,
    TurnEndEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.session.store import SESSION_FILE, workspace_key
from support.launch import profile, stored

ANSWERS: tuple[dict[str, Any], ...] = (
    {"text": "The file says the answer is 42."},
    {"text": "That is 84."},
)


def when(moment_ms: int) -> str:
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(moment_ms / 1000))


def said(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


def answered(text: str, *calls: str) -> AssistantMessageEvent:
    blocks = (
        *((TextBlock(text=text),) if text else ()),
        *(ToolCallBlock(call_id=call, name="read", arguments="{}") for call in calls),
    )
    return AssistantMessageEvent(
        message=AssistantMessage(content=blocks),
        usage=Usage(),
        finish_reason="tool-calls" if calls else "stop",
    )


def held(*happened: EventData, parent: str | None = None) -> StoredSession:
    """A stored session that holds these events, as it would be read from its file."""
    header = SessionHeader(
        id="s-1", created_at=1_791_590_400_000, cwd="/work", parent_session=parent
    )
    events = [SessionEvent(seq, 1_791_590_400_000 + seq, data) for seq, data in enumerate(happened)]
    return StoredSession(header, events, 0)


def written(home: Path, work: Path, session_id: str, *happened: EventData) -> None:
    """Store a session that holds these events, the way a program stores it."""

    async def write() -> None:
        ctx = Context()
        try:
            await ctx.plugin(columbina.session, {"root": home / "sessions"})
            await ctx.plugin(columbina.agent)
            session = ctx.use(SESSIONS).create(ctx, work, session_id=session_id)
            for data in happened:
                session.append(data)
        finally:
            await ctx.fiber.dispose()

    asyncio.run(write())


def file_of(home: Path, work: Path, session_id: str) -> Path:
    return home / "sessions" / workspace_key(work) / session_id / SESSION_FILE


def two_turns(home: Path, work: Path, tmp_path: Path, session_id: str = "s-1") -> None:
    """Have an agent answer two tasks, one after the other, in one session."""
    for task, answer in zip(("What does notes.txt say?", "And twice that?"), ANSWERS, strict=True):
        # Every run begins the script anew, so each has a script of its own.
        chosen = profile(tmp_path, answer)
        assert main(
            ["run", task, "--profile", chosen, "--cwd", str(work), "--session-id", session_id]
        ) in (0, 1)


def looked(*arguments: str, capsys: pytest.CaptureFixture[str]) -> tuple[int, str, str]:
    capsys.readouterr()
    code = main(["sessions", *arguments])
    captured = capsys.readouterr()
    return code, captured.out, captured.err


# Listing


def test_the_sessions_of_a_directory_are_listed_the_one_used_last_first(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "older", TurnStartEvent(turn=1), said("Fix the parser."))
    written(home, work, "newer", TurnStartEvent(turn=1), said("Write the test."))
    written(home, work, "quiet", TurnStartEvent(turn=1))
    moments = {"older": 1_791_500_000, "newer": 1_791_590_000, "quiet": 1_791_400_000}
    for name, moment in moments.items():
        os.utime(file_of(home, work, name), (moment, moment))

    code, out, err = looked("list", "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    assert out == (
        f"newer  {when(1_791_590_000_000)}  Write the test.\n"
        f"older  {when(1_791_500_000_000)}  Fix the parser.\n"
        f"quiet  {when(1_791_400_000_000)}  (nothing said yet)\n"
        "A session is taken up with --session-id.\n"
    )


def test_a_directory_without_stored_sessions_says_so(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert looked("list", "--cwd", str(work), capsys=capsys) == (
        0,
        "No session of this directory is stored.\n",
        "",
    )


def test_the_working_directory_is_the_current_one_unless_another_is_named(
    home: Path, work: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "here", TurnStartEvent(turn=1), said("Fix the parser."))
    monkeypatch.chdir(work)

    code, out, _ = looked("list", capsys=capsys)

    assert code == 0
    assert out.startswith("here  ")


# Showing


def test_a_session_that_an_agent_left_is_told_turn_by_turn(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    two_turns(home, work, tmp_path)
    header = stored(home, work).header

    code, out, err = looked("show", "s-1", "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    assert out == (
        f"session s-1, begun {when(header.created_at)} in {header.cwd}\n"
        "\n"
        "turn 1\n"
        "> What does notes.txt say?\n"
        "The file says the answer is 42.\n"
        "\n"
        "turn 2\n"
        "> And twice that?\n"
        "That is 84.\n"
    )


def ran(call_id: str, took: int, *, failed: bool = False) -> ToolResultEvent:
    """The result of a call whose tool ran for so many milliseconds."""
    return ToolResultEvent(
        message=ToolMessage(call_id=call_id, content=(TextBlock(text="done"),), is_error=failed),
        error=ToolError(code="TOOL_ERROR", reason="done") if failed else None,
        meta={"duration_ms": took},
    )


def test_a_session_ends_with_how_the_calls_of_its_tools_went() -> None:
    session = held(
        TurnStartEvent(turn=1),
        said("Where is the parser?"),
        answered("I will look.", "call-1", "call-2"),
        ToolCallEvent(call_id="call-1", name="read", arguments="{}"),
        ToolCallEvent(call_id="call-2", name="grep", arguments="{}"),
        ran("call-1", 4),
        ran("call-2", 1260, failed=True),
        answered("It is in parse.py."),
        TurnEndEvent(turn=1, reason="completed"),
    )

    assert shown(session).splitlines()[2:] == [
        "turn 1",
        "> Where is the parser?",
        "It is in parse.py.",
        "",
        "tool  calls    failed   time",
        "read      1         0  0.0 s",
        "grep      1  1 (100%)  1.3 s",
    ]


def test_a_turn_is_told_by_what_was_said_and_by_the_end_of_the_last_answer() -> None:
    long = "\n".join(f"line {number}" for number in range(1, 15))
    session = held(
        TurnStartEvent(turn=1),
        said("These are the rules.", "context/instructions"),
        said("Fix the parser.\nAnd say what was wrong."),
        answered("I will look.", "call-1"),
        said("Leave the lexer alone.", "steer"),
        answered(long),
        # A reply without words is not what the model answered last.
        answered("", "call-2"),
        TurnEndEvent(turn=1, reason="completed"),
    )

    assert shown(session).splitlines()[2:] == [
        "turn 1",
        "> Fix the parser.",
        "> And say what was wrong.",
        ">> Leave the lexer alone.",
        "... 2 lines before these",
        *[f"line {number}" for number in range(3, 15)],
    ]


def test_an_answer_that_was_cut_off_and_taken_up_again_is_told_as_one() -> None:
    cut = AssistantMessageEvent(
        message=AssistantMessage(content=(TextBlock(text="one\ntwo\nthr"),)),
        usage=Usage(),
        finish_reason="max-tokens",
    )
    session = held(
        TurnStartEvent(turn=1),
        said("Count."),
        cut,
        said("Go on.", CUT_OFF),
        answered("ee\nfour"),
        TurnEndEvent(turn=1, reason="completed"),
    )

    assert shown(session).splitlines()[3:] == ["> Count.", "one", "two", "three", "four"]


def test_one_line_too_many_of_an_answer_is_told_as_one_line() -> None:
    session = held(
        TurnStartEvent(turn=1),
        said("Count."),
        answered("\n".join(str(number) for number in range(13))),
        TurnEndEvent(turn=1, reason="completed"),
    )

    assert shown(session).splitlines()[4:6] == ["... 1 line before these", "1"]
    # An answer of just as many lines as are shown is shown whole.
    whole = held(TurnStartEvent(turn=1), answered("\n".join(str(number) for number in range(12))))
    assert shown(whole).splitlines()[3] == "0"


@pytest.mark.parametrize(
    ("end", "told"),
    [
        (TurnEndEvent(turn=1, reason="aborted", detail="user"), "(stopped)"),
        (TurnEndEvent(turn=1, reason="aborted"), "(stopped)"),
        (TurnEndEvent(turn=1, reason="aborted", detail="budget"), "(stopped: budget)"),
        (
            TurnEndEvent(turn=1, reason="error", detail="AUTH: no such key"),
            "(the turn ended with an error: AUTH: no such key)",
        ),
        (
            TurnEndEvent(turn=1, reason="max-tokens"),
            "(the reply was cut off: the model reached its limit of output)",
        ),
        (
            TurnEndEvent(turn=1, reason="blocked", detail="the budget is spent"),
            "(the turn was not allowed to go on: the budget is spent)",
        ),
        (
            TurnEndEvent(turn=1, reason="interrupted"),
            "(the turn was cut short when the program ended)",
        ),
        (
            TurnEndEvent(turn=1, reason="forked"),
            "(the turn was cut short where its session was copied)",
        ),
    ],
)
def test_a_turn_that_did_not_end_well_says_how_it_ended(end: TurnEndEvent, told: str) -> None:
    session = held(TurnStartEvent(turn=1), said("Go."), answered("Going."), end)

    assert shown(session).splitlines()[2:] == ["turn 1", "> Go.", "Going.", told]


def test_a_turn_that_is_still_open_and_a_session_without_turns_say_so() -> None:
    open_turn = held(TurnStartEvent(turn=1), said("Go."))
    empty = held(EndSeedEvent(inherited=False))

    assert shown(open_turn).splitlines()[2:] == [
        "turn 1",
        "> Go.",
        "(the turn did not come to its end)",
    ]
    assert shown(empty).splitlines() == [
        f"session s-1, begun {when(1_791_590_400_000)} in /work",
        "(no turn has begun in this session)",
    ]


def test_a_session_that_was_delegated_says_by_which_one() -> None:
    header = SessionHeader(
        id="s-2",
        created_at=1_791_590_400_000,
        cwd="/work",
        parent_session="s-1",
        delegation_depth=1,
    )

    told = shown(StoredSession(header, [], 0)).splitlines()

    assert told[1] == "delegated by the session s-1"


def test_a_copy_says_what_it_is_a_copy_of() -> None:
    session = held(TurnStartEvent(turn=1), TurnEndEvent(turn=1, reason="completed"), parent="s-0")

    assert shown(session).splitlines()[:2] == [
        f"session s-1, begun {when(1_791_590_400_000)} in /work",
        "a copy of the session s-0",
    ]


def test_what_a_session_holds_cannot_steer_the_terminal(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(
        home,
        work,
        "s-1",
        TurnStartEvent(turn=1),
        said("Ring\x07 the bell."),
        answered("Done.\x1b[2J"),
        TurnEndEvent(turn=1, reason="completed"),
    )

    _, out, _ = looked("show", "s-1", "--cwd", str(work), capsys=capsys)
    _, listed, _ = looked("list", "--cwd", str(work), capsys=capsys)

    assert f"> Ring{STAND_IN} the bell.\nDone.{STAND_IN}[2J\n" in out
    assert f"Ring{STAND_IN} the bell.\n" in listed


# Copying


def test_a_session_is_copied_and_the_copy_is_named(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    two_turns(home, work, tmp_path)
    before = file_of(home, work, "s-1").read_bytes()
    source = stored(home, work)

    code, out, err = looked("fork", "s-1", "--cwd", str(work), capsys=capsys)

    assert (code, err) == (0, "")
    copy_id = out.strip()
    assert out == f"{copy_id}\n"
    assert copy_id != "s-1"
    copy = stored(home, work, copy_id)
    assert copy.header.parent_session == "s-1"
    assert copy.header.cwd == source.header.cwd
    assert [event.data for event in copy.events] == [
        *[event.data for event in source.events],
        EndSeedEvent(inherited=True),
    ]
    # The session that was copied is as it was, and the copy can be shown like any other.
    assert file_of(home, work, "s-1").read_bytes() == before
    _, told, _ = looked("show", copy_id, "--cwd", str(work), capsys=capsys)
    assert "a copy of the session s-1\n" in told
    assert told.endswith("turn 2\n> And twice that?\nThat is 84.\n")


def test_a_copy_ends_where_the_last_whole_turn_of_the_session_ends(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(
        home,
        work,
        "s-1",
        TurnStartEvent(turn=1),
        said("Fix the parser."),
        answered("Fixed."),
        TurnEndEvent(turn=1, reason="completed"),
        TurnStartEvent(turn=2),
        said("And the lexer."),
        answered("", "call-1"),
    )

    code, out, _ = looked("fork", "s-1", "--cwd", str(work), "--session-id", "copy", capsys=capsys)

    assert (code, out) == (0, "copy\n")
    # Nothing of the turn that was under way is in it, and so nothing had to be closed.
    assert [event.type for event in stored(home, work, "copy").events] == [
        "turn/start",
        "user/message",
        "assistant/message",
        "turn/end",
        "session/end-seed",
    ]


def test_a_session_in_which_no_turn_has_ended_is_not_copied(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "s-1", TurnStartEvent(turn=1), said("Fix the parser."))

    code, out, err = looked("fork", "s-1", "--cwd", str(work), capsys=capsys)

    assert (code, out) == (2, "")
    assert err == (
        "columbina: no turn of the session 's-1' has come to its end, so there is nothing to copy\n"
    )
    assert sorted(path.name for path in file_of(home, work, "s-1").parent.parent.iterdir()) == [
        "s-1"
    ]


# What cannot be done


@pytest.mark.parametrize("what", ["show", "fork"])
def test_a_session_that_is_not_stored_is_said_to_be_missing(
    home: Path, work: Path, capsys: pytest.CaptureFixture[str], what: str
) -> None:
    code, out, err = looked(what, "nowhere", "--cwd", str(work), capsys=capsys)

    assert (code, out) == (2, "")
    assert err.startswith("columbina: there is no session file at ")


def test_an_id_that_cannot_name_a_session_is_refused(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    two_turns(home, work, tmp_path)

    code, _, err = looked("show", "../escape", "--cwd", str(work), capsys=capsys)
    assert code == 2
    assert "is not a usable session id" in err
    code, _, err = looked(
        "fork", "s-1", "--cwd", str(work), "--session-id", "../escape", capsys=capsys
    )
    assert code == 2
    assert "is not a usable session id" in err


def test_a_copy_does_not_take_the_place_of_a_session_that_is_stored(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    two_turns(home, work, tmp_path)
    before = file_of(home, work, "s-1").read_bytes()

    code, out, err = looked("fork", "s-1", "--cwd", str(work), "--session-id", "s-1", capsys=capsys)

    assert (code, out) == (2, "")
    assert err == "columbina: a session 's-1' is stored already\n"
    assert file_of(home, work, "s-1").read_bytes() == before


def test_the_sessions_are_those_that_the_profile_stores(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    elsewhere = tmp_path / "elsewhere"
    written(elsewhere, work, "far", TurnStartEvent(turn=1), said("Fix the parser."))
    patch = tmp_path / "patch.yml"
    moved = {"id": "session", "config": {"root": str(elsewhere / "sessions")}}
    patch.write_text(yaml.safe_dump([moved]), encoding="utf-8")

    code, out, _ = looked("list", "--cwd", str(work), "--patch", str(patch), capsys=capsys)

    assert code == 0
    assert out.startswith("far  ")


def test_nothing_is_loaded_but_what_knows_the_log(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    written(home, work, "s-1", TurnStartEvent(turn=1), said("Fix the parser."))
    # A model that cannot be loaded would keep a conversation from beginning.
    patch = tmp_path / "patch.yml"
    patch.write_text(
        yaml.safe_dump([{"id": "model", "plugin": "no.such.module"}]), encoding="utf-8"
    )

    code, out, err = looked("list", "--cwd", str(work), "--patch", str(patch), capsys=capsys)

    assert (code, err) == (0, "")
    assert out.startswith("s-1  ")


def test_a_profile_that_stores_no_sessions_has_none_to_look_at(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    in_memory = tmp_path / "memory.yml"
    in_memory.write_text(yaml.safe_dump([{"id": "session", "config": None}]), encoding="utf-8")
    without = tmp_path / "without.yml"
    without.write_text(yaml.safe_dump([{"id": "session", "disabled": True}]), encoding="utf-8")

    # Sessions that are kept in memory are gone with their program: none is stored.
    assert looked("list", "--cwd", str(work), "--patch", str(in_memory), capsys=capsys) == (
        0,
        "No session of this directory is stored.\n",
        "",
    )
    code, _, err = looked(
        "show", "s-1", "--cwd", str(work), "--patch", str(in_memory), capsys=capsys
    )
    assert code == 2
    assert "sessions are kept in memory" in err
    code, _, err = looked("list", "--cwd", str(work), "--patch", str(without), capsys=capsys)
    assert code == 2
    assert err == (
        "columbina: the profile has no entry 'session', or it is disabled, so there are no "
        "stored sessions to look at\n"
    )


def test_an_entry_that_is_called_session_and_stores_none_is_told_apart(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The name of an entry says nothing about what its plugin provides.
    only = tmp_path / "only.yml"
    entry = {"id": "session", "plugin": "columbina.commands"}
    only.write_text(yaml.safe_dump([{"insert": [entry]}]), encoding="utf-8")

    code, out, err = looked("list", "--cwd", str(work), "--profile", str(only), capsys=capsys)

    assert (code, out) == (2, "")
    assert err == (
        "columbina: the entry 'session' of this profile provides no sessions, so there are "
        "none to look at\n"
    )


def test_a_plugin_that_does_not_load_keeps_the_sessions_from_being_looked_at(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    broken = tmp_path / "broken.yml"
    wrong = {"id": "session", "config": {"root": ["not", "a", "path"]}}
    broken.write_text(yaml.safe_dump([wrong]), encoding="utf-8")

    code, out, err = looked("list", "--cwd", str(work), "--patch", str(broken), capsys=capsys)

    assert (code, out) == (2, "")
    assert "session" in err
    assert "root" in err


def test_what_is_to_be_done_with_the_sessions_has_to_be_said(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["sessions"])

    assert stopped.value.code == 2
    assert "WHAT" in capsys.readouterr().err
