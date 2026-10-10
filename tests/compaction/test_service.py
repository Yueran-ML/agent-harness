"""The service that makes conversations shorter: when it does, and what it leaves in the log."""

import asyncio
import logging
from typing import Any

import pytest
from pydantic import BaseModel, ValidationError

import columbina.compaction.basic
import columbina.token_meter
from columbina.agent import (
    PRE_STEP,
    REQUEST,
    Agent,
    Enter,
    Reject,
    RequestDraft,
    StepDecision,
    StepInfo,
)
from columbina.compaction import (
    COMPACTION,
    COMPACTIONS,
    EVENTS,
    SUMMARY_SOURCE,
    Compaction,
    CompactionEndEvent,
    CompactionStartEvent,
    CompactionSummaryEvent,
    Outcome,
)
from columbina.compaction.basic import Basic, Budget, Config, Refusal, budget, refusal
from columbina.compaction.basic.summary import INSTRUCTION, SummaryError, carried
from columbina.kernel import Next
from columbina.llm import (
    LLM,
    CallConfig,
    LlmFailure,
    LlmRequest,
    TextBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import (
    USAGE,
    AssistantAttemptEvent,
    Session,
    SessionStoreError,
    ToolResultEvent,
    TurnEndEvent,
    UsageTotal,
    UserMessageEvent,
)
from columbina.token_meter import TOKEN_METER, Measure, estimate
from columbina.tools import Tool, ToolRun
from support.loop import SYSTEM, Harness, assistant, idle, types, user

WINDOW = 20_000
MODEL = CallConfig(provider="scripted", model="script", context_window=WINDOW)
"""A model that takes twenty thousand tokens: fifteen thousand may be filled, 3200 stay."""
ROOMY = MODEL.model_copy(update={"context_window": 60_000})
ASK = UserMessage(content=(TextBlock(text=INSTRUCTION),))
NO_KEY = LlmFailure(message="no such key", code="AUTH", status=401)
TOO_LONG = LlmFailure(message="too long", code="CONTEXT_WINDOW_EXCEEDED", status=400)


def refused(window: int, requested: int, messages: int) -> LlmFailure:
    completion = requested - messages
    return LlmFailure(
        message=(
            f"This model's maximum context length is {window} tokens. However, you requested "
            f"{requested} tokens ({messages} in the messages, {completion} in the completion)."
        ),
        code="CONTEXT_WINDOW_EXCEEDED",
        status=400,
    )


def words(tokens: int) -> str:
    """A text that is estimated at about so many tokens."""
    text = "word " * round(tokens / 1.11)
    assert abs(estimate(text) - tokens) <= 2
    return text


class Size(BaseModel):
    tokens: int


async def big(args: Size, run: ToolRun) -> str:
    return words(args.tokens)


BIG = Tool(name="big", description="Gives so many words.", parameters=Size, execute=big)


def wants(tokens: int) -> Reply:
    """A reply of the model that asks for so many words."""
    return Reply(calls=(Call(name="big", arguments={"tokens": tokens}),))


async def loaded(harness: Harness, **config: Any) -> Compaction:
    await harness.ctx.plugin(columbina.token_meter)
    await harness.ctx.plugin(columbina.compaction.basic, config or None)
    return harness.ctx.use(COMPACTION)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


def sums(harness: Harness) -> list[LlmRequest]:
    """The requests in which the model was asked for a summary."""
    return [request for request in harness.model.requests if request.messages[-1] == ASK]


def attempts(agent: Agent) -> list[tuple[str, Outcome | None]]:
    """Why each attempt to make the conversation shorter began, and how it ended."""
    told: list[tuple[str, Outcome | None]] = []
    for event in agent.session.events:
        if isinstance(event.data, CompactionStartEvent):
            told.append((event.data.why, None))
        if isinstance(event.data, CompactionEndEvent):
            assert told[-1] == (event.data.why, None)
            told[-1] = (event.data.why, event.data.outcome)
    return told


def measured(harness: Harness, agent: Agent) -> int:
    return harness.ctx.use(TOKEN_METER).measure(agent.session).tokens


# The numbers that it goes by


def test_how_full_a_conversation_may_get_follows_from_what_the_model_takes() -> None:
    flash = CallConfig(provider="deepseek", model="flash", context_window=1_048_576)

    assert budget(flash, Config()) == Budget(
        window=1_048_576, headroom=65_536, threshold=838_860, retain=167_772
    )
    # A smaller model: a quarter of it is less than the headroom in tokens, and what is left
    # beside it is less than four fifths.
    small = CallConfig(provider="p", model="m", context_window=100_000)
    assert budget(small, Config()) == Budget(
        window=100_000, headroom=25_000, threshold=75_000, retain=16_000
    )
    # Room that requests leave for the reply is not there for the conversation.
    replying = small.model_copy(update={"max_output_tokens": 10_000})
    assert budget(replying, Config()) == Budget(
        window=100_000, headroom=25_000, threshold=65_000, retain=14_400
    )
    assert budget(CallConfig(provider="p", model="m"), Config()) is None
    # What a service has said about its model counts for more than the config.
    assert budget(small, Config(), 50_000) == Budget(
        window=50_000, headroom=12_500, threshold=37_500, retain=8000
    )


def test_the_config_says_how_much_of_the_window_may_be_filled_and_what_stays() -> None:
    model = CallConfig(provider="p", model="m", context_window=1000)
    config = Config(threshold_ratio=0.5, headroom_tokens=100, headroom_ratio=0.2, retain_ratio=0.3)

    assert budget(model, config) == Budget(window=1000, headroom=100, threshold=500, retain=300)
    assert budget(model, Config(headroom_tokens=1, headroom_ratio=0.0001)).headroom == 1  # pyright: ignore[reportOptionalMemberAccess]


@pytest.mark.parametrize(
    "bad",
    [
        {"trim_over": 100, "trim_head": 60, "trim_tail": 40},
        {"trim_over": 1000, "trim_head": 500, "trim_tail": 373},
        {"trim_head": 8192, "trim_tail": 0},
        {"threshold_ratio": 0},
        {"threshold_ratio": 1.1},
        {"retain_ratio": 1},
        {"headroom_tokens": 0},
        {"headroom_ratio": 1},
        {"trim_over": 0},
    ],
)
def test_a_config_that_cannot_work_is_refused(bad: dict[str, Any]) -> None:
    with pytest.raises(ValidationError):
        Config(**bad)


def test_ends_that_come_to_less_than_the_whole_are_taken() -> None:
    # The line that says what was left out needs room between them: 128 characters.
    config = Config(trim_over=1000, trim_head=500, trim_tail=372)

    assert (config.auto, config.trim_head) == (True, 500)
    with pytest.raises(ValidationError, match="would be cut down again"):
        Config(trim_over=1000, trim_head=500, trim_tail=373)


@pytest.mark.parametrize(
    ("said", "read"),
    [
        (
            "This model's maximum context length is 1048576 tokens. However, you requested "
            "1070000 tokens (1069000 in the messages, 1000 in the completion). Please reduce "
            "the length of the messages or completion.",
            Refusal(window=1_048_576, requested=1_070_000, messages=1_069_000),
        ),
        (
            "This model's maximum context length is 8192 tokens. However, your messages "
            "resulted in 8205 tokens. Please reduce the length of the messages.",
            Refusal(window=8192, requested=8205, messages=8205),
        ),
        ("The request is too long.", None),
        ("This model's maximum context length is 8192 tokens.", None),
        ("You requested 9000 tokens.", None),
    ],
)
def test_what_a_service_says_when_it_refuses_is_read_for_its_numbers(
    said: str, read: Refusal | None
) -> None:
    assert refusal(said) == read


# Before a step


async def test_old_results_are_cut_down_when_that_alone_is_enough(harness: Harness) -> None:
    await loaded(harness)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(wants(12_000), wants(12_000), Reply(text="Done."), model=MODEL)

    await turn(agent, "Go.")

    # Two results of twelve thousand tokens are more than may be filled. The first has been
    # read, and with it cut down the conversation fits again. No model is asked for anything.
    (attempt,) = attempts(agent)
    assert attempt[0] == "pressure"
    outcome = attempt[1]
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (1, 0, None)
    assert outcome.before > 15_000 >= outcome.after
    assert len(harness.model.requests) == 3
    assert sums(harness) == []
    first, second = (
        message
        for message in harness.model.requests[2].messages
        if isinstance(message, ToolMessage)
    )
    assert "characters of this result were left out" in first.content[0].text
    assert second.content[0].text == words(12_000)
    # It is done before the step begins, between the result and the request that follows.
    order = types(agent.session)
    start = order.index("compaction/start")
    assert order[start - 2 : start + 4] == [
        "tool/result",
        "step/end",
        "compaction/start",
        "tool/result",
        "compaction/end",
        "step/start",
    ]
    assert COMPACTIONS.of(agent.session).count == 1


async def test_what_was_cut_before_the_log_refused_an_event_is_counted(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    await loaded(harness)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(
        Reply(calls=(Call(name="big", arguments={"tokens": 9000}),) * 2),
        wants(9000),
        Reply(text="Done."),
        model=ROOMY.model_copy(update={"context_window": 30_000}),
    )
    kept: Any = getattr(agent.session, "_store")  # noqa: B009
    store = kept.__class__
    really_write = store.write
    refused: list[int] = []

    def write(self: Any, event: Any) -> None:
        # The second of the two results that are cut down cannot be written, once.
        if isinstance(event.data, ToolResultEvent) and event.replaces:
            refused.append(event.seq)
            if len(refused) == 2:
                raise SessionStoreError("the disk is full")
        really_write(self, event)

    monkeypatch.setattr(store, "write", write)

    await turn(agent, "Go.")

    # Three results of nine thousand tokens where 22500 may be filled. Two have been read.
    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert outcome.error == "SessionStoreError: the disk is full"
    assert (outcome.trimmed, outcome.summarised) == (1, 0)
    assert outcome.changed
    assert outcome.before > outcome.after
    assert agent.session.messages[-1] == assistant("Done.")


async def test_the_oldest_part_is_summed_up_when_cutting_is_not_enough(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(text="They said words."),
        Reply(text="Three."),
        model=MODEL,
    )
    await turn(agent, words(6000))
    await turn(agent, words(6000))
    before = agent.session.messages

    await turn(agent, words(6000))

    # The request for the summary shows the model what it was shown before, and asks.
    (asked,) = sums(harness)
    assert asked.messages == (*before, ASK)
    assert asked.tools == ()
    # A summary may be as long as the room that is kept free: a quarter of this small model.
    assert asked.config == MODEL.model_copy(update={"max_output_tokens": 5000})
    # What the model is asked next is the summary and what the user has just said.
    summary = carried("They said words.")
    assert harness.model.requests[-1].messages == (SYSTEM, summary, user(words(6000)))
    assert agent.session.messages == (SYSTEM, summary, user(words(6000)), assistant("Three."))
    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (0, 4, None)
    assert outcome.before > 15_000 > outcome.after
    # The log tells it in this order, and the one message takes the place of the four.
    order = types(agent.session)
    start = order.index("compaction/start")
    assert order[start - 1 : start + 6] == [
        "user/message",
        "compaction/start",
        "request/aside",
        "compaction/summary",
        "user/message",
        "compaction/end",
        "step/start",
    ]
    told, carrier = agent.session.events[start + 2], agent.session.events[start + 3]
    assert isinstance(told.data, CompactionSummaryEvent)
    assert isinstance(carrier.data, UserMessageEvent)
    assert carrier.data.source == SUMMARY_SOURCE
    assert carrier.replaces == told.data.replaces
    assert told.data.summary == "They said words."
    assert told.data.region_tokens > 12_000 > told.data.summary_tokens > 0
    assert len(carrier.replaces) == 4


async def test_the_model_that_requests_really_go_to_is_the_one_that_counts(
    harness: Harness,
) -> None:
    # The agent was created for a model of which nothing says how much it takes. Something
    # sends its requests to another one, as a frontend does when the user names a model.
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(text="They said words."),
        Reply(text="Three."),
    )
    other = CallConfig(provider="scripted", model="other", context_window=WINDOW)

    async def elsewhere(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        await proceed(draft)
        return other

    agent.ctx.on(REQUEST, elsewhere)

    for _ in range(3):
        await turn(agent, words(6000))

    # That model takes twenty thousand tokens, so the conversation is made shorter in time,
    # and that model writes the summary.
    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert (outcome.summarised, outcome.error) == (4, None)
    (asked,) = sums(harness)
    assert asked.config == other.model_copy(update={"max_output_tokens": 5000})
    assert {request.config.model for request in harness.model.requests} == {"other"}


async def test_the_model_of_the_step_that_is_about_to_begin_is_the_one_that_counts(
    harness: Harness,
) -> None:
    # Something sends the first step of every turn to a model that takes much, and the
    # steps after it to one that takes little.
    await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."))
    much = CallConfig(provider="scripted", model="much", context_window=100_000)
    little = CallConfig(provider="scripted", model="little", context_window=10_000)
    asked: list[tuple[int, int]] = []

    async def by_step(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        await proceed(draft)
        asked.append((draft.turn, draft.step))
        return much if draft.step == 1 else little

    agent.ctx.on(REQUEST, by_step)

    await turn(agent, words(9000))
    await turn(agent, words(100))

    # Nine thousand tokens are too much for the one and little for the other. The step
    # that was about to begin was the first of its turn each time, so nothing was done.
    assert attempts(agent) == []
    assert set(asked) == {(1, 1), (2, 1)}
    assert [request.config.model for request in harness.model.requests] == ["much", "much"]


async def test_whoever_says_how_a_request_calls_its_model_is_told_which_request_it_is(
    harness: Harness,
) -> None:
    # Something may send some of the requests to another model. It goes by the turn, the
    # step and the attempt, so it is asked about the request that is about to be made.
    service = await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(failure=refused(16_000, 21_000, 19_000)),
        Reply(text="They said words."),
        Reply(text="Two."),
    )
    asked: list[tuple[int, int, int]] = []

    async def noting(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        asked.append((draft.turn, draft.step, draft.attempt))
        return await proceed(draft)

    agent.ctx.on(REQUEST, noting)

    await turn(agent, words(6000))
    # It is asked before the step, and then for the request itself.
    assert asked == [(1, 1, 1), (1, 1, 1)]

    del asked[:]
    await turn(agent, words(6000))
    # What is sent again after the model refused is the second attempt of its step.
    assert asked == [(2, 1, 1), (2, 1, 1), (2, 1, 2), (2, 1, 2)]

    del asked[:]
    harness.model.add(Reply(text="They said less."))
    await service.compact(agent, "manual")
    # Between two turns, the request to come is the first of the turn that follows.
    assert asked == [(3, 1, 1)]


async def test_a_model_without_a_window_in_its_config_is_not_watched(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), Reply(text="Three."))

    for _ in range(3):
        await turn(agent, words(30_000))

    assert attempts(agent) == []
    assert len(harness.model.requests) == 3
    # It is said once, not before every step.
    (record,) = (r for r in caplog.records if r.name == "columbina.compaction.basic")
    assert record.levelno == logging.WARNING
    assert "the model script has no context window" in record.getMessage()


async def test_a_step_that_something_keeps_from_beginning_makes_nothing_shorter(
    harness: Harness,
) -> None:
    refusing: list[bool] = []

    async def spent(info: StepInfo, proceed: Next[StepInfo, StepDecision]) -> StepDecision:
        # It lets the others have their say first, as a listener may.
        decision = await proceed(info)
        return Reject(reason="the budget is spent") if refusing else decision

    # It was there before the service was loaded, and the service still waits for it.
    harness.ctx.on(PRE_STEP, spent)
    await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="never"), model=MODEL)
    await turn(agent, words(14_000))

    refusing.append(True)
    await turn(agent, words(6000))

    assert measured(harness, agent) > 15_000
    assert attempts(agent) == []
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=2, reason="blocked", detail="the budget is spent"
    )


