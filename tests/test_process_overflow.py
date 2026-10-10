"""The whole of what a program writes, kept somewhere else once it is more than memory keeps."""

import logging
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.process import HeadTail
from support.processes import python

pytestmark = pytest.mark.timeout(60)

LINES = 20_000
NUMBERS = (
    f"import sys\nfor number in range({LINES}):\n    sys.stdout.buffer.write(b'%07d\\n' % number)\n"
)
"""Writes twenty thousand numbers, one to a line: 160000 bytes."""
WHOLE = b"".join(b"%07d\n" % number for number in range(LINES))


def noted(caplog: pytest.LogCaptureFixture) -> list[logging.LogRecord]:
    """What running a program wrote into the log."""
    return [record for record in caplog.records if record.name == "columbina.process"]


def writes(size: int) -> list[str]:
    """A program that writes so many bytes and ends."""
    return python(f"import sys; sys.stdout.buffer.write(b'x' * {size})")


class Elsewhere:
    """Somewhere to write to. It remembers what it was given, and can be full.

    A piece that it has no room for is taken as far as there is room, the way a disk
    takes it, and then refused.
    """

    def __init__(self, room: int | None = None, fault: Exception | None = None) -> None:
        self.written = bytearray()
        self.room = room
        self.fault = fault
        self.asked = 0
        self.whole_pieces = 0

    def write(self, data: bytes) -> None:
        self.asked += 1
        if self.fault is not None:
            raise self.fault
        if self.room is not None and len(self.written) + len(data) > self.room:
            self.written += data[: self.room - len(self.written)]
            raise OSError(28, "No space left on device")
        self.written += data
        self.whole_pieces += 1


class Opened:
    """Hands out places to write to, and keeps them."""

    def __init__(self, room: int | None = None, fault: Exception | None = None) -> None:
        self.places: list[Elsewhere] = []
        self.room = room
        self.fault = fault

    def __call__(self) -> Elsewhere:
        self.places.append(Elsewhere(self.room, self.fault))
        return self.places[-1]


async def test_output_that_memory_keeps_whole_goes_nowhere_else(tmp_path: Path) -> None:
    opened = Opened()

    done = await process.run(
        writes(2000), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )

    assert opened.places == []
    assert done.saved_bytes == 0
    assert done.output_bytes == 2000
    assert done.output == "x" * 2000


async def test_one_byte_more_than_memory_keeps_and_all_of_it_goes_somewhere_else(
    tmp_path: Path,
) -> None:
    opened = Opened()

    done = await process.run(
        writes(2001), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )

    (place,) = opened.places
    assert bytes(place.written) == b"x" * 2001
    assert done.saved_bytes == 2001
    assert done.output_bytes == 2001
    assert done.omitted_bytes == 1


async def test_the_whole_of_a_long_output_is_written_in_the_order_it_came(tmp_path: Path) -> None:
    opened = Opened()

    done = await process.run(
        python(NUMBERS), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )

    # One place, however many pieces the output came in, and nothing twice or missing.
    (place,) = opened.places
    assert bytes(place.written) == WHOLE
    assert done.saved_bytes == len(WHOLE)
    assert done.output_bytes == len(WHOLE)
    assert done.omitted_bytes == len(WHOLE) - 2000
    assert done.output.startswith("0000000\n0000001\n")
    assert done.output.endswith("0019999\n")
    assert done.exit_code == 0


async def test_without_a_place_for_it_a_long_output_is_kept_by_its_ends_only(
    tmp_path: Path,
) -> None:
    done = await process.run(
        python(NUMBERS), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000
    )

    assert done.saved_bytes == 0
    assert done.output_bytes == len(WHOLE)
    assert done.omitted_bytes == len(WHOLE) - 2000


async def test_no_more_is_written_somewhere_else_than_there_is_room_for(tmp_path: Path) -> None:
    opened = Opened()

    done = await process.run(
        python(NUMBERS),
        cwd=tmp_path,
        timeout_seconds=30,
        max_output_bytes=2000,
        overflow=opened,
        max_overflow_bytes=50_001,
    )

    (place,) = opened.places
    assert bytes(place.written) == WHOLE[:50_001]
    assert done.saved_bytes == 50_001
    # The program is not stopped for it, and what memory keeps is as it would be anyway.
    assert done.exit_code == 0
    assert done.output_bytes == len(WHOLE)
    assert done.output.endswith("0019999\n")


async def test_room_for_exactly_what_was_written_is_room_enough(tmp_path: Path) -> None:
    opened = Opened()

    done = await process.run(
        writes(5000),
        cwd=tmp_path,
        timeout_seconds=30,
        max_output_bytes=2000,
        overflow=opened,
        max_overflow_bytes=5000,
    )

    (place,) = opened.places
    assert bytes(place.written) == b"x" * 5000
    assert done.saved_bytes == done.output_bytes == 5000


async def test_a_limit_of_less_than_nothing_is_refused_before_anything_runs(
    tmp_path: Path,
) -> None:
    with pytest.raises(ValueError, match="written elsewhere is -1"):
        await process.run(
            writes(1), cwd=tmp_path, timeout_seconds=30, overflow=Opened(), max_overflow_bytes=-1
        )


