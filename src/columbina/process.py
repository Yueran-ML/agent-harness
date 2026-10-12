"""Run a program within limits, and leave nothing of it behind.

A run ends whatever the program does. It ends when the program does, or when
its time is up, or when the caller is cancelled. In each case all that the
program started is stopped before the run is over, so that nothing goes on
holding a port or a file that the next command needs.

What the program started is known by where the operating system keeps it. On
Windows the program is in a job object from before its first instruction: it
is created suspended, put into the job, and only then let run. Elsewhere it
leads a session and a process group of its own. A process that leaves on
purpose, by starting a session of its own or by having a system service start
it, is not reached.

Starting a program takes a moment, and the caller may be cancelled in it. The
start then goes on by itself, and what it brings into being is ended as soon
as it is there. Where a program runs from the moment it is made, it may have
started others by then, and they are ended with it.

Whether the program has ended is heard from the event loop, not read off its
pipes. A process that it left behind may hold the pipes open, and waiting for
them would wait for that process. So the waits here are bounded, each of them:
the wait for a stopped program to be gone, and the wait for the rest of its
output.

Of what the program writes, memory keeps the beginning and the end. Once
it has written more than that, all of it can go somewhere else as well, as
it comes: a build may write more than memory should hold, and nobody knows
how much it will be before it has ended.

The program is cut off from the terminal of the user. On Windows it has a
console without a window, elsewhere a session without a terminal, and its
input is what the caller gives it. So a program that asks the user for
something finds its input at an end. One that turns to the console itself gets
no answer there, and ends when its time is up.

A program may only stand for something that runs elsewhere: it asked a
service to run a command, and waits for it. Stopping such a program stops
nothing of the command. So a caller can ask to be heard first. When the
program is to be stopped before it has ended, what the caller does then is
waited for, also if the caller is cancelled meanwhile, and only then is the
program stopped. This wait is bounded like the others.
"""

import asyncio
import codecs
import contextlib
import logging
import os
import signal
import subprocess
import sys
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, Protocol, cast

__all__ = ["Completed", "HeadTail", "Writable", "cleaned", "run"]

logger = logging.getLogger(__name__)

_REAP_SECONDS = 5.0
"""How long a program that was stopped is waited for."""

_BEFORE_STOP_SECONDS = 30.0
"""How long a caller may take over what it does before its program is stopped."""

_DRAIN_SECONDS = 2.0
"""How long the rest of the output is waited for once the program is gone."""

_CLOSE_SECONDS = 5.0
"""How long a run waits for the loop to take note that its program is gone for good."""

_SECRET_WORDS = ("KEY", "PASSWORD", "SECRET", "TOKEN")
_Utf8 = codecs.getincrementaldecoder("utf-8")


@dataclass(frozen=True, slots=True)
class Completed:
    """How a run ended.

    Attributes:
        exit_code: The exit code of the program, or ``None`` if it was stopped
            because its time was up. Where the system says so, a program that
            a signal ended has the negative number of the signal.
        timed_out: Whether the program was stopped because its time was up.
        output: What the program wrote, to its output and to its error stream,
            in the order it was written.
        output_bytes: How many bytes it wrote.
        omitted_bytes: How many of them are not in ``output``.
        saved_bytes: How many of them went somewhere else as well, from the
            first one on. It is 0 if none did.
        error_output: What the program wrote to its error stream, if the two
            streams were kept apart. ``output`` is then what it wrote to its
            output alone, and the three counts above are of that. Where the
            streams are one, this is empty.
        error_bytes: How many bytes it wrote to its error stream, if the
            streams were kept apart.
        error_omitted_bytes: How many of them are not in ``error_output``.
    """

    exit_code: int | None
    timed_out: bool
    output: str
    output_bytes: int
    omitted_bytes: int
    saved_bytes: int
    error_output: str = ""
    error_bytes: int = 0
    error_omitted_bytes: int = 0


class Writable(Protocol):
    """Somewhere that the whole output of a program can be written to."""

    def write(self, data: bytes) -> None:
        """Take the next piece.

        Raises:
            OSError: No more can be taken.
        """
        ...


