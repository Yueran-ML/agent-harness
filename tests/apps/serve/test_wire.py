"""The pipe that a service process speaks through, over real pipes of the system."""

import asyncio
import contextlib
import os
import subprocess
import sys
import threading
from collections.abc import AsyncIterator
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

import pytest

from columbina.apps.serve import wire as wire_module
from columbina.apps.serve.wire import Wire, keep, over
from support.terminal import until

NEWLINE = bytes([10])


def closed(descriptor: int) -> None:
    """Close a descriptor, which may be closed already."""
    with contextlib.suppress(OSError):
        os.close(descriptor)


def reading() -> set[threading.Thread]:
    """The threads that read for a wire at this moment."""
    return {thread for thread in threading.enumerate() if thread.name == "serve-in"}


def writing() -> set[threading.Thread]:
    """The threads that write for a wire at this moment."""
    return {thread for thread in threading.enumerate() if thread.name == "serve-out"}


def drained(descriptor: int) -> None:
    """Read a descriptor to its end and throw away what comes."""
    with contextlib.suppress(OSError):
        while os.read(descriptor, 1 << 16):
            pass


class Listener:
    """The other side of the way out: a thread that reads what a wire writes, to its end."""

    def __init__(self, descriptor: int, *, at_once: bool = True) -> None:
        self._descriptor = descriptor
        self.heard = b""
        self.ended = threading.Event()
        self._thread = threading.Thread(target=self._listen, daemon=True)
        if at_once:
            self._thread.start()

    def begin(self) -> None:
        self._thread.start()

    def _listen(self) -> None:
        while True:
            try:
                chunk = os.read(self._descriptor, 1 << 16)
            except OSError:
                chunk = b""
            if not chunk:
                self.ended.set()
                return
            self.heard += chunk

    async def to_the_end(self) -> bytes:
        """Return everything that was written, once the way out was closed."""
        await asyncio.wait_for(asyncio.to_thread(self.ended.wait, 10), 12)
        assert self.ended.is_set()
        return self.heard


@dataclass
class Ends:
    """A wire, and the far ends of its two pipes.

    Attributes:
        wire: The wire.
        far_in: Where a test writes what the wire is to read.
        far_out: Where what the wire writes can be read.
        writer: The thread that writes for the wire.
    """

    wire: Wire
    far_in: int
    far_out: int
    writer: threading.Thread
    opened: list[int] = field(default_factory=list[int])

    def say(self, data: bytes) -> None:
        os.write(self.far_in, data)

    def hang_up(self) -> None:
        closed(self.far_in)

    async def read(self, at_most: int | None = None) -> list[bytes | None]:
        """Return the lines that the wire gives: all of them, or the first few."""
        got: list[bytes | None] = []
        async with asyncio.timeout(10):
            async for line in self.wire.lines():
                got.append(line)
                if at_most is not None and len(got) == at_most:
                    break
        return got


@pytest.fixture
async def make() -> AsyncIterator["Maker"]:
    maker = Maker()
    yield maker
    for ends in maker.made:
        # Each end is closed only when no thread can be waiting at it any more. On Windows,
        # closing a descriptor waits for whoever reads from it or writes to it at that moment.
        near_in, near_out = ends.opened
        # Nothing more comes in, so the thread that reads for the wire comes to its end.
        closed(ends.far_in)
        # What was sent is written out, and the thread that writes comes to its end. A wire
        # that is broken may never come back from this, and is not waited for for good.
        with contextlib.suppress(TimeoutError):
            async with asyncio.timeout(10):
                await ends.wire.close()
        drain = None
        if ends.writer.is_alive():
            # A test that failed left the thread waiting for somebody to read. What it has
            # is read and thrown away, so that it lets go of its descriptor. Closing that
            # under it would wait for it for good, and nothing would end the run then: the
            # limit that a test is given is taken off the moment the test has failed.
            drain = threading.Thread(target=drained, args=(ends.far_out,), daemon=True)
            drain.start()
            with contextlib.suppress(TimeoutError):
                await until(lambda writer=ends.writer: not writer.is_alive(), 10)
        # Whoever listens on the far side hears the end and stops reading.
        closed(near_out)
        if drain is not None:
            with contextlib.suppress(TimeoutError):
                await until(lambda drain=drain: not drain.is_alive(), 10)
        closed(ends.far_out)
        closed(near_in)


