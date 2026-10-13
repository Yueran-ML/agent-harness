"""The replay adapter: a recorded session answers again."""

from pathlib import Path
from typing import ClassVar

import pytest

import columbina.session
from columbina.kernel import Context
from columbina.llm import (
    NOTHING_ARRIVED,
    Assembler,
    AssistantMessage,
    CallConfig,
    Finish,
    Llm,
    LlmFailure,
    LlmRequest,
    ReasoningBlock,
    TextBlock,
    TextDelta,
    ToolCallBlock,
    Usage,
    UserMessage,
)
from columbina.llm.adapters import replay
from columbina.llm.adapters.replay import replies_of
from columbina.session import (
    SESSIONS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    SessionEvent,
    TurnStartEvent,
)

CALLING = AssistantMessage(
    content=(
        ReasoningBlock(text="I need the file.", replay_state={"adapter": "deepseek"}),
        TextBlock(text="I will look."),
        ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
        ToolCallBlock(call_id="call-2", name="read", arguments='{"path": "b.txt"}'),
    )
)
ANSWER = AssistantMessage(content=(TextBlock(text="Both say hello."),))
BUSY = LlmFailure(message="overloaded", code="SERVER", status=529)

RECORDED: list[EventData] = [
    TurnStartEvent(turn=1),
    AssistantMessageEvent(
        message=CALLING, usage=Usage(input_tokens=40, output_tokens=12), finish_reason="tool-calls"
    ),
    AssistantAttemptEvent(
        message=AssistantMessage(content=(TextBlock(text="Bo"),)),
        usage=Usage(input_tokens=60),
        failure=BUSY,
    ),
    AssistantMessageEvent(
        message=ANSWER,
        usage=Usage(input_tokens=60, output_tokens=4, cache_read_tokens=30),
        finish_reason="stop",
    ),
]
REQUEST = LlmRequest(
    config=CallConfig(provider="recorded", model="any"),
    messages=(UserMessage(content=(TextBlock(text="go"),)),),
)


class NoteEvent(EventData):
    event_type: ClassVar[str] = "test/note"

    text: str


async def answer_of(llm: Llm) -> tuple[AssistantMessage, Finish | None]:
    """Make one call, read it to its end, and return what it gave."""
    call = llm.call(REQUEST)
    async for _chunk in call:
        pass
    return call.message, call.finish


def events(*data: EventData) -> list[SessionEvent[EventData]]:
    return [SessionEvent(seq=seq, time=0, data=item) for seq, item in enumerate(data)]


def test_each_reply_of_a_session_becomes_chunks_that_assemble_into_it_again() -> None:
    replies = replies_of(events(*RECORDED))

    assembled: list[tuple[AssistantMessage, Usage, Finish | None]] = []
    for chunks in replies:
        assembler = Assembler()
        for chunk in chunks:
            assembler.push(chunk)
        assembled.append((assembler.message(), assembler.usage, assembler.finish))

    assert assembled == [
        (CALLING, Usage(input_tokens=40, output_tokens=12), Finish(reason="tool-calls")),
        (
            AssistantMessage(content=(TextBlock(text="Bo"),)),
            Usage(input_tokens=60),
            Finish(reason="error", failure=BUSY),
        ),
        (
            ANSWER,
            Usage(input_tokens=60, output_tokens=4, cache_read_tokens=30),
            Finish(reason="stop"),
        ),
    ]


def test_a_reply_that_was_cut_off_before_it_held_anything_is_replayed_as_cut_off() -> None:
    # A log keeps it like a call that failed, since it left no message. Played as a failure,
    # it would end a turn that had gone on.
    nothing = AssistantAttemptEvent(
        message=AssistantMessage(content=()),
        usage=Usage(input_tokens=40, output_tokens=8),
        failure=NOTHING_ARRIVED,
    )

    (chunks,) = replies_of(events(TurnStartEvent(turn=1), nothing))

    assembler = Assembler()
    for chunk in chunks:
        assembler.push(chunk)
    assert assembler.finish == Finish(reason="max-tokens")
    assert assembler.message() == AssistantMessage(content=())
    assert assembler.usage == Usage(input_tokens=40, output_tokens=8)


def test_a_reply_that_was_interrupted_cannot_be_replayed() -> None:
    interrupted = AssistantMessageEvent(message=ANSWER, usage=Usage(), finish_reason=None)

    with pytest.raises(ValueError, match="event 1 is a reply that was interrupted"):
        replies_of(events(TurnStartEvent(turn=1), interrupted))


