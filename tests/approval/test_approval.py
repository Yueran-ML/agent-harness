"""Approval: questions about actions, their answers, and the policy of a session."""

import asyncio
import logging
from pathlib import Path
from typing import Any

import pytest
from pydantic import ValidationError

import columbina.approval
import columbina.session
from columbina.approval import (
    APPROVAL,
    APPROVAL_ASK,
    Approval,
    ApprovalAskedEvent,
    ApprovalDecidedEvent,
    ApprovalPolicyEvent,
    ApprovalRequest,
    Outcome,
    Policy,
)
from columbina.kernel import Context
from columbina.session import SESSIONS, Session, Sessions

OUTCOMES: tuple[Outcome, ...] = ("allowed-once", "rejected", "cancelled", "unavailable")


async def program(ctx: Context, tmp_path: Path, **config: str) -> tuple[Sessions, Approval]:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.approval, config)
    return ctx.use(SESSIONS), ctx.use(APPROVAL)


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    return directory


@pytest.fixture
async def approval(ctx: Context, tmp_path: Path) -> Approval:
    return (await program(ctx, tmp_path))[1]


@pytest.fixture
def session(ctx: Context, approval: Approval, work: Path) -> Session:
    return ctx.use(SESSIONS).create(ctx, work)


def question(session: Session, call_id: str = "call-1") -> ApprovalRequest:
    return ApprovalRequest(
        session=session,
        call_id=call_id,
        tool="powershell",
        summary="powershell: git push",
        reason="No rule allows this command.",
    )


def written(session: Session) -> list[tuple[str, ...]]:
    """What the log says about questions and policies, in its order."""
    said: list[tuple[str, ...]] = []
    for event in session.events:
        data = event.data
        if isinstance(data, ApprovalAskedEvent):
            said.append(("asked", data.call_id, data.tool, data.summary))
        elif isinstance(data, ApprovalDecidedEvent):
            said.append(("decided", data.call_id, data.outcome))
        elif isinstance(data, ApprovalPolicyEvent):
            said.append(("policy", data.policy))
    return said


def answering(ctx: Context, answer: Any) -> list[ApprovalRequest]:
    """Have every question answered with ``answer``, and write down the questions."""
    asked: list[ApprovalRequest] = []

    def responder(request: ApprovalRequest) -> Any:
        asked.append(request)
        return answer

    ctx.on(APPROVAL_ASK, responder)
    return asked


# Questions and their answers


@pytest.mark.parametrize("outcome", OUTCOMES)
async def test_the_answer_of_whoever_was_asked_is_the_outcome(
    ctx: Context, approval: Approval, session: Session, outcome: Outcome
) -> None:
    asked = answering(ctx, outcome)
    request = question(session)

    assert await approval.ask(ctx, request) == outcome
    assert asked == [request]


async def test_a_question_and_its_answer_are_written_into_the_log(
    ctx: Context, approval: Approval, session: Session
) -> None:
    answering(ctx, "allowed-once")

    await approval.ask(ctx, question(session, "call-1"))
    await approval.ask(ctx, question(session, "call-2"))

    assert written(session) == [
        ("asked", "call-1", "powershell", "powershell: git push"),
        ("decided", "call-1", "allowed-once"),
        ("asked", "call-2", "powershell", "powershell: git push"),
        ("decided", "call-2", "allowed-once"),
    ]


async def test_the_question_is_in_the_log_before_anybody_is_asked(
    ctx: Context, approval: Approval, session: Session
) -> None:
    seen: list[list[tuple[str, ...]]] = []

    def responder(request: ApprovalRequest) -> Outcome:
        seen.append(written(request.session))
        return "rejected"

    ctx.on(APPROVAL_ASK, responder)

    await approval.ask(ctx, question(session))

    assert seen == [[("asked", "call-1", "powershell", "powershell: git push")]]


async def test_whoever_answers_may_take_its_time(
    ctx: Context, approval: Approval, session: Session
) -> None:
    async def responder(request: ApprovalRequest) -> Outcome:
        await asyncio.sleep(0)
        return "allowed-once"

    ctx.on(APPROVAL_ASK, responder)

    assert await approval.ask(ctx, question(session)) == "allowed-once"


async def test_a_listener_that_says_nothing_leaves_the_question_to_the_next(
    ctx: Context, approval: Approval, session: Session
) -> None:
    answering(ctx, None)
    answering(ctx, "rejected")
    last = answering(ctx, "allowed-once")

    assert await approval.ask(ctx, question(session)) == "rejected"
    assert last == []


async def test_without_anybody_to_ask_nobody_is_available(
    ctx: Context, approval: Approval, session: Session
) -> None:
    assert await approval.ask(ctx, question(session)) == "unavailable"
    assert written(session)[-1] == ("decided", "call-1", "unavailable")


async def test_listeners_that_all_say_nothing_are_nobody(
    ctx: Context, approval: Approval, session: Session
) -> None:
    answering(ctx, None)

    assert await approval.ask(ctx, question(session)) == "unavailable"


async def test_a_listener_that_fails_is_nobody_and_the_failure_is_logged(
    ctx: Context, approval: Approval, session: Session, caplog: pytest.LogCaptureFixture
) -> None:
    def responder(request: ApprovalRequest) -> Outcome:
        raise RuntimeError("the window is gone")

    ctx.on(APPROVAL_ASK, responder)

    with caplog.at_level(logging.ERROR, logger="columbina.approval"):
        outcome = await approval.ask(ctx, question(session))

    assert outcome == "unavailable"
    assert written(session)[-1] == ("decided", "call-1", "unavailable")
    assert "a listener of 'approval/ask' failed" in caplog.text
    assert "the window is gone" in caplog.text


