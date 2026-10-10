"""The line below the prompt: the model, what the session may do, and how much was sent."""

import pytest

import columbina.fs
import columbina.sandbox
import columbina.session
import columbina.token_meter
import columbina.tools
from columbina.apps.cli.status import status
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    CallConfig,
    LlmFailure,
    TextBlock,
    Usage,
    UserMessage,
)
from columbina.sandbox import SANDBOX, Mode, Sandbox, Strength
from columbina.session import (
    SESSIONS,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    Session,
    TurnStartEvent,
    UserMessageEvent,
)
from columbina.token_meter import TOKEN_METER, TokenMeter

MODEL = CallConfig(provider="deepseek", model="deepseek-flash")
SAID = AssistantMessage(content=(TextBlock(text="Hello."),))


@pytest.fixture
async def session(ctx: Context) -> Session:
    await ctx.plugin(columbina.session)
    return ctx.use(SESSIONS).create(ctx, ".", session_id="s-1")


async def sandbox_of(ctx: Context, **config: object) -> Sandbox:
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox, config)
    return ctx.use(SANDBOX)


def reply(**usage: int) -> AssistantMessageEvent:
    return AssistantMessageEvent(message=SAID, usage=Usage(**usage), finish_reason="stop")


async def test_a_session_that_has_not_begun_shows_the_model(session: Session) -> None:
    assert status(session, MODEL, None, busy=False) == "deepseek-flash"


@pytest.mark.parametrize(
    ("mode", "told"),
    [
        ("read-only", "read-only, commands not held"),
        ("workspace-write", "workspace-write, commands not held"),
        ("danger-full-access", "danger-full-access, commands not held"),
    ],
)
async def test_the_mode_of_the_sandbox_is_shown_as_it_is_now(
    ctx: Context, session: Session, mode: Mode, told: str
) -> None:
    sandbox = await sandbox_of(ctx)

    before = status(session, MODEL, sandbox, busy=False)
    sandbox.set_mode(session, mode)

    assert before == "deepseek-flash | workspace-write, commands not held"
    assert status(session, MODEL, sandbox, busy=False) == f"deepseek-flash | {told}"


async def test_a_mode_that_is_not_written_down_yet_is_shown_if_somebody_knows_it(
    ctx: Context, session: Session
) -> None:
    # Before the first turn the log names no mode, and the permissions know which it will be.
    sandbox = await sandbox_of(ctx)

    told = status(session, MODEL, sandbox, busy=False, mode="danger-full-access")

    assert told == "deepseek-flash | danger-full-access, commands not held"


@pytest.mark.parametrize(
    ("strength", "told"), [("partial", "commands held in part"), ("full", "commands held")]
)
async def test_how_well_commands_are_held_is_shown(
    ctx: Context, session: Session, strength: Strength, told: str
) -> None:
    class Holding:
        def __init__(self) -> None:
            self.strength: Strength = strength

        def confine(self, argv: object, *, mode: Mode, cwd: object) -> object:
            return argv

    sandbox = await sandbox_of(ctx, provider="holding")
    sandbox.providers.add(ctx, "holding", Holding())  # pyright: ignore[reportArgumentType]

    assert (
        status(session, MODEL, sandbox, busy=False) == f"deepseek-flash | workspace-write, {told}"
    )


@pytest.mark.parametrize(
    ("sent", "told"),
    [
        (1, "1"),
        (999, "999"),
        (1000, "1.0k"),
        (9240, "9.2k"),
        (99_949, "99.9k"),
        (100_000, "100k"),
        (128_500, "128k"),
        (1_000_000, "1.0M"),
        (2_340_000, "2.3M"),
    ],
)
async def test_what_the_last_request_sent_is_shown_in_few_digits(
    session: Session, sent: int, told: str
) -> None:
    session.append(reply(input_tokens=sent))

    assert status(session, MODEL, None, busy=False) == f"deepseek-flash | {told} tokens | cache 0%"


