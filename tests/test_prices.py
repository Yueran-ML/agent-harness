"""What the requests of a session cost in money."""

import datetime
import logging
from pathlib import Path

import pytest

import columbina.prices
import columbina.session
from columbina.kernel import Context
from columbina.llm import AssistantMessage, CallConfig, LlmFailure, Usage
from columbina.prices import (
    PRICES,
    SPENDING,
    Price,
    Prices,
    PricesError,
    Spending,
    Spent,
    spent,
)
from columbina.session import (
    SESSIONS,
    AsideEvent,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    TurnStartEvent,
)


def at(day: int, hour: int) -> int:
    """A moment of the week of 2026-10-26, a Monday, in milliseconds."""
    monday = datetime.datetime(2026, 10, 26, hour, 30, tzinfo=datetime.UTC)
    moment = monday + datetime.timedelta(days=day)
    return round(moment.timestamp() * 1000)


MILLION = 1_000_000


def test_a_request_costs_what_each_kind_of_token_costs() -> None:
    price = Price(provider="p", model="m", input=2, output=10, cache_read=0.1, cache_write=2.5)
    usage = Usage(
        input_tokens=MILLION,
        output_tokens=MILLION,
        cache_read_tokens=MILLION,
        cache_write_tokens=MILLION,
    )

    assert price.of(usage, at(0, 12)) == pytest.approx(2 + 10 + 0.1 + 2.5)


def test_what_is_kept_costs_what_is_sent_unless_the_price_says() -> None:
    price = Price(provider="p", model="m", input=1, output=0)
    usage = Usage(cache_read_tokens=MILLION, cache_write_tokens=MILLION)

    assert price.of(usage, 0) == pytest.approx(2)


@pytest.mark.parametrize(
    ("day", "hour", "times"),
    [(0, 1, 2), (0, 9, 2), (4, 6, 2), (0, 4, 1), (0, 5, 1), (0, 10, 1), (5, 2, 1), (6, 8, 1)],
)
def test_deepseek_asks_twice_its_price_at_the_hours_of_most_use(
    day: int, hour: int, times: int
) -> None:
    price = Prices().of("deepseek", "deepseek-flash")
    assert price is not None

    cost = price.of(Usage(output_tokens=MILLION), at(day, hour))

    assert cost == pytest.approx(0.60 * times)


def test_a_large_request_costs_more_where_the_model_says_so() -> None:
    price = Prices().of("anthropic", "claude-haiku-5-5")
    assert price is not None

    small = price.of(Usage(input_tokens=100_000), 0)
    large = price.of(Usage(input_tokens=60_000, cache_read_tokens=50_000), 0)

    assert small == pytest.approx(0.01)
    assert large == pytest.approx((60_000 * 0.50 + 50_000 * 0.05) / MILLION)


def test_a_model_is_found_by_the_longest_beginning_of_its_name() -> None:
    prices = Prices()

    sonnet = prices.of("anthropic", "claude-sonnet-5-5-20261001")
    older = prices.of("anthropic", "claude-sonnet-5")

    assert sonnet is not None
    assert sonnet.input == 2
    assert sonnet.cache_read == 0.10
    assert older is not None
    assert older.cache_read == 0.20
    assert prices.of("openai", "gpt-5") is None
    assert prices.of("deepseek", "other") is None


def test_the_known_prices_are_dated() -> None:
    assert columbina.prices.AS_OF == "2026-10-26"
    assert {(price.provider, price.model) for price in PRICES} >= {
        ("deepseek", "deepseek-flash"),
        ("anthropic", "claude-opus-5-5"),
    }


def test_the_prices_of_the_user_go_first(tmp_path: Path) -> None:
    (tmp_path / "prices.yml").write_text(
        "- provider: deepseek\n  model: deepseek-flash\n  input: 1\n  output: 2\n"
        "- provider: moonshot\n  model: kimi\n  currency: CNY\n  input: 4\n  output: 16\n",
        encoding="utf-8",
    )

    prices = Prices.load(tmp_path)

    flash = prices.of("deepseek", "deepseek-flash")
    kimi = prices.of("moonshot", "kimi-k3")
    assert flash is not None
    assert flash.input == 1
    assert kimi is not None
    assert kimi.currency == "CNY"
    assert prices.of("anthropic", "claude-opus-5-5") is not None


@pytest.mark.parametrize("text", ["", "# nothing\n"])
def test_an_empty_file_of_prices_adds_nothing(tmp_path: Path, text: str) -> None:
    (tmp_path / "prices.yml").write_text(text, encoding="utf-8")

    assert Prices.load(tmp_path).own == ()
    assert Prices.load(tmp_path / "nowhere").own == ()