def cleaned(env: Mapping[str, str]) -> dict[str, str]:
    """Return an environment without the variables whose names sound like secrets."""
    return {
        name: value
        for name, value in env.items()
        if not any(word in name.upper() for word in _SECRET_WORDS)
    }


def _note(count: int) -> str:
    if count == 1:
        return "[1 byte of output is left out here]"
    return f"[{count} bytes of output are left out here]"


class HeadTail:
    """Keeps the beginning and the end of a stream of bytes, and counts the rest.

    Half of what may be kept is the beginning and half is the end. However the
    bytes arrive, the same ones are kept.

    Attributes:
        limit: How many bytes are kept at most.
        total: How many bytes have been added.

    Raises:
        ValueError: ``limit`` is negative.
    """

    __slots__ = ("_head", "_head_room", "_tail", "_tail_room", "limit", "total")

    def __init__(self, limit: int) -> None:
        if limit < 0:
            raise ValueError(f"the limit of what is kept is {limit}, which is less than nothing")
        self.limit = limit
        self._head_room = limit // 2
        self._tail_room = limit - self._head_room
        self._head = bytearray()
        self._tail = bytearray()
        self.total = 0

    def add(self, data: bytes) -> None:
        """Take the next bytes of the stream."""
        self.total += len(data)
        room = self._head_room - len(self._head)
        self._head += data[:room]
        self._tail += data[room:]
        del self._tail[: max(0, len(self._tail) - self._tail_room)]

    def held(self) -> bytes:
        """Return the bytes that are kept. While no more was added than the limit, that is all."""
        return bytes(self._head + self._tail)

    def _parts(self) -> tuple[bytes, bytes] | None:
        """Return the two ends as they are shown, or ``None`` if nothing is missing."""
        if self.total <= self._head_room + self._tail_room:
            return None
        # A cut may fall inside a character. What it leaves of one is not shown.
        decoder = _Utf8(errors="replace")
        decoder.decode(bytes(self._head[-3:]))
        unfinished = len(decoder.getstate()[0])
        head = bytes(self._head[: len(self._head) - unfinished])
        rest = 0
        while rest < min(3, len(self._tail)) and self._tail[rest] & 0xC0 == 0x80:
            rest += 1
        return head, bytes(self._tail[rest:])

    @property
    def omitted(self) -> int:
        """How many of the bytes are not in the text."""
        parts = self._parts()
        return 0 if parts is None else self.total - len(parts[0]) - len(parts[1])

    def text(self) -> str:
        """Return what was kept as text, with a line that says how much is missing.

        Bytes that are no UTF-8 are shown as replacement characters.
        """
        parts = self._parts()
        if parts is None:
            return bytes(self._head + self._tail).decode("utf-8", errors="replace")
        head = parts[0].decode("utf-8", errors="replace")
        tail = parts[1].decode("utf-8", errors="replace")
        # The line stands on its own, whether or not the beginning ends a line.
        gap = "" if not head or head.endswith("\n") else "\n"
        return f"{head}{gap}{_note(self.total - len(parts[0]) - len(parts[1]))}\n{tail}"


