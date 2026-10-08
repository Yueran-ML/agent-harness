"""Making a request again when it failed for a reason that passes.

This module is a plugin. It listens to ``agent/request-error``. An adapter
never retries by itself, so this is the one place where that is decided.
"""

import asyncio
import random
from typing import ClassVar

from pydantic import BaseModel, Field

from tack.agent import REQUEST_ERROR, ErrorDecision, RequestFailure, Retry
from tack.kernel import Context, Next
from tack.llm.types import LlmFailure
from tack.session import SESSIONS, EventData

__all__ = ["Config", "RetryEvent", "apply", "delay_for", "inject", "name"]

name = "retry"
inject = (SESSIONS,)


class Config(BaseModel):
    """The config of the retry plugin.

    Attributes:
        max_attempts: How often a request is made at most, the first time
            included.
        codes: The failure codes that are worth another attempt.
        base_delay_seconds: The wait before the second attempt. It doubles
            with every further one.
        max_delay_seconds: The longest wait.
        jitter: How far a wait is spread around its value, as a fraction of
            it, so that many agents do not come back at the same moment.
        seed: The seed of the spreading. Set it to get the same waits again.
    """

    max_attempts: int = Field(default=5, ge=1)
    codes: tuple[str, ...] = (
        "EMPTY_RESPONSE",
        "RATE_LIMIT",
        "SERVER",
        "TIMEOUT",
        "TRANSPORT",
        "STREAM_CLOSED",
    )
    base_delay_seconds: float = Field(default=0.5, ge=0)
    max_delay_seconds: float = Field(default=10.0, ge=0)
    jitter: float = Field(default=0.1, ge=0, le=1)
    seed: int | None = None


class RetryEvent(EventData):
    """A request is about to be made again.

    The event is written before the wait begins. A reader that does not know
    it may skip it: it changes nothing about the conversation.

    Attributes:
        turn: The number of the turn.
        step: The number of the step in that turn.
        attempt: Which attempt of the step failed, counted from 1.
        code: The code of the failure.
        delay_ms: The wait before the next attempt.
    """

    event_type: ClassVar[str] = "llm/retry"
    ignorable: ClassVar[bool] = True

    turn: int
    step: int
    attempt: int
    code: str
    delay_ms: int


def delay_for(
    failure: LlmFailure, attempt: int, config: Config, randomness: random.Random
) -> float:
    """Return how many seconds to wait after ``attempt`` failed.

    A wait that the provider asked for is kept as it is, if it is within the
    limit. Otherwise the wait doubles with every attempt up to the limit, and
    is then spread by the jitter.
    """
    asked = failure.retry_after_ms
    if asked is not None and asked / 1000 <= config.max_delay_seconds:
        return asked / 1000
    delay = min(config.max_delay_seconds, config.base_delay_seconds * 2 ** (attempt - 1))
    return delay * (1 + randomness.uniform(-config.jitter, config.jitter))


def apply(ctx: Context, config: Config) -> None:
    """Register the event of a retry and listen for failed requests."""
    randomness = random.Random(config.seed)

    async def on_failure(
        failed: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        failure = failed.failure
        if failure.code not in config.codes or failed.attempt >= config.max_attempts:
            return await proceed(failed)
        delay = delay_for(failure, failed.attempt, config, randomness)
        failed.agent.session.append(
            RetryEvent(
                turn=failed.turn,
                step=failed.step,
                attempt=failed.attempt,
                code=failure.code,
                delay_ms=round(delay * 1000),
            )
        )
        await asyncio.sleep(delay)
        return Retry()

    ctx.use(SESSIONS).declare(ctx, RetryEvent)
    ctx.on(REQUEST_ERROR, on_failure)
