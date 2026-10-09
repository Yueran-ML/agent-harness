"""How many tokens a conversation comes to, without asking a model.

This module is a plugin. Loading it provides :data:`TOKEN_METER`.

Whoever decides when a conversation has grown too long needs its size before
the next request is made, and a model is not asked for that. So the size is
estimated, in two steps.

**By the kinds of the characters.** A text is priced by a small table: so
much for a space, for a digit, for a letter, for a letter in a long run of
letters and digits, for a sign, for a character that East Asian scripts
write into a square, and for whatever is left. The table was chosen on 28
texts that the service of DeepSeek had counted: prose, code, documents in
Chinese, JSON, paths, hashes, tables of numbers, the output of tools. On
them it is off by 7% on average and by 19% at worst. A text that took no
part in choosing the rates is off by 10% on average. The rule of thumb,
four characters to a token, is off by 23% on average: a hash costs three
times what a line of prose does, and a sign costs more than a letter.

**By what the provider counted.** A reply comes with the number of tokens
that its request held. That number, over what the table made of the same
request, is the rate at which the table is off in this conversation, and
what the table says from then on is put right by it. The rate is believed
if the provider and the model are still the same and if it is not too far
from one: a provider that counts a tenth of what was sent has not counted.

The second step needs no care when a part of the conversation is replaced
by a summary. It scales what is there now, whatever was there before.
"""

import json
import re
from collections.abc import Sequence
from dataclasses import dataclass, field
from weakref import WeakKeyDictionary

from columbina.kernel import Context, ServiceKey
from columbina.llm import (
    Message,
    ReasoningBlock,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
)
from columbina.session import (
    AssistantMessageEvent,
    EventData,
    RequestHeaderEvent,
    Session,
    SessionEvent,
)

__all__ = [
    "PER_MESSAGE",
    "RATES",
    "TOKEN_METER",
    "TOOLS_DECLARED",
    "Measure",
    "TokenMeter",
    "apply",
    "estimate",
    "estimate_message",
    "estimate_tools",
    "name",
]

name = "token-meter"

RATES: dict[str, float] = {
    "space": 0.15,
    "digit": 0.50,
    "letter": 0.24,
    "dense": 0.85,
    "sign": 0.70,
    "wide": 0.62,
    "other": 0.50,
}
"""Tokens for one character of each kind.

``space`` is the white space of ASCII, ``digit`` a digit and ``letter`` a
letter of ASCII, ``dense`` a letter in a run of :data:`DENSE_RUN` or more
letters and digits, ``sign`` any other character of ASCII, ``wide`` a
character that East Asian scripts write into a square, and ``other``
whatever is left. Nothing was measured for ``other``.
"""

DENSE_RUN = 24
"""How many letters and digits in a row are no word any more, but a hash or a key."""

PER_MESSAGE = 3
"""What a message costs beside its content."""

TOOLS_DECLARED = 210
"""What it costs that tools are declared at all, beside what is written about each."""

_BELIEVED = (0.5, 2.0)
"""How far from one the rate of a provider may be for it to be believed."""

_PER_HUNDRED = {kind: round(rate * 100) for kind, rate in RATES.items()}
_DIGITS = b"0123456789"
_SPACES = b" \t\r\n"
_LETTERS = bytes(range(ord("a"), ord("z") + 1)) + bytes(range(ord("A"), ord("Z") + 1))
_RUN = re.compile(rb"[0-9A-Za-z]{%d,}" % DENSE_RUN)
_WIDE = re.compile(
    "["
    + "".join(
        f"{chr(first)}-{chr(last)}"
        for first, last in (
            (0x1100, 0x115F),
            (0x2E80, 0x303E),
            (0x3041, 0x33FF),
            (0x3400, 0x4DBF),
            (0x4E00, 0x9FFF),
            (0xA000, 0xA4CF),
            (0xAC00, 0xD7A3),
            (0xF900, 0xFAFF),
            (0xFE30, 0xFE4F),
            (0xFF00, 0xFF60),
            (0xFFE0, 0xFFE6),
            (0x20000, 0x3FFFD),
        )
    )
    + "]"
)
"""The characters that East Asian scripts write into a square."""


def _without(raw: bytes, taken_out: bytes) -> int:
    """Return how many of the bytes are among those to take out."""
    return len(raw) - len(raw.translate(None, taken_out))


