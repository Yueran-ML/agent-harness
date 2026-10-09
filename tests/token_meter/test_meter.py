"""The size of a session, kept up with its log and put right by what the provider counted."""

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

import columbina.session
import columbina.token_meter
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    CallConfig,
    LlmFailure,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    Usage,
    UserMessage,
)
from columbina.session import (
    SESSIONS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    RequestHeaderEvent,
    Session,
    SystemMessageEvent,
    ToolCallEvent,
    ToolResultEvent,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.token_meter import (
    TOKEN_METER,
    Measure,
    TokenMeter,
    estimate_message,
    estimate_tools,
)

MODEL = CallConfig(provider="deepseek", model="deepseek-flash")
SYSTEM = SystemMessage(content=(TextBlock(text="You are helpful. " * 20),))
READ = ToolSchema(name="read", description="Read a text file.", parameters={"type": "object"})
WRITE = ToolSchema(name="write", description="Write a text file.", parameters={"type": "object"})


def said(text: str) -> UserMessage:
    return UserMessage(content=(TextBlock(text=text),))


def answered(text: str) -> AssistantMessage:
    return AssistantMessage(content=(TextBlock(text=text),))


def reply(
    text: str, sent: int = 0, cached: int = 0, *, cut_short: bool = False
) -> AssistantMessageEvent:
    usage = Usage(input_tokens=sent, cache_read_tokens=cached, output_tokens=5)
    return AssistantMessageEvent(
        message=answered(text), usage=usage, finish_reason=None if cut_short else "stop"
    )


@pytest.fixture
async def session(ctx: Context) -> Session:
    await ctx.plugin(columbina.session)
    return ctx.use(SESSIONS).create(ctx, ".", session_id="s-1")


def test_a_session_without_events_comes_to_nothing(session: Session) -> None:
    assert TokenMeter().measure(session) == Measure(tokens=0, estimated=0, rate=None)


def test_the_messages_that_the_model_sees_are_added_up(session: Session) -> None:
    meter = TokenMeter()
    question = said("What does notes.txt say? " * 10)
    session.append(SystemMessageEvent(message=SYSTEM))
    session.append(UserMessageEvent(message=question))

    measured = meter.measure(session)

    assert measured.estimated == estimate_message(SYSTEM) + estimate_message(question)
    assert measured.tokens == measured.estimated
    assert measured.rate is None


def test_what_the_model_does_not_see_adds_nothing(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("Hello.")))
    before = meter.measure(session)

    session.append(TurnStartEvent(turn=1))
    session.append(ToolCallEvent(call_id="c-1", name="read", arguments='{"path": "x" }'))
    failure = LlmFailure(message="The service is busy. " * 50, code="SERVER")
    session.append(
        AssistantAttemptEvent(
            message=answered("half a reply " * 50), usage=Usage(input_tokens=9000), failure=failure
        )
    )

    assert meter.measure(session) == before


def test_the_tools_count_as_the_last_request_declared_them(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("Hello.")))
    alone = meter.measure(session).estimated

    session.append(RequestHeaderEvent(config=MODEL, tools=(READ,)))
    with_one = meter.measure(session).estimated
    session.append(RequestHeaderEvent(config=MODEL, tools=(READ, WRITE)))
    with_two = meter.measure(session).estimated
    session.append(RequestHeaderEvent(config=MODEL, tools=()))

    assert with_one == alone + estimate_tools((READ,))
    assert with_two == alone + estimate_tools((READ, WRITE))
    assert meter.measure(session).estimated == alone


def test_a_message_that_takes_the_place_of_others_counts_in_their_place(session: Session) -> None:
    meter = TokenMeter()
    session.append(SystemMessageEvent(message=SYSTEM))
    first = session.append(UserMessageEvent(message=said("A long story. " * 200)))
    second = session.append(reply("Another long story. " * 200))
    last = said("And now?")
    session.append(UserMessageEvent(message=last))
    long = meter.measure(session).estimated

    summary = said("They told each other stories.")
    session.append(UserMessageEvent(message=summary), replaces=[first.seq, second.seq])

    short = meter.measure(session).estimated
    assert short == estimate_message(SYSTEM) + estimate_message(summary) + estimate_message(last)
    assert short < long / 10


def test_a_result_that_is_cut_down_counts_as_what_is_left_of_it(session: Session) -> None:
    meter = TokenMeter()
    call = ToolCallBlock(call_id="c-1", name="read", arguments="{}")
    session.append(
        AssistantMessageEvent(
            message=AssistantMessage(content=(call,)), usage=Usage(), finish_reason="tool-calls"
        )
    )
    whole = ToolMessage(call_id="c-1", content=(TextBlock(text="line\n" * 5000),))
    held = session.append(ToolResultEvent(message=whole))
    before = meter.measure(session).estimated

    cut = ToolMessage(call_id="c-1", content=(TextBlock(text="line\n" * 50),))
    session.append(ToolResultEvent(message=cut), replaces=[held.seq])

    assert meter.measure(session).estimated == before - estimate_message(whole) + estimate_message(
        cut
    )


# What the provider counted


def test_what_the_provider_counted_puts_the_estimate_right(session: Session) -> None:
    meter = TokenMeter()
    session.append(SystemMessageEvent(message=SYSTEM))
    session.append(UserMessageEvent(message=said("What does notes.txt say? " * 40)))
    session.append(RequestHeaderEvent(config=MODEL, tools=(READ,)))
    request = meter.measure(session).estimated

    # The provider counted a fifth more than the table made of the same request.
    session.append(reply("It says 42.", sent=round(request * 1.2)))
    session.append(UserMessageEvent(message=said("And what else? " * 40)))

    measured = meter.measure(session)
    assert measured.rate == pytest.approx(1.2, abs=0.001)
    assert measured.rate is not None
    assert measured.estimated > request
    assert measured.tokens == round(measured.estimated * measured.rate)


