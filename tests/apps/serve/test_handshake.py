"""The service process: how a conversation with it begins and ends, and what it refuses."""

import asyncio
import logging

import pytest

import columbina
from columbina.agent import Agent
from columbina.apps.serve import wire as wire_module
from columbina.apps.serve.rpc import (
    INTERNAL_ERROR,
    INVALID_PARAMS,
    INVALID_REQUEST,
    METHOD_NOT_FOUND,
    NOT_INITIALIZED,
    PARSE_ERROR,
)
from columbina.commands import COMMANDS, Command
from columbina.session import Sessions
from support.loop import Harness
from support.served import Served, loaded
from support.wires import Far

LOGGER = "columbina.apps.serve.server"
NOT_GREETED = "Nothing was asked for yet with initialize, and that comes first."


# The first thing that is asked for


async def test_the_first_answer_says_what_the_program_is_and_what_it_can_be_told(
    served: Served,
) -> None:
    hello = await served.far.result(
        "initialize", {"protocol": 1, "client": {"name": "a test", "version": "0.3"}}
    )

    assert hello == {
        "protocol": 1,
        "server": {"name": "columbina", "version": columbina.__version__},
        "cwd": str(served.harness.cwd),
        # By their names, and without the slash.
        "commands": [
            {"name": "help", "summary": "List the commands."},
            {"name": "model", "summary": "Show the model, or name another."},
        ],
    }


async def test_the_commands_are_told_by_their_names_whenever_they_were_registered(
    served: Served,
) -> None:
    harness = served.harness

    async def nothing(agent: Agent, rest: str) -> str:
        return ""

    for name in ("zebra", "aardvark"):
        harness.ctx.use(COMMANDS).registry.add(harness.ctx, name, Command(nothing, "Nothing."))

    hello = await served.greeted()

    assert [command["name"] for command in hello["commands"]] == [
        "aardvark",
        "help",
        "model",
        "zebra",
    ]


@pytest.mark.parametrize(("theirs", "spoken"), [(1, 1), (2, 1), (40, 1)])
async def test_the_version_that_is_spoken_is_no_higher_than_either_side_speaks(
    served: Served, theirs: int, spoken: int
) -> None:
    hello = await served.far.result("initialize", {"protocol": theirs})

    assert hello["protocol"] == spoken


@pytest.mark.parametrize(
    ("params", "wrong"),
    [
        ({}, "protocol: Field required"),
        ({"protocol": 0}, "protocol: Input should be greater than or equal to 1"),
        ({"protocol": "1"}, "protocol: Input should be a valid integer"),
        ({"protocol": True}, "protocol: Input should be a valid integer"),
        ({"protocol": 1.0}, "protocol: Input should be a valid integer"),
        ({"protocol": 1, "client": "me"}, "client: Input should be a valid dictionary"),
        ({"protocol": 1, "client": {"name": 7}}, "client.name: Input should be a valid string"),
    ],
)
async def test_a_first_request_that_is_not_right_says_what_is_wrong_with_it(
    served: Served, params: dict[str, object], wrong: str
) -> None:
    code, message = await served.far.refusal("initialize", params)

    assert code == INVALID_PARAMS
    assert message.startswith(f"The params of initialize are not right: {wrong}")
    # It was not taken, so it can be asked for again.
    assert (await served.greeted())["protocol"] == 1


async def test_what_a_newer_interface_adds_to_a_request_is_passed_over(served: Served) -> None:
    hello = await served.far.result(
        "initialize", {"protocol": 1, "window": {"width": 900}, "client": {"build": "x"}}
    )

    assert hello["protocol"] == 1


@pytest.mark.parametrize(
    ("client", "who"),
    [
        ({"name": "a test", "version": "0.3"}, "a test 0.3"),
        ({"name": "a test"}, "a test"),
        ({}, "an interface that gives no name"),
    ],
)
async def test_who_the_interface_is_goes_into_the_log(
    served: Served, caplog: pytest.LogCaptureFixture, client: dict[str, str], who: str
) -> None:
    with caplog.at_level(logging.INFO, logger=LOGGER):
        await served.far.result("initialize", {"protocol": 3, "client": client})

    assert caplog.messages == [f"the interface is {who}. It speaks version 3 of the protocol"]


async def test_nothing_else_is_done_before_the_first_request(served: Served) -> None:
    refusals = [
        await served.far.refusal(method, params)
        for method, params in [
            ("session/list", None),
            ("session/open", {}),
            ("session/events", {"from": 0}),
            ("session/prompt", {"text": "Hi"}),
            ("session/steer", {"text": "Hi"}),
            ("session/cancel", None),
            ("session/command", {"line": "/help"}),
        ]
    ]

    assert refusals == [(NOT_INITIALIZED, NOT_GREETED)] * 7
    assert list(served.harness.agents.live) == []


async def test_the_first_request_is_made_once(served: Served) -> None:
    await served.greeted()

    assert await served.far.refusal("initialize", {"protocol": 1}) == (
        NOT_INITIALIZED,
        "initialize was asked for already. It is asked for once.",
    )


# What is no request


@pytest.mark.parametrize("greeted", [False, True])
async def test_a_method_that_there_is_not_is_said_to_be_none(served: Served, greeted: bool) -> None:
    if greeted:
        await served.greeted()

    assert await served.far.refusal("session/frobnicate", {"x": 1}) == (
        METHOD_NOT_FOUND,
        "There is no method session/frobnicate.",
    )