async def test_what_is_left_is_cut_down_if_the_summary_was_not_enough(harness: Harness) -> None:
    # A model of sixty thousand tokens of which four tenths may be filled: twenty-four
    # thousand. What the user said last stays with all that followed it, and that alone is
    # more: two results of 12500 tokens.
    await loaded(harness, threshold_ratio=0.4)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(
        Reply(text="One."),
        wants(12_500),
        wants(12_500),
        Reply(text="They said words."),
        Reply(text="Done."),
        model=ROOMY,
    )
    await turn(agent, words(10_500))

    await turn(agent, "Go.")

    # Before the third step: cutting the result that was read leaves too much, so the first
    # exchange is summed up. That leaves too much as well, so the result is cut after all.
    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (1, 2, None)
    assert outcome.before > 24_000 > outcome.after
    order = types(agent.session)
    start = order.index("compaction/start")
    assert order[start : start + 6] == [
        "compaction/start",
        "request/aside",
        "compaction/summary",
        "user/message",
        "tool/result",
        "compaction/end",
    ]
    # What was summed up is the first exchange. The results belong to what stays.
    (asked,) = sums(harness)
    assert asked.messages == (SYSTEM, user(words(10_500)), assistant("One."), ASK)


async def test_a_summary_that_fails_leaves_the_results_cut_down(harness: Harness) -> None:
    await loaded(harness, threshold_ratio=0.4)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(
        Reply(text="One."),
        wants(12_500),
        wants(12_500),
        Reply(failure=NO_KEY),
        Reply(text="Done."),
        model=ROOMY,
    )
    await turn(agent, words(10_500))

    await turn(agent, "Go.")

    # Cutting the result that was read leaves too much, so a summary is asked for, and the
    # request fails. The result is cut all the same: it is what can be done.
    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (1, 0, "AUTH: no such key")
    assert outcome.before > outcome.after > 24_000
    assert agent.session.events[-1].data == TurnEndEvent(turn=2, reason="completed")
    assert not any(isinstance(event.data, CompactionSummaryEvent) for event in agent.session.events)


