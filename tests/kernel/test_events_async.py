import asyncio
import logging

import pytest

from columbina.kernel import (
    Context,
    EventModeError,
    Next,
    ParallelEvent,
    SerialEvent,
    WaterfallEvent,
)

FLUSH = ParallelEvent[str]("test/flush")
DECIDE = SerialEvent[str, int]("test/decide")
RENDER = WaterfallEvent[str, str]("test/render")


async def final(payload: str) -> str:
    return f"<{payload}>"


async def test_parallel_runs_listeners_concurrently_and_waits_for_all(ctx: Context) -> None:
    both_started = asyncio.Event()
    started: list[str] = []
    finished: list[str] = []

    async def listener(payload: str) -> None:
        started.append(payload)
        if len(started) == 2:
            both_started.set()
        await both_started.wait()
        finished.append(payload)

    ctx.on(FLUSH, listener)
    ctx.on(FLUSH, listener)

    await ctx.parallel(FLUSH, "x")

    assert finished == ["x", "x"]


async def test_parallel_accepts_plain_functions(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(FLUSH, log.append)

    await ctx.parallel(FLUSH, "x")

    assert log == ["x"]


async def test_parallel_logs_a_failing_listener_and_waits_for_the_rest(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    async def failing(payload: str) -> None:
        raise RuntimeError("flush failed")

    async def slow(payload: str) -> None:
        await asyncio.sleep(0)
        log.append(payload)

    ctx.on(FLUSH, failing)
    ctx.on(FLUSH, slow)

    with caplog.at_level(logging.ERROR, logger="columbina.kernel"):
        await ctx.parallel(FLUSH, "x")

    assert log == ["x"]
    assert "flush failed" in caplog.text


async def test_serial_stops_at_the_first_answer(ctx: Context) -> None:
    asked: list[str] = []

    async def undecided(payload: str) -> int | None:
        asked.append("undecided")
        await asyncio.sleep(0)
        return None

    def decided(payload: str) -> int | None:
        asked.append("decided")
        return len(payload)

    async def never_asked(payload: str) -> int | None:
        asked.append("never")
        return -1

    ctx.on(DECIDE, undecided)
    ctx.on(DECIDE, decided)
    ctx.on(DECIDE, never_asked)

    assert await ctx.serial(DECIDE, "four") == 4
    assert asked == ["undecided", "decided"]


async def test_serial_returns_none_when_nobody_answers(ctx: Context) -> None:
    assert await ctx.serial(DECIDE, "x") is None


async def test_serial_propagates_a_listener_error(ctx: Context) -> None:
    async def failing(payload: str) -> int | None:
        raise RuntimeError("cannot decide")

    ctx.on(DECIDE, failing)

    with pytest.raises(RuntimeError, match="cannot decide"):
        await ctx.serial(DECIDE, "x")


async def test_waterfall_without_listeners_runs_the_final_handler(ctx: Context) -> None:
    assert await ctx.waterfall(RENDER, "x", final) == "<x>"


async def test_waterfall_wraps_the_final_handler_outermost_first(ctx: Context) -> None:
    async def outer(payload: str, next: Next[str, str]) -> str:
        return f"outer({await next(payload)})"

    async def inner(payload: str, next: Next[str, str]) -> str:
        return f"inner({await next(payload)})"

    ctx.on(RENDER, outer)
    ctx.on(RENDER, inner)

    assert await ctx.waterfall(RENDER, "x", final) == "outer(inner(<x>))"


async def test_a_waterfall_listener_can_replace_the_payload(ctx: Context) -> None:
    async def shout(payload: str, next: Next[str, str]) -> str:
        return await next(payload.upper())

    ctx.on(RENDER, shout)

    assert await ctx.waterfall(RENDER, "x", final) == "<X>"


async def test_a_waterfall_listener_can_short_circuit(ctx: Context) -> None:
    reached: list[str] = []

    async def veto(payload: str, next: Next[str, str]) -> str:
        return "vetoed"

    async def unreachable(payload: str, next: Next[str, str]) -> str:
        reached.append(payload)
        return await next(payload)

    ctx.on(RENDER, veto)
    ctx.on(RENDER, unreachable)

    assert await ctx.waterfall(RENDER, "x", final) == "vetoed"
    assert reached == []


async def test_a_waterfall_listener_can_run_the_rest_of_the_chain_again(ctx: Context) -> None:
    attempts: list[str] = []

    async def retry(payload: str, next: Next[str, str]) -> str:
        try:
            return await next(payload)
        except RuntimeError:
            return await next(payload)

    async def flaky(payload: str, next: Next[str, str]) -> str:
        attempts.append(payload)
        if len(attempts) == 1:
            raise RuntimeError("try again")
        return await next(payload)

    ctx.on(RENDER, retry)
    ctx.on(RENDER, flaky)

    assert await ctx.waterfall(RENDER, "x", final) == "<x>"
    assert attempts == ["x", "x"]


async def test_waterfall_propagates_a_listener_error(ctx: Context) -> None:
    async def failing(payload: str, next: Next[str, str]) -> str:
        raise RuntimeError("cannot render")

    ctx.on(RENDER, failing)

    with pytest.raises(RuntimeError, match="cannot render"):
        await ctx.waterfall(RENDER, "x", final)


async def test_async_dispatch_checks_the_mode(ctx: Context) -> None:
    with pytest.raises(EventModeError, match=r"serial\(\)"):
        await ctx.parallel(DECIDE, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"waterfall\(\)"):
        await ctx.serial(RENDER, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"parallel\(\)"):
        await ctx.waterfall(FLUSH, "x", final)  # pyright: ignore[reportArgumentType]
