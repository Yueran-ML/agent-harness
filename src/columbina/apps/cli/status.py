"""The line below the prompt: what somebody at the terminal wants to see at a glance.

It says which model answers, what the session may do to files, how well
commands are held, how much the conversation comes to, and how much of
everything the model was sent it could take from its cache. The numbers come
from the log of the session, so they are right also for a session that was
taken up again.

How much the conversation comes to is what the next request will send. Where
the program has an estimate of that, the estimate is shown, with a mark that
says so: it goes down at once when the conversation is made shorter. Without
one, what is shown is what the last request sent, as the service counted it.
"""

from columbina.llm import CallConfig
from columbina.prices import Spending
from columbina.sandbox import Mode, Sandbox, Strength
from columbina.session import (
    USAGE,
    AssistantAttemptEvent,
    AssistantMessageEvent,
    Session,
)
from columbina.token_meter import TokenMeter

__all__ = ["status"]

_HELD: dict[Strength, str] = {
    "none": "commands not held",
    "partial": "commands held in part",
    "full": "commands held",
}
_BETWEEN = " | "


def _short(tokens: int) -> str:
    """Write a number of tokens the way a status line has room for."""
    if tokens < 1000:
        return str(tokens)
    if tokens < 100_000:
        return f"{tokens / 1000:.1f}k"
    if tokens < 1_000_000:
        return f"{tokens // 1000}k"
    return f"{tokens / 1_000_000:.1f}M"


def _last_sent(session: Session) -> int:
    """Return how many tokens the model was sent with the last request, or 0 before the first."""
    for event in reversed(session.events):
        data = event.data
        if isinstance(data, AssistantMessageEvent | AssistantAttemptEvent):
            usage = data.usage
            return usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens
    return 0


def _held(session: Session, meter: TokenMeter) -> int:
    """Return what the conversation comes to now, as it is estimated."""
    return meter.measure(session).tokens


def status(
    session: Session,
    model: CallConfig,
    sandbox: Sandbox | None,
    *,
    busy: bool,
    mode: Mode | None = None,
    meter: TokenMeter | None = None,
    spending: Spending | None = None,
) -> str:
    """Return the status line of a session.

    Args:
        session: The session.
        model: The call config that requests are made with.
        sandbox: The sandbox, if the program has one.
        busy: Whether a turn is running, which Escape would stop.
        mode: The mode of the sandbox that the session goes by, if somebody
            knows it better than the sandbox. Before the first turn the log
            names none, and the permissions know which one it will be.
        meter: The estimate of the size of sessions, if the program has one.
        spending: What the requests of sessions cost, if the program knows.
    """
    parts = [model.model]
    if sandbox is not None:
        parts.append(f"{mode or sandbox.mode(session)}, {_HELD[sandbox.strength(session)]}")
    # An estimate is marked as one. What the service counted is not.
    sent, about = (_last_sent(session), "") if meter is None else (_held(session, meter), "~")
    if sent:
        told = f"{about}{_short(sent)} tokens"
        if model.context_window:
            told += f" ({round(100 * sent / model.context_window)}%)"
        parts.append(told)
    total = USAGE.of(session).usage
    everything = total.input_tokens + total.cache_read_tokens + total.cache_write_tokens
    if everything:
        parts.append(f"cache {round(100 * total.cache_read_tokens / everything)}%")
    money = "" if spending is None else spending.of(session).told()
    if money:
        parts.append(f"~{money}")
    if busy:
        parts.append("esc stops")
    return _BETWEEN.join(parts)
