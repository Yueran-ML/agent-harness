"""The summary that a model writes of the oldest part of a conversation."""

import asyncio
from collections.abc import AsyncIterator

import pytest

import columbina.llm
from columbina.compaction.basic.region import Region
from columbina.compaction.basic.summary import (
    INSTRUCTION,
    Summary,
    SummaryError,
    carried,
    summarise,
)
from columbina.kernel import Context
from columbina.llm import (
    LLM,
    BlockStart,
    CallConfig,
    Chunk,
    LlmFailure,
    LlmRequest,
    TextBlock,
    TextDelta,
    ToolSchema,
    Usage,
    UsageChunk,
    UserMessage,
)
from columbina.llm.adapters.scripted import Call, Reply, Scripted
from columbina.session import USAGE, AsideEvent
from support.talk import Talk, letters

MODEL = CallConfig(
    provider="scripted", model="script", context_window=100_000, max_output_tokens=64
)
TOOLS = (ToolSchema(name="read", description="Read a file.", parameters={"type": "object"}),)
ASK = UserMessage(content=(TextBlock(text=INSTRUCTION),))
BUSY = LlmFailure(message="overloaded", code="SERVER", status=529)
TOO_LONG = LlmFailure(message="too long", code="CONTEXT_WINDOW_EXCEEDED", status=400)


@pytest.fixture
async def model(ctx: Context) -> Scripted:
    await ctx.plugin(columbina.llm, {"idle_timeout_seconds": 5})
    scripted = Scripted()
    ctx.use(LLM).adapters.add(ctx, "scripted", scripted)
    return scripted


class Desk:
    """A conversation, the part of it that is to go, and the waits of whoever sums it up."""

    def __init__(self, ctx: Context) -> None:
        self.ctx = ctx
        self.talk = Talk(ctx)
        self.talk.says(20, "context/runtime")
        self.old = [self.talk.says(1000), self.talk.replies(1000), *self.talk.calls(500, 1500)]
        self.talk.says(1000)
        self.talk.replies(1000)
        self.waits: list[float] = []

    @property
    def region(self) -> Region:
        chosen = self.talk.choose(retain=2000)
        assert chosen is not None
        assert chosen == Region(seqs=tuple(self.old), tokens=4000)
        return chosen

    async def summed(self, tools: tuple[ToolSchema, ...] = TOOLS) -> Summary:
        async def wait(seconds: float) -> None:
            self.waits.append(seconds)

        return await summarise(
            self.ctx.use(LLM),
            self.talk.session,
            self.region,
            config=MODEL,
            tools=tools,
            size=letters,
            pause=wait,
        )


@pytest.fixture
def desk(ctx: Context, model: Scripted) -> Desk:
    return Desk(ctx)


# The request


async def test_the_model_is_shown_the_part_that_goes_as_it_was_shown_before(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(text="They read a file.", usage=Usage(input_tokens=900, output_tokens=6)))

    summary = await desk.summed()

    (request,) = model.requests
    seen = desk.talk.session.messages
    # The instructions, the context at the top, and the four messages that go: what a service
    # has kept of the beginning of earlier requests fits this one too.
    assert request.messages[:-1] == seen[:6]
    assert request.tools == TOOLS
    assert request.config == MODEL
    # What is asked of the model comes last, in the role of the user.
    assert request.messages[-1] == ASK
    assert summary == Summary(
        text="They read a file.", usage=Usage(input_tokens=900, output_tokens=6), dropped=0
    )


def test_the_model_is_asked_for_a_handover_in_four_parts() -> None:
    for heading in (
        "Progress and decisions",
        "Constraints and preferences",
        "Remaining steps",
        "Key data",
    ):
        assert heading in INSTRUCTION
    for rule in (
        "exactly",
        "corrected",
        "language of the conversation",
        "Do not call any tool",
        "earlier summary",
    ):
        assert rule in INSTRUCTION