async def test_a_line_that_is_no_message_is_answered_with_what_is_wrong_and_the_next_is_read(
    served: Served,
) -> None:
    served.far.write(b"{not json")
    served.far.write(b'{"jsonrpc":"2.0","id":"q","method":7}')
    served.far.write(b"[]")

    hello = await served.greeted()

    assert hello["protocol"] == 1
    first, second, third = served.far.heard[:3]
    assert (first["id"], first["error"]["code"]) == (None, PARSE_ERROR)
    assert first["error"]["message"].startswith("The line is not JSON: ")
    assert second == {
        "jsonrpc": "2.0",
        "id": "q",
        "error": {"code": INVALID_REQUEST, "message": "The method of a message is a string."},
    }
    assert third["error"] == {
        "code": INVALID_REQUEST,
        "message": "A line holds one message. Batches are not taken.",
    }


async def test_a_line_that_is_more_than_is_read_is_answered_and_the_program_goes_on(
    served: Served,
) -> None:
    deep = b"[" * 100_000 + b"]" * 100_000
    served.far.write(b'{"jsonrpc":"2.0","id":1,"method":"shutdown","params":{"x":' + deep + b"}}")
    served.far.write(b'{"jsonrpc":"2.0","id":' + b"7" * 5000 + b',"method":"shutdown"}')

    hello = await served.greeted()

    # Neither was done, whatever it asked for: the program is still there.
    assert hello["protocol"] == 1
    assert [(one["id"], one["error"]) for one in served.far.heard[:2]] == [
        (
            None,
            {
                "code": PARSE_ERROR,
                "message": "The line is nested more deeply than this program reads.",
            },
        ),
        (
            None,
            {
                "code": PARSE_ERROR,
                "message": "The line holds a whole number that is too long to be read.",
            },
        ),
    ]


async def test_a_line_that_was_too_long_to_be_taken_is_said_to_have_been(served: Served) -> None:
    served.far.write(None)

    await served.greeted()

    assert served.far.heard[0] == {
        "jsonrpc": "2.0",
        "id": None,
        "error": {
            "code": INVALID_REQUEST,
            "message": "The line is longer than this program takes, and what it held was dropped.",
        },
    }


async def test_lines_of_nothing_are_passed_over(served: Served) -> None:
    served.far.write(b"")
    served.far.write(b"   \t ")
    served.far.write(b"\r")

    await served.greeted()

    assert len(served.far.heard) == 1


async def test_what_an_interface_says_without_asking_is_passed_over(
    served: Served, caplog: pytest.LogCaptureFixture
) -> None:
    await served.greeted()

    with caplog.at_level(logging.DEBUG, logger=LOGGER):
        served.far.say({"jsonrpc": "2.0", "method": "window/resized", "params": {"width": 900}})
        # And an answer to something that nobody asked.
        served.far.reply(77, {"outcome": "allowed-once"})
        sessions = await served.far.result("session/list")

    assert sessions == {"sessions": []}
    assert len(served.far.heard) == 2
    assert caplog.messages == [
        "the interface says window/resized, which means nothing here",
        "an answer to 77 came, and nothing waits for it",
    ]


async def test_an_answer_that_names_no_request_is_not_answered(served: Served) -> None:
    await served.greeted()

    # What another program says of a line that it could not read, and an answer without an id.
    served.far.write(b'{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}')
    served.far.write(b'{"jsonrpc":"2.0","result":{}}')
    sessions = await served.far.result("session/list")

    assert sessions == {"sessions": []}
    # The greeting and the list. Nothing was sent back for the two answers.
    assert len(served.far.heard) == 2


async def test_what_goes_wrong_inside_is_answered_as_that_and_the_program_goes_on(
    served: Served, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    await served.greeted()

    def broken(self: Sessions, cwd: object) -> list[object]:
        raise RuntimeError("the disk is on fire at F:/secret/place")

    monkeypatch.setattr(Sessions, "stored", broken)

    with caplog.at_level(logging.ERROR, logger=LOGGER):
        refusal = await served.far.refusal("session/list")
    monkeypatch.undo()

    # Nothing of what went wrong is told to the other side. The log has it.
    assert refusal == (
        INTERNAL_ERROR,
        "Something went wrong inside the service process. Its log says what.",
    )
    assert caplog.messages == ["the request session/list failed"]
    assert "the disk is on fire" in caplog.text
    assert await served.far.result("session/list") == {"sessions": []}


# The end


async def test_the_program_leaves_when_it_is_told_to_and_answers_first(served: Served) -> None:
    await served.greeted()

    answer = served.far.ask("shutdown")
    # What comes after the order to leave is not read.
    late = served.far.ask("session/list")
    code = await served.left()

    assert code == 0
    assert served.far.answers(answer) == [{"jsonrpc": "2.0", "id": answer, "result": {}}]
    assert served.far.answers(late) == []
    assert served.far.closed


async def test_the_program_can_be_told_to_leave_before_anything_else(served: Served) -> None:
    assert await served.far.result("shutdown") == {}
    assert await served.left() == 0


async def test_the_program_leaves_when_the_other_side_closes_its_end(served: Served) -> None:
    await served.greeted()

    served.far.hang_up()

    assert await served.left() == 0
    assert served.far.closed
    assert len(served.far.heard) == 1


# The wire of a real process


async def test_without_a_wire_of_its_own_the_program_takes_that_of_its_process(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    far = Far()
    monkeypatch.setattr(wire_module, "taken", lambda: far)
    running = await loaded(harness, None)

    assert await far.result("shutdown") == {}
    assert await asyncio.wait_for(running, 10) == 0


async def test_a_process_without_standard_input_and_output_cannot_serve_and_says_so(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def none() -> Far:
        raise OSError(9, "Bad file descriptor")

    monkeypatch.setattr(wire_module, "taken", none)
    running = await loaded(harness, None)

    assert await asyncio.wait_for(running, 10) == 2
    assert capsys.readouterr().err == (
        "columbina: serve talks over its standard input and output, and this process has "
        "none: [Errno 9] Bad file descriptor\n"
    )
