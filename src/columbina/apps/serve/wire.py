"""The pipe that a service process speaks through: its standard input and output, kept for it.

The protocol goes over the standard input and output of the process, a
message to a line. Two things could spoil that. A plugin that prints
something, or a program that the process starts and that writes to the output
it inherits, puts its words into the middle of a message. And a program that
is started and reads its input takes away what was meant for the service
process.

So the process keeps copies of the two for itself, see :func:`keep`, and puts
its standard error where its standard output was and nothing where its
standard input was. From then on a stray word goes to the log, and a stray
reader finds nothing to read.

The copies are read and written by a thread each. A thread that reads can
wait for the other side without holding up the loop, and a thread that writes
can wait for an other side that reads slowly while the loop goes on taking
requests, the one that stops a turn among them. What waits to be written is
kept, up to a limit. An other side that lets more than that pile up does not
read, whatever it sends, and is taken for gone.

The reading is done with the calls of the system and not through a buffered
file of Python. A thread that waits in such a file when the interpreter ends
makes the interpreter end with a fatal error.
"""

import asyncio
import contextlib
import os
import queue
import sys
import threading
from collections.abc import AsyncIterator
from typing import Final, Protocol

from columbina.apps.serve.rpc import on_the_wire
from columbina.kernel import ServiceKey

__all__ = ["MAX_LINE_BYTES", "MAX_UNSENT_BYTES", "WIRE", "Wire", "keep", "over", "taken"]

MAX_LINE_BYTES: Final = 16 * 1024 * 1024
"""How long a line that comes in may be. A longer one is dropped."""

MAX_UNSENT_BYTES: Final = 64 * 1024 * 1024
"""How much may wait to be written out. An other side that lets more pile up is taken for gone."""

_READ_BYTES = 1 << 16
_LAST_WORDS_SECONDS = 5.0
"""How long the end of the process waits for what was sent to be written out."""

_END: Final = object()
"""What stands at the end of the lines: the other side has closed, or is gone."""


class Wire(Protocol):
    """Lines in and lines out, between a service process and whoever drives it."""

    def lines(self) -> AsyncIterator[bytes | None]:
        """Give the lines that come in, each without its line break, until the other side closes.

        ``None`` stands for a line that was longer than the wire takes. What
        it held is gone.
        """
        ...

    def send(self, line: str) -> None:
        """Send a line. This does not wait for it to be written."""
        ...

    async def close(self) -> None:
        """Write out what was sent, as far as the other side takes it, and close the way out."""
        ...


WIRE = ServiceKey[Wire]("serve/wire")
"""The wire of the service process. A test provides one. Otherwise the process takes its own."""


class _Pipes:
    """A wire over two descriptors of the system, read and written by a thread each."""

    def __init__(
        self,
        incoming: int,
        outgoing: int,
        loop: asyncio.AbstractEventLoop,
        limit: int,
        backlog: int,
    ) -> None:
        self._incoming = incoming
        self._outgoing = outgoing
        self._loop = loop
        self._limit = limit
        self._backlog = backlog
        self._arrived: asyncio.Queue[object] = asyncio.Queue()
        self._to_write: queue.SimpleQueue[bytes | None] = queue.SimpleQueue()
        # How many bytes were sent and are not written out yet. Both threads count.
        self._unsent = 0
        self._counting = threading.Lock()
        # Set when nobody reads on the other side any more: writing failed, or what was
        # sent piled up.
        self._gone = False
        self._closed = False
        # Set when the thread that writes has come to its end.
        self._written = asyncio.Event()
        self._reader = threading.Thread(target=self._read, name="serve-in", daemon=True)
        self._writer = threading.Thread(target=self._write, name="serve-out", daemon=True)
        self._reader.start()
        self._writer.start()

    async def lines(self) -> AsyncIterator[bytes | None]:
        """Give the lines that come in, until the other side closes or is gone."""
        while True:
            arrived = await self._arrived.get()
            if arrived is _END:
                return
            assert arrived is None or isinstance(arrived, bytes)
            yield arrived

    def send(self, line: str) -> None:
        """Send a line. What is sent to an other side that is gone is dropped."""
        if self._gone or self._closed:
            return
        data = on_the_wire(line)
        with self._counting:
            self._unsent += len(data)
            piled_up = self._unsent > self._backlog
        if piled_up:
            # The other side sends and does not read, or it does neither. Nothing more is
            # kept for it. Whoever waits for its lines is told that it is gone, which is the
            # order to leave. What was sent before stays to be written, should it read yet.
            self._gone = True
            self._arrived.put_nowait(_END)
            return
        self._to_write.put(data)

    async def close(self) -> None:
        """Write out what was sent and close the way out. A second call does nothing."""
        if self._closed:
            return
        self._closed = True
        self._to_write.put(None)
        # The other side may have stopped reading without closing. It is not waited for
        # for good: the thread is left to end with the process. No thread of the pool is
        # asked to wait for it either. The pool may be full of threads that are stuck.
        try:
            async with asyncio.timeout(_LAST_WORDS_SECONDS):
                await self._written.wait()
        except TimeoutError:
            return
        # The thread writes nothing more, so the way out can be closed under it.
        with contextlib.suppress(OSError):
            os.close(self._outgoing)

    def _hand(self, arrived: object) -> bool:
        """Hand something to the loop. Tell whether the loop is still there to take it."""
        try:
            self._loop.call_soon_threadsafe(self._arrived.put_nowait, arrived)
        except RuntimeError:
            return False
        return True

    def _read(self) -> None:
        """Read the way in to its end, and hand over every line."""
        parts: list[bytes] = []
        held = 0
        too_long = False
        while True:
            try:
                chunk = os.read(self._incoming, _READ_BYTES)
            except OSError:
                chunk = b""
            if not chunk:
                break
            while chunk:
                end = chunk.find(b"\n")
                if end < 0:
                    parts.append(chunk)
                    held += len(chunk)
                    if held > self._limit:
                        # Nobody keeps a line without end. Its rest is passed over as it comes.
                        parts.clear()
                        held = 0
                        too_long = True
                    break
                parts.append(chunk[:end])
                line = b"".join(parts)
                chunk = chunk[end + 1 :]
                parts.clear()
                held = 0
                if too_long or len(line) > self._limit:
                    too_long = False
                    arrived: bytes | None = None
                else:
                    arrived = line.removesuffix(b"\r")
                if not self._hand(arrived):
                    return
        # A last line that was not ended is a line all the same.
        if too_long:
            self._hand(None)
        elif parts:
            self._hand(b"".join(parts).removesuffix(b"\r"))
        self._hand(_END)

    def _write(self) -> None:
        """Write out what is sent, line by line, until the wire is closed."""
        try:
            while True:
                data = self._to_write.get()
                if data is None:
                    return
                try:
                    rest = memoryview(data)
                    while rest:
                        rest = rest[os.write(self._outgoing, rest) :]
                except OSError:
                    # Nobody reads any more. Whoever waits for lines is told that the other
                    # side is gone, which is the order to leave.
                    self._gone = True
                    self._hand(_END)
                    return
                with self._counting:
                    self._unsent -= len(data)
        finally:
            # A loop that has ended has nobody left to tell.
            with contextlib.suppress(RuntimeError):
                self._loop.call_soon_threadsafe(self._written.set)


