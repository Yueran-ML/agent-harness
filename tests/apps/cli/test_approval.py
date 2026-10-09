"""The person at the terminal is asked before something is done, and answers with one key."""

import asyncio

import pytest

import columbina.session
from columbina.approval import ApprovalRequest, Outcome
from columbina.apps.cli.approval import Responder
from columbina.apps.cli.prompt import Prompt
from columbina.kernel import Context
from columbina.session import SESSIONS, Session
from support.terminal import ESCAPE, Desk, desk

WHY = "No rule allows this command, and nothing holds it here."
LONGER = "  a  yes, and do not ask again in this session about this command, word for word\n"


class Grants:
    """What was allowed for the rest of a session, written down."""

    def __init__(self, refuses: bool = False) -> None:
        self.given: list[tuple[str, str, str | None]] = []
        self._refuses = refuses

    def grant(self, session: Session, tool: str, target: str | None) -> None:
        if self._refuses:
            raise ValueError("a call like this one has to be asked about every time")
        self.given.append((session.id, tool, target))


@pytest.fixture
async def session(ctx: Context) -> Session:
    await ctx.plugin(columbina.session)
    return ctx.use(SESSIONS).create(ctx, ".", session_id="s-1")


def request(
    session: Session,
    *,
    tool: str = "bash",
    target: str | None = "git status",
    standing: str | None = "this command, word for word",
) -> ApprovalRequest:
    return ApprovalRequest(
        session=session,
        call_id="c-1",
        tool=tool,
        summary=f"{tool}: {target}",
        reason=WHY,
        target=target,
        standing=standing,
    )


async def answered(
    at: Desk, responder: Responder, asked: ApprovalRequest, key: str, mark: str
) -> Outcome:
    """Put a question, press a key once the question is on the screen, and return the outcome."""
    waiting = asyncio.ensure_future(responder.prompt.line())
    answering = asyncio.ensure_future(responder(asked))
    await at.draws(mark)
    at.type(key)
    try:
        return await asyncio.wait_for(answering, 5)
    finally:
        waiting.cancel()


async def test_the_question_shows_the_tool_the_whole_of_what_it_acts_on_and_why(
    session: Session,
) -> None:
    command = "git status &&\n  git diff --stat " + "x" * 300
    with desk() as at:
        responder = Responder(Prompt(at.terminal), None)

        await answered(at, responder, request(session, target=command), "y", "[y/n]")

        await at.sees(
            "bash asks for a yes:\n"
            "    git status &&\n"
            "      git diff --stat " + "x" * 300 + "\n"
            f"{WHY}\n"
            "  y  yes, this once\n"
            "  n  no\n",
        )


async def test_a_call_that_acts_on_nothing_in_particular_is_asked_about_by_its_name(
    session: Session,
) -> None:
    with desk() as at:
        responder = Responder(Prompt(at.terminal), None)

        await answered(at, responder, request(session, tool="note", target=None), "y", "[y/n]")

        await at.sees(f"note asks for a yes.\n{WHY}\n  y  yes, this once\n")


@pytest.mark.parametrize(
    ("key", "outcome", "said"),
    [
        ("y", "allowed-once", "(allowed, this once)"),
        ("n", "rejected", "(not allowed)"),
        (ESCAPE, "rejected", "(not allowed)"),
    ],
    ids=["yes", "no", "escape"],
)
async def test_one_key_answers_for_this_one_call(
    session: Session, key: str, outcome: Outcome, said: str
) -> None:
    granted = Grants()
    with desk() as at:
        responder = Responder(Prompt(at.terminal), granted)

        assert await answered(at, responder, request(session), key, "[y/a/n]") == outcome

        await at.sees(f"  y  yes, this once\n{LONGER}  n  no\n")
        await at.sees(f"{said}\n")
        assert granted.given == []


async def test_a_yes_for_the_session_is_written_down_where_the_rules_are(
    session: Session,
) -> None:
    granted = Grants()
    with desk() as at:
        responder = Responder(Prompt(at.terminal), granted)

        outcome = await answered(at, responder, request(session), "a", "[y/a/n]")

        assert outcome == "allowed-once"
        assert granted.given == [("s-1", "bash", "git status")]
        await at.sees("(allowed, and not asked about again in this session)\n")


async def only_yes_and_no(at: Desk, responder: Responder, asked: ApprovalRequest) -> None:
    waiting = asyncio.ensure_future(responder.prompt.line())
    answering = asyncio.ensure_future(responder(asked))
    await at.draws("[y/n]")

    # The key of the longer yes is no answer here. The next key is.
    at.type("ay")

    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    await at.sees("(allowed, this once)\n")
    assert "  a  " not in at.seen()
    waiting.cancel()


async def test_a_call_that_has_to_be_asked_about_every_time_is_offered_no_longer_yes(
    session: Session,
) -> None:
    granted = Grants()
    with desk() as at:
        responder = Responder(Prompt(at.terminal), granted)

        await only_yes_and_no(at, responder, request(session, standing=None))

        assert granted.given == []


async def test_without_a_place_to_write_it_down_no_longer_yes_is_offered(
    session: Session,
) -> None:
    with desk() as at:
        await only_yes_and_no(at, Responder(Prompt(at.terminal), None), request(session))


async def test_a_longer_yes_that_cannot_be_written_down_still_allows_this_call(
    session: Session,
) -> None:
    with desk() as at:
        responder = Responder(Prompt(at.terminal), Grants(refuses=True))

        outcome = await answered(at, responder, request(session), "a", "[y/a/n]")

        assert outcome == "allowed-once"
        await at.sees("(allowed, this once)\n")


async def test_a_question_that_is_withdrawn_is_withdrawn(session: Session) -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        responder = Responder(prompt, None)
        waiting = asyncio.ensure_future(prompt.line())
        answering = asyncio.ensure_future(responder(request(session)))
        await at.draws("[y/n]")

        answering.cancel()

        with pytest.raises(asyncio.CancelledError):
            await answering
        await at.sees("(the question was withdrawn)\n")
        waiting.cancel()
