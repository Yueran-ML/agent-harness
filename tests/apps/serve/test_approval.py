"""The service process: a call that needs a yes is put to the interface, and only a yes is one."""

import asyncio
import json
import logging
from typing import Any

import pytest
from pydantic import BaseModel

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
from columbina.agent import Agent
from columbina.approval import APPROVAL, ApprovalRequest, Outcome
from columbina.permissions import PERMISSIONS
from columbina.tools import Tool, ToolRun
from support.served import Served
from support.terminal import until

LOGGER = "columbina.apps.serve.server"


class NoArgs(BaseModel):
    pass


async def peeked(args: NoArgs, run: ToolRun) -> str:
    return "peeked"


def question(agent: Agent, call_id: str = "c-1", **other: Any) -> ApprovalRequest:
    """A question about a call of an agent: a command, unless something else is said."""
    asked: dict[str, Any] = {
        "tool": "bash",
        "summary": "bash: git push origin main",
        "reason": "No rule allows this command, and nothing holds it here.",
        "target": "git push origin main",
        **other,
    }
    return ApprovalRequest(session=agent.session, call_id=call_id, **asked)


async def put(served: Served, request: ApprovalRequest) -> "asyncio.Future[Outcome]":
    """Put a question as a tool would, and return who waits for how it ends."""
    approval = served.harness.ctx.use(APPROVAL)
    before = len(served.far.asked())
    answering = asyncio.ensure_future(approval.ask(served.agent().ctx, request))
    await served.far.until(lambda: len(served.far.asked()) > before or answering.done())
    return answering


async def approving(served: Served) -> None:
    """Load what puts questions, and open a session."""
    await served.harness.ctx.plugin(columbina.approval)
    await served.opened("mine")


async def permitting(served: Served) -> None:
    """Load what writes a longer yes down as well, with a tool that such a yes can be for."""
    harness = served.harness
    for plugin in (columbina.fs, columbina.sandbox, columbina.approval):
        await harness.ctx.plugin(plugin)
    await harness.ctx.plugin(columbina.permissions, {"preset": "workspace-write"})
    harness.tools.add(
        harness.ctx,
        Tool(name="peek", description="Look.", parameters=NoArgs, execute=peeked, effect="read"),
    )
    await served.opened("mine")


def kinds(served: Served) -> list[str]:
    """What the process sent, in short: the type of an event, or else the method."""
    told: list[str] = []
    for message in served.far.heard:
        if message.get("method") == "session.event":
            told.append(message["params"]["event"]["type"])
        elif "method" in message:
            told.append(message["method"])
    return told


# The question


async def test_a_question_is_put_to_the_interface_with_all_that_is_needed_to_answer_it(
    served: Served,
) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    (asked,) = served.far.asked()
    served.far.reply(asked["id"], {"outcome": "allowed-once"})

    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    assert asked == {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "approval/ask",
        "params": {
            "session_id": "mine",
            "call_id": "c-1",
            "tool": "bash",
            "summary": "bash: git push origin main",
            "reason": "No rule allows this command, and nothing holds it here.",
            # The whole of it: nobody can say yes to a command of which a part is seen.
            "target": "git push origin main",
            "standing": None,
        },
    }
    # The log says that it was asked before it is asked, and how it ended after.
    await served.far.until(lambda: "approval/decided" in kinds(served))
    assert kinds(served)[-3:] == ["approval/asked", "approval/ask", "approval/decided"]
    decided = served.far.told("session.event")[-1]["event"]["data"]
    assert decided == {"call_id": "c-1", "outcome": "allowed-once"}


async def test_a_no_is_a_no(served: Served) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    served.far.reply(served.far.asked()[0]["id"], {"outcome": "rejected"})

    assert await asyncio.wait_for(answering, 5) == "rejected"


@pytest.mark.parametrize(
    "answer",
    [
        {"outcome": "allowed"},
        {"outcome": "allowed-always"},
        {"outcome": "ALLOWED-ONCE"},
        {"outcome": " allowed-once"},
        {"outcome": ["allowed-once"]},
        {"outcome": True},
        {"allowed-once": True},
        {},
        "allowed-once",
        ["allowed-once"],
        True,
        None,
        # Somebody who says both yes and that it was withdrawn has not said yes.
        {"outcome": "cancelled"},
        {"outcome": "unavailable"},
    ],
)
async def test_whatever_is_not_a_plain_yes_is_a_no(served: Served, answer: object) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    served.far.reply(served.far.asked()[0]["id"], answer)

    assert await asyncio.wait_for(answering, 5) == "rejected"