def estimate(text: str) -> int:
    """Return how many tokens a text comes to. A part of a token counts as one.

    The characters are counted by their kinds and never looked at one by one,
    so a text of a million characters takes a few hundredths of a second.
    """
    if not text:
        return 0
    # Whatever is beyond ASCII becomes a sign here that ends a run, and is counted apart.
    raw = text.encode("ascii", "replace")
    beyond = len(raw) - len(text.encode("ascii", "ignore"))
    wide = 0 if not beyond else len(text) - len(_WIDE.sub("", text))
    digits = _without(raw, _DIGITS)
    spaces = _without(raw, _SPACES)
    letters = _without(raw, _LETTERS)
    dense = sum(len(run) - _without(run, _DIGITS) for run in _RUN.findall(raw))
    counts = {
        "space": spaces,
        "digit": digits,
        "letter": letters - dense,
        "dense": dense,
        "sign": len(raw) - beyond - digits - spaces - letters,
        "wide": wide,
        "other": beyond - wide,
    }
    hundredths = sum(_PER_HUNDRED[kind] * count for kind, count in counts.items())
    return -(-hundredths // 100)


def estimate_message(message: Message) -> int:
    """Return how many tokens a message comes to, with what it costs beside its content."""
    total = PER_MESSAGE
    for block in message.content:
        if isinstance(block, TextBlock | ReasoningBlock):
            total += estimate(block.text)
        else:
            total += _call(block)
    if isinstance(message, ToolMessage):
        total += estimate(message.call_id)
    return total


def _call(block: ToolCallBlock) -> int:
    return estimate(block.name) + estimate(block.arguments) + estimate(block.call_id)


def estimate_tools(tools: Sequence[ToolSchema]) -> int:
    """Return how many tokens the declarations of tools come to. No tools cost nothing."""
    if not tools:
        return 0
    written = (
        json.dumps(
            {"name": tool.name, "description": tool.description, "parameters": tool.parameters}
        )
        for tool in tools
    )
    return TOOLS_DECLARED + sum(estimate(one) for one in written)


@dataclass(frozen=True, slots=True)
class Measure:
    """The size of what a model would be sent now.

    Attributes:
        tokens: The estimate, put right by the rate of the provider if one
            is believed.
        estimated: What the table alone makes of it.
        rate: The rate that was applied: what the provider counted over what
            the table made of the same request. ``None`` if none is believed.
    """

    tokens: int
    estimated: int
    rate: float | None


@dataclass(slots=True)
class _Ledger:
    """What is known of one session, kept up with its log event by event."""

    folded: int = 0
    prices: dict[int, int] = field(default_factory=dict[int, int])
    messages: int = 0
    tools: int = 0
    model: tuple[str, str] | None = None
    rate: float | None = None
    rate_of: tuple[str, str] | None = None


class TokenMeter:
    """Estimates the size of sessions, each kept up with its log."""

    __slots__ = ("_ledgers",)

    def __init__(self) -> None:
        self._ledgers: WeakKeyDictionary[Session, _Ledger] = WeakKeyDictionary()

    def measure(self, session: Session) -> Measure:
        """Return the size of what the model of a session would be sent now.

        That is every message that the model sees, and the tools as the last
        request declared them. Each event of the log is looked at once,
        whenever this is asked.
        """
        ledger = self._ledgers.setdefault(session, _Ledger())
        events = session.events
        while ledger.folded < len(events):
            self._fold(ledger, events[ledger.folded])
            ledger.folded += 1
        estimated = ledger.messages + ledger.tools
        rate = ledger.rate if ledger.rate_of == ledger.model else None
        return Measure(
            tokens=estimated if rate is None else round(estimated * rate),
            estimated=estimated,
            rate=rate,
        )

    def _fold(self, ledger: _Ledger, event: SessionEvent[EventData]) -> None:
        data = event.data
        if isinstance(data, RequestHeaderEvent):
            ledger.tools = estimate_tools(data.tools)
            ledger.model = (data.config.provider, data.config.model)
        if isinstance(data, AssistantMessageEvent) and data.finish_reason is not None:
            # What the provider counted is the request, which did not hold this reply yet.
            sent = (
                data.usage.input_tokens
                + data.usage.cache_read_tokens
                + data.usage.cache_write_tokens
            )
            before = ledger.messages + ledger.tools
            rate = sent / before if before else 0.0
            if _BELIEVED[0] <= rate <= _BELIEVED[1]:
                ledger.rate, ledger.rate_of = rate, ledger.model
        message = data.for_model() if event.on_surface else None
        if message is not None:
            for seq in event.replaces:
                ledger.messages -= ledger.prices.pop(seq)
            price = ledger.prices[event.seq] = estimate_message(message)
            ledger.messages += price


TOKEN_METER = ServiceKey[TokenMeter]("token-meter")
"""The estimate of the size of sessions."""


def apply(ctx: Context, config: object) -> None:
    """Provide the estimate."""
    ctx.provide(TOKEN_METER, TokenMeter())