async def test_after_two_failures_in_a_row_it_is_not_tried_again_until_one_succeeds(
    harness: Harness,
) -> None:
    service = await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(failure=NO_KEY),
        Reply(text="Three."),
        Reply(failure=NO_KEY),
        Reply(text="Four."),
        Reply(text="Five."),
        model=MODEL,
    )
    for _ in range(3):
        await turn(agent, words(6000))
    await turn(agent, "And?")

    await turn(agent, "And?")

    # Each of the two failures left the turn to go on. The fifth turn tries nothing.
    assert [(why, o.error) for why, o in attempts(agent) if o is not None] == [
        ("pressure", "AUTH: no such key"),
        ("pressure", "AUTH: no such key"),
    ]
    assert harness.model.remaining == 0
    assert agent.session.messages[-1] == assistant("Five.")
    # The user asks for it, and it works. From then on it is tried again when it is time.
    harness.model.add(
        Reply(text="They said words."), Reply(text="They said more."), Reply(text="Six.")
    )
    assert (await service.compact(agent, "manual")).error is None
    await turn(agent, words(14_000))

    assert [why for why, _ in attempts(agent)] == ["pressure", "pressure", "manual", "pressure"]
    assert attempts(agent)[-1][1] == COMPACTIONS.of(agent.session).last
    assert agent.session.messages[-1] == assistant("Six.")