class Maker:
    """Makes wires over pipes, and remembers them so that they can be put away."""

    def __init__(self) -> None:
        self.made: list[Ends] = []

    def __call__(self, limit: int | None = None, backlog: int | None = None) -> Ends:
        near_in, far_in = os.pipe()
        far_out, near_out = os.pipe()
        chosen = {name: value for name, value in (("limit", limit), ("backlog", backlog)) if value}
        before = writing()
        wire = over(near_in, near_out, **chosen)
        (writer,) = writing() - before
        ends = Ends(wire, far_in, far_out, writer, [near_in, near_out])
        self.made.append(ends)
        return ends


# What comes in


async def test_lines_come_in_one_by_one_without_their_line_breaks(make: Maker) -> None:
    ends = make()

    ends.say(b"one\ntwo\r\n\nfour\n")
    ends.hang_up()

    # A line break of Windows is a line break. A line of nothing is a line.
    assert await ends.read() == [b"one", b"two", b"", b"four"]


async def test_a_line_that_comes_in_pieces_is_one_line(make: Maker) -> None:
    ends = make()

    ends.say(b'{"jsonrpc":')
    await asyncio.sleep(0.05)
    ends.say(b'"2.0"}\nnext')
    await asyncio.sleep(0.05)
    ends.say(b"\n")

    assert await ends.read(2) == [b'{"jsonrpc":"2.0"}', b"next"]


async def test_the_lines_end_when_the_other_side_closes(make: Maker) -> None:
    ends = make()

    ends.hang_up()

    assert await ends.read() == []


async def test_a_last_line_that_was_not_ended_is_a_line_all_the_same(make: Maker) -> None:
    ends = make()

    ends.say(b"one\nlast\r")
    ends.hang_up()

    assert await ends.read() == [b"one", b"last"]


async def test_a_long_line_comes_in_whole(make: Maker) -> None:
    ends = make()
    long = b"y" * 3_000_000

    # Whoever writes that much waits until it is read, so it is written on the side.
    writing = asyncio.ensure_future(asyncio.to_thread(ends.say, long + b"\nshort\n"))
    lines = await ends.read(2)
    await asyncio.wait_for(writing, 10)

    assert [len(line or b"") for line in lines] == [3_000_000, 5]
    assert lines[0] == long


async def test_bytes_that_mean_something_to_a_text_file_are_bytes_like_any_others(
    make: Maker,
) -> None:
    ends = make()
    # The end of a text file of old, a carriage return in the middle, and a nothing.
    line = b"a\x1ab\rc\x00d"

    ends.say(line + b"\n")

    assert await ends.read(1) == [line]


@pytest.mark.parametrize(
    ("sent", "lines"),
    [
        (b"x" * 100 + b"\nok\n", [b"x" * 100, b"ok"]),
        (b"x" * 101 + b"\nok\n", [None, b"ok"]),
        (b"x" * 250 + b"\n" + b"y" * 300 + b"\nok\n", [None, None, b"ok"]),
        (b"ok\n" + b"x" * 101 + b"\n", [b"ok", None]),
        # The line break of Windows counts until the line is told from one that is too long.
        (b"x" * 100 + b"\r\nok\n", [None, b"ok"]),
    ],
    ids=["as long as may be", "one byte more", "two in a row", "at the end", "with a return"],
)
async def test_a_line_that_is_longer_than_the_wire_takes_is_dropped_and_the_next_one_is_read(
    make: Maker, sent: bytes, lines: list[bytes | None]
) -> None:
    ends = make(limit=100)

    ends.say(sent)
    ends.hang_up()

    assert await ends.read() == lines