if sys.platform == "win32":  # pragma: windows only
    import ctypes
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _ntdll = ctypes.WinDLL("ntdll")

    _kernel32.CreateJobObjectW.argtypes = (wintypes.LPVOID, wintypes.LPCWSTR)
    _kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    _kernel32.SetInformationJobObject.argtypes = (
        wintypes.HANDLE,
        ctypes.c_int,
        wintypes.LPVOID,
        wintypes.DWORD,
    )
    _kernel32.SetInformationJobObject.restype = wintypes.BOOL
    _kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.AssignProcessToJobObject.argtypes = (wintypes.HANDLE, wintypes.HANDLE)
    _kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    _kernel32.TerminateJobObject.argtypes = (wintypes.HANDLE, wintypes.UINT)
    _kernel32.TerminateJobObject.restype = wintypes.BOOL
    _kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
    _kernel32.CloseHandle.restype = wintypes.BOOL
    # Not in the documentation of the system, but in every version of it since XP, and what
    # psutil resumes a process with. The documented way is to look for the first thread of
    # the process among all threads of the machine.
    _ntdll.NtResumeProcess.argtypes = (wintypes.HANDLE,)
    _ntdll.NtResumeProcess.restype = wintypes.LONG

    _CREATE_SUSPENDED = 0x00000004
    _PROCESS_TERMINATE = 0x0001
    _PROCESS_SET_QUOTA = 0x0100
    _PROCESS_SUSPEND_RESUME = 0x0800
    _EXTENDED_LIMIT_INFORMATION = 9
    _KILL_ON_JOB_CLOSE = 0x2000

    class _BasicLimits(ctypes.Structure):
        _fields_ = (
            ("PerProcessUserTimeLimit", wintypes.LARGE_INTEGER),
            ("PerJobUserTimeLimit", wintypes.LARGE_INTEGER),
            ("LimitFlags", wintypes.DWORD),
            ("MinimumWorkingSetSize", ctypes.c_size_t),
            ("MaximumWorkingSetSize", ctypes.c_size_t),
            ("ActiveProcessLimit", wintypes.DWORD),
            ("Affinity", ctypes.c_size_t),
            ("PriorityClass", wintypes.DWORD),
            ("SchedulingClass", wintypes.DWORD),
        )

    class _IoCounters(ctypes.Structure):
        _fields_ = tuple((name, ctypes.c_ulonglong) for name in "abcdef")

    class _ExtendedLimits(ctypes.Structure):
        _fields_ = (
            ("BasicLimitInformation", _BasicLimits),
            ("IoInfo", _IoCounters),
            ("ProcessMemoryLimit", ctypes.c_size_t),
            ("JobMemoryLimit", ctypes.c_size_t),
            ("PeakProcessMemoryUsed", ctypes.c_size_t),
            ("PeakJobMemoryUsed", ctypes.c_size_t),
        )

    def _check(succeeded: object, attempt: str) -> None:
        if not succeeded:
            code = ctypes.get_last_error()
            raise ctypes.WinError(code, f"could not {attempt}: {ctypes.FormatError(code)}")

    _SPAWN: dict[str, Any] = {"creationflags": _CREATE_SUSPENDED | subprocess.CREATE_NO_WINDOW}
    """The program does not run until it is in its job, and it gets no window."""

    class _Tree:
        """A program and all that it starts, held in a job object.

        The system ends what is in the job when the job is given up, so
        nothing of a command outlives this process either.

        Raises:
            OSError: The program could not be put into a job or let run.
        """

        __slots__ = ("_job",)

        def __init__(self, pid: int) -> None:
            job = cast("int | None", _kernel32.CreateJobObjectW(None, None))
            _check(job, "create a job for the program")
            self._job = job
            try:
                limits = _ExtendedLimits()
                limits.BasicLimitInformation.LimitFlags = _KILL_ON_JOB_CLOSE
                _check(
                    _kernel32.SetInformationJobObject(
                        job,
                        _EXTENDED_LIMIT_INFORMATION,
                        ctypes.byref(limits),
                        ctypes.sizeof(limits),
                    ),
                    "set up the job of the program",
                )
                access = _PROCESS_TERMINATE | _PROCESS_SET_QUOTA | _PROCESS_SUSPEND_RESUME
                process = cast("int | None", _kernel32.OpenProcess(access, False, pid))
                _check(process, "reach the program")
                try:
                    _check(
                        _kernel32.AssignProcessToJobObject(job, process),
                        "put the program into its job",
                    )
                    status = cast("int", _ntdll.NtResumeProcess(process))
                    if status < 0:
                        code = status & 0xFFFFFFFF
                        raise OSError(f"could not let the program run: status {code:#010x}")
                finally:
                    _kernel32.CloseHandle(process)
            except BaseException:
                self.close()
                raise

        def stop(self) -> None:
            """End every process of the job. A failure is written down, not raised."""
            if not _kernel32.TerminateJobObject(self._job, 1):
                code = ctypes.get_last_error()
                logger.error("a program could not be stopped: %s", ctypes.FormatError(code))

        def close(self) -> None:
            """Give the job up, which is done once."""
            _kernel32.CloseHandle(self._job)

    def _end_unheld(transport: asyncio.SubprocessTransport) -> None:
        """End a program that is in no job yet. It was made suspended and has not run."""
        transport.close()