async def test_an_answer_that_is_an_error_is_a_no(served: Served) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    served.far.fail(served.far.asked()[0]["id"], -32603, "The window was closed.")

    assert await asyncio.wait_for(answering, 5) == "rejected"


async def test_an_answer_that_is_no_proper_answer_is_a_no(served: Served) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    asked = served.far.asked()[0]["id"]
    # A yes, and an error with it.
    served.far.say(
        {
            "jsonrpc": "2.0",
            "id": asked,
            "result": {"outcome": "allowed-once"},
            "error": {"code": 1, "message": "or not"},
        }
    )

    assert await asyncio.wait_for(answering, 5) == "rejected"


async def test_an_answer_that_does_not_say_what_it_speaks_is_a_no_and_nothing_is_sent_back(
    served: Served,
) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    asked = served.far.asked()[0]["id"]
    before = len(served.far.heard)
    # A yes, as far as it goes. It does not say that it is a message of JSON-RPC.
    served.far.write(json.dumps({"id": asked, "result": {"outcome": "allowed-once"}}).encode())

    # The question does not go on waiting: whatever is not a plain yes is a no.
    assert await asyncio.wait_for(answering, 5) == "rejected"
    # And no error answers the answer. It would name a request of the interface.
    await served.far.result("session/list")
    assert [one for one in served.far.heard[before:] if "error" in one] == []


async def test_a_question_is_answered_once_and_what_comes_for_it_after_that_means_nothing(
    served: Served,
) -> None:
    await approving(served)

    answering = await put(served, question(served.agent()))
    asked = served.far.asked()[0]["id"]
    # Two answers in a row, before the one who asked has heard of the first.
    served.far.reply(asked, {"outcome": "rejected"})
    served.far.reply(asked, {"outcome": "allowed-once"})

    assert await asyncio.wait_for(answering, 5) == "rejected"
    # The program goes on as before.
    listed = await served.far.result("session/list")
    assert [one["id"] for one in listed["sessions"]] == ["mine"]


async def test_each_question_has_a_number_of_its_own_and_is_answered_by_it(served: Served) -> None:
    await approving(served)
    agent = served.agent()

    first = await put(served, question(agent, "c-1"))
    second = await put(served, question(agent, "c-2"))
    one, two = served.far.asked()
    # The second one is answered first.
    served.far.reply(two["id"], {"outcome": "rejected"})
    assert await asyncio.wait_for(second, 5) == "rejected"
    assert not first.done()
    served.far.reply(one["id"], {"outcome": "allowed-once"})

    assert await asyncio.wait_for(first, 5) == "allowed-once"
    assert (one["id"], two["id"]) == (1, 2)
    assert (one["params"]["call_id"], two["params"]["call_id"]) == ("c-1", "c-2")
    # And the next one goes on counting.
    third = await put(served, question(agent, "c-3"))
    assert served.far.asked()[-1]["id"] == 3
    served.far.reply(3, {"outcome": "rejected"})
    await asyncio.wait_for(third, 5)


# A question that is withdrawn


