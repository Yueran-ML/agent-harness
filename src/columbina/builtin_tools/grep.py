"""The grep tool: the lines of files that a pattern matches.

This module is a plugin. Loading it registers the tool ``grep``.

The files are found and read here, through the seam of the file system, which
is asked about every one of them. The matching is done elsewhere: in a
process of its own, which is given the pattern and the content of the files,
and which is killed when the time of the search is up or the call is
cancelled. A pattern that a model writes can keep the matcher of Python busy
for longer than anybody will wait, and while it is busy, nothing else in its
process runs. In a process of its own it holds up nobody.

That process runs the program of ``grep_worker.py``. The program is read
when this module is loaded and handed to Python as text, with ``-I -S``. So
what runs is what was loaded, whatever becomes of the file: the tools that
write files cannot make a search run something else. And it loads nothing but
what comes with Python: neither what is installed beside it nor what lies in
the working directory, where a file may well be named like a module of the
standard library.

A search is bounded in what it looks at and in the time it takes. A file that
is large, or that does not look like text, is passed over, and so is
everything beyond a total. What was passed over is said, so that the model
does not take silence for absence. The time is one for the whole search:
finding the files, reading them and matching.
"""

import asyncio
import json
import stat
import sys
import threading
from collections import Counter
from collections.abc import AsyncGenerator, Callable, Sequence
from contextlib import aclosing
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, Field

from columbina import process
from columbina.builtin_tools.glob import matcher
from columbina.fs import FS, Access, Fs
from columbina.fs.walk import Found, Tally, remarks, walk
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "BATCH_BYTES",
    "MAX_BYTES",
    "MAX_ENTRIES",
    "MAX_FILE_BYTES",
    "MAX_LINE",
    "MAX_MATCHES",
    "SECONDS",
    "GrepArgs",
    "apply",
    "grep_tool",
    "inject",
    "name",
]

name = "tool-grep"
inject = (TOOLS, FS)

_MIB = 1024 * 1024

MAX_MATCHES = 250
"""How many lines a search shows."""

MAX_LINE = 200
"""How many characters of a line are shown."""

MAX_ENTRIES = 20_000
"""How many files and directories a search looks at."""

MAX_FILE_BYTES = 16 * _MIB
"""A file that is larger is not searched."""

MAX_BYTES = 64 * _MIB
"""How much text a search reads before it stops."""

BATCH_BYTES = 8 * _MIB
"""About how much is read before it is handed to a process that matches."""

SECONDS = 20.0
"""How long a search may take."""

_SNIFF_BYTES = 8192
_OUTPUT_BYTES = 2 * _MIB
_PROGRAM = Path(__file__).with_name("grep_worker.py").read_text(encoding="utf-8")
"""The program of the matching process, as it was when this module was loaded."""

_Unread = Literal["large", "not text", "unreadable"]
"""Why a file was not searched."""


class GrepArgs(BaseModel):
    """The arguments of the grep tool."""

    pattern: str = Field(
        min_length=1,
        description=(
            "What to look for in each line: a regular expression as Python writes them, "
            "for example def \\w+\\( or TODO|FIXME."
        ),
    )
    path: str = Field(
        default=".",
        description=(
            "The file or directory to search. A relative path starts at the working directory."
        ),
    )
    glob: str | None = Field(
        default=None,
        description=(
            "Below a directory, search only the files whose paths fit this pattern, for "
            "example *.py or src/**/*.md. It is read the way the glob tool reads a pattern."
        ),
    )
    ignore_case: bool = Field(
        default=False, description="Take upper and lower case letters for the same."
    )


class _Answer(BaseModel):
    """One line of what the matching process writes. See ``grep_worker.py``."""

    id: int = 0
    line: int = 0
    text: str = ""
    more: int = 0
    done: bool = False
    files: int = 0
    cut: bool = False
    error: str | None = None


@dataclass(slots=True)
class _Search:
    """What a search has found and passed over so far.

    Attributes:
        lines: The lines to show.
        searched_bytes: How much text was handed to a matching process.
        unread: How many files were not searched, by the reason.
        cut: Whether the search stopped because it had as many lines as it
            shows, with files or lines left that it did not look at.
        out_of_room: Whether the search stopped because it had read as much
            as it may, with files left.
        asked: Whether a matching process has been asked anything yet.
    """

    lines: list[str] = field(default_factory=list[str])
    searched_bytes: int = 0
    unread: Counter[_Unread] = field(default_factory=Counter[_Unread])
    cut: bool = False
    out_of_room: bool = False
    asked: bool = False


async def _only(found: Found) -> AsyncGenerator[Found, None]:
    yield found


