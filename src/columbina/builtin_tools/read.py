"""The read tool: text files of the working directory, with line numbers and limits.

This module is a plugin. Loading it registers the tool ``read``.

A read is bounded however large the file is and however long its lines are.
It holds little more in memory than it returns, it looks at no more than the
first ``_SCAN_BYTES`` of the file, and it stops when its caller is cancelled.

The file is found, allowed and opened through the seam of the file system. A
file that was read is one that the model has been shown, which the seam
remembers for the tools that write. It is told so when the result of the
call is on its way to the model with what the tool returned in it: not if
something withheld the result or put other words in its place, and not if
the call showed no line of a file that has lines.

The checks of a call are told where its path leads, and the tool looks again
when it runs. If the path leads elsewhere by then, the file is not read: the
checks spoke about another file.
"""

import asyncio
import codecs
import threading
from dataclasses import dataclass

from pydantic import BaseModel, Field

from columbina.fs import FS, Access, Fingerprint, Fs, Permit
from columbina.kernel import Context
from columbina.tools import RESULT, TOOLS, Tool, ToolFailure, ToolOutcome, ToolRun

__all__ = ["ReadArgs", "ReadResult", "apply", "inject", "name", "read_tool"]

name = "tool-read"
inject = (TOOLS, FS)

MAX_LINES = 2000
MAX_LINE_CHARACTERS = 2000
MAX_BYTES = 50 * 1024
_MIB = 1024 * 1024
_SCAN_BYTES = 64 * _MIB
_CHUNK_BYTES = 64 * 1024
_SNIFF_BYTES = 8192
_Utf8 = codecs.getincrementaldecoder("utf-8")
"""Decodes a line that arrives in pieces. A piece may end in the middle of a character."""


class ReadArgs(BaseModel):
    """The arguments of the read tool."""

    path: str = Field(
        description="The file to read. A relative path starts at the working directory."
    )
    offset: int = Field(default=0, ge=0, description="How many lines to skip at the beginning.")
    limit: int = Field(
        default=MAX_LINES, ge=1, le=MAX_LINES, description="How many lines to return at most."
    )


class _Reader:
    """Takes a file piece by piece and keeps the lines that were asked for.

    Each kept line is numbered and cut to size as soon as it ends. A wanted
    line is decoded as it passes. Of one that is longer than is shown, only
    the beginning is held, and the characters after it are counted. Lines
    that were not asked for are only counted, and are not decoded.

    Attributes:
        shown: The lines to return.
        lines: How many lines have ended.
        open: Whether a line has begun and not ended.
    """

    __slots__ = (
        "_decoder",
        "_end",
        "_first",
        "_head",
        "_passed",
        "_returned",
        "_size",
        "lines",
        "open",
        "shown",
    )

    def __init__(self, offset: int, limit: int) -> None:
        self.shown: list[str] = []
        self.lines = 0
        self.open = False
        self._first = offset
        self._end = offset + limit
        self._size = 0
        self._decoder = _Utf8(errors="replace")
        self._head = ""
        self._passed = 0
        self._returned = False

    def feed(self, chunk: bytes) -> None:
        """Take in the next piece of the file."""
        breaks = chunk.count(b"\n")
        if self.lines + breaks < self._first or self.lines >= self._end:
            # Nothing of this piece is wanted.
            self.lines += breaks
        else:
            start = 0
            while (end := chunk.find(b"\n", start)) >= 0:
                self._take(chunk[start:end])
                self.end_line()
                start = end + 1
            self._take(chunk[start:])
        self.open = not chunk.endswith(b"\n")

    def end_line(self, *, cut_short: bool = False) -> None:
        """End the line that is open.

        Args:
            cut_short: Whether the file was given up before the line ended.
                Such a line is shown as far as it was read, and it does not
                count as a line that ended.
        """
        if self._wanted():
            if not cut_short:
                # Bytes at the end that begin a character and do not finish it are shown too.
                self._take(b"", final=True)
            if self._returned:
                # A carriage return at the end belongs to the line break, and is not shown.
                if self._passed:
                    self._passed -= 1
                else:
                    self._head = self._head[:-1]
            text = self._head
            if self._passed:
                counted = f"at least {self._passed}" if cut_short else str(self._passed)
                text = f"{text} [{counted} more characters]"
            numbered = f"{self.lines + 1:>6}\t{text}"
            self._size += len(numbered.encode()) + 1
            if self.shown and self._size > MAX_BYTES:
                # What is shown is full. From here on, lines are only counted.
                self._end = self.lines
            else:
                self.shown.append(numbered)
            self._head = ""
            self._passed = 0
            self._returned = False
        if not cut_short:
            self.lines += 1

    def _wanted(self) -> bool:
        return self._first <= self.lines < self._end

    def _take(self, piece: bytes, *, final: bool = False) -> None:
        """Add a piece to the line that is open, if that line is wanted."""
        if not self._wanted():
            return
        text = self._decoder.decode(piece, final)
        if not text:
            return
        room = MAX_LINE_CHARACTERS - len(self._head)
        self._head += text[:room]
        self._passed += max(len(text) - room, 0)
        self._returned = text.endswith("\r")


