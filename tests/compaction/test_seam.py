"""Making a conversation shorter: the events of it, and what a session says about it."""

from pathlib import Path
from typing import get_args

import pytest

import columbina.session
from columbina.agent import Agent
from columbina.compaction import (
    COMPACTION,
    COMPACTIONS,
    EVENTS,
    SUMMARY_SOURCE,
    Compaction,
    CompactionEndEvent,
    Compactions,
    CompactionStartEvent,
    CompactionSummaryEvent,
    Outcome,
    Why,
)
from columbina.kernel import Context
from columbina.llm import TextBlock, Usage, UserMessage
from columbina.session import (
    SESSIONS,
    EndSeedEvent,
    Session,
    Sessions,
    TurnStartEvent,
    UnknownEvent,
    UserMessageEvent,
)

TRIMMED = Outcome(trimmed=3, before=900, after=400)
SUMMARISED = Outcome(trimmed=1, summarised=12, before=900, after=120)
FAILED = Outcome(before=900, after=900, error="the model did not answer")


async def service(ctx: Context, root: Path | None = None) -> Sessions:
    await ctx.plugin(columbina.session, None if root is None else {"root": root})
    sessions = ctx.use(SESSIONS)
    sessions.declare(ctx, *EVENTS)
    return sessions


@pytest.fixture
async def session(ctx: Context, tmp_path: Path) -> Session:
    return (await service(ctx)).create(ctx, tmp_path, session_id="s-1")


def begin(session: Session, why: Why = "pressure") -> None:
    session.append(CompactionStartEvent(why=why, tokens=900))


def end(session: Session, outcome: Outcome, why: Why = "pressure") -> None:
    session.append(CompactionEndEvent(why=why, outcome=outcome))


# The events


def test_a_conversation_is_made_shorter_for_one_of_three_reasons() -> None:
    assert get_args(Why) == ("pressure", "overflow", "manual")


def test_the_events_have_their_names_and_can_be_passed_over_by_a_reader() -> None:
    assert [kind.event_type for kind in EVENTS] == [
        "compaction/start",
        "compaction/summary",
        "compaction/end",
    ]
    # The message that takes the place of others is an event that every reader knows. These
    # three only tell how it came about.
    assert all(kind.ignorable for kind in EVENTS)
    assert CompactionStartEvent(why="manual", tokens=1).for_model() is None
    assert CompactionEndEvent(why="manual", outcome=TRIMMED).for_model() is None


def test_a_summary_is_told_from_what_the_user_said_by_its_source() -> None:
    assert SUMMARY_SOURCE == "compaction"


async def test_the_events_are_written_down_and_read_back(ctx: Context, tmp_path: Path) -> None:
    sessions = await service(ctx, tmp_path / "sessions")
    written = sessions.create(ctx, tmp_path, session_id="s-1")
    begin(written, "overflow")
    summary = CompactionSummaryEvent(
        summary="They read a.txt.",
        replaces=(3, 4, 6),
        region_tokens=800,
        summary_tokens=20,
        usage=Usage(input_tokens=810, output_tokens=25, cache_read_tokens=700),
    )
    written.append(summary)
    end(written, SUMMARISED, "overflow")
    await written.flush()
    written.close()

    read = sessions.read(tmp_path, "s-1")

    assert [event.data for event in read.events[-3:]] == [
        CompactionStartEvent(why="overflow", tokens=900),
        summary,
        CompactionEndEvent(why="overflow", outcome=SUMMARISED),
    ]


async def test_a_program_that_does_not_know_the_events_reads_the_session_all_the_same(
    ctx: Context, tmp_path: Path
) -> None:
    sessions = await service(ctx, tmp_path / "sessions")
    written = sessions.create(ctx, tmp_path, session_id="s-1")
    begin(written)
    end(written, TRIMMED)
    await written.flush()
    written.close()
    other = Context()
    await other.plugin(columbina.session, {"root": tmp_path / "sessions"})

    read = other.use(SESSIONS).read(tmp_path, "s-1")

    await other.fiber.dispose()
    assert [isinstance(event.data, UnknownEvent) for event in read.events[-2:]] == [True, True]
    assert [event.data.event_type for event in read.events[-2:]] == [
        "compaction/start",
        "compaction/end",
    ]


# What a session says about it


def test_a_session_that_was_never_made_shorter_says_so(session: Session) -> None:
    session.append(TurnStartEvent(turn=1))

    assert COMPACTIONS.of(session) == Compactions(count=0, last=None, open=False, orphans=0)


def test_how_the_last_one_ended_is_what_a_session_says(session: Session) -> None:
    begin(session)
    during = COMPACTIONS.of(session)
    end(session, TRIMMED)
    after_one = COMPACTIONS.of(session)
    begin(session, "manual")
    end(session, SUMMARISED, "manual")

    assert during == Compactions(open=True)
    assert after_one == Compactions(count=1, last=TRIMMED)
    assert COMPACTIONS.of(session) == Compactions(count=2, last=SUMMARISED)


def test_one_that_changed_nothing_is_not_counted_but_it_is_the_last(session: Session) -> None:
    begin(session)
    end(session, TRIMMED)
    begin(session)
    end(session, FAILED)
    begin(session)
    end(session, Outcome(before=10, after=10))

    said = COMPACTIONS.of(session)
    assert said.count == 1
    assert said.last == Outcome(before=10, after=10)
    assert not FAILED.changed
    assert TRIMMED.changed
    assert Outcome(summarised=12, before=900, after=120).changed
    # What was trimmed stays trimmed, whatever became of the summary after it.
    assert Outcome(trimmed=2, before=900, after=700, error="the summary was no shorter").changed


def test_one_that_the_program_did_not_live_to_end_is_an_orphan(session: Session) -> None:
    begin(session)
    # The session is taken up again. What was under way then is not under way now.
    session.append(EndSeedEvent(inherited=False))
    taken_up = COMPACTIONS.of(session)
    begin(session)
    end(session, TRIMMED)

    assert taken_up == Compactions(orphans=1)
    assert COMPACTIONS.of(session) == Compactions(count=1, last=TRIMMED, orphans=1)


def test_one_that_begins_while_another_is_open_makes_the_other_an_orphan(session: Session) -> None:
    begin(session)
    begin(session)

    assert COMPACTIONS.of(session) == Compactions(open=True, orphans=1)


def test_taking_a_session_up_again_makes_no_orphan_of_what_had_ended(session: Session) -> None:
    begin(session)
    end(session, TRIMMED)
    session.append(EndSeedEvent(inherited=False))

    assert COMPACTIONS.of(session) == Compactions(count=1, last=TRIMMED)


def test_other_events_change_nothing(session: Session) -> None:
    begin(session)
    end(session, TRIMMED)
    before = COMPACTIONS.of(session)
    summary = UserMessage(content=(TextBlock(text="(a summary)"),))
    session.append(UserMessageEvent(message=summary, source=SUMMARY_SOURCE))

    assert COMPACTIONS.of(session) is before


# The service


async def test_whatever_makes_conversations_shorter_is_found_by_one_key(ctx: Context) -> None:
    class Never:
        async def compact(self, agent: Agent, why: Why, focus: str | None = None) -> Outcome:
            return Outcome(before=0, after=0)

    provider: Compaction = Never()
    ctx.provide(COMPACTION, provider)

    assert COMPACTION.name == "compaction"
    assert ctx.use(COMPACTION) is provider