async def test_a_line_that_is_too_long_is_passed_over_however_many_pieces_it_comes_in(
    make: Maker, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(wire_module, "_READ_BYTES", 64)
    ends = make(limit=100)

    # Far more than the wire reads at once: its rest is passed over as it comes.
    ends.say(b"x" * 1000 + b"\nok\n")
    ends.hang_up()

    assert await ends.read() == [None, b"ok"]


async def test_a_line_that_is_too_long_and_never_ends_is_said_to_be_one(make: Maker) -> None:
    ends = make(limit=100)

    ends.say(b"ok\n" + b"x" * 500)
    ends.hang_up()

    assert await ends.read() == [b"ok", None]


async def test_a_way_in_that_cannot_be_read_is_one_that_has_ended() -> None:
    near_in, far_in = os.pipe()
    far_out, near_out = os.pipe()
    # Nothing is opened between this and the wire, so the number stays what it is: closed.
    os.close(near_in)
    wire = over(near_in, near_out)
    try:
        async with asyncio.timeout(10):
            assert [line async for line in wire.lines()] == []
    finally:
        await wire.close()
        for descriptor in (far_in, far_out, near_out):
            closed(descriptor)


def test_a_wire_whose_loop_has_ended_hands_nothing_more_to_it() -> None:
    near_in, far_in = os.pipe()
    far_out, near_out = os.pipe()
    before = reading()
    before_writing = writing()
    loop = asyncio.new_event_loop()
    try:

        async def made() -> Wire:
            return over(near_in, near_out)

        wire = loop.run_until_complete(made())
    finally:
        loop.close()
    (reader,) = reading() - before
    (writer,) = writing() - before_writing
    try:
        # There is nobody to hand a line to. The thread that reads stops at the first one,
        # and fails nobody.
        os.write(far_in, b"one" + NEWLINE + b"two" + NEWLINE)
        reader.join(10)
        assert not reader.is_alive()
        # The same goes for the thread that writes, once it finds the other side gone.
        os.close(far_out)
        wire.send("anybody?")
        writer.join(10)
        assert not writer.is_alive()
    finally:
        for descriptor in (far_in, far_out, near_in, near_out):
            closed(descriptor)


# What goes out


async def test_what_is_sent_goes_out_a_line_each_in_the_order_it_was_sent(make: Maker) -> None:
    ends = make()
    listener = Listener(ends.far_out)

    ends.wire.send('{"n":1}')
    ends.wire.send('{"text":"停 ✓"}')
    ends.wire.send('{"n":3}')
    await ends.wire.close()

    assert await listener.to_the_end() == '{"n":1}\n{"text":"停 ✓"}\n{"n":3}\n'.encode()


async def test_sending_does_not_wait_for_an_other_side_that_reads_late(make: Maker) -> None:
    ends = make()
    listener = Listener(ends.far_out, at_once=False)
    long = "z" * 2_000_000

    # Nobody reads yet, and a pipe takes far less than this. Sending comes back all the same.
    ends.wire.send(long)
    ends.wire.send("after")
    await asyncio.sleep(0.05)
    listener.begin()
    await ends.wire.close()

    assert await listener.to_the_end() == (long + "\nafter\n").encode()


async def test_closing_a_second_time_does_nothing(make: Maker) -> None:
    ends = make()
    listener = Listener(ends.far_out)

    ends.wire.send("one")
    await ends.wire.close()
    await ends.wire.close()
    # What is sent after the wire was closed goes nowhere, and fails nobody.
    ends.wire.send("late")

    assert await listener.to_the_end() == b"one\n"


async def test_closing_does_not_wait_for_good_for_an_other_side_that_does_not_read(
    make: Maker, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(wire_module, "_LAST_WORDS_SECONDS", 0.2)
    ends = make()

    ends.wire.send("z" * 2_000_000)
    async with asyncio.timeout(5):
        await ends.wire.close()

    # The way out was left open for the thread that still writes. Once somebody reads,
    # everything arrives after all.
    listener = Listener(ends.far_out)
    await until(lambda: len(listener.heard) == 2_000_001, 10)
    closed(ends.opened[1])
    assert await listener.to_the_end() == b"z" * 2_000_000 + NEWLINE


async def test_closing_needs_no_thread_of_the_pool(make: Maker) -> None:
    ends = make()
    listener = Listener(ends.far_out)
    loop = asyncio.get_running_loop()
    # The pool has one thread, and it is stuck, as one that reads a file that never comes is.
    stuck = threading.Event()
    loop.set_default_executor(ThreadPoolExecutor(max_workers=1))
    waiting = loop.run_in_executor(None, stuck.wait)
    try:
        ends.wire.send("last words")
        async with asyncio.timeout(3):
            await ends.wire.close()
    finally:
        stuck.set()
        await waiting

    assert await listener.to_the_end() == b"last words" + NEWLINE


@pytest.mark.parametrize("line", ["z" * 299_999, "\u6708" * 100_000], ids=["ascii", "beyond"])
async def test_an_other_side_that_lets_too_much_pile_up_is_taken_for_gone(
    make: Maker, monkeypatch: pytest.MonkeyPatch, line: str
) -> None:
    monkeypatch.setattr(wire_module, "_LAST_WORDS_SECONDS", 0.2)
    size = len(line.encode()) + 1
    # Two lines may wait, counted in bytes as they are written, and not one byte more.
    ends = make(backlog=2 * size)

    # Nobody reads. The first line is taken to be written. It is longer than any pipe
    # holds, so it does not get through, and it still counts.
    ends.wire.send(line)
    await asyncio.sleep(0.1)
    # The second fills what may wait to the brim, and the third is too much.
    ends.wire.send(line)
    ends.wire.send(line)

    # Whoever waits for lines hears that the other side is gone, which is the order to leave.
    assert await ends.read() == []
    ends.wire.send("no more")
    async with asyncio.timeout(5):
        await ends.wire.close()
    # What was kept is whole lines: the one that was too much was dropped, not cut.
    listener = Listener(ends.far_out)
    await until(lambda: len(listener.heard) == 2 * size, 10)
    closed(ends.opened[1])
    assert await listener.to_the_end() == (line.encode() + NEWLINE) * 2


async def test_what_was_written_out_does_not_count_as_piled_up(make: Maker) -> None:
    ends = make(backlog=150_000)
    listener = Listener(ends.far_out)
    line = "z" * 59_999

    # Twice what may pile up, to an other side that reads each line as it comes. A line
    # that was read may not be taken off the count yet when the next one is sent, so there
    # is room for two.
    for sent in range(1, 6):
        ends.wire.send(line)
        await until(lambda sent=sent: len(listener.heard) == sent * 60_000, 10)
    ends.wire.send("still here")
    ends.say(b"and you?" + NEWLINE)

    assert await ends.read(1) == [b"and you?"]
    await ends.wire.close()
    assert (await listener.to_the_end()).endswith(b"z" + NEWLINE + b"still here" + NEWLINE)


async def test_an_other_side_that_is_gone_ends_the_lines_and_takes_nothing_more(
    make: Maker,
) -> None:
    ends = make()

    # The way in is still open. The way out leads to nobody.
    closed(ends.far_out)
    ends.wire.send("anybody?")

    assert await ends.read() == []
    ends.wire.send("no")
    await ends.wire.close()


# What a process keeps for itself


def test_what_a_process_keeps_is_read_and_written_by_its_copies_alone() -> None:
    source, far_in = os.pipe()
    far_out, sink = os.pipe()
    far_errors, errors = os.pipe()
    kept: tuple[int, ...] = ()
    try:
        kept = incoming, outgoing = keep(source, sink, errors)

        # Whatever writes where the answers went writes into the log, and the copy is the
        # way out: of the two that are written here, only the answer goes that way.
        os.write(sink, b"a stray word" + NEWLINE)
        os.write(outgoing, b"answer" + NEWLINE)
        assert os.read(far_out, 100) == b"answer" + NEWLINE
        assert os.read(far_errors, 100) == b"a stray word" + NEWLINE
        # Whatever reads where the requests came finds nothing, at once, although a
        # request waits. The copy is the way in.
        os.write(far_in, b"request" + NEWLINE)
        assert os.read(source, 100) == b""
        assert os.read(incoming, 100) == b"request" + NEWLINE
        # A program that is started inherits neither copy.
        assert (os.get_inheritable(incoming), os.get_inheritable(outgoing)) == (False, False)
    finally:
        for descriptor in (source, far_in, far_out, sink, far_errors, errors, *kept):
            closed(descriptor)


def test_a_process_without_a_log_gets_one_that_leads_nowhere() -> None:
    source, far_in = os.pipe()
    far_out, sink = os.pipe()
    far_errors, errors = os.pipe()
    os.close(errors)
    kept: tuple[int, ...] = ()
    try:
        kept = incoming, outgoing = keep(source, sink, errors)

        # The number of the log was free, and the system gives a copy the lowest number
        # that is. Neither copy has it: something that leads nowhere stands there now.
        assert errors not in (incoming, outgoing)
        assert os.get_inheritable(errors)
        os.write(errors, b"a word for the log" + NEWLINE)
        os.write(sink, b"a stray word" + NEWLINE)
        os.write(outgoing, b"answer" + NEWLINE)
        # Only the answer went the way of the answers.
        assert os.read(far_out, 100) == b"answer" + NEWLINE
        os.write(far_in, b"request" + NEWLINE)
        assert os.read(incoming, 100) == b"request" + NEWLINE
    finally:
        for descriptor in (source, far_in, far_out, sink, far_errors, errors, *kept):
            closed(descriptor)


def next_free() -> int:
    """Return the number that the system would give the next descriptor."""
    probe = os.open(os.devnull, os.O_RDONLY)
    os.close(probe)
    return probe


@pytest.mark.parametrize("which", ["source", "sink"])
def test_a_process_without_a_way_in_or_a_way_out_keeps_nothing(which: str) -> None:
    source, far_in = os.pipe()
    far_out, sink = os.pipe()
    far_errors, errors = os.pipe()
    # Its number is free now, and the first copy would be given it.
    os.close(source if which == "source" else sink)
    free = next_free()
    try:
        # What the system says of a descriptor that is not open differs from one to the next.
        with pytest.raises(OSError):  # noqa: PT011
            keep(source, sink, errors)

        # Nothing was copied, and nothing was put in the place of anything.
        assert next_free() == free
        os.write(sink if which == "source" else far_in, b"as it was" + NEWLINE)
        assert os.read(far_out if which == "source" else source, 100) == b"as it was" + NEWLINE
    finally:
        for descriptor in (source, far_in, far_out, sink, far_errors, errors):
            closed(descriptor)


def test_a_copy_that_cannot_be_made_leaves_the_one_before_it_closed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source, far_in = os.pipe()
    far_out, sink = os.pipe()
    far_errors, errors = os.pipe()
    copy = os.dup
    made: list[int] = []

    def once(descriptor: int) -> int:
        if made:
            raise OSError(24, "Too many open files")
        made.append(copy(descriptor))
        return made[0]

    monkeypatch.setattr(os, "dup", once)
    try:
        with pytest.raises(OSError, match="Too many open files"):
            keep(source, sink, errors)

        monkeypatch.undo()
        # The copy that was made is closed again, and the way out is still the way out.
        with pytest.raises(OSError):  # noqa: PT011
            os.fstat(made[0])
        os.write(sink, b"as it was" + NEWLINE)
        assert os.read(far_out, 100) == b"as it was" + NEWLINE
    finally:
        for descriptor in (source, far_in, far_out, sink, far_errors, errors):
            closed(descriptor)


# A real process

CHILD = """
import asyncio, os, subprocess, sys
from columbina.apps.serve.wire import taken

async def main():
    wire = taken()
    async for line in wire.lines():
        if line == b"stray":
            print("printed")
            os.write(1, b"written to the standard output" + os.linesep.encode())
            subprocess.run([sys.executable, "-c", "print('said by a program that was started')"])
            wire.send("strayed")
        elif line == b"heirs":
            def inherited(descriptor):
                try:
                    return os.get_inheritable(descriptor)
                except OSError:
                    return False
            wire.send(f"heirs {[one for one in range(3, 64) if inherited(one)]}")
        elif line == b"steal":
            read = subprocess.run(
                [sys.executable, "-c", "import sys; print(len(sys.stdin.read()))"],
                stdout=subprocess.PIPE,
            )
            wire.send("stolen " + read.stdout.decode().strip())
        else:
            wire.send(f"echo {len(line)}")
    await wire.close()

asyncio.run(main())
"""


def test_a_process_keeps_its_standard_input_and_output_for_its_wire() -> None:
    env = {name: value for name, value in os.environ.items() if "KEY" not in name.upper()}
    child = subprocess.Popen(
        [sys.executable, "-c", CHILD],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    assert child.stdin is not None
    assert child.stdout is not None
    assert child.stderr is not None
    try:

        def say(line: bytes) -> bytes:
            assert child.stdin is not None
            assert child.stdout is not None
            child.stdin.write(line + b"\n")
            child.stdin.flush()
            return child.stdout.readline()

        assert say(b"hello") == b"echo 5\n"
        assert say(b"y" * 300_000) == b"echo 300000\n"
        assert say(b"stray") == b"strayed\n"
        # A program that reads its standard input reads nothing of what is meant for the wire.
        assert say(b"steal") == b"stolen 0\n"
        # Nothing that the process holds beside its three standard places can be inherited
        # by a program that it starts, and so neither copy that it keeps for the wire.
        assert say(b"heirs") == b"heirs []\n"
        assert say(b"after") == b"echo 5\n"
        child.stdin.close()
        code = child.wait(20)
        rest = child.stdout.read()
        errors = child.stderr.read().decode("utf-8", "replace")
    finally:
        child.kill()
        child.wait(20)
        for stream in (child.stdin, child.stdout, child.stderr):
            stream.close()

    # It left when its input was closed. Nothing but answers stood in its output.
    assert (code, rest) == (0, b"")
    assert [line.strip() for line in errors.splitlines()] == [
        "printed",
        "written to the standard output",
        "said by a program that was started",
    ]