async def test_a_place_that_cannot_be_opened_does_not_keep_the_program_from_running(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    asked: list[str] = []

    def nowhere() -> Elsewhere:
        asked.append("asked")
        raise PermissionError(13, "Permission denied")

    done = await process.run(
        python(NUMBERS), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=nowhere
    )

    assert asked == ["asked"]
    assert done.saved_bytes == 0
    assert done.exit_code == 0
    assert done.output.startswith("0000000\n")
    assert done.output.endswith("0019999\n")
    assert done.omitted_bytes == len(WHOLE) - 2000
    (record,) = noted(caplog)
    assert "the whole output of a program cannot be kept" in record.getMessage()
    assert "Permission denied" in record.getMessage()
    # What the system refuses is no fault of this program, so no trace of one is written.
    assert record.exc_info is None


async def test_a_place_that_fills_up_keeps_what_it_took_and_the_program_runs_on(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    opened = Opened(room=40_000)

    done = await process.run(
        python(NUMBERS), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )

    (place,) = opened.places
    assert bytes(place.written) == WHOLE[:40_000]
    # Only a piece that it took whole is counted. How large a piece is, the system decides.
    assert done.saved_bytes <= 40_000
    assert done.exit_code == 0
    assert done.output_bytes == len(WHOLE)
    assert done.output.endswith("0019999\n")
    # It is asked once more than it took, and then left alone.
    assert place.asked == place.whole_pieces + 1
    (record,) = noted(caplog)
    assert "the whole output of a program could not be written" in record.getMessage()
    assert "No space left" in record.getMessage()
    assert record.exc_info is None


@pytest.mark.parametrize("failing", ["to open", "to write"])
async def test_a_place_that_is_at_fault_is_given_up_and_nothing_of_the_output_is_lost(
    tmp_path: Path, caplog: pytest.LogCaptureFixture, failing: str
) -> None:
    fault = RuntimeError("a fault of the place")
    opened = Opened(fault=fault)

    def faulty() -> Elsewhere:
        raise fault

    done = await process.run(
        python(NUMBERS),
        cwd=tmp_path,
        timeout_seconds=30,
        max_output_bytes=2000,
        overflow=faulty if failing == "to open" else opened,
    )

    assert done.saved_bytes == 0
    assert done.exit_code == 0
    assert done.output_bytes == len(WHOLE)
    assert done.output.startswith("0000000\n")
    assert done.output.endswith("0019999\n")
    assert [place.asked for place in opened.places] == ([] if failing == "to open" else [1])
    # It is written down once, with where it came from: this is a fault to be found.
    (record,) = noted(caplog)
    assert "a fault of the place" in record.getMessage()
    assert record.exc_info is not None
    assert record.exc_info[1] is fault


@pytest.mark.parametrize("size", [10, 2001])
async def test_nothing_is_written_somewhere_else_once_the_run_is_over(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, size: int
) -> None:
    watches: list[Any] = []
    make: Any = vars(process)["_Watch"]

    def remembered(kept: Any) -> Any:
        watches.append(make(kept))
        return watches[-1]

    monkeypatch.setattr(process, "_Watch", remembered)
    opened = Opened()

    await process.run(
        writes(size), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )
    # A program that was left behind may write on. Whoever opened the place has closed it.
    written = [bytes(place.written) for place in opened.places]
    watches[0].pipe_data_received(1, b"y" * 5000)

    assert [bytes(place.written) for place in opened.places] == written
    assert written == ([] if size == 10 else [b"x" * 2001])


async def test_what_memory_had_kept_until_then_is_the_first_thing_to_go_somewhere_else(
    tmp_path: Path,
) -> None:
    # The first piece fits and is in memory alone. The place is opened when the second comes.
    in_two = (
        "import sys, time\n"
        "sys.stdout.buffer.write(b'a' * 1500)\n"
        "sys.stdout.flush()\n"
        "time.sleep(1)\n"
        "sys.stdout.buffer.write(b'b' * 1500)\n"
    )
    opened = Opened()

    done = await process.run(
        python(in_two), cwd=tmp_path, timeout_seconds=30, max_output_bytes=2000, overflow=opened
    )

    (place,) = opened.places
    assert bytes(place.written) == b"a" * 1500 + b"b" * 1500
    assert done.saved_bytes == done.output_bytes == 3000


def test_the_pieces_go_to_the_place_in_the_order_in_which_they_came() -> None:
    # Without a program and a pipe, whose pieces are of a size that nobody can count on.
    opened = Opened()
    output: Any = vars(process)["_Output"](2000, opened, 10_000)

    output.add(b"a" * 1500)
    nothing_yet = list(opened.places)
    output.add(b"b" * 1500)
    output.add(b"c" * 100)

    assert nothing_yet == []
    (place,) = opened.places
    # What memory held is handed over first, as one piece, and then each piece as it comes.
    assert bytes(place.written) == b"a" * 1500 + b"b" * 1500 + b"c" * 100
    assert place.asked == 3
    assert (output.saved, output.kept.total) == (3100, 3100)


def test_what_is_kept_of_an_output_that_fits_is_all_of_it() -> None:
    kept = HeadTail(10)
    kept.add(b"abc")
    kept.add(b"defg")

    assert kept.limit == 10
    assert kept.held() == b"abcdefg"
    kept.add(b"hij")
    assert kept.held() == b"abcdefghij"