else:  # pragma: posix only
    _SPAWN: dict[str, Any] = {"start_new_session": True}
    """The program leads a session and a process group of its own."""

    class _Tree:
        """A program and all that it starts, held in a process group."""

        __slots__ = ("_group",)

        def __init__(self, pid: int) -> None:
            self._group = pid

        def stop(self) -> None:
            """End every process of the group."""
            # The group is gone if nothing of it is left. One of its processes may run as
            # somebody else, and is then not for this process to end.
            with contextlib.suppress(ProcessLookupError, PermissionError):
                os.killpg(self._group, signal.SIGKILL)

        def close(self) -> None:
            """Give the group up, which needs nothing."""

    def _end_unheld(transport: asyncio.SubprocessTransport) -> None:
        """End a program that nobody holds yet, with all that it has started meanwhile."""
        _Tree(transport.get_pid()).stop()
        transport.close()


class _Output:
    """What a program writes: its two ends in memory, and all of it elsewhere if it is more.

    The place elsewhere is asked for when the first byte arrives that memory
    does not keep. It is given what came before, and then every piece as it
    comes, up to a limit. A place that cannot be opened, or that fails to
    take a piece, is given up, and the program runs on.

    A piece is written when it arrives, by the event loop. The loop reads no
    more from the program meanwhile. So a program that writes faster than the
    place takes it has to wait, and nothing piles up in memory.

    Attributes:
        kept: The two ends.
        saved: How many bytes went elsewhere.

    Raises:
        ValueError: One of the two limits is negative.
    """

    __slots__ = ("_elsewhere", "_open", "_room", "kept", "saved")

    def __init__(
        self, limit: int, overflow: Callable[[], Writable] | None, max_overflow_bytes: int
    ) -> None:
        if max_overflow_bytes < 0:
            raise ValueError(
                f"the limit of what is written elsewhere is {max_overflow_bytes}, "
                "which is less than nothing"
            )
        self.kept = HeadTail(limit)
        self.saved = 0
        self._open = overflow
        self._elsewhere: Writable | None = None
        self._room = max_overflow_bytes

    def add(self, data: bytes) -> None:
        """Take the next piece of what the program writes."""
        if self._open is not None and self.kept.total + len(data) > self.kept.limit:
            opening, self._open = self._open, None
            try:
                self._elsewhere = opening()
            except Exception as error:
                self._give_up("cannot be kept", error)
            else:
                self._save(self.kept.held())
        self.kept.add(data)
        self._save(data)

    def end(self) -> None:
        """Write nothing elsewhere from now on. Whoever opened the place is about to close it."""
        self._open = None
        self._elsewhere = None

    def _save(self, data: bytes) -> None:
        if self._elsewhere is None:
            return
        taken = data[: self._room - self.saved]
        try:
            self._elsewhere.write(taken)
        except Exception as error:
            self._give_up("could not be written", error)
            return
        self.saved += len(taken)
        if len(taken) < len(data):
            # There is room for no more. What memory keeps goes on as before.
            self._elsewhere = None

    def _give_up(self, what: str, error: Exception) -> None:
        """Leave the place alone from now on, and write down why."""
        self._elsewhere = None
        # What the system refuses is said in its own words. Anything else is a fault of the
        # place, and nobody is there to hear of it but the log: this runs inside the loop.
        fault = None if isinstance(error, OSError) else error
        logger.error("the whole output of a program %s: %s", what, error, exc_info=fault)