async def test_an_answer_that_comes_after_its_question_was_withdrawn_means_nothing(
    served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    await approving(served)
    agent = served.agent()

    answering = await put(served, question(agent))
    asked = served.far.asked()[0]["id"]
    # As a turn does that is stopped while it waits for the answer.
    answering.cancel()
    await asyncio.wait([answering])
    await served.far.until(lambda: "approval/decided" in kinds(served))
    with caplog.at_level(logging.DEBUG, logger=LOGGER):
        served.far.reply(asked, {"outcome": "allowed-once"})
        # The next question is asked and answered as if nothing had come in between.
        again = await put(served, question(agent, "c-2"))
        served.far.reply(served.far.asked()[-1]["id"], {"outcome": "rejected"})
        outcome = await asyncio.wait_for(again, 5)

    assert answering.cancelled()
    decided = [
        one["event"]["data"]
        for one in served.far.told("session.event")
        if one["event"]["type"] == "approval/decided"
    ]
    assert decided == [
        {"call_id": "c-1", "outcome": "cancelled"},
        {"call_id": "c-2", "outcome": "rejected"},
    ]
    assert outcome == "rejected"
    assert "an answer to 1 came, and nothing waits for it" in caplog.messages
    # The late answer was answered with nothing: there is nothing to answer an answer with.
    assert [message for message in served.far.heard if message.get("id") is None] == [
        message for message in served.far.heard if "id" not in message
    ]


async def test_a_question_is_withdrawn_when_the_turn_that_asked_it_is_stopped(
    served: Served,
) -> None:
    await approving(served)
    agent = served.agent()
    approval = served.harness.ctx.use(APPROVAL)
    asked_in_turn: list[asyncio.Future[Outcome]] = []

    async def work() -> str:
        # What a tool does in a turn: it asks, and waits.
        asking = asyncio.ensure_future(approval.ask(agent.ctx, question(agent)))
        asked_in_turn.append(asking)
        return await asking

    waiting = asyncio.ensure_future(agent.run_maintenance(work))
    await served.far.until(lambda: bool(served.far.asked()))

    assert await served.far.result("session/cancel") == {}
    await asyncio.wait([waiting])

    await served.far.until(lambda: "approval/decided" in kinds(served))
    decided = served.far.told("session.event")[-1]["event"]["data"]
    assert decided == {"call_id": "c-1", "outcome": "cancelled"}


# Who is not asked


async def test_nobody_is_asked_where_the_session_is_set_to_ask_nobody(served: Served) -> None:
    await approving(served)
    agent = served.agent()
    served.harness.ctx.use(APPROVAL).set_policy(agent.session, "never")

    answering = await put(served, question(agent))

    assert await asyncio.wait_for(answering, 5) == "rejected"
    assert served.far.asked() == []
    # The log has both all the same.
    assert kinds(served)[-2:] == ["approval/asked", "approval/decided"]


async def test_a_question_about_another_session_is_not_for_the_interface(served: Served) -> None:
    await approving(served)
    harness = served.harness
    other = await harness.agents.create(harness.ctx, harness.spec(), session_id="other")

    # Asked outside every scope, where every listener hears it.
    outcome = await harness.ctx.use(APPROVAL).ask(harness.ctx, question(other))

    assert outcome == "unavailable"
    assert served.far.asked() == []
    await other.close()


async def test_an_agent_that_the_conversation_has_left_asks_nobody(served: Served) -> None:
    await approving(served)
    harness = served.harness
    left = served.agent()
    approval = harness.ctx.use(APPROVAL)
    asked: list[asyncio.Future[Outcome]] = []

    def at_the_answer(message: dict[str, Any]) -> None:
        # The moment the conversation is with the other session: the agent that it left
        # has not been ended yet, and asks.
        if "result" in message and message["result"].get("session_id") == "second":
            asked.append(asyncio.ensure_future(approval.ask(left.ctx, question(left))))

    served.far.on_sent = at_the_answer
    await served.far.result("session/open", {"session_id": "second"})
    (answering,) = asked

    assert await asyncio.wait_for(answering, 5) == "unavailable"
    assert served.far.asked() == []


# A longer yes


async def test_a_yes_for_the_rest_of_the_session_is_offered_where_it_can_be_given(
    served: Served,
) -> None:
    await permitting(served)
    agent = served.agent()
    permissions = served.harness.ctx.use(PERMISSIONS)
    longer = ApprovalRequest(
        session=agent.session,
        call_id="c-1",
        tool="peek",
        summary="peek: notes.txt",
        reason="The rules say to ask.",
        target="notes.txt",
        standing="peek on notes.txt",
    )

    answering = await put(served, longer)
    asked = served.far.asked()[0]
    served.far.reply(asked["id"], {"outcome": "allowed-once", "remember": True})

    # The answer is a yes for this one call, and the longer yes is written down.
    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    assert asked["params"]["standing"] == "peek on notes.txt"
    assert permissions.granted(agent.session) == (("peek", "notes.txt"),)


async def test_a_longer_yes_holds_when_its_session_is_opened_again_and_for_no_other(
    served: Served,
) -> None:
    await permitting(served)
    live = served.harness.agents.live
    permissions = served.harness.ctx.use(PERMISSIONS)
    first = served.agent()
    longer = question(first, tool="peek", target="notes.txt", standing="peek on notes.txt")
    answering = await put(served, longer)
    served.far.reply(served.far.asked()[0]["id"], {"outcome": "allowed-once", "remember": True})
    await asyncio.wait_for(answering, 5)

    await served.far.result("session/open", {"session_id": "other"})
    await until(lambda: list(live) == ["other"])
    in_the_other = permissions.granted(served.agent().session)
    await served.far.result("session/open", {"session_id": "mine"})
    await until(lambda: list(live) == ["mine"])
    again = served.agent()

    # The yes was for that session, for as long as the program runs. It is not asked for
    # anew because the person looked at another task in between.
    assert in_the_other == ()
    assert again is not first
    assert permissions.granted(again.session) == (("peek", "notes.txt"),)


async def test_a_longer_yes_that_can_no_longer_be_given_is_passed_over(
    served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    await permitting(served)
    harness = served.harness
    # A tool that comes and goes, as the tools of a server that is connected do.
    glance = Tool(
        name="glance", description="Look.", parameters=NoArgs, execute=peeked, effect="read"
    )
    registered = harness.tools.registry.add(harness.ctx, "glance", glance)
    longer = question(
        served.agent(), tool="glance", target="notes.txt", standing="glance on notes.txt"
    )
    answering = await put(served, longer)
    served.far.reply(served.far.asked()[0]["id"], {"outcome": "allowed-once", "remember": True})
    await asyncio.wait_for(answering, 5)
    await registered.dispose()

    with caplog.at_level(logging.WARNING, logger=LOGGER):
        again = await served.far.result("session/open", {"session_id": "mine"})

    # The session is open all the same, without the yes for a tool that is gone.
    assert again["session_id"] == "mine"
    assert harness.ctx.use(PERMISSIONS).granted(served.agent().session) == ()
    assert caplog.messages == [
        "a longer yes could not be given again: "
        "a call of glance like this one has to be asked about every time"
    ]


@pytest.mark.parametrize(
    "answer",
    [
        {"outcome": "allowed-once"},
        {"outcome": "allowed-once", "remember": False},
        {"outcome": "allowed-once", "remember": "yes"},
        {"outcome": "allowed-once", "remember": 1},
        {"outcome": "rejected", "remember": True},
    ],
)
async def test_nothing_is_written_down_without_a_yes_that_asks_for_it_in_plain_words(
    served: Served, answer: dict[str, object]
) -> None:
    await permitting(served)
    agent = served.agent()
    longer = question(agent, tool="peek", target="notes.txt", standing="peek on notes.txt")

    answering = await put(served, longer)
    served.far.reply(served.far.asked()[0]["id"], answer)
    await asyncio.wait_for(answering, 5)

    assert served.harness.ctx.use(PERMISSIONS).granted(agent.session) == ()


async def test_a_call_that_has_to_be_asked_about_every_time_is_not_remembered(
    served: Served,
) -> None:
    await permitting(served)
    agent = served.agent()

    answering = await put(served, question(agent, tool="peek", target="notes.txt"))
    asked = served.far.asked()[0]
    served.far.reply(asked["id"], {"outcome": "allowed-once", "remember": True})

    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    assert asked["params"]["standing"] is None
    assert served.harness.ctx.use(PERMISSIONS).granted(agent.session) == ()


async def test_a_longer_yes_that_cannot_be_written_down_is_a_yes_for_the_call_all_the_same(
    served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    await permitting(served)
    agent = served.agent()
    # Whoever asked offers a longer yes for a tool that there is not.
    unknown = question(agent, standing="this command, word for word")

    answering = await put(served, unknown)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        served.far.reply(served.far.asked()[0]["id"], {"outcome": "allowed-once", "remember": True})
        outcome = await asyncio.wait_for(answering, 5)
        # What was not written down is not tried again when the session is opened again.
        await served.far.result("session/open", {"session_id": "mine"})

    assert outcome == "allowed-once"
    assert caplog.messages == [
        "the longer yes could not be written down: a call of bash like this one has to be "
        "asked about every time"
    ]
    assert served.harness.ctx.use(PERMISSIONS).granted(agent.session) == ()


async def test_no_longer_yes_is_offered_where_nothing_could_write_it_down(served: Served) -> None:
    await approving(served)
    agent = served.agent()

    answering = await put(served, question(agent, standing="this command, word for word"))
    asked = served.far.asked()[0]
    served.far.reply(asked["id"], {"outcome": "allowed-once", "remember": True})

    assert await asyncio.wait_for(answering, 5) == "allowed-once"
    assert asked["params"]["standing"] is None