async def test_a_conversation_of_which_nothing_can_go_is_left_alone_after_two_tries(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."), Reply(text="Two."), Reply(text="Three."), model=MODEL
    )

    await turn(agent, words(17_000))
    await turn(agent, "And?")
    await turn(agent, "And?")

    # One message is nearly all there is, and it is what the model is asked about.
    assert [(why, o.error) for why, o in attempts(agent) if o is not None] == [
        ("pressure", "nothing of this conversation can go"),
        ("pressure", "nothing of this conversation can go"),
    ]
    assert sums(harness) == []
    assert agent.session.messages[-1] == assistant("Three.")


async def test_a_turn_that_is_cancelled_while_a_summary_is_written_says_so_in_the_log(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(text="They", hold=True),
        Reply(text="Three."),
        model=MODEL,
    )
    await turn(agent, words(6000))
    await turn(agent, words(6000))
    agent.followup(words(6000))
    await asyncio.wait_for(harness.model.held(), timeout=5)
    seen = agent.session.messages

    agent.cancel()
    await idle(agent)

    ((why, outcome),) = attempts(agent)
    assert why == "pressure"
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (0, 0, "cancelled")
    assert agent.session.messages == seen
    assert agent.session.events[-1].data == TurnEndEvent(turn=3, reason="aborted", detail="user")
    # Being stopped by the user is no failure of the service: the next time, it tries again.
    harness.model.release()
    harness.model.add(Reply(text="They said words."), Reply(text="Four."))
    await turn(agent, "And?")
    assert [why for why, _ in attempts(agent)] == ["pressure", "pressure"]
    assert attempts(agent)[-1][1].error is None  # pyright: ignore[reportOptionalMemberAccess]


