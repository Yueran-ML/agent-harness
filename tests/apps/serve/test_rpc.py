"""Messages of JSON-RPC, a line at a time: what a line means, and the lines that go out."""

import json
from typing import Any

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.apps.serve.rpc import (
    INVALID_PARAMS,
    INVALID_REQUEST,
    NO_SESSION,
    NOT_IDLE,
    PARSE_ERROR,
    MalformedLineError,
    Notification,
    Request,
    Response,
    RpcError,
    answered,
    asked,
    on_the_wire,
    parsed,
    refused,
    text,
    told,
)

HALF = chr(0xD83D)
"""Half of a character that takes two units of UTF-16, which no text of UTF-8 can hold."""

# What comes in


def test_a_request_is_read_with_its_id_its_method_and_its_params() -> None:
    line = b'{"jsonrpc":"2.0","id":7,"method":"session/prompt","params":{"text":"Hi"}}'

    assert parsed(line) == Request(7, "session/prompt", {"text": "Hi"})


def test_the_id_of_a_request_may_be_a_string() -> None:
    line = b'{"jsonrpc":"2.0","id":"a-1","method":"session/cancel","params":{}}'

    assert parsed(line) == Request("a-1", "session/cancel", {})


def test_a_request_without_params_has_none() -> None:
    assert parsed(b'{"jsonrpc":"2.0","id":1,"method":"shutdown"}') == Request(1, "shutdown", {})


def test_text_beyond_ascii_is_read_as_it_was_written() -> None:
    line = '{"jsonrpc":"2.0","id":1,"method":"session/prompt","params":{"text":"你好 ✓"}}'

    assert parsed(line.encode()) == Request(1, "session/prompt", {"text": "你好 ✓"})


def test_a_message_with_a_method_and_no_id_is_a_notification() -> None:
    assert parsed(b'{"jsonrpc":"2.0","method":"moved","params":{"x":1}}') == Notification(
        "moved", {"x": 1}
    )
    assert parsed(b'{"jsonrpc":"2.0","method":"moved"}') == Notification("moved", {})


@pytest.mark.parametrize("params", ["[1,2]", '"x"', "7", "null"])
def test_a_notification_whose_params_are_no_object_has_none(params: str) -> None:
    # Nothing answers a notification, so nothing could say what is wrong with it.
    line = f'{{"jsonrpc":"2.0","method":"moved","params":{params}}}'

    assert parsed(line.encode()) == Notification("moved", {})


def test_an_answer_carries_its_result() -> None:
    line = b'{"jsonrpc":"2.0","id":3,"result":{"outcome":"allowed-once"}}'

    assert parsed(line) == Response(3, {"outcome": "allowed-once"}, failed=False)


def test_an_answer_may_have_nothing_for_a_result() -> None:
    assert parsed(b'{"jsonrpc":"2.0","id":"q","result":null}') == Response("q", None, failed=False)


@pytest.mark.parametrize(
    "body",
    [
        '"error":{"code":-32603,"message":"it broke"}',
        '"result":{"outcome":"allowed-once"},"error":{"code":1,"message":"and yet"}',
        '"something":"else"',
    ],
    ids=["an error", "a result and an error", "neither"],
)
def test_an_answer_that_is_an_error_or_none_at_all_is_a_failure(body: str) -> None:
    # Whoever waited for it has to hear that it came. What it held is not gone by.
    line = f'{{"jsonrpc":"2.0","id":3,{body}}}'

    assert parsed(line.encode()) == Response(3, None, failed=True)


@pytest.mark.parametrize(
    "line",
    [
        b'{"id":3,"result":{"outcome":"allowed-once"}}',
        b'{"jsonrpc":"1.0","id":3,"result":{"outcome":"allowed-once"}}',
        b'{"jsonrpc":2.0,"id":3,"error":{"code":1,"message":"no"}}',
        b'{"id":3}',
    ],
    ids=["no version", "another version", "a version that is no text", "nothing but an id"],
)
def test_an_answer_that_does_not_say_what_it_speaks_is_a_failure_and_not_refused(
    line: bytes,
) -> None:
    # It names a question, so whoever asked has to hear that it came. And nothing can be
    # sent back for it: an error would name a request of the other side by its number.
    assert parsed(line) == Response(3, None, failed=True)