def test_what_came_from_the_cache_of_the_provider_was_sent_too(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("What does notes.txt say? " * 40)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated

    session.append(reply("It says 42.", sent=10, cached=request - 10))

    assert meter.measure(session).rate == pytest.approx(1.0)


@pytest.mark.parametrize("factor", [0.0, 0.1, 0.49, 2.01, 30.0])
def test_a_count_that_is_far_from_the_estimate_is_not_believed(
    session: Session, factor: float
) -> None:
    # A model of the tests counts nothing, and some servers count only a part of what they
    # were sent. The estimate is better than such a count.
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("What does notes.txt say? " * 40)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated

    session.append(reply("It says 42.", sent=round(request * factor)))

    measured = meter.measure(session)
    assert measured.rate is None
    assert measured.tokens == measured.estimated


@pytest.mark.parametrize("factor", [0.5, 2.0])
def test_a_count_of_half_or_twice_the_estimate_is_still_believed(
    session: Session, factor: float
) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("word" * 1000)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated

    session.append(reply("ok", sent=round(request * factor)))

    assert meter.measure(session).rate == pytest.approx(factor, abs=0.001)


def test_a_count_that_is_not_believed_leaves_the_one_before_it_in_place(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("word" * 1000)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated
    session.append(reply("ok", sent=round(request * 1.5)))
    believed = meter.measure(session).rate

    session.append(UserMessageEvent(message=said("again")))
    session.append(reply("ok", sent=3))

    assert meter.measure(session).rate == believed == pytest.approx(1.5, abs=0.001)


def test_a_reply_that_was_cut_short_by_the_user_tells_nothing(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("word" * 1000)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated

    session.append(reply("half", sent=round(request * 1.2), cut_short=True))

    assert meter.measure(session).rate is None


def test_the_count_for_one_model_says_nothing_about_another(session: Session) -> None:
    meter = TokenMeter()
    session.append(UserMessageEvent(message=said("word" * 1000)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated
    session.append(reply("ok", sent=round(request * 1.3)))
    with_the_first = meter.measure(session).rate

    other = MODEL.model_copy(update={"model": "another"})
    session.append(RequestHeaderEvent(config=other, tools=()))
    with_the_second = meter.measure(session).rate
    session.append(RequestHeaderEvent(config=MODEL, tools=()))

    assert with_the_first == pytest.approx(1.3, abs=0.001)
    assert with_the_second is None
    # Back with the model that was counted, its count holds again.
    assert meter.measure(session).rate == with_the_first


def test_after_a_part_was_replaced_the_rate_applies_to_what_is_there_now(session: Session) -> None:
    meter = TokenMeter()
    session.append(SystemMessageEvent(message=SYSTEM))
    long = session.append(UserMessageEvent(message=said("A long story. " * 400)))
    session.append(RequestHeaderEvent(config=MODEL, tools=()))
    request = meter.measure(session).estimated
    answer = session.append(reply("A long answer. " * 400, sent=round(request * 0.8)))

    summary = said("A story and an answer.")
    session.append(UserMessageEvent(message=summary), replaces=[long.seq, answer.seq])

    measured = meter.measure(session)
    assert measured.estimated == estimate_message(SYSTEM) + estimate_message(summary)
    assert measured.tokens == round(measured.estimated * 0.8)


# Keeping up with the log


def test_every_session_has_an_account_of_its_own(ctx: Context, session: Session) -> None:
    meter = TokenMeter()
    other = ctx.use(SESSIONS).create(ctx, ".", session_id="s-2")
    session.append(UserMessageEvent(message=said("word " * 20)))
    other.append(UserMessageEvent(message=said("word " * 60)))

    assert meter.measure(session).estimated == 23 + 3
    assert meter.measure(other).estimated == 67 + 3


@settings(max_examples=60, deadline=None)
@given(
    st.lists(
        st.tuples(
            st.sampled_from(["say", "answer", "header", "merge", "note"]), st.integers(0, 400)
        ),
        max_size=25,
    )
)
async def test_measuring_on_the_way_comes_to_the_same_as_measuring_at_the_end(
    steps: list[tuple[str, int]],
) -> None:
    ctx = Context()
    try:
        await ctx.plugin(columbina.session)
        log = ctx.use(SESSIONS).create(ctx, ".", session_id="s-1")
        on_the_way = TokenMeter()
        for what, size in steps:
            if what == "say":
                log.append(UserMessageEvent(message=said("word " * size)))
            elif what == "answer":
                log.append(reply("word " * size or "ok", sent=size * 3))
            elif what == "header":
                tools = (READ, WRITE)[: size % 3]
                model = MODEL if size % 2 else MODEL.model_copy(update={"model": "another"})
                log.append(RequestHeaderEvent(config=model, tools=tools))
            elif what == "merge" and len(log.surface) >= 2:
                first, second = log.surface[-2:]
                log.append(UserMessageEvent(message=said("merged")), replaces=[first, second])
            else:
                log.append(TurnStartEvent(turn=size))
            on_the_way.measure(log)

        assert on_the_way.measure(log) == TokenMeter().measure(log)
    finally:
        await ctx.fiber.dispose()


async def test_the_plugin_provides_the_estimate(ctx: Context, session: Session) -> None:
    await ctx.plugin(columbina.token_meter)

    meter = ctx.use(TOKEN_METER)

    assert meter.measure(session) == Measure(tokens=0, estimated=0, rate=None)
