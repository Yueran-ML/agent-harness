"""The line format of a session file."""

import datetime
import json
from typing import ClassVar

import pytest
from pydantic import ConfigDict

from columbina.llm import TextBlock, UserMessage
from columbina.session import (
    CORE_EVENTS,
    EventData,
    SessionEvent,
    SessionHeader,
    TurnStartEvent,
    UnknownEvent,
    UnreadableSessionError,
    UserMessageEvent,
    decode_event,
    decode_header,
    encode_event,
    encode_header,
)


class HintEvent(EventData):
    event_type: ClassVar[str] = "test/hint"
    ignorable: ClassVar[bool] = True

    text: str


KINDS: dict[str, type[EventData]] = {kind.event_type: kind for kind in (*CORE_EVENTS, HintEvent)}

SAID = UserMessageEvent(message=UserMessage(content=(TextBlock(text="你好"),)))
SAID_JSON = '{"message":{"role":"user","content":[{"type":"text","text":"你好"}]},"source":"user"}'

HEADER = SessionHeader(id="s-1", created_at=1_791_234_567_890, cwd="F:\\work")


def test_a_plain_event_is_a_line_with_four_fields() -> None:
    event = SessionEvent(seq=0, time=1000, data=TurnStartEvent(turn=1))

    assert encode_event(event) == '{"type":"turn/start","seq":0,"time":1000,"data":{"turn":1}}'


def test_a_message_says_that_it_is_appended_to_the_surface() -> None:
    event = SessionEvent(seq=3, time=1000, data=SAID, on_surface=True)

    assert encode_event(event) == (
        '{"type":"user/message","seq":3,"time":1000,"data":' + SAID_JSON + ',"surface_op":"append"}'
    )


def test_a_replacement_names_its_range_and_every_event_it_shadows() -> None:
    event = SessionEvent(seq=9, time=1000, data=SAID, on_surface=True, replaces=(3, 4, 6))

    assert json.loads(encode_event(event)) == {
        "type": "user/message",
        "seq": 9,
        "time": 1000,
        "data": json.loads(SAID_JSON),
        "surface_op": {"op": "replace", "start_seq": 3, "end_seq": 6},
        "source_event_seqs": [3, 4, 6],
    }


def test_an_ignorable_event_says_so_on_its_line() -> None:
    event = SessionEvent(seq=1, time=1000, data=HintEvent(text="x"))

    assert encode_event(event) == (
        '{"type":"test/hint","seq":1,"time":1000,"data":{"text":"x"},"ignorable":true}'
    )


@pytest.mark.parametrize(
    "event",
    [
        SessionEvent(seq=0, time=1000, data=TurnStartEvent(turn=1)),
        SessionEvent(seq=3, time=1000, data=SAID, on_surface=True),
        SessionEvent(seq=9, time=1000, data=SAID, on_surface=True, replaces=(3, 4, 6)),
        SessionEvent(seq=9, time=1000, data=SAID, on_surface=True, replaces=(3,)),
        SessionEvent(seq=1, time=1000, data=HintEvent(text="x")),
    ],
)
def test_a_line_reads_back_as_the_event_it_was_written_from(event: SessionEvent[EventData]) -> None:
    assert decode_event(encode_event(event), KINDS) == event


def test_a_line_may_be_given_as_bytes() -> None:
    event = SessionEvent(seq=3, time=1000, data=SAID, on_surface=True)

    assert decode_event(encode_event(event).encode(), KINDS) == event


def test_an_unknown_event_that_may_be_skipped_is_kept_as_it_was_read() -> None:
    line = (
        '{"type":"later/idea","seq":4,"time":1000,'
        '"data":{"n":1,"deep":{"k":[1,2]},"text":"你好"},"ignorable":true}'
    )

    event = decode_event(line, KINDS)

    assert isinstance(event.data, UnknownEvent)
    assert event.type == "later/idea"
    assert event.data.ignorable
    assert not event.on_surface
    assert encode_event(event) == line


def test_two_unknown_events_of_one_type_share_a_class() -> None:
    first = decode_event(
        '{"type":"later/idea","seq":4,"time":1,"data":{"n":1},"ignorable":true}', KINDS
    )
    second = decode_event(
        '{"type":"later/idea","seq":5,"time":2,"data":{"n":2},"ignorable":true}', KINDS
    )
    other = decode_event(
        '{"type":"later/other","seq":6,"time":3,"data":{"n":2},"ignorable":true}', KINDS
    )

    assert type(first.data) is type(second.data)
    assert type(first.data) is not type(other.data)
    assert first.data != second.data


def test_an_unknown_event_that_may_not_be_skipped_is_refused() -> None:
    line = '{"type":"later/idea","seq":4,"time":1000,"data":{}}'

    with pytest.raises(UnreadableSessionError, match="the event type 'later/idea' is not known"):
        decode_event(line, KINDS)