def test_a_message_that_takes_the_place_of_a_reply_is_no_reply_of_the_model() -> None:
    def said(text: str, *, cut_short: bool = False) -> AssistantMessageEvent:
        message = AssistantMessage(content=(TextBlock(text=text),))
        reason = None if cut_short else "stop"
        return AssistantMessageEvent(message=message, usage=Usage(), finish_reason=reason)

    log = [
        SessionEvent(seq=0, time=0, data=said("the first answer"), on_surface=True),
        # History was rewritten here, for example to shorten it. The model was not asked.
        SessionEvent(
            seq=1, time=0, data=said("a shorter first answer"), on_surface=True, replaces=(0,)
        ),
        SessionEvent(seq=2, time=0, data=said("the second answer"), on_surface=True),
        SessionEvent(
            seq=3, time=0, data=said("cut short", cut_short=True), on_surface=True, replaces=(2,)
        ),
    ]

    replies = replies_of(log)

    texts = ["".join(c.text for c in chunks if isinstance(c, TextDelta)) for chunks in replies]
    assert texts == ["the first answer", "the second answer"]


async def test_the_plugin_answers_with_the_replies_of_a_stored_session(
    ctx: Context, llm: Llm, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    sessions = ctx.use(SESSIONS)
    recorded = sessions.create(ctx, tmp_path, session_id="recorded")
    for data in RECORDED:
        recorded.append(data)
    recorded.close()
    fiber = await ctx.plugin(
        replay, {"provider": "recorded", "directory": sessions.directory(tmp_path, "recorded")}
    )
    adapter = llm.adapters["recorded"]
    assert isinstance(adapter, replay.Replay)
    assert adapter.requests == []

    outcomes = [await answer_of(llm) for _ in range(4)]

    assert outcomes[0] == (CALLING, Finish(reason="tool-calls"))
    assert outcomes[1][1] == Finish(reason="error", failure=BUSY)
    assert outcomes[2] == (ANSWER, Finish(reason="stop"))
    assert outcomes[3][1] is not None
    assert outcomes[3][1].failure is not None
    assert outcomes[3][1].failure.code == "SCRIPT_EXHAUSTED"
    assert adapter.requests == [REQUEST] * 4

    await fiber.dispose()

    assert list(llm.adapters) == []


async def test_the_session_is_read_when_the_first_call_is_made(
    ctx: Context, llm: Llm, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    sessions = ctx.use(SESSIONS)
    directory = sessions.directory(tmp_path, "recorded")
    await ctx.plugin(replay, {"provider": "recorded", "directory": directory})
    # Neither the session nor the type of its first event exists when the adapter loads.
    sessions.declare(ctx, NoteEvent)
    recorded = sessions.create(ctx, tmp_path, session_id="recorded")
    recorded.append(NoteEvent(text="from a plugin that loaded later"))
    recorded.append(AssistantMessageEvent(message=ANSWER, usage=Usage(), finish_reason="stop"))
    recorded.close()

    assert await answer_of(llm) == (ANSWER, Finish(reason="stop"))


async def test_a_session_that_cannot_be_read_fails_every_call_and_says_why(
    ctx: Context, llm: Llm, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(replay, {"provider": "recorded", "directory": tmp_path / "nowhere"})

    for _ in range(2):
        _, finish = await answer_of(llm)

        assert finish is not None
        assert finish.failure is not None
        assert finish.failure.code == "INTERNAL"
        assert "there is no session file" in finish.failure.message


async def test_a_session_with_an_interrupted_reply_fails_the_call_and_says_why(
    ctx: Context, llm: Llm, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    sessions = ctx.use(SESSIONS)
    recorded = sessions.create(ctx, tmp_path, session_id="recorded")
    recorded.append(AssistantMessageEvent(message=ANSWER, usage=Usage(), finish_reason=None))
    recorded.close()
    directory = sessions.directory(tmp_path, "recorded")
    await ctx.plugin(replay, {"provider": "recorded", "directory": directory})

    _, finish = await answer_of(llm)

    assert finish is not None
    assert finish.failure is not None
    assert finish.failure.code == "INTERNAL"
    assert "event 0 is a reply that was interrupted" in finish.failure.message
