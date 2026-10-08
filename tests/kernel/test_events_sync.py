import logging

import pytest

from tack.kernel import BailEvent, Context, EmitEvent, EventModeError

PING = EmitEvent[str]("test/ping")
ASK = BailEvent[str, int]("test/ask")


async def test_emit_calls_listeners_in_registration_order(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(PING, lambda payload: log.append(f"a:{payload}"))
    ctx.on(PING, lambda payload: log.append(f"b:{payload}"))

    ctx.emit(PING, "x")

    assert log == ["a:x", "b:x"]


async def test_prepend_puts_a_listener_first(ctx: Context) -> None:
    log: list[str] = []
    ctx.on(PING, lambda payload: log.append("a"))
    ctx.on(PING, lambda payload: log.append("b"), prepend=True)

    ctx.emit(PING, "x")

    assert log == ["b", "a"]


async def test_emit_without_listeners_does_nothing(ctx: Context) -> None:
    ctx.emit(PING, "x")


async def test_a_handle_removes_its_listener(ctx: Context) -> None:
    log: list[str] = []
    handle = ctx.on(PING, log.append)
    ctx.on(PING, lambda payload: log.append("kept"))

    await handle.dispose()
    ctx.emit(PING, "x")

    assert log == ["kept"]


async def test_listeners_are_removed_when_their_fiber_is_disposed() -> None:
    ctx = Context()
    log: list[str] = []
    ctx.on(PING, log.append)

    await ctx.fiber.dispose()
    ctx.emit(PING, "x")

    assert log == []


async def test_emit_logs_a_failing_listener_and_continues(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    log: list[str] = []

    def failing(payload: str) -> None:
        raise RuntimeError(f"cannot handle {payload}")

    ctx.on(PING, failing)
    ctx.on(PING, log.append)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        ctx.emit(PING, "x")

    assert log == ["x"]
    assert "cannot handle x" in caplog.text
    assert "test/ping" in caplog.text


async def test_a_listener_added_during_dispatch_waits_for_the_next_one(ctx: Context) -> None:
    log: list[str] = []

    def first(payload: str) -> None:
        log.append(f"first:{payload}")
        if payload == "one":
            ctx.on(PING, lambda late: log.append(f"late:{late}"))

    ctx.on(PING, first)

    ctx.emit(PING, "one")
    ctx.emit(PING, "two")

    assert log == ["first:one", "first:two", "late:two"]


async def test_bail_returns_the_first_answer(ctx: Context) -> None:
    asked: list[str] = []

    def undecided(payload: str) -> int | None:
        asked.append("undecided")
        return None

    def decided(payload: str) -> int | None:
        asked.append("decided")
        return len(payload)

    def never_asked(payload: str) -> int | None:
        asked.append("never")
        return -1

    ctx.on(ASK, undecided)
    ctx.on(ASK, decided)
    ctx.on(ASK, never_asked)

    assert ctx.bail(ASK, "four") == 4
    assert asked == ["undecided", "decided"]


async def test_bail_treats_zero_and_false_as_answers(ctx: Context) -> None:
    ctx.on(ASK, lambda payload: 0)
    ctx.on(ASK, lambda payload: 7)

    assert ctx.bail(ASK, "x") == 0


async def test_bail_returns_none_when_nobody_answers(ctx: Context) -> None:
    ctx.on(ASK, lambda payload: None)

    assert ctx.bail(ASK, "x") is None


async def test_bail_propagates_a_listener_error(ctx: Context) -> None:
    def failing(payload: str) -> int | None:
        raise RuntimeError("no answer")

    ctx.on(ASK, failing)

    with pytest.raises(RuntimeError, match="no answer"):
        ctx.bail(ASK, "x")


async def test_dispatching_with_the_wrong_mode_is_an_error(ctx: Context) -> None:
    with pytest.raises(EventModeError, match=r"bail\(\)"):
        ctx.emit(ASK, "x")  # pyright: ignore[reportArgumentType]

    with pytest.raises(EventModeError, match=r"emit\(\)"):
        ctx.bail(PING, "x")  # pyright: ignore[reportArgumentType]


async def test_one_name_cannot_be_declared_as_two_kinds(ctx: Context) -> None:
    ctx.on(EmitEvent[str]("test/twice"), lambda payload: None)

    with pytest.raises(EventModeError, match="already declared as EmitEvent"):
        ctx.bail(BailEvent[str, int]("test/twice"), "x")


async def a_coroutine(payload: str) -> int:
    return len(payload)


class AsyncCallable:
    async def __call__(self, payload: str) -> int:
        return len(payload)


@pytest.mark.parametrize("listener", [a_coroutine, AsyncCallable()], ids=["function", "object"])
async def test_events_that_do_not_await_reject_async_listeners(
    ctx: Context, listener: object
) -> None:
    with pytest.raises(EventModeError, match="plain functions"):
        ctx.on(PING, listener)  # pyright: ignore[reportArgumentType, reportCallIssue]

    with pytest.raises(EventModeError, match="plain functions"):
        ctx.on(ASK, listener)  # pyright: ignore[reportArgumentType, reportCallIssue]


async def test_emit_closes_a_coroutine_returned_by_a_plain_listener(
    ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    def sneaky(payload: str) -> None:
        return a_coroutine(payload)  # pyright: ignore[reportReturnType]

    ctx.on(PING, sneaky)

    with caplog.at_level(logging.ERROR, logger="tack.kernel"):
        ctx.emit(PING, "x")

    assert "returned a coroutine" in caplog.text


async def test_bail_refuses_a_coroutine_as_an_answer(ctx: Context) -> None:
    def sneaky(payload: str) -> int | None:
        return a_coroutine(payload)  # pyright: ignore[reportReturnType]

    ctx.on(ASK, sneaky)
    ctx.on(ASK, lambda payload: 7)

    with pytest.raises(EventModeError, match="returned a coroutine"):
        ctx.bail(ASK, "four")