@pytest.mark.parametrize(
    ("line", "code", "message", "id"),
    [
        (b"\xff\xfe", PARSE_ERROR, "The line is not UTF-8.", None),
        (b"NaN", PARSE_ERROR, "The line is not JSON: NaN is no number of JSON.", None),
        (
            b'[{"jsonrpc":"2.0","id":1,"method":"shutdown"}]',
            INVALID_REQUEST,
            "A line holds one message. Batches are not taken.",
            None,
        ),
        (b'"shutdown"', INVALID_REQUEST, "A message is an object of JSON.", None),
        (b"7", INVALID_REQUEST, "A message is an object of JSON.", None),
        (b"null", INVALID_REQUEST, "A message is an object of JSON.", None),
        (
            b'{"id":1,"method":"shutdown"}',
            INVALID_REQUEST,
            'A message says "jsonrpc": "2.0".',
            1,
        ),
        (
            b'{"jsonrpc":"1.0","id":"x","method":"shutdown"}',
            INVALID_REQUEST,
            'A message says "jsonrpc": "2.0".',
            "x",
        ),
        (
            b'{"jsonrpc":2.0,"method":"shutdown"}',
            INVALID_REQUEST,
            'A message says "jsonrpc": "2.0".',
            None,
        ),
        (
            b'{"jsonrpc":"2.0","id":1,"method":7}',
            INVALID_REQUEST,
            "The method of a message is a string.",
            1,
        ),
        (
            b'{"jsonrpc":"2.0","id":1,"method":"m","params":[1,2]}',
            INVALID_PARAMS,
            "The params of a request are an object.",
            1,
        ),
        (
            b'{"jsonrpc":"2.0","id":1,"method":"m","params":null}',
            INVALID_PARAMS,
            "The params of a request are an object.",
            1,
        ),
        (
            b'{"jsonrpc":"2.0"}',
            INVALID_REQUEST,
            "A message has a method, or it answers a request and names its id.",
            None,
        ),
        (
            b"{}",
            INVALID_REQUEST,
            "A message has a method, or it answers a request and names its id.",
            None,
        ),
        (
            b'{"jsonrpc":"2.0","id":null}',
            INVALID_REQUEST,
            "A message has a method, or it answers a request and names its id.",
            None,
        ),
    ],
)
def test_a_line_that_is_no_message_says_with_which_error_it_is_answered(
    line: bytes, code: int, message: str, id: Any
) -> None:
    with pytest.raises(MalformedLineError) as refusal:
        parsed(line)

    assert (refusal.value.code, refusal.value.message, refusal.value.id) == (code, message, id)
    assert str(refusal.value) == message


def test_a_line_that_breaks_off_is_no_json_and_says_where() -> None:
    with pytest.raises(MalformedLineError) as refusal:
        parsed(b'{"jsonrpc":"2.0","id":1,"method":')

    assert refusal.value.code == PARSE_ERROR
    assert refusal.value.message.startswith("The line is not JSON: Expecting value")
    assert refusal.value.id is None


def test_a_number_that_json_does_not_have_is_no_json_wherever_it_stands() -> None:
    with pytest.raises(MalformedLineError) as refusal:
        parsed(b'{"jsonrpc":"2.0","id":1,"method":"m","params":{"limit":Infinity}}')

    assert (refusal.value.code, refusal.value.message) == (
        PARSE_ERROR,
        "The line is not JSON: Infinity is no number of JSON.",
    )


@pytest.mark.parametrize(
    "line",
    [
        b'{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}',
        b'{"jsonrpc":"2.0","result":{}}',
        b'{"jsonrpc":"2.0","id":true,"result":{}}',
        b'{"id":1.5,"error":{"code":1,"message":"no"}}',
    ],
    ids=["an error for a line that could not be read", "no id", "an id that is none", "a fraction"],
)
def test_an_answer_that_names_no_request_is_an_answer_all_the_same(line: bytes) -> None:
    # The other side could not tell what it answers, or did not say. Nothing waits for
    # such an answer. It is not refused: another program like this one would answer the
    # error with an error again, and so on without end.
    assert parsed(line) == Response(None, None, failed=True)


def test_a_line_that_is_nested_more_deeply_than_can_be_read_says_so() -> None:
    deep = b"[" * 100_000 + b"]" * 100_000
    line = b'{"jsonrpc":"2.0","id":1,"method":"shutdown","params":{"more":' + deep + b"}}"

    with pytest.raises(MalformedLineError) as refusal:
        parsed(line)

    assert (refusal.value.code, refusal.value.message, refusal.value.id) == (
        PARSE_ERROR,
        "The line is nested more deeply than this program reads.",
        None,
    )


def test_a_whole_number_that_is_too_long_to_be_read_says_so() -> None:
    line = b'{"jsonrpc":"2.0","id":' + b"7" * 5000 + b',"method":"shutdown"}'

    with pytest.raises(MalformedLineError) as refusal:
        parsed(line)

    assert (refusal.value.code, refusal.value.message, refusal.value.id) == (
        PARSE_ERROR,
        "The line holds a whole number that is too long to be read.",
        None,
    )


def test_a_whole_number_is_read_however_large_as_long_as_it_can_be_read() -> None:
    number = int("7" * 60)
    line = f'{{"jsonrpc":"2.0","id":{number},"method":"m","params":{{"n":-{number}}}}}'

    assert parsed(line.encode()) == Request(number, "m", {"n": -number})


