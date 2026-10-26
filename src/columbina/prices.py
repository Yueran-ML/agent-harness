"""What the requests of a session cost in money, as the price lists of the services say.

This module is a plugin. Loading it provides :data:`SPENDING`, which says what
the requests of an open session have cost so far, for a status line.

A service bills by the token: what was sent and was not kept, what was sent
and was found kept from an earlier request, what was written to be kept, and
what the model wrote. :data:`PRICES` holds the prices of the services that
are known by name, as their price lists said on :data:`AS_OF`, per million
tokens. A user who pays other prices, or uses another service, writes them
into ``prices.yml`` in the home of the program, which goes before the list::

    - provider: moonshot
      model: kimi-k3
      currency: CNY
      input: 4
      output: 16
      cache_read: 1

A model is found by the beginning of its name, the longest beginning first,
so ``claude-sonnet-5-5`` is also ``claude-sonnet-5-5-20261001``. A model that
no entry names costs nothing that is known, and is said to be so.

What is worked out here is an estimate. DeepSeek asks for twice its price at
the hours of the most use, on weekdays, and its holidays are not known here.
Anthropic asks more for what is kept for an hour than for five minutes, and
what is written here is the price of five minutes, which is what is asked by
default. A service that adds a tax or rounds its bill comes to another sum.
"""

import datetime
import logging
from collections.abc import Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal, cast
from weakref import WeakKeyDictionary

from pydantic import BaseModel, Field, ValidationError

from columbina import plain_yaml
from columbina.kernel import Context, ServiceKey
from columbina.llm import Usage
from columbina.session import (
    AsideEvent,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    Session,
    SessionEvent,
)

__all__ = [
    "AS_OF",
    "FILE",
    "PRICES",
    "SPENDING",
    "Config",
    "Price",
    "Prices",
    "PricesError",
    "Spending",
    "Spent",
    "apply",
    "name",
    "spent",
]

name = "prices"

logger = logging.getLogger("columbina.prices")

AS_OF = "2026-10-26"
"""The day on which the prices of :data:`PRICES` were read from the price lists."""

FILE = "prices.yml"
"""The file of the prices of the user, in the home of the program."""

_MILLION = 1_000_000

_PEAK_HOURS = frozenset({1, 2, 3, 6, 7, 8, 9})
"""The hours, in UTC, at which DeepSeek asks twice its price on weekdays."""


class Price(BaseModel):
    """What a model costs, per million tokens.

    Attributes:
        provider: The service.
        model: The beginning of the name of the model.
        currency: What the prices are in.
        input: What was sent and not found kept.
        output: What the model wrote, its thinking with it.
        cache_read: What was sent and found kept. By default the price of input.
        cache_write: What was written to be kept. By default the price of input.
        above: For a model whose price changes with the size of a request,
            the number of tokens sent above which ``beyond`` holds.
        beyond: The price of a request above that size.
        peak: How the price changes with the hour: ``deepseek`` for twice
            the price at the hours of most use on weekdays.
    """

    provider: str = Field(min_length=1)
    model: str = Field(min_length=1)
    currency: str = Field(default="USD", min_length=1)
    input: float = Field(ge=0)
    output: float = Field(ge=0)
    cache_read: float | None = Field(default=None, ge=0)
    cache_write: float | None = Field(default=None, ge=0)
    above: int | None = Field(default=None, gt=0)
    beyond: "Price | None" = None
    peak: Literal["deepseek"] | None = None

    def of(self, usage: Usage, at_ms: int) -> float:
        """Return what a request cost, in the currency of the price."""
        price = self
        sent = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens
        if self.above is not None and self.beyond is not None and sent > self.above:
            price = self.beyond
        read = price.input if price.cache_read is None else price.cache_read
        write = price.input if price.cache_write is None else price.cache_write
        cost = (
            usage.input_tokens * price.input
            + usage.output_tokens * price.output
            + usage.cache_read_tokens * read
            + usage.cache_write_tokens * write
        ) / _MILLION
        if self.peak == "deepseek" and _at_the_peak(at_ms):
            cost *= 2
        return cost


def _at_the_peak(at_ms: int) -> bool:
    moment = datetime.datetime.fromtimestamp(at_ms / 1000, tz=datetime.UTC)
    return moment.weekday() < 5 and moment.hour in _PEAK_HOURS


def _price(provider: str, model: str, currency: str = "USD", **more: object) -> Price:
    return Price.model_validate(
        {"provider": provider, "model": model, "currency": currency, **more}
    )


def _claude(model: str, input: float, read: float, output: float) -> Price:
    return _price(
        "anthropic", model, input=input, output=output, cache_read=read, cache_write=input * 1.25
    )


PRICES: tuple[Price, ...] = (
    # https://api-docs.deepseek.com/quick_start/pricing: the prices of the hours of little use.
    _price(
        "deepseek", "deepseek-flash", input=0.15, output=0.60, cache_read=0.003, peak="deepseek"
    ),
    _price(
        "deepseek", "deepseek-v4-flash", input=0.15, output=0.60, cache_read=0.003, peak="deepseek"
    ),
    _price(
        "deepseek", "deepseek-v4-pro", input=0.66, output=1.98, cache_read=0.022, peak="deepseek"
    ),
    # https://platform.claude.com/docs/en/about-claude/pricing: what is kept for five minutes.
    _claude("claude-fable-5-1", 10, 0.25, 50),
    _claude("claude-fable-5", 10, 1, 50),
    _claude("claude-opus-5-5", 4, 0.20, 20),
    _claude("claude-opus-5", 5, 0.50, 25),
    _claude("claude-opus-4", 5, 0.50, 25),
    _claude("claude-sonnet-5-5", 2, 0.10, 10),
    _claude("claude-sonnet-5", 2, 0.20, 10),
    _claude("claude-sonnet-4", 3, 0.30, 15),
    _claude("claude-haiku-4-5", 1, 0.10, 5),
    _price(
        "anthropic",
        "claude-haiku-5-5",
        input=0.10,
        output=0.50,
        cache_read=0.01,
        cache_write=0.125,
        above=100_000,
        beyond=_price(
            "anthropic",
            "claude-haiku-5-5",
            input=0.50,
            output=2.50,
            cache_read=0.05,
            cache_write=0.625,
        ),
    ),
)
"""The prices of the services that are known by name, as they were on :data:`AS_OF`."""