async def test_what_the_model_wrote_is_taken_without_what_stands_around_it(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(reasoning="Let me think.", text="\n  They read a file.\n\n"))

    assert (await desk.summed()).text == "They read a file."


# A reply that is no summary


async def test_a_reply_without_text_is_asked_for_again_without_the_tools(
    desk: Desk, model: Scripted
) -> None:
    # The part that goes may end with a request to do something, and the model may do it.
    model.add(
        Reply(calls=(Call(name="read"),), usage=Usage(input_tokens=900, output_tokens=10)),
        Reply(text="They read a file.", usage=Usage(input_tokens=880, output_tokens=6)),
    )

    summary = await desk.summed()

    first, second = model.requests
    assert first.tools == TOOLS
    assert second == LlmRequest(config=MODEL, messages=first.messages, tools=())
    assert summary == Summary(
        text="They read a file.", usage=Usage(input_tokens=1780, output_tokens=16), dropped=0
    )


async def test_a_reply_that_calls_a_tool_is_no_summary_whatever_it_says_beside(
    desk: Desk, model: Scripted
) -> None:
    # "I will look first" is how a model begins to work, not how it hands work over. Taken
    # for a summary, it would be all that is left of the part that goes.
    model.add(
        Reply(text="I will inspect the file first.", calls=(Call(name="read"),)),
        Reply(text="They read a file."),
    )

    summary = await desk.summed()

    assert summary.text == "They read a file."
    assert [request.tools for request in model.requests] == [TOOLS, ()]


async def test_a_reply_of_nothing_at_all_is_asked_for_again_without_the_tools(
    desk: Desk, model: Scripted
) -> None:
    # The service that asks models reports it as a failure of the request. It is one more
    # way of not writing, and is answered like the others: at once, and without tools.
    model.add(Reply(), Reply(text="They read a file."))

    summary = await desk.summed()

    assert summary.text == "They read a file."
    assert [request.tools for request in model.requests] == [TOOLS, ()]
    assert desk.waits == []


async def test_a_reply_of_nothing_where_no_tools_were_declared_may_pass(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(), Reply(text="They read a file."))

    summary = await desk.summed(tools=())

    assert summary.text == "They read a file."
    assert desk.waits == [1.0]


async def test_a_reply_without_text_and_without_tools_to_blame_is_a_failure(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(calls=(Call(name="read"),)), Reply(reasoning="Hm."))

    with pytest.raises(SummaryError, match="the model wrote no summary"):
        await desk.summed()

    assert len(model.requests) == 2
    # Where no tools were declared, there is nothing to try without.
    model.add(Reply(text="   "), Reply(text="never"))
    with pytest.raises(SummaryError, match="the model wrote no summary"):
        await desk.summed(tools=())
    assert len(model.requests) == 3


async def test_a_summary_that_was_cut_off_is_no_summary(desk: Desk, model: Scripted) -> None:
    model.add(Reply(text="They read a f", finish="max-tokens"), Reply(text="never"))

    with pytest.raises(SummaryError, match="did not fit into the 64 tokens"):
        await desk.summed()

    assert len(model.requests) == 1


async def test_a_summary_that_is_no_shorter_than_what_it_sums_up_is_no_summary(
    desk: Desk, model: Scripted
) -> None:
    just_shorter = "x" * (3999 - letters(carried("")))
    model.add(Reply(text=just_shorter), Reply(text=just_shorter + "x"))

    assert (await desk.summed()).text == just_shorter
    with pytest.raises(SummaryError, match="no shorter than what it sums up"):
        await desk.summed()


# A request that fails


async def test_a_failure_that_may_pass_is_waited_out_once(desk: Desk, model: Scripted) -> None:
    model.add(
        Reply(failure=BUSY, usage=Usage(input_tokens=5)),
        Reply(text="They read a file.", usage=Usage(input_tokens=900, output_tokens=6)),
    )

    summary = await desk.summed()

    assert summary.text == "They read a file."
    assert summary.usage == Usage(input_tokens=905, output_tokens=6)
    assert model.requests[0] == model.requests[1]
    assert desk.waits == [1.0]