async def test_what_was_sent_is_everything_that_was_sent_and_the_last_request_counts(
    session: Session,
) -> None:
    session.append(reply(input_tokens=5000, output_tokens=900))
    session.append(TurnStartEvent(turn=2))
    session.append(
        reply(input_tokens=200, cache_read_tokens=5000, cache_write_tokens=800, output_tokens=50)
    )

    # The last request sent 6000 tokens. Of the 11000 that were sent in all, 5000 were cached.
    assert status(session, MODEL, None, busy=False) == ("deepseek-flash | 6.0k tokens | cache 45%")


async def test_a_request_that_failed_counts_as_the_last_one(session: Session) -> None:
    session.append(reply(input_tokens=5000))
    session.append(
        AssistantAttemptEvent(
            message=SAID,
            usage=Usage(input_tokens=7000),
            failure=LlmFailure(message="the stream broke", code="STREAM_CLOSED"),
        )
    )

    assert status(session, MODEL, None, busy=False).split(" | ")[1] == "7.0k tokens"


async def test_with_a_window_of_known_size_the_share_that_is_used_is_shown(
    session: Session,
) -> None:
    roomy = MODEL.model_copy(update={"context_window": 128_000})
    session.append(reply(input_tokens=1000, cache_read_tokens=15_000))

    assert status(session, roomy, None, busy=False) == (
        "deepseek-flash | 16.0k tokens (12%) | cache 94%"
    )


async def test_a_running_turn_says_how_it_is_stopped(ctx: Context, session: Session) -> None:
    sandbox = await sandbox_of(ctx)
    session.append(reply(input_tokens=1000))

    assert status(session, MODEL, sandbox, busy=True) == (
        "deepseek-flash | workspace-write, commands not held | 1.0k tokens | cache 0% | esc stops"
    )


# With an estimate


async def meter_of(ctx: Context) -> TokenMeter:
    await ctx.plugin(columbina.token_meter)
    return ctx.use(TOKEN_METER)


def said(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


async def test_where_there_is_an_estimate_it_is_what_is_shown(
    ctx: Context, session: Session
) -> None:
    meter = await meter_of(ctx)
    roomy = MODEL.model_copy(update={"context_window": 128_000})
    session.append(said("word " * 8000))
    held = meter.measure(session).tokens
    assert 8000 < held < 10_000

    # The mark says that nobody counted this.
    assert status(session, roomy, None, busy=False, meter=meter) == (
        f"deepseek-flash | ~{held / 1000:.1f}k tokens ({round(100 * held / 128_000)}%)"
    )
    # Without the estimate nothing is known yet: no request has been made.
    assert status(session, roomy, None, busy=False) == "deepseek-flash"


async def test_an_estimate_goes_down_as_soon_as_the_conversation_is_shorter(
    ctx: Context, session: Session
) -> None:
    meter = await meter_of(ctx)
    long = session.append(said("word " * 8000)).seq
    session.append(reply(input_tokens=9000))
    before, held = status(session, MODEL, None, busy=False, meter=meter), meter.measure(session)
    assert held.tokens > 8000

    session.append(said("What was said before, in short.", "compaction"), replaces=[long])

    after, left = status(session, MODEL, None, busy=False, meter=meter), meter.measure(session)
    assert left.tokens < 100
    assert before.split(" | ")[1] == f"~{held.tokens / 1000:.1f}k tokens"
    assert after.split(" | ")[1] == f"~{left.tokens} tokens"
    # What the last request sent is what it was, whatever happened to the conversation since.
    assert status(session, MODEL, None, busy=False).split(" | ")[1] == "9.0k tokens"


async def test_an_estimate_of_nothing_is_not_shown(ctx: Context, session: Session) -> None:
    meter = await meter_of(ctx)

    assert status(session, MODEL, None, busy=False, meter=meter) == "deepseek-flash"