async def test_being_stopped_by_the_user_is_no_failure_of_the_service(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."), Reply(text="Two."), Reply(text="They", hold=True), model=MODEL
    )
    await turn(agent, words(6000))
    await turn(agent, words(6000))
    agent.followup(words(6000))
    await asyncio.wait_for(harness.model.held(), timeout=5)
    agent.cancel()
    await idle(agent)
    harness.model.release()
    harness.model.add(
        Reply(failure=NO_KEY),
        Reply(text="Three."),
        Reply(text="They said words."),
        Reply(text="Four."),
    )

    await turn(agent, "And?")
    await turn(agent, "And?")

    # One that was stopped and one that failed are not two failures in a row.
    assert [(why, o.error) for why, o in attempts(agent) if o is not None] == [
        ("pressure", "cancelled"),
        ("pressure", "AUTH: no such key"),
        ("pressure", None),
    ]


async def test_a_fault_of_the_service_does_not_end_the_turn(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await loaded(harness)

    async def broken(*given: Any, **more: Any) -> Any:
        raise RuntimeError("the pencil broke")

    monkeypatch.setattr(columbina.compaction.basic, "summarise", broken)
    agent = await harness.agent(
        Reply(text="One."), Reply(text="Two."), Reply(text="Three."), model=MODEL
    )

    for _ in range(3):
        await turn(agent, words(6000))

    ((_, outcome),) = attempts(agent)
    assert outcome is not None
    assert outcome.error == "RuntimeError: the pencil broke"
    assert not outcome.changed
    assert agent.session.messages[-1] == assistant("Three.")
    assert any(
        record.exc_info is not None and "shorter failed" in record.getMessage()
        for record in caplog.records
    )


class Fixed:
    """An estimate that says what a test wants it to say."""

    def __init__(self, tokens: int) -> None:
        self.tokens = tokens

    def measure(self, session: Session) -> Measure:
        return Measure(tokens=self.tokens, estimated=self.tokens, rate=None)


@pytest.mark.parametrize(("tokens", "tried"), [(15_000, 0), (15_001, 1)])
async def test_a_conversation_that_fills_just_what_may_be_filled_is_left_alone(
    harness: Harness, tokens: int, tried: int
) -> None:
    harness.sessions.declare(harness.ctx, *EVENTS)
    agent = await harness.agent(model=MODEL)
    service = Basic(Config(), harness.ctx.use(LLM), Fixed(tokens))  # pyright: ignore[reportArgumentType]

    async def enter(info: StepInfo) -> StepDecision:
        return Enter()

    assert await service.before_a_step(StepInfo(agent, 1, 1), enter) == Enter()

    assert len(attempts(agent)) == tried


async def test_a_window_that_leaves_no_room_for_a_conversation_is_not_watched(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    await loaded(harness)
    # Requests ask for more room for the reply than the model takes in all.
    model = CallConfig(
        provider="scripted", model="script", context_window=1000, max_output_tokens=2000
    )
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=model)

    await turn(agent, words(900))
    await turn(agent, words(900))

    assert attempts(agent) == []
    (record,) = (r for r in caplog.records if r.name == "columbina.compaction.basic")
    assert "no context window that leaves room" in record.getMessage()


# After the model has refused a request


async def test_a_conversation_that_the_model_refuses_as_too_long_is_made_shorter_and_sent_again(
    harness: Harness,
) -> None:
    # Nothing in the config says how much the model takes, so nothing was done before.
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(failure=refused(16_000, 21_000, 19_000)),
        Reply(text="They said words."),
        Reply(text="Three."),
    )
    await turn(agent, words(6000))
    await turn(agent, words(6000))

    await turn(agent, words(6000))

    ((why, outcome),) = attempts(agent)
    assert why == "overflow"
    assert outcome is not None
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (0, 4, None)
    summary = carried("They said words.")
    assert harness.model.requests[-1].messages == (SYSTEM, summary, user(words(6000)))
    assert agent.session.events[-1].data == TurnEndEvent(turn=3, reason="completed")
    # The room for the summary is a quarter of what the service said its model takes.
    (asked,) = sums(harness)
    assert asked.config.max_output_tokens == 4000
    # The attempt that failed is in the log before it.
    order = types(agent.session)
    assert order.index("assistant/attempt") < order.index("compaction/start")
    assert sum(isinstance(e.data, AssistantAttemptEvent) for e in agent.session.events) == 1