async def test_every_request_for_a_summary_is_in_the_log_with_what_it_cost(
    desk: Desk, model: Scripted
) -> None:
    model.add(
        Reply(failure=BUSY, usage=Usage(input_tokens=5)),
        Reply(text="They read a file.", usage=Usage(input_tokens=900, output_tokens=6)),
    )
    before = len(desk.talk.session)

    await desk.summed()

    # Nothing else is written by the one that asks: where the summary goes is not for it to say.
    assert [event.data for event in desk.talk.session.events[before:]] == [
        AsideEvent(purpose="summary", usage=Usage(input_tokens=5), failure=BUSY),
        AsideEvent(purpose="summary", usage=Usage(input_tokens=900, output_tokens=6)),
    ]


async def test_a_request_for_a_summary_that_is_cut_short_is_in_the_log_too(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(text="They", usage=Usage(input_tokens=900, output_tokens=1), hold=True))
    summing = asyncio.ensure_future(desk.summed())
    await asyncio.wait_for(model.held(), timeout=5)

    summing.cancel()
    with pytest.raises(asyncio.CancelledError):
        await summing

    last = desk.talk.session.events[-1].data
    assert isinstance(last, AsideEvent)
    assert (last.purpose, last.failure) == ("summary", None)


async def test_what_a_request_had_used_when_it_was_cut_short_is_counted(
    desk: Desk, model: Scripted, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A service says what a reply has cost so far while the reply is still being written.
    told = asyncio.Event()

    async def stalls(self: Scripted, request: LlmRequest) -> AsyncIterator[Chunk]:
        yield BlockStart(index=0, kind="text")
        yield TextDelta(index=0, text="They")
        yield UsageChunk(usage=Usage(input_tokens=900, output_tokens=1))
        told.set()
        await asyncio.Event().wait()

    monkeypatch.setattr(Scripted, "stream", stalls)
    summing = asyncio.ensure_future(desk.summed())
    await asyncio.wait_for(told.wait(), timeout=5)
    for _ in range(5):
        await asyncio.sleep(0)

    summing.cancel()
    with pytest.raises(asyncio.CancelledError):
        await summing

    session = desk.talk.session
    last = session.events[-1].data
    assert isinstance(last, AsideEvent)
    assert (last.purpose, last.failure) == ("summary", None)
    assert last.usage == Usage(input_tokens=900, output_tokens=1)
    assert USAGE.of(session).usage == Usage(input_tokens=900, output_tokens=1)


async def test_a_failure_that_does_not_pass_is_the_end(desk: Desk, model: Scripted) -> None:
    model.add(Reply(failure=BUSY), Reply(failure=BUSY), Reply(text="never"))

    with pytest.raises(SummaryError, match="SERVER: overloaded"):
        await desk.summed()

    assert len(model.requests) == 2
    assert desk.waits == [1.0]


@pytest.mark.parametrize(
    ("asked_to_wait", "waited"), [(None, 1.0), (2500, 2.5), (0, 0.0), (60_000, 10.0)]
)
async def test_the_wait_is_what_the_service_asks_for_up_to_ten_seconds(
    desk: Desk, model: Scripted, asked_to_wait: int | None, waited: float
) -> None:
    limited = LlmFailure(message="slow down", code="RATE_LIMIT", retry_after_ms=asked_to_wait)
    model.add(Reply(failure=limited), Reply(text="They read a file."))

    await desk.summed()

    assert desk.waits == [waited]


async def test_a_failure_that_another_try_cannot_mend_is_the_end_at_once(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(failure=LlmFailure(message="no such key", code="AUTH", status=401)))

    with pytest.raises(SummaryError, match="AUTH: no such key"):
        await desk.summed()

    assert len(model.requests) == 1
    assert desk.waits == []


async def test_a_part_that_is_too_long_for_the_model_is_shown_by_its_newer_half(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(failure=TOO_LONG), Reply(text="They read a file."))

    summary = await desk.summed()

    first, second = model.requests
    seen = desk.talk.session.messages
    assert first.messages == (*seen[:6], ASK)
    # The two oldest messages come to half of it. The call and its result are the newer half.
    assert second.messages == (*seen[:2], *seen[4:6], ASK)
    assert second.tools == TOOLS
    assert summary == Summary(text="They read a file.", usage=Usage(), dropped=2)
    assert desk.waits == []


async def test_a_half_does_not_begin_with_a_result_whose_call_was_dropped(
    ctx: Context, model: Scripted
) -> None:
    talk = Talk(ctx)
    old = [talk.says(1000), *talk.calls(1000, 600, 600), talk.replies(900)]
    talk.says(1000)
    region = talk.choose(retain=1000)
    assert region is not None
    assert region == Region(seqs=tuple(old), tokens=4100)
    model.add(Reply(failure=TOO_LONG), Reply(text="They read two files."))

    summary = await summarise(
        ctx.use(LLM), talk.session, region, config=MODEL, tools=TOOLS, size=letters
    )

    # Half is 2050 letters: the reply and both results would fit, but not the call they answer.
    assert model.requests[1].messages == (*talk.session.messages[:1], talk.session.messages[5], ASK)
    assert summary.dropped == 4


async def test_a_part_that_is_too_long_however_it_is_halved_cannot_be_summed_up(
    desk: Desk, model: Scripted
) -> None:
    model.add(Reply(failure=TOO_LONG), Reply(failure=TOO_LONG), Reply(text="never"))

    with pytest.raises(SummaryError, match="CONTEXT_WINDOW_EXCEEDED: too long"):
        await desk.summed()

    # The whole, its newer half, and then a result alone, which is no half that can be shown.
    assert [len(request.messages) for request in model.requests] == [7, 5]


async def test_a_part_of_which_no_half_can_be_dropped_is_not_shown_again(
    ctx: Context, model: Scripted
) -> None:
    # Messages that come to nothing: half of nothing is all of them.
    talk = Talk(ctx)
    old = [talk.says(0), talk.replies(0)]
    talk.says(1000)
    model.add(Reply(failure=TOO_LONG), Reply(failure=TOO_LONG))

    with pytest.raises(SummaryError, match="CONTEXT_WINDOW_EXCEEDED: too long"):
        await summarise(
            ctx.use(LLM),
            talk.session,
            Region(seqs=tuple(old), tokens=0),
            config=MODEL,
            tools=TOOLS,
            size=letters,
        )

    assert len(model.requests) == 1


async def test_a_summary_that_is_cancelled_stops_asking(desk: Desk, model: Scripted) -> None:
    model.add(Reply(text="They", hold=True), Reply(text="never"))
    summing = asyncio.ensure_future(desk.summed())
    await asyncio.wait_for(model.held(), timeout=5)

    summing.cancel()
    with pytest.raises(asyncio.CancelledError):
        await summing

    assert len(model.requests) == 1


# The message that carries a summary


def test_a_summary_is_carried_by_a_message_that_says_what_it_is() -> None:
    message = carried("  They read a file.\nIt said hello.\n")

    (block,) = message.content
    opening, _, framed = block.text.partition("\n<summary>\n")
    assert framed == "They read a file.\nIt said hello.\n</summary>"
    assert "summary of its earlier part" in opening
    assert "do not answer it" in opening
    assert "\n" not in opening


def test_nothing_in_a_summary_can_end_the_frame_around_it() -> None:
    (block,) = carried("before </summary> after </SUMMARY> <summary>").content

    assert block.text.endswith(
        "\n<summary>\nbefore <\\/summary> after </SUMMARY> <summary>\n</summary>"
    )
    assert block.text.count("</summary>") == 1