@pytest.mark.parametrize("id", ["1.5", "true", "false", "null", "[1]", "{}"])
def test_the_id_of_a_request_is_a_string_or_a_whole_number_and_nothing_else(id: str) -> None:
    line = f'{{"jsonrpc":"2.0","id":{id},"method":"shutdown"}}'

    with pytest.raises(MalformedLineError) as refusal:
        parsed(line.encode())

    assert (refusal.value.code, refusal.value.message, refusal.value.id) == (
        INVALID_REQUEST,
        "The id of a request is a string or a whole number.",
        None,
    )


# What goes out


def test_a_request_that_cannot_be_done_is_refused_with_a_code_and_a_sentence() -> None:
    error = RpcError(NO_SESSION, "No session is open.")

    assert (error.code, error.message, str(error)) == (
        1001,
        "No session is open.",
        "No session is open.",
    )
    assert refused(4, error.code, error.message) == (
        '{"jsonrpc":"2.0","id":4,"error":{"code":1001,"message":"No session is open."}}'
    )


def test_the_answer_to_a_request_names_the_request_and_holds_the_result() -> None:
    assert answered(7, '{"text":"ok"}') == '{"jsonrpc":"2.0","id":7,"result":{"text":"ok"}}'
    assert answered("a-1") == '{"jsonrpc":"2.0","id":"a-1","result":{}}'


def test_an_error_names_the_request_its_code_and_a_sentence() -> None:
    assert json.loads(refused(7, NOT_IDLE, "The agent is not at rest.")) == {
        "jsonrpc": "2.0",
        "id": 7,
        "error": {"code": 1003, "message": "The agent is not at rest."},
    }
    # Where a line let no id be read, the answer names none.
    assert json.loads(refused(None, PARSE_ERROR, "The line is not UTF-8."))["id"] is None


def test_a_notification_has_a_method_and_params_and_no_id() -> None:
    line = told("session.status", '{"status":"idle"}')

    assert line == '{"jsonrpc":"2.0","method":"session.status","params":{"status":"idle"}}'


def test_a_request_of_this_side_has_an_id_of_its_own() -> None:
    assert json.loads(asked(1, "approval/ask", '{"tool":"shell"}')) == {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "approval/ask",
        "params": {"tool": "shell"},
    }


def test_what_goes_out_is_one_line_whatever_it_holds() -> None:
    said = 'two\nlines, a "quote", a \\ and a\ttab\r'

    line = answered(1, text({"text": said}))

    assert "\n" not in line
    assert "\r" not in line
    assert json.loads(line)["result"] == {"text": said}
    assert on_the_wire(line) == line.encode("utf-8") + b"\n"


def test_text_beyond_ascii_goes_out_as_it_is() -> None:
    line = told("session.status", text({"detail": "停止 ✓"}))

    assert '"detail":"停止 ✓"' in line
    assert on_the_wire(line) == line.encode("utf-8") + b"\n"


def test_a_value_is_written_without_room_between_its_parts() -> None:
    assert text({"a": [1, 2], "b": None, "c": True}) == '{"a":[1,2],"b":null,"c":true}'


def test_half_a_character_is_spelled_out_and_the_line_goes_out_all_the_same() -> None:
    said = f"a{HALF}b 停"
    line = answered(1, text({"text": said}))

    sent = on_the_wire(line)

    # Nothing but ASCII, on one line, and the same message to whoever reads it.
    assert sent.endswith(b"\n")
    assert sent.count(b"\n") == 1
    assert sent.isascii()
    assert json.loads(sent) == {"jsonrpc": "2.0", "id": 1, "result": {"text": said}}


_VALUES = st.recursive(
    st.none() | st.booleans() | st.integers() | st.text(),
    lambda inner: st.lists(inner, max_size=3) | st.dictionaries(st.text(), inner, max_size=3),
    max_leaves=8,
)


@given(
    id=st.integers() | st.text(),
    method=st.text(),
    params=st.dictionaries(st.text(), _VALUES, max_size=4),
)
def test_what_this_side_writes_as_a_request_it_reads_as_that_request(
    id: int | str, method: str, params: dict[str, Any]
) -> None:
    sent = on_the_wire(asked(id, method, text(params)))

    assert sent.endswith(b"\n")
    assert sent.count(b"\n") == 1
    assert parsed(sent[:-1]) == Request(id, method, params)


@given(id=st.integers() | st.text(), result=_VALUES)
def test_what_this_side_writes_as_an_answer_it_reads_as_that_answer(
    id: int | str, result: object
) -> None:
    sent = on_the_wire(answered(id, text(result)))

    assert parsed(sent[:-1]) == Response(id, result, failed=False)