@pytest.mark.parametrize(
    ("failure", "summed_up"),
    [
        # Of sixty thousand tokens, 9600 stay: nine of the twelve turns, as the service counts.
        (TOO_LONG, 4),
        # The service says that the request was four thousand too many, most of them room for
        # the reply. That and the headroom is more than the whole conversation, so all of it
        # goes but what the model is asked about.
        (refused(60_000, 64_000, 13_000), 24),
        # The service counts twice what was estimated, and its model takes a third of what
        # the config says. Of twenty thousand, 3200 stay: two turns as the service counts.
        (refused(20_000, 26_000, 24_200), 20),
    ],
)
async def test_as_much_goes_as_the_service_said_was_too_much(
    harness: Harness, failure: LlmFailure, summed_up: int
) -> None:
    await loaded(harness)
    agent = await harness.agent(*(Reply(text=f"{number}.") for number in range(12)), model=ROOMY)
    for _ in range(12):
        await turn(agent, words(1000))
    harness.model.add(Reply(failure=failure), Reply(text="They said words."), Reply(text="Done."))

    await turn(agent, "And?")

    ((why, outcome),) = attempts(agent)
    assert why == "overflow"
    assert outcome is not None
    assert (outcome.summarised, outcome.error) == (summed_up, None)
    assert agent.session.messages[-2:] == (user("And?"), assistant("Done."))


async def test_a_refusal_that_gives_no_numbers_is_followed_by_a_summary_all_the_same(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(failure=TOO_LONG),
        Reply(text="They said words."),
        Reply(text="Three."),
        model=MODEL,
    )
    await turn(agent, words(4000))
    await turn(agent, words(4000))

    await turn(agent, words(4000))

    ((why, outcome),) = attempts(agent)
    assert why == "overflow"
    assert outcome is not None
    assert (outcome.summarised, outcome.error) == (4, None)
    assert agent.session.messages[-1] == assistant("Three.")


async def test_a_request_is_made_again_once_in_a_step(harness: Harness) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(failure=TOO_LONG),
        Reply(text="They said words."),
        Reply(failure=TOO_LONG),
        Reply(text="never"),
        model=MODEL,
    )
    await turn(agent, words(4000))
    await turn(agent, words(4000))

    await turn(agent, words(4000))

    assert len(attempts(agent)) == 1
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=3, reason="error", detail="CONTEXT_WINDOW_EXCEEDED: too long"
    )
    assert harness.model.remaining == 1


async def test_a_request_is_not_made_again_if_the_model_would_see_the_same(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(Reply(failure=TOO_LONG), Reply(text="never"), model=MODEL)

    await turn(agent, words(4000))

    ((why, outcome),) = attempts(agent)
    assert why == "overflow"
    assert outcome is not None
    assert outcome.error == "nothing of this conversation can go"
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=1, reason="error", detail="CONTEXT_WINDOW_EXCEEDED: too long"
    )
    assert harness.model.remaining == 1


async def test_no_other_failure_of_a_request_makes_a_conversation_shorter(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."), Reply(text="Two."), Reply(failure=NO_KEY), model=MODEL
    )
    await turn(agent, words(4000))
    await turn(agent, words(4000))

    await turn(agent, words(4000))

    assert attempts(agent) == []
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=3, reason="error", detail="AUTH: no such key"
    )


async def test_a_refusal_is_followed_by_an_attempt_also_after_two_failures(
    harness: Harness,
) -> None:
    await loaded(harness)
    agent = await harness.agent(
        Reply(text="One."),
        Reply(text="Two."),
        Reply(failure=NO_KEY),
        Reply(text="Three."),
        Reply(failure=NO_KEY),
        Reply(failure=TOO_LONG),
        Reply(text="They said words."),
        Reply(text="Four."),
        model=MODEL,
    )
    for _ in range(3):
        await turn(agent, words(6000))

    await turn(agent, "And?")

    assert [(why, o.error) for why, o in attempts(agent) if o is not None] == [
        ("pressure", "AUTH: no such key"),
        ("pressure", "AUTH: no such key"),
        ("overflow", None),
    ]
    assert agent.session.messages[-1] == assistant("Four.")


# When it is asked for