def test_an_unknown_event_cannot_be_part_of_what_the_model_sees() -> None:
    line = (
        '{"type":"later/idea","seq":4,"time":1000,"data":{},"ignorable":true,"surface_op":"append"}'
    )

    with pytest.raises(UnreadableSessionError, match="carries a message and cannot be skipped"):
        decode_event(line, KINDS)


@pytest.mark.parametrize(
    ("line", "complaint"),
    [
        ('{"type":"turn/start","seq":0,"time":1000,"data":{"turn":1}', "is not valid JSON"),
        ("[1, 2]", "is not an event"),
        ('{"type":"turn/start","seq":0,"data":{"turn":1}}', "time"),
        ('{"type":"turn/start","seq":0,"time":1,"data":{"turn":1},"colour":"red"}', "colour"),
        ('{"type":"turn/start","seq":0,"time":1,"data":{"turn":"one"}}', "turn"),
        ('{"type":"turn/start","seq":0,"time":1,"data":{}}', "turn"),
        (
            '{"type":"turn/start","seq":0,"time":1,"data":{"turn":1},"surface_op":"append"}',
            "is marked as a message but holds none",
        ),
        (
            '{"type":"user/message","seq":3,"time":1,"data":'
            + SAID_JSON
            + ',"surface_op":"append","source_event_seqs":[1]}',
            "lists the events it replaces without replacing any",
        ),
        (
            '{"type":"user/message","seq":3,"time":1,"data":'
            + SAID_JSON
            + ',"source_event_seqs":[1]}',
            "lists the events it replaces without replacing any",
        ),
        (
            '{"type":"user/message","seq":3,"time":1,"data":'
            + SAID_JSON
            + ',"surface_op":{"op":"replace","start_seq":1,"end_seq":2}}',
            "does not list the events it replaces",
        ),
        (
            '{"type":"user/message","seq":3,"time":1,"data":'
            + SAID_JSON
            + ',"surface_op":{"op":"replace","start_seq":1,"end_seq":2},"source_event_seqs":[1,3]}',
            "does not match the events it lists",
        ),
    ],
)
def test_a_line_that_is_not_a_well_formed_event_is_refused(line: str, complaint: str) -> None:
    with pytest.raises(UnreadableSessionError, match=complaint):
        decode_event(line, KINDS)


def test_a_header_is_the_first_line_of_a_session_file() -> None:
    assert encode_header(HEADER) == (
        '{"type":"session","version":1,"id":"s-1","created_at":1791234567890,'
        '"cwd":"F:\\\\work","parent_session":null}'
    )
    assert decode_header(encode_header(HEADER)) == HEADER


def test_a_header_can_name_the_session_it_was_forked_from() -> None:
    header = SessionHeader(id="s-2", created_at=1, cwd="/work", parent_session="s-1")

    assert decode_header(encode_header(header)) == header


@pytest.mark.parametrize(
    ("line", "complaint"),
    [
        (
            '{"type":"session","version":2,"id":"s-1","created_at":1,"cwd":"/w","parent_session":null}',
            "was written in format version 2, and this program reads version 1",
        ),
        (
            '{"type":"session","version":0,"id":"s-1","created_at":1,"cwd":"/w","parent_session":null}',
            "was written in format version 0, and this program reads version 1",
        ),
        ('{"type":"turn/start","seq":0,"time":1000,"data":{"turn":1}}', "is not a session header"),
        ('{"type":"session","version":1,"id":"s-1"}', "is not a session header"),
        ("not json", "is not a session header"),
        ('{"version":"new"}', "is not a session header"),
    ],
)
def test_a_first_line_that_is_not_a_header_of_this_format_is_refused(
    line: str, complaint: str
) -> None:
    with pytest.raises(UnreadableSessionError, match=complaint):
        decode_header(line)


def test_an_event_with_strict_data_reads_back() -> None:
    class Measured(EventData):
        model_config = ConfigDict(frozen=True, extra="forbid", strict=True)

        event_type: ClassVar[str] = "test/measured"

        values: tuple[int, ...]
        on: datetime.date

    event = SessionEvent(seq=0, time=1, data=Measured(values=(1, 2), on=datetime.date(2026, 10, 9)))

    assert decode_event(encode_event(event), {"test/measured": Measured}) == event


def test_an_unknown_event_is_written_back_exactly_as_it_was_read() -> None:
    line = (
        '{"type":"later/idea", "seq":4,"time":1000,'
        '"data":{"word":"\\u0061","big":12345678901234567890123,"twice":1,"twice":2},'
        '"ignorable":true}'
    )

    event = decode_event(line, KINDS)

    assert encode_event(event) == line
    assert encode_event(decode_event(line.encode(), KINDS)) == line
