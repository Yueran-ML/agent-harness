"""The frames of the long connection to Feishu: ``columbina.feishu.frames``."""

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.feishu.frames import CONTROL, DATA, Frame, FrameError, decoded, encoded

PING = Frame(service=12345, headers=(("type", "ping"),))
PING_BYTES = bytes.fromhex("0800 1000 18b960 2000 2a0c 0a0474797065 120470696e67".replace(" ", ""))
"""A ping as Protocol Buffers write it: the four numbers, then the header in its own message."""


def test_a_ping_is_written_as_protocol_buffers_write_it() -> None:
    assert encoded(PING) == PING_BYTES


def test_a_ping_is_read_back() -> None:
    frame = decoded(PING_BYTES)

    assert frame == PING
    assert frame.method == CONTROL
    assert frame.payload is None


def test_the_fields_that_may_be_left_out_are_written_only_where_they_are_given() -> None:
    frame = Frame(
        seq_id=3,
        log_id=7,
        service=1,
        method=DATA,
        payload_encoding="",
        payload_type="json",
        payload=b"{}",
        log_id_new="n",
    )

    assert encoded(frame) == bytes.fromhex(
        "0803 1007 1801 2001 3200 3a046a736f6e 42027b7d 4a016e".replace(" ", "")
    )
    assert decoded(encoded(frame)) == frame


def test_an_empty_payload_is_one_that_is_there() -> None:
    frame = Frame(payload=b"")

    assert decoded(encoded(frame)).payload == b""


numbers = st.integers(min_value=0, max_value=(1 << 64) - 1)
signed = st.integers(min_value=-(1 << 31), max_value=(1 << 31) - 1)
texts = st.text(max_size=20)


@given(
    seq_id=numbers,
    log_id=numbers,
    service=signed,
    method=signed,
    headers=st.lists(st.tuples(texts, texts), max_size=5),
    payload_encoding=st.none() | texts,
    payload_type=st.none() | texts,
    payload=st.none() | st.binary(max_size=64),
    log_id_new=st.none() | texts,
)
def test_every_frame_is_read_back_as_it_was_written(
    seq_id: int,
    log_id: int,
    service: int,
    method: int,
    headers: list[tuple[str, str]],
    payload_encoding: str | None,
    payload_type: str | None,
    payload: bytes | None,
    log_id_new: str | None,
) -> None:
    frame = Frame(
        seq_id=seq_id,
        log_id=log_id,
        service=service,
        method=method,
        headers=tuple(headers),
        payload_encoding=payload_encoding,
        payload_type=payload_type,
        payload=payload,
        log_id_new=log_id_new,
    )

    assert decoded(encoded(frame)) == frame


def test_a_number_below_zero_takes_ten_bytes() -> None:
    written = encoded(Frame(service=-1))

    assert bytes.fromhex("18ffffffffffffffffff01") in written
    assert decoded(written).service == -1


def test_a_field_that_is_not_known_is_passed_over() -> None:
    unknown = bytes.fromhex(
        # 10, a number; 11, four bytes; 12, eight bytes; 13, a run of two bytes.
        "5001 5d01020304 610102030405060708 6a02abcd".replace(" ", "")
    )

    assert decoded(PING_BYTES + unknown) == PING


def test_a_known_field_written_in_another_way_than_its_kind_is_passed_over() -> None:
    # The number of field 1 as a run of bytes, and field 8, the payload, as a number.
    other_ways = bytes.fromhex("0a0101 4001")

    assert decoded(PING_BYTES + other_ways) == PING


def test_a_field_of_a_header_that_is_not_known_is_passed_over() -> None:
    header = bytes.fromhex("0a0161 120162 1801".replace(" ", ""))
    frame = bytes.fromhex("0800 1000 1800 2000".replace(" ", "")) + bytes([0x2A, len(header)])

    assert decoded(frame + header).headers == (("a", "b"),)


def test_of_a_field_that_is_given_twice_the_last_one_counts() -> None:
    assert decoded(PING_BYTES + bytes.fromhex("0809")).seq_id == 9


def test_the_header_of_a_key_is_the_first_one() -> None:
    frame = Frame(headers=(("sum", "2"), ("seq", "0"), ("sum", "3")))

    assert frame.header("sum") == "2"
    assert frame.header("trace_id") is None


def test_a_header_is_added_after_the_others() -> None:
    frame = Frame(headers=(("type", "event"),)).with_header("biz_rt", "12")

    assert frame.headers == (("type", "event"), ("biz_rt", "12"))


@pytest.mark.parametrize(
    ("written", "said"),
    [
        pytest.param("0880", "a number runs past the end", id="a number cut off"),
        pytest.param("08" + "80" * 10 + "01", "takes more than 10 bytes", id="a number too long"),
        pytest.param("2a05 0a01", "runs past the end", id="a run of bytes cut off"),
        pytest.param("5d0102", "runs past the end", id="four bytes cut off"),
        pytest.param("61010203", "runs past the end", id="eight bytes cut off"),
        pytest.param("0b", "that is not used", id="a group"),
        pytest.param("0700", "has the number 0", id="a field numbered 0"),
        pytest.param("0800 1000 1801", "has no method", id="a number left out"),
        pytest.param("", "no SeqID and no LogID and no service and no method", id="nothing"),
        pytest.param("2a03 0a0174", "a header of the frame has no key or no value", id="a header"),
        pytest.param("0800 1000 1801 2000 3201ff", "not UTF-8", id="a text that is no text"),
    ],
)
def test_what_is_no_frame_is_said_to_be_none(written: str, said: str) -> None:
    with pytest.raises(FrameError, match=said):
        decoded(bytes.fromhex(written.replace(" ", "")))