async def test_the_user_can_have_a_conversation_summed_up_at_any_time(harness: Harness) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."))

    outcome = await service.compact(agent, "manual")

    # Far from full, and summed up because it was asked for. What the user said last stays.
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (0, 2, None)
    assert outcome.before > outcome.after
    assert outcome.before < 15_000
    assert agent.session.messages == (
        SYSTEM,
        carried("They said words."),
        user(words(4000)),
        assistant("Two."),
    )
    assert attempts(agent) == [("manual", outcome)]
    assert COMPACTIONS.of(agent.session).last == outcome


async def test_a_summary_that_was_asked_for_leaves_what_stays_as_it_is(harness: Harness) -> None:
    service = await loaded(harness)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(Reply(text="One."), wants(9000), Reply(text="Done."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, "Go.")
    harness.model.add(Reply(text="They said words."))

    outcome = await service.compact(agent, "manual")

    # The result has been read and is long, and it stays whole: a summary was asked for,
    # and no number says that more has to go.
    assert (outcome.trimmed, outcome.summarised, outcome.error) == (0, 2, None)
    (result,) = (m for m in agent.session.messages if isinstance(m, ToolMessage))
    assert result.content[0].text == words(9000)


async def test_attempts_that_the_user_asked_for_do_not_count_as_failures_in_a_row(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))

    async def too_long(*given: Any, **more: Any) -> Any:
        raise SummaryError("the summary is no shorter than what it sums up")

    with monkeypatch.context() as patched:
        patched.setattr(columbina.compaction.basic, "summarise", too_long)
        for _ in range(2):
            assert (await service.compact(agent, "manual")).error is not None
    harness.model.add(Reply(text="They said words."), Reply(text="Three."))

    await turn(agent, words(8000))

    assert [(why, o.error) for why, o in attempts(agent) if o is not None][-1] == ("pressure", None)


async def blank(args: Size, run: ToolRun) -> str:
    return " " * args.tokens


async def test_a_result_that_would_cost_no_less_is_not_cut_down(harness: Harness) -> None:
    # A result of nothing but spaces is long and cheap. The line that would take its place
    # says how many characters were left out, and that line costs more than they did.
    service = await loaded(harness, trim_over=128, trim_head=0, trim_tail=0)
    line = "[... 129 characters of this result were left out to shorten the conversation ...]"
    assert estimate(line) > estimate(" " * 129)
    harness.tools.add(
        harness.ctx,
        Tool(name="blank", description="Gives spaces.", parameters=Size, execute=blank),
    )
    agent = await harness.agent(
        Reply(calls=(Call(name="blank", arguments={"tokens": 129}),)),
        Reply(text="Done."),
        model=MODEL,
    )
    await turn(agent, "Go.")
    seen = agent.session.messages

    outcome = await service.compact(agent, "manual")

    # Shorter in characters it would be. Nothing is gained by it, so nothing is done.
    assert (outcome.trimmed, outcome.changed, outcome.error) == (0, False, None)
    assert agent.session.messages == seen


async def test_what_a_summary_cost_is_in_what_the_session_cost(harness: Harness) -> None:
    service = await loaded(harness)
    agent = await harness.agent(
        Reply(text="One.", usage=Usage(input_tokens=10, output_tokens=1)),
        Reply(text="Two.", usage=Usage(input_tokens=20, output_tokens=1)),
        model=MODEL,
    )
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(
        Reply(text="They said words.", usage=Usage(input_tokens=1000, output_tokens=100))
    )

    await service.compact(agent, "manual")

    assert USAGE.of(agent.session) == UsageTotal(
        calls=3, usage=Usage(input_tokens=1030, output_tokens=102)
    )


async def test_asking_for_it_where_nothing_can_go_is_no_failure(harness: Harness) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), model=MODEL)
    await turn(agent, "Say one.")

    outcome = await service.compact(agent, "manual")

    assert not outcome.changed
    assert outcome.error is None
    assert outcome.before == outcome.after
    assert sums(harness) == []


async def test_a_summary_can_be_asked_for_where_nothing_says_how_much_the_model_takes(
    harness: Harness,
) -> None:
    service = await loaded(harness, auto=False)
    agent = await harness.agent(*(Reply(text=f"{number}.") for number in range(4)))
    for _ in range(4):
        await turn(agent, words(2000))
    harness.model.add(Reply(text="They said words."))

    outcome = await service.compact(agent, "manual")

    # The conversation is its own measure then: about a sixth of it stays, which is the last
    # of the four turns. A quarter of it is the room for the summary.
    assert (outcome.summarised, outcome.error) == (6, None)
    (asked,) = sums(harness)
    assert asked.config.max_output_tokens == outcome.before // 4


@pytest.mark.parametrize(
    ("replies_may_have", "summary_may_have", "said"), [(700, 700, 4000), (9000, 5000, 2000)]
)
async def test_a_summary_is_no_longer_than_a_reply_may_be_nor_than_the_headroom(
    harness: Harness, replies_may_have: int, summary_may_have: int, said: int
) -> None:
    # (What is said is of a size at which nothing happens by itself: the room for the reply
    # is taken from what may be filled.)
    service = await loaded(harness)
    model = MODEL.model_copy(update={"max_output_tokens": replies_may_have})
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=model)
    await turn(agent, words(said))
    await turn(agent, words(said))
    assert attempts(agent) == []
    harness.model.add(Reply(text="They said words."))

    outcome = await service.compact(agent, "manual")

    assert (outcome.summarised, outcome.error) == (2, None)
    (asked,) = sums(harness)
    assert asked.config.max_output_tokens == summary_may_have