def over(
    incoming: int,
    outgoing: int,
    *,
    limit: int = MAX_LINE_BYTES,
    backlog: int = MAX_UNSENT_BYTES,
) -> Wire:
    """Return a wire over two descriptors: one to read lines from, one to write lines to.

    It belongs to the loop that is running, and its lines are read there.

    Args:
        incoming: The descriptor to read from.
        outgoing: The descriptor to write to.
        limit: How many bytes a line that comes in may have.
        backlog: How many bytes may wait to be written out, at most.
    """
    return _Pipes(incoming, outgoing, asyncio.get_running_loop(), limit, backlog)


def _nothing_at(where: int, mode: int) -> None:
    """Put the device that holds nothing in the place of a descriptor."""
    nothing = os.open(os.devnull, mode)
    if nothing == where:
        # The place was free, and the system gave out its very number. A program that is
        # started is to find something there, like at every other place of this kind.
        os.set_inheritable(where, True)
        return
    try:
        os.dup2(nothing, where)
    finally:
        os.close(nothing)


def keep(source: int, sink: int, errors: int) -> tuple[int, int]:
    """Keep the way in and the way out of a process for the protocol alone.

    Afterwards whatever writes to ``sink`` writes where ``errors`` leads, and
    whatever reads from ``source`` finds nothing to read. A program that is
    started inherits these two, and not the copies.

    The system gives a copy the lowest number that is free. So all three are
    looked at before anything is copied: a number that is not open would be
    given to a copy, and then be taken for what it was meant to be. A process
    that was started without a log has ``errors`` made to lead nowhere first.

    Args:
        source: The descriptor that the requests come in by.
        sink: The descriptor that the answers go out by.
        errors: The descriptor of the log.

    Returns:
        The copies of ``source`` and ``sink``, which only this process holds.

    Raises:
        OSError: ``source`` or ``sink`` is no open descriptor.
    """
    for needed in (source, sink):
        os.fstat(needed)
    try:
        os.fstat(errors)
    except OSError:
        _nothing_at(errors, os.O_WRONLY)
    incoming = os.dup(source)
    try:
        outgoing = os.dup(sink)
    except OSError:
        os.close(incoming)
        raise
    os.dup2(errors, sink)
    _nothing_at(source, os.O_RDONLY)
    return incoming, outgoing


def taken() -> Wire:  # pragma: no cover - takes the standard streams of its process
    """Take the standard input and output of this process for the protocol.

    What is printed from now on goes to the standard error.

    Raises:
        OSError: The process has no standard input or output.
    """
    incoming, outgoing = keep(0, 1, 2)
    sys.stdout = sys.stderr
    return over(incoming, outgoing)