@dataclass(frozen=True, slots=True)
class _Part:
    """What a read found.

    Attributes:
        shown: The lines to return, numbered.
        lines: How many lines were seen.
        complete: Whether the file was read to its end. If not, it has at
            least ``lines`` lines.
        fingerprint: The state of the file when it was opened.
    """

    shown: list[str]
    lines: int
    complete: bool
    fingerprint: Fingerprint


def _load(permit: Permit, offset: int, limit: int, stopped: threading.Event) -> _Part | None:
    """Read ``limit`` lines of the file after the first ``offset``.

    The file is read piece by piece, as far as ``_SCAN_BYTES`` and no further,
    and it is given up as soon as ``stopped`` is set.

    Returns:
        What was found, or ``None`` if the file does not look like text.
    """
    reader = _Reader(offset, limit)
    scanned = 0
    complete = False
    with permit.open() as opened:
        file = opened.file
        while scanned < _SCAN_BYTES and not stopped.is_set():
            chunk = file.read(min(_CHUNK_BYTES, _SCAN_BYTES - scanned))
            if not chunk:
                complete = True
                break
            if scanned == 0 and b"\x00" in chunk[:_SNIFF_BYTES]:
                return None
            scanned += len(chunk)
            reader.feed(chunk)
        else:
            # The file may end exactly where the read stops looking.
            complete = not stopped.is_set() and not file.read(1)
    seen = reader.lines
    if reader.open:
        reader.end_line(cut_short=not complete)
        seen += 1
    return _Part(reader.shown, seen, complete, opened.fingerprint)


def _amount(size: int) -> str:
    return f"{size // _MIB} MiB" if size % _MIB == 0 else f"{size} bytes"


def _note(part: _Part, offset: int) -> str | None:
    """Return what is said after the lines, or instead of them: where they are in the file."""
    first = offset + 1
    last = offset + len(part.shown)
    read_on = f" Pass offset={last} to read on." if last < part.lines else ""
    if part.complete:
        if not part.shown:
            return f"[the file has {part.lines} lines, and the offset skips them all]"
        return f"[lines {first} to {last} of {part.lines}.{read_on}]" if read_on else None
    looked_at = f"Only the first {_amount(_SCAN_BYTES)} of a file are looked at."
    if not part.shown:
        return f"[{looked_at} They hold {part.lines} lines here, and the offset skips them all.]"
    return f"[lines {first} to {last} of at least {part.lines}.{read_on} {looked_at}]"


@dataclass(frozen=True, slots=True)
class ReadResult:
    """What a read returns.

    Attributes:
        text: What the model reads.
        permit: The permit that the file was opened through.
        fingerprint: The state of the file when it was opened.
        shows: Whether the text shows the model something of the file: at
            least one of its lines, or that it has none.
    """

    text: str
    permit: Permit
    fingerprint: Fingerprint
    shows: bool


def _text(args: ReadArgs, result: ReadResult) -> str:
    return result.text


def read_tool(fs: Fs) -> Tool[ReadArgs, ReadResult]:
    """Make the read tool, which reaches files through ``fs``."""

    def target(args: ReadArgs, run: ToolRun) -> str:
        return fs.locate(run.session, run.cwd, args.path).relative

    async def read(args: ReadArgs, run: ToolRun) -> ReadResult:
        where = fs.locate(run.session, run.cwd, args.path)
        if where.relative != run.target:
            raise ToolFailure(
                f"{args.path} leads somewhere else than it did when the call was checked, "
                "so it was not read. Try again.",
                code="MOVED",
            )
        permit = fs.permit(Access(run.session, run.cwd, where, "read", "read"))
        if where.path.is_dir():
            raise ToolFailure(f"{args.path} is a directory, not a file.", code="NOT_A_FILE")
        stopped = threading.Event()
        try:
            part = await asyncio.to_thread(_load, permit, args.offset, args.limit, stopped)
        except FileNotFoundError:
            raise ToolFailure(f"There is no file {args.path}.", code="NOT_FOUND") from None
        except OSError as error:
            raise ToolFailure(f"{args.path} cannot be read: {error}") from error
        finally:
            # A thread cannot be cancelled. It is told to stop, and does so at its next piece.
            stopped.set()
        if part is None:
            raise ToolFailure(f"{args.path} does not look like a text file.", code="NOT_TEXT")
        if part.lines == 0:
            return ReadResult("(the file is empty)", permit, part.fingerprint, True)
        note = _note(part, args.offset)
        text = "\n".join(part.shown if note is None else [*part.shown, note])
        return ReadResult(text, permit, part.fingerprint, bool(part.shown))

    return Tool(
        name="read",
        description=(
            "Read a text file of the working directory. The lines come back with their "
            f"numbers. At most {MAX_LINES} lines are returned at once: pass offset and limit "
            "to read a long file in parts."
        ),
        parameters=ReadArgs,
        execute=read,
        render=_text,
        concurrency_safe=True,
        effect="read",
        target=target,
        paged=True,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the read tool, and note which files the model was shown."""
    fs = ctx.use(FS)

    def remember(outcome: ToolOutcome) -> None:
        # A result that was withheld on its way shows the model nothing, and neither does
        # one in which other words have taken the place of what was read.
        result = outcome.value
        if (
            outcome.error is None
            and isinstance(result, ReadResult)
            and result.shows
            and result.text in outcome.text
        ):
            fs.showing(result.permit, result.fingerprint)

    ctx.use(TOOLS).add(ctx, read_tool(fs))
    ctx.on(RESULT, remember)