async def test_the_summary_is_asked_for_with_the_tools_that_were_declared(
    harness: Harness,
) -> None:
    service = await loaded(harness)
    harness.tools.add(harness.ctx, BIG)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."))

    await service.compact(agent, "manual")

    (asked,) = sums(harness)
    assert asked.tools == harness.model.requests[0].tools != ()


async def test_without_being_told_to_it_does_nothing_by_itself(harness: Harness) -> None:
    await loaded(harness, auto=False)
    agent = await harness.agent(
        Reply(text="One."), Reply(text="Two."), Reply(failure=TOO_LONG), model=MODEL
    )
    await turn(agent, words(9000))
    await turn(agent, words(9000))

    await turn(agent, "And?")

    assert attempts(agent) == []
    assert agent.session.events[-1].data == TurnEndEvent(
        turn=3, reason="error", detail="CONTEXT_WINDOW_EXCEEDED: too long"
    )


async def test_a_conversation_is_made_shorter_by_one_attempt_at_a_time(harness: Harness) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words.", hold=True))
    first = asyncio.ensure_future(service.compact(agent, "manual"))
    await asyncio.wait_for(harness.model.held(), timeout=5)
    count = len(agent.session)

    second = await service.compact(agent, "manual")

    assert second.error == "this conversation is being made shorter already"
    assert not second.changed
    assert len(agent.session) == count
    harness.model.release()
    assert (await first).summarised == 2
    # When it is over, another can begin.
    assert (await service.compact(agent, "manual")).error is None


async def test_two_that_ask_at_the_same_moment_are_one_attempt_and_one_refusal(
    harness: Harness,
) -> None:
    # Finding out which model the next request goes to takes a moment, if a listener of
    # requests takes one. Whoever asks second in that moment is told that it is being done.
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."), Reply(text="They said words again."))
    reached = asyncio.Event()
    go_on = asyncio.Event()

    async def slow(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        config = await proceed(draft)
        reached.set()
        await go_on.wait()
        return config

    agent.ctx.on(REQUEST, slow)
    both = [asyncio.ensure_future(service.compact(agent, "manual")) for _ in range(2)]
    await asyncio.wait_for(reached.wait(), timeout=5)
    for _ in range(5):
        await asyncio.sleep(0)

    go_on.set()
    first, second = await asyncio.wait_for(asyncio.gather(*both), timeout=5)

    assert (first.summarised, first.error) == (2, None)
    assert second.error == "this conversation is being made shorter already"
    assert not second.changed
    assert len(sums(harness)) == 1
    assert [why for why, _ in attempts(agent)] == ["manual"]


async def test_an_attempt_that_could_not_begin_is_in_nobodys_way(harness: Harness) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    harness.model.add(Reply(text="They said words."))
    count = len(agent.session)
    broken = [True]

    async def once(draft: RequestDraft, proceed: Next[RequestDraft, CallConfig]) -> CallConfig:
        if broken.pop() if broken else False:
            raise RuntimeError("nobody knows which model")
        return await proceed(draft)

    agent.ctx.on(REQUEST, once)

    with pytest.raises(RuntimeError, match="nobody knows which model"):
        await service.compact(agent, "manual")

    # Nothing was written, and the next one that asks is not told that it is being done.
    assert len(agent.session) == count
    assert (await service.compact(agent, "manual")).summarised == 2


async def test_a_start_that_an_earlier_program_left_without_an_end_is_in_nobodys_way(
    harness: Harness,
) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))
    # The program ended while a summary was being written.
    agent.session.append(CompactionStartEvent(why="pressure", tokens=9000))
    harness.model.add(Reply(text="They said words."))

    outcome = await service.compact(agent, "manual")

    assert (outcome.summarised, outcome.error) == (2, None)
    said = COMPACTIONS.of(agent.session)
    assert (said.count, said.open, said.orphans) == (1, False, 1)


async def test_a_summary_whose_place_is_gone_leaves_the_conversation_as_it_was(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    service = await loaded(harness)
    agent = await harness.agent(Reply(text="One."), Reply(text="Two."), model=MODEL)
    await turn(agent, words(4000))
    await turn(agent, words(4000))

    async def too_long(*given: Any, **more: Any) -> Any:
        raise SummaryError("the summary is no shorter than what it sums up")

    monkeypatch.setattr(columbina.compaction.basic, "summarise", too_long)
    seen = agent.session.messages

    outcome = await service.compact(agent, "manual")

    assert outcome.error == "the summary is no shorter than what it sums up"
    assert not outcome.changed
    assert agent.session.messages == seen
    assert not any(isinstance(e.data, ToolResultEvent) for e in agent.session.events)