def _load(
    files: Sequence[Found], stopped: threading.Event
) -> tuple[list[tuple[str, bytes]], Counter[_Unread], int]:
    """Read files until a batch is full, and give up as soon as ``stopped`` is set.

    How much a batch holds is counted by what was read. What a directory says
    about the size of a file may be less than the file holds by now.

    Returns:
        The path and the content of every file to search, how many files are
        not searched, by the reason, and how many of the files were taken.
    """
    texts: list[tuple[str, bytes]] = []
    unread: Counter[_Unread] = Counter()
    taken = 0
    size = 0
    for found in files:
        if stopped.is_set() or size >= BATCH_BYTES:
            break
        taken += 1
        try:
            with found.permit.open() as opened:
                content = opened.file.read(_SNIFF_BYTES)
                if b"\x00" not in content:
                    content += opened.file.read(MAX_FILE_BYTES + 1 - len(content))
        except (OSError, ToolFailure):
            # It was removed meanwhile, or it is no file, or it is not where it was.
            unread["unreadable"] += 1
            continue
        if b"\x00" in content[:_SNIFF_BYTES]:
            unread["not text"] += 1
        elif len(content) > MAX_FILE_BYTES:
            unread["large"] += 1
        else:
            texts.append((found.where.relative, content))
            size += len(content)
    return texts, unread, taken


def _request(args: GrepArgs, room: int, texts: Sequence[tuple[str, bytes]]) -> bytes:
    """Return what the matching process is given: the pattern, and the files one by one."""
    asked = {
        "pattern": args.pattern,
        "ignore_case": args.ignore_case,
        "max_matches": room,
        "max_line": MAX_LINE,
    }
    # JSON as Python writes it holds nothing but ASCII, whatever the pattern holds.
    parts = [json.dumps(asked).encode("ascii"), b"\n"]
    for number, (_, content) in enumerate(texts):
        parts.append(b'{"id": %d, "size": %d}\n' % (number, len(content)))
        parts.append(content)
    return b"".join(parts)


def _broken(done: process.Completed) -> ToolFailure:
    said = done.output.strip()[-300:] or "nothing"
    return ToolFailure(
        "The search failed. The process that matches the lines ended with the exit code "
        f"{done.exit_code} and said: {said}",
        code="SEARCH_FAILED",
    )


async def _match(
    args: GrepArgs, cwd: Path, texts: Sequence[tuple[str, bytes]], search: _Search
) -> None:
    """Have the lines of a batch of files matched, and add those that match to the search.

    A batch of no files is of use too: the process says whether the pattern
    is one.
    """
    search.asked = True
    try:
        # The time of the whole search is watched by the caller, who stops this when it is up.
        done = await process.run(
            [sys.executable, "-I", "-S", "-c", _PROGRAM],
            cwd=cwd,
            stdin=_request(args, MAX_MATCHES - len(search.lines), texts),
            timeout_seconds=SECONDS,
            max_output_bytes=_OUTPUT_BYTES,
        )
    except OSError as error:
        raise ToolFailure(f"The search failed: {error}", code="SEARCH_FAILED") from error
    try:
        answers = [_Answer.model_validate_json(line) for line in done.output.splitlines()]
    except ValueError:
        raise _broken(done) from None
    if answers and answers[0].error is not None:
        raise ToolFailure(
            f"The pattern is no regular expression that can be used: {answers[0].error}",
            code="INVALID_PATTERN",
        )
    hits = answers[:-1]
    # Output of which a part was left out holds a line about that, which is no answer.
    complete = (
        done.exit_code == 0
        and bool(answers)
        and answers[-1].done
        and (answers[-1].cut or answers[-1].files == len(texts))
        and all(0 <= hit.id < len(texts) for hit in hits)
    )
    if not complete:
        raise _broken(done)
    for hit in hits:
        more = f" [{hit.more} more characters]" if hit.more else ""
        search.lines.append(f"{texts[hit.id][0]}:{hit.line}:{hit.text}{more}")
    search.cut = answers[-1].cut


def _passed(number: int, one: str, many: str) -> str:
    if number == 1:
        return f"[1 file was not searched, because {one}.]"
    return f"[{number} files were not searched, because {many}.]"


def _said(search: _Search, tally: Tally, args: GrepArgs) -> str:
    """Return what the model reads: the lines, and what was not looked at."""
    lines = list(search.lines)
    if not lines:
        lines.append(f"No line matches {args.pattern} in {args.path}.")
    if search.cut:
        lines.append(
            f"[The search stopped after {MAX_MATCHES} lines that match, which is as many as "
            "it shows. There may be more: narrow the pattern or the path.]"
        )
    if search.out_of_room:
        lines.append(
            f"[The search stopped after {MAX_BYTES // _MIB} MiB of text, so there may be "
            "more. Search a narrower path.]"
        )
    if large := search.unread["large"]:
        size = f"larger than {MAX_FILE_BYTES // _MIB} MiB"
        lines.append(_passed(large, f"it is {size}", f"they are {size}"))
    if strange := search.unread["not text"]:
        lines.append(_passed(strange, "it does not look like text", "they do not look like text"))
    if unreadable := search.unread["unreadable"]:
        lines.append(
            "[1 file could not be read.]"
            if unreadable == 1
            else f"[{unreadable} files could not be read.]"
        )
    return "\n".join([*lines, *remarks(tally)])