class _Watch(asyncio.SubprocessProtocol):
    """Hears what a program writes, and when it ends.

    Attributes:
        exited: Done when the program has ended.
        silent: Done when nothing more can arrive of what it wrote.
        closed: Done when everything of the program has been let go.
    """

    def __init__(self, kept: _Output, errors: HeadTail | None = None) -> None:
        loop = asyncio.get_running_loop()
        self._kept = kept
        self._errors = errors
        # The pipes that the program writes to. With one stream there is one.
        self._writing = {1} if errors is None else {1, 2}
        self.exited: asyncio.Future[None] = loop.create_future()
        self.silent: asyncio.Future[None] = loop.create_future()
        self.closed: asyncio.Future[None] = loop.create_future()

    def pipe_data_received(self, fd: int, data: bytes) -> None:
        if fd == 2 and self._errors is not None:
            self._errors.add(data)
        else:
            self._kept.add(data)

    def pipe_connection_lost(self, fd: int, exc: Exception | None) -> None:
        # The pipe that is not among them is the one that the program reads its input from.
        if fd in self._writing:
            self._writing.remove(fd)
            if not self._writing:
                self.silent.set_result(None)

    def process_exited(self) -> None:
        self.exited.set_result(None)

    def connection_lost(self, exc: Exception | None) -> None:
        self.closed.set_result(None)


def _abandoned(creating: "asyncio.Future[tuple[asyncio.SubprocessTransport, _Watch]]") -> None:
    """End a program that came into being after the one who asked for it had left."""
    if creating.cancelled() or creating.exception() is not None:
        return
    _end_unheld(creating.result()[0])


async def _within(awaited: asyncio.Future[None], seconds: float) -> bool:
    """Wait for something for a limited time, and tell whether it came."""
    done, _ = await asyncio.wait([awaited], timeout=seconds)
    return bool(done)


async def _heard(
    before_stop: Callable[[asyncio.Future[None]], Awaitable[None]], exited: asyncio.Future[None]
) -> None:
    """Wait for what a caller does before its program is stopped.

    Whoever waits here may be cancelled meanwhile. The wait goes on then,
    because what the caller stops in its own way would otherwise go on
    running, and the cancellation is passed on afterwards. The wait does not
    go on for ever, and a caller that fails is written down and no more: the
    program is stopped in any case.

    Raises:
        asyncio.CancelledError: Whoever waited was cancelled meanwhile.
    """
    # The caller waits on a future of its own. Whatever it does to that one, the run goes
    # on hearing of the end of the program.
    doing = asyncio.ensure_future(before_stop(asyncio.shield(exited)))
    loop = asyncio.get_running_loop()
    until = loop.time() + _BEFORE_STOP_SECONDS
    cancelled = False
    while not doing.done():
        try:
            await asyncio.wait({doing}, timeout=max(until - loop.time(), 0))
        except asyncio.CancelledError:
            cancelled = True
            continue
        if not doing.done():
            doing.cancel()
            logger.error(
                "what was to be done before a program is stopped took longer than %g seconds "
                "and was given up",
                _BEFORE_STOP_SECONDS,
            )
            break
    if doing.done() and not doing.cancelled() and doing.exception() is not None:
        logger.error(
            "what was to be done before a program is stopped failed", exc_info=doing.exception()
        )
    if cancelled:
        raise asyncio.CancelledError