@pytest.mark.parametrize(
    ("content", "said"),
    [
        (b"provider: x\n", "is no list of prices"),
        (b"- [unclosed\n", "is not plain YAML"),
        (b"- provider: x\n", "has a price that is not right"),
        (b"\xff\xfe", "cannot be read"),
    ],
)
def test_a_file_of_prices_that_cannot_be_used_is_said(
    tmp_path: Path, content: bytes, said: str
) -> None:
    (tmp_path / "prices.yml").write_bytes(content)

    with pytest.raises(PricesError, match=said):
        Prices.load(tmp_path)


def events(*happened: tuple[int, EventData]) -> list[SessionEvent[EventData]]:
    return [SessionEvent(seq, time, data) for seq, (time, data) in enumerate(happened)]


def header(provider: str, model: str) -> RequestHeaderEvent:
    return RequestHeaderEvent(config=CallConfig(provider=provider, model=model), tools=())


def replied(usage: Usage) -> AssistantMessageEvent:
    return AssistantMessageEvent(
        message=AssistantMessage(content=()), usage=usage, finish_reason="stop"
    )


def test_a_session_costs_what_its_requests_cost_by_the_model_of_each() -> None:
    sunday = at(6, 12)
    failure = LlmFailure(code="RATE_LIMITED", message="later")

    log = events(
        (sunday, TurnStartEvent(turn=1)),
        (sunday, replied(Usage(output_tokens=MILLION))),
        (sunday, header("deepseek", "deepseek-flash")),
        (sunday, replied(Usage(output_tokens=MILLION))),
        (sunday, replied(Usage())),
        (
            sunday,
            AssistantAttemptEvent(
                message=AssistantMessage(content=()),
                usage=Usage(input_tokens=MILLION),
                failure=failure,
            ),
        ),
        (sunday, header("anthropic", "claude-sonnet-5-5")),
        (sunday, AsideEvent(purpose="summary", usage=Usage(output_tokens=MILLION))),
        (sunday, header("openai", "gpt-5")),
        (sunday, replied(Usage(output_tokens=1))),
        (sunday, replied(Usage(output_tokens=1))),
    )

    cost = spent(log, Prices())

    assert cost.money == {"USD": pytest.approx(0.60 + 0.15 + 10)}
    assert cost.unpriced == ["openai/gpt-5"]


def test_what_was_spent_is_told_in_few_words() -> None:
    assert Spent().told() == ""
    assert Spent({"USD": 0.4249}).told() == "$0.42"
    assert (
        Spent({"USD": 1, "CNY": 3.1, "EUR": 2, "JPY": 100}).told()
        == "¥3.10 + €2.00 + 100.00 JPY + $1.00"
    )


def test_what_was_spent_adds_up() -> None:
    total = Spent({"USD": 1}, ["a/b"])

    total.add(Spent({"USD": 2, "CNY": 1}, ["a/b", "c/d"]))

    assert total == Spent({"USD": 3, "CNY": 1}, ["a/b", "c/d"])


async def test_what_an_open_session_has_cost_is_kept_up_with_its_log(
    ctx: Context, tmp_path: Path
) -> None:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.prices)
    session = ctx.use(SESSIONS).create(ctx, tmp_path)
    spending = ctx.use(SPENDING)

    assert spending.of(session).money == {}
    session.append(header("anthropic", "claude-opus-5-5"))
    session.append(replied(Usage(output_tokens=MILLION)))
    assert spending.of(session).money == {"USD": pytest.approx(20)}
    session.append(replied(Usage(output_tokens=MILLION)))
    assert spending.of(session).money == {"USD": pytest.approx(40)}
    assert isinstance(spending, Spending)


async def test_the_plugin_reads_the_prices_of_the_user(ctx: Context, tmp_path: Path) -> None:
    (tmp_path / "prices.yml").write_text(
        "- provider: openai\n  model: gpt\n  input: 1\n  output: 1\n", encoding="utf-8"
    )

    await ctx.plugin(columbina.prices, {"home": str(tmp_path)})

    assert ctx.use(SPENDING).prices.of("openai", "gpt-5") is not None


async def test_prices_that_cannot_be_read_leave_the_known_ones(
    ctx: Context, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    (tmp_path / "prices.yml").write_text("nonsense: [", encoding="utf-8")

    with caplog.at_level(logging.WARNING, "columbina.prices"):
        await ctx.plugin(columbina.prices, {"home": str(tmp_path)})

    assert "the prices that are known count alone" in caplog.text
    assert ctx.use(SPENDING).prices.own == ()