class Agreeable:
    """Something that says it is equal to whatever it is compared with."""

    def __eq__(self, other: object) -> bool:
        return True

    def __hash__(self) -> int:
        return 0


@pytest.mark.parametrize(
    "answer",
    ["yes", "allowed", "ALLOWED-ONCE", "allowed-once ", "", True, 1, ["allowed-once"], Agreeable()],
)
async def test_an_answer_that_is_none_of_the_four_allows_nothing(
    ctx: Context,
    approval: Approval,
    session: Session,
    caplog: pytest.LogCaptureFixture,
    answer: object,
) -> None:
    answering(ctx, answer)

    with caplog.at_level(logging.ERROR, logger="columbina.approval"):
        outcome = await approval.ask(ctx, question(session))

    assert isinstance(outcome, str)
    assert outcome == "unavailable"
    assert written(session)[-1] == ("decided", "call-1", "unavailable")
    assert "which is no answer" in caplog.text


async def test_a_caller_cancelled_while_it_waits_is_cancelled_and_the_log_says_so(
    ctx: Context, approval: Approval, session: Session
) -> None:
    reached = asyncio.Event()

    async def responder(request: ApprovalRequest) -> Outcome:
        reached.set()
        await asyncio.Event().wait()
        return "allowed-once"

    ctx.on(APPROVAL_ASK, responder)
    asking = asyncio.create_task(approval.ask(ctx, question(session)))
    await reached.wait()

    asking.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asking

    assert written(session) == [
        ("asked", "call-1", "powershell", "powershell: git push"),
        ("decided", "call-1", "cancelled"),
    ]


# The policy of a session


async def test_with_the_policy_never_nobody_is_asked_and_the_answer_is_no(
    ctx: Context, approval: Approval, session: Session
) -> None:
    asked = answering(ctx, "allowed-once")
    approval.set_policy(session, "never")

    assert await approval.ask(ctx, question(session)) == "rejected"
    assert asked == []
    assert written(session) == [
        ("policy", "never"),
        ("asked", "call-1", "powershell", "powershell: git push"),
        ("decided", "call-1", "rejected"),
    ]


async def test_a_session_has_the_policy_of_the_config_until_its_log_names_one(
    ctx: Context, tmp_path: Path, work: Path
) -> None:
    sessions, approval = await program(ctx, tmp_path, policy="never")
    session = sessions.create(ctx, work)

    assert approval.policy(session) == "never"
    assert approval.recorded(session) is None

    approval.set_policy(session, "ask")

    assert approval.policy(session) == "ask"
    assert approval.recorded(session) == "ask"
    assert written(session) == [("policy", "ask")]


async def test_questions_are_put_unless_the_config_says_otherwise(
    approval: Approval, session: Session
) -> None:
    assert approval.policy(session) == "ask"


async def test_a_policy_that_the_log_names_already_is_not_written_again(
    approval: Approval, session: Session
) -> None:
    approval.set_policy(session, "ask")
    approval.set_policy(session, "ask")
    approval.set_policy(session, "never")
    approval.set_policy(session, "never")
    approval.set_policy(session, "ask")

    assert written(session) == [("policy", "ask"), ("policy", "never"), ("policy", "ask")]


async def test_a_change_of_the_policy_counts_from_the_next_question_on(
    ctx: Context, approval: Approval, session: Session
) -> None:
    answering(ctx, "allowed-once")

    approval.set_policy(session, "never")
    first = await approval.ask(ctx, question(session, "call-1"))
    approval.set_policy(session, "ask")
    second = await approval.ask(ctx, question(session, "call-2"))

    assert (first, second) == ("rejected", "allowed-once")


async def test_the_policy_of_one_session_is_not_the_policy_of_another(
    ctx: Context, approval: Approval, session: Session, work: Path
) -> None:
    other = ctx.use(SESSIONS).create(ctx, work)

    approval.set_policy(session, "never")

    assert approval.policy(session) == "never"
    assert approval.policy(other) == "ask"


async def test_a_policy_that_does_not_exist_is_refused(
    ctx: Context, tmp_path: Path, approval: Approval, session: Session
) -> None:
    policy: Policy = "always"  # pyright: ignore[reportAssignmentType]

    with pytest.raises(ValidationError):
        approval.set_policy(session, policy)

    assert written(session) == []
    with pytest.raises(ValidationError):
        await Context().plugin(columbina.approval, {"policy": "always"})


async def test_a_session_that_is_taken_up_again_has_the_policy_it_had(
    tmp_path: Path, work: Path
) -> None:
    first = Context()
    sessions, approval = await program(first, tmp_path)
    session = sessions.create(first, work, session_id="s-1")
    approval.set_policy(session, "never")
    await session.flush()
    await first.fiber.dispose()

    second = Context()
    try:
        sessions, approval = await program(second, tmp_path)
        resumed = sessions.resume(second, work, "s-1")

        assert approval.policy(resumed) == "never"
        assert approval.recorded(resumed) == "never"
    finally:
        await second.fiber.dispose()


async def test_a_session_with_questions_can_be_read_by_a_program_without_approval(
    tmp_path: Path, work: Path
) -> None:
    first = Context()
    sessions, approval = await program(first, tmp_path)
    session = sessions.create(first, work, session_id="s-1")
    approval.set_policy(session, "ask")
    await approval.ask(first, question(session))
    await session.flush()
    await first.fiber.dispose()

    second = Context()
    try:
        await second.plugin(columbina.session, {"root": tmp_path / "sessions"})

        resumed = second.use(SESSIONS).resume(second, work, "s-1")

        assert not resumed.closed
    finally:
        await second.fiber.dispose()