async def run(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    stdin: bytes | None = None,
    timeout_seconds: float,
    max_output_bytes: int = 64_000,
    overflow: Callable[[], Writable] | None = None,
    max_overflow_bytes: int = 64 * 1024 * 1024,
    streams: Literal["joined", "apart"] = "joined",
    before_stop: Callable[[asyncio.Future[None]], Awaitable[None]] | None = None,
) -> Completed:
    """Run a program to its end, or until its time is up.

    When this returns or raises, everything that the program started has been
    stopped. A process that cannot be stopped does not keep the run from
    ending: it is waited for a few seconds and then left.

    Args:
        argv: The program and its arguments. No shell reads them.
        cwd: The directory to run it in.
        env: The environment of the program. Without one it gets the
            environment of this process, :func:`cleaned`.
        stdin: What the program can read from its input. Without it the input
            is empty.
        timeout_seconds: How long the program may run.
        max_output_bytes: How much of what it writes is kept. Of more, the
            beginning and the end are kept.
        overflow: Opens a place for the whole of what the program writes. It
            is called once, when the program has written more than is kept,
            and never if it does not. Nothing is written to the place once
            the run is over, however it ended, so whoever passes this closes
            what it opened then.
        max_overflow_bytes: How much is written to that place at most.
        streams: Whether what the program writes to its error stream is put
            with its output, in the order in which the two were written, or
            kept apart. Apart, each of the two is kept up to
            ``max_output_bytes``. Nobody can tell afterwards which of the two
            a piece was written to first, so the order between them is lost.
        before_stop: Called when the program is to be stopped before it has
            ended, because its time is up or the caller was cancelled. It is
            for a program that stands for something elsewhere, which
            stopping the program does not stop. It is given a future that is
            done once the program has ended, and what it returns is waited
            for before the program is stopped. It is not called for a
            program that ended by itself, nor for one that never came to
            run. If it fails, that is written down.

    Raises:
        OSError: The program could not be started.
        ValueError: ``argv`` is empty, or one of the limits is negative, or
            the whole output is to go elsewhere while the streams are apart.
    """
    if not argv:
        raise ValueError("there is no program to run")
    if streams == "apart" and overflow is not None:
        raise ValueError(
            "the whole output of a program is kept elsewhere only where the two streams are one"
        )
    written = _Output(max_output_bytes, overflow, max_overflow_bytes)
    errors = HeadTail(max_output_bytes) if streams == "apart" else None
    try:
        return await _run(argv, cwd, env, stdin, timeout_seconds, written, errors, before_stop)
    finally:
        # A program that was left behind may write on, and one whose caller left as it was
        # started has yet to come into being. Neither finds a place that its caller closed.
        written.end()


async def _run(
    argv: Sequence[str],
    cwd: Path,
    env: Mapping[str, str] | None,
    stdin: bytes | None,
    timeout_seconds: float,
    written: _Output,
    errors: HeadTail | None,
    before_stop: Callable[[asyncio.Future[None]], Awaitable[None]] | None,
) -> Completed:
    """Run a program, and hand what it writes to ``written``, or its error stream to ``errors``."""
    kept = written.kept
    watch = _Watch(written, errors)
    loop = asyncio.get_running_loop()
    creating = asyncio.ensure_future(
        loop.subprocess_exec(
            lambda: watch,
            *argv,
            stdin=subprocess.DEVNULL if stdin is None else subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT if errors is None else subprocess.PIPE,
            cwd=cwd,
            env=cleaned(os.environ) if env is None else dict(env),
            **_SPAWN,
        )
    )
    try:
        transport, _ = await asyncio.shield(creating)
    except asyncio.CancelledError:
        # The start goes on without its caller. What it makes is ended when it is there.
        creating.add_done_callback(_abandoned)
        raise
    except (OSError, ValueError) as error:
        raise OSError(f"the program {argv[0]!r} could not be started: {error}") from error
    try:
        tree = _Tree(transport.get_pid())
    except BaseException:
        # The program has not run a single instruction, and it does not get to.
        transport.close()
        await _within(watch.closed, _CLOSE_SECONDS)
        raise
    timed_out = False
    try:
        if stdin is not None:
            feed = cast("asyncio.WriteTransport", transport.get_pipe_transport(0))
            feed.write(stdin)
            feed.write_eof()
        timed_out = not await _within(watch.exited, timeout_seconds)
    finally:
        try:
            if before_stop is not None and not watch.exited.done():
                await _heard(before_stop, watch.exited)
        finally:
            # However the wait ended, nothing that the program started goes on.
            tree.stop()
            try:
                await _within(watch.exited, _REAP_SECONDS)
                await _within(watch.silent, _DRAIN_SECONDS)
            finally:
                # A caller that is cancelled again gets here at once. Letting go is done
                # without waiting, and then the event loop is given the moment it needs to
                # take note of it.
                exit_code = transport.get_returncode()
                transport.close()
                tree.close()
                await _within(watch.closed, _CLOSE_SECONDS)
    return Completed(
        exit_code=None if timed_out else exit_code,
        timed_out=timed_out,
        output=kept.text(),
        output_bytes=kept.total,
        omitted_bytes=kept.omitted,
        saved_bytes=written.saved,
        error_output="" if errors is None else errors.text(),
        error_bytes=0 if errors is None else errors.total,
        error_omitted_bytes=0 if errors is None else errors.omitted,
    )