class PricesError(Exception):
    """The file of the prices of the user cannot be used, and why."""


@dataclass(frozen=True, slots=True)
class Prices:
    """The prices that count: those of the user, then those that are known.

    Attributes:
        own: The prices of the user.
        known: The prices that come with the program.
    """

    own: tuple[Price, ...] = ()
    known: tuple[Price, ...] = PRICES

    @classmethod
    def load(cls, home: Path) -> "Prices":
        """Read the prices of the user, beside those that are known.

        Raises:
            PricesError: The file is there and cannot be read or used.
        """
        path = home / FILE
        try:
            text = path.read_text(encoding="utf-8")
        except FileNotFoundError:
            return cls()
        except (OSError, UnicodeDecodeError) as error:
            raise PricesError(f"{path} cannot be read: {error}") from None
        try:
            said = plain_yaml.load(text)
        except plain_yaml.PlainYamlError as error:
            raise PricesError(f"{path} is not plain YAML: {error}") from None
        if said is None:
            return cls()
        if not isinstance(said, list):
            raise PricesError(f"{path} is no list of prices")
        try:
            own = tuple(Price.model_validate(entry) for entry in cast("list[object]", said))
        except ValidationError as error:
            raise PricesError(f"{path} has a price that is not right: {error}") from None
        return cls(own=own)

    def of(self, provider: str, model: str) -> Price | None:
        """Return the price of a model, or ``None`` if none is known."""
        for prices in (self.own, self.known):
            fitting = [
                price
                for price in prices
                if price.provider == provider and model.startswith(price.model)
            ]
            if fitting:
                return max(fitting, key=lambda price: len(price.model))
        return None


@dataclass(slots=True)
class Spent:
    """What the requests of a session cost.

    Attributes:
        money: The sum in each currency.
        unpriced: The models whose requests have no known price, as
            ``provider/model``.
    """

    money: dict[str, float] = field(default_factory=dict[str, float])
    unpriced: list[str] = field(default_factory=list[str])

    def add(self, other: "Spent") -> None:
        """Add what another cost to this."""
        for currency, amount in other.money.items():
            self.money[currency] = self.money.get(currency, 0.0) + amount
        self.unpriced += [model for model in other.unpriced if model not in self.unpriced]

    def told(self) -> str:
        """Say the sum in a few words, such as ``$0.42`` or ``$0.42 + ¥3.10``."""
        signs = {"USD": "$", "CNY": "¥", "EUR": "€"}
        parts = [
            f"{signs[currency]}{amount:.2f}" if currency in signs else f"{amount:.2f} {currency}"
            for currency, amount in sorted(self.money.items())
        ]
        return " + ".join(parts)


@dataclass(slots=True)
class _Kept:
    folded: int = 0
    model: tuple[str, str] | None = None
    spent: Spent = field(default_factory=Spent)


def _fold(kept: _Kept, event: SessionEvent[EventData], prices: Prices) -> None:
    data = event.data
    if isinstance(data, RequestHeaderEvent):
        kept.model = (data.config.provider, data.config.model)
        return
    if not isinstance(data, AssistantMessageEvent | AssistantAttemptEvent | AsideEvent):
        return
    if kept.model is None:
        return
    usage = data.usage
    if not (
        usage.input_tokens
        or usage.output_tokens
        or usage.cache_read_tokens
        or usage.cache_write_tokens
    ):
        return
    provider, model = kept.model
    price = prices.of(provider, model)
    if price is None:
        named = f"{provider}/{model}"
        if named not in kept.spent.unpriced:
            kept.spent.unpriced.append(named)
        return
    money = kept.spent.money
    money[price.currency] = money.get(price.currency, 0.0) + price.of(usage, event.time)


def spent(events: Iterable[SessionEvent[EventData]], prices: Prices) -> Spent:
    """Return what the requests of some events of a log cost, such as those of a stored one."""
    kept = _Kept()
    for event in events:
        _fold(kept, event, prices)
    return kept.spent


class Spending:
    """What the requests of open sessions cost, kept up with their logs."""

    __slots__ = ("_kept", "prices")

    def __init__(self, prices: Prices) -> None:
        self.prices = prices
        self._kept: WeakKeyDictionary[Session, _Kept] = WeakKeyDictionary()

    def of(self, session: Session) -> Spent:
        """Return what the requests of a session have cost so far."""
        kept = self._kept.setdefault(session, _Kept())
        events: Sequence[SessionEvent[EventData]] = session.events
        while kept.folded < len(events):
            _fold(kept, events[kept.folded], self.prices)
            kept.folded += 1
        return kept.spent


SPENDING = ServiceKey[Spending]("spending")
"""What the requests of open sessions cost."""


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        home: The home of the program, where the file of the prices of the
            user is. Without it only the prices that are known count.
    """

    home: Path | None = None


def apply(ctx: Context, config: Config) -> None:
    """Provide what the requests of open sessions cost."""
    prices = Prices()
    if config.home is not None:
        try:
            prices = Prices.load(config.home)
        except PricesError as error:
            logger.warning("%s; the prices that are known count alone", error)
    ctx.provide(SPENDING, Spending(prices))