def grep_tool(fs: Fs) -> Tool[GrepArgs, str]:
    """Make the grep tool, which reaches files through ``fs``."""

    def target(args: GrepArgs, run: ToolRun) -> str:
        return fs.locate(run.session, run.cwd, args.path).relative

    def files(
        args: GrepArgs, run: ToolRun, tally: Tally
    ) -> tuple[AsyncGenerator[Found, None], Callable[[tuple[str, ...]], bool] | None]:
        """Return the files that the call leads to, and what tells which of them to search."""
        root = fs.locate(run.session, run.cwd, args.path)
        if root.relative != run.target:
            raise ToolFailure(
                f"{args.path} leads somewhere else than it did when the call was checked, "
                "so it was not searched. Try again.",
                code="MOVED",
            )
        fits = None if args.glob is None else matcher(args.glob)
        permit = fs.permit(Access(run.session, run.cwd, root, "read", "grep"))
        try:
            status = root.path.stat()
        except FileNotFoundError:
            raise ToolFailure(
                f"There is no file or directory {args.path}.", code="NOT_FOUND"
            ) from None
        except OSError as error:
            raise ToolFailure(f"{args.path} cannot be searched: {error}") from error
        if not stat.S_ISDIR(status.st_mode):
            # A file that the call names is searched, whatever the pattern of paths says.
            return _only(Found(root, (), status.st_size, status.st_mtime_ns, permit)), None
        below = walk(fs, run.session, run.cwd, root, "grep", limit=MAX_ENTRIES, tally=tally)
        return below, fits

    async def grep(args: GrepArgs, run: ToolRun) -> str:
        deadline = asyncio.get_running_loop().time() + SECONDS
        tally = Tally()
        search = _Search()
        batch: list[Found] = []
        stopped = threading.Event()

        async def look() -> None:
            """Read the files that wait, batch by batch, and have their lines matched."""
            waiting = list(batch)
            batch.clear()
            while waiting and not (search.cut or search.out_of_room):
                texts, unread, taken = await asyncio.to_thread(_load, waiting, stopped)
                del waiting[:taken]
                search.unread.update(unread)
                if texts:
                    search.searched_bytes += sum(len(content) for _, content in texts)
                    await _match(args, run.cwd, texts, search)
                if waiting:
                    # The files held more than their directories said, so some are left.
                    search.cut = search.cut or len(search.lines) >= MAX_MATCHES
                    search.out_of_room = search.searched_bytes >= MAX_BYTES

        try:
            async with asyncio.timeout_at(deadline):
                size = 0
                led_to, fits = files(args, run, tally)
                # The walk is closed here, and not whenever the collector comes by.
                async with aclosing(led_to):
                    async for found in led_to:
                        if fits is not None and not fits(found.below):
                            continue
                        if len(search.lines) >= MAX_MATCHES:
                            search.cut = True
                            break
                        if search.searched_bytes >= MAX_BYTES:
                            search.out_of_room = True
                            break
                        if found.size > MAX_FILE_BYTES:
                            search.unread["large"] += 1
                            continue
                        batch.append(found)
                        size += found.size
                        if size >= BATCH_BYTES:
                            size = 0
                            await look()
                if batch:
                    await look()
                if not search.asked:
                    # Nothing was there to search. The pattern is looked at all the same.
                    await _match(args, run.cwd, [], search)
        except TimeoutError:
            raise ToolFailure(
                f"The search took longer than {SECONDS:g} seconds and was stopped. Use a "
                "simpler pattern, or search a narrower path.",
                code="SEARCH_TIMEOUT",
            ) from None
        finally:
            # A thread cannot be cancelled. It is told to stop, and does so at its next file.
            stopped.set()
        return _said(search, tally, args)

    return Tool(
        name="grep",
        description=(
            "Find the lines that match a regular expression, in a file or in the files "
            "below a directory. Each line comes back as path:line:text, at most "
            f"{MAX_MATCHES} of them. Directories named .git are passed over, links are not "
            "followed, and files that do not look like text are not searched."
        ),
        parameters=GrepArgs,
        execute=grep,
        concurrency_safe=True,
        effect="read",
        target=target,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the grep tool."""
    ctx.use(TOOLS).add(ctx, grep_tool(ctx.use(FS)))
