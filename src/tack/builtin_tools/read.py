"""The read tool: text files of the working directory, with line numbers and limits.

This module is a plugin. Loading it registers the tool ``read``.
"""

import asyncio
from dataclasses import dataclass
from pathlib import Path

from pydantic import BaseModel, Field

from tack.kernel import Context
from tack.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = ["READ", "ReadArgs", "apply", "inject", "name"]

name = "tool-read"
inject = (TOOLS,)

MAX_LINES = 2000
MAX_LINE_CHARACTERS = 2000
MAX_BYTES = 50 * 1024
_SNIFF_BYTES = 8192


class ReadArgs(BaseModel):
    """The arguments of the read tool."""

    path: str = Field(
        description="The file to read. A relative path starts at the working directory."
    )
    offset: int = Field(default=0, ge=0, description="How many lines to skip at the beginning.")
    limit: int = Field(
        default=MAX_LINES, ge=1, le=MAX_LINES, description="How many lines to return at most."
    )


@dataclass(frozen=True, slots=True)
class _Lines:
    """A part of a text file."""

    lines: list[str]
    total: int


def _load(path: Path, offset: int, limit: int) -> _Lines | None:
    """Read ``limit`` lines of the file after the first ``offset``.

    Returns:
        The lines and the number of lines in the file, or ``None`` if the
        file does not look like text.
    """
    lines: list[str] = []
    total = 0
    with path.open("rb") as file:
        if b"\x00" in file.read(_SNIFF_BYTES):
            return None
        file.seek(0)
        for raw in file:
            if offset <= total < offset + limit:
                lines.append(raw.rstrip(b"\r\n").decode("utf-8", errors="replace"))
            total += 1
    return _Lines(lines, total)


def _numbered(number: int, line: str) -> str:
    extra = len(line) - MAX_LINE_CHARACTERS
    if extra > 0:
        line = f"{line[:MAX_LINE_CHARACTERS]} [{extra} more characters]"
    return f"{number:>6}\t{line}"


async def _read(args: ReadArgs, run: ToolRun) -> str:
    root = run.cwd.resolve()
    target = (root / args.path).resolve()
    if not target.is_relative_to(root):
        raise ToolFailure(
            f"The path {args.path} is outside the working directory. "
            "Only files inside it can be read.",
            code="OUTSIDE_WORKING_DIRECTORY",
        )
    if target.is_dir():
        raise ToolFailure(f"{args.path} is a directory, not a file.", code="NOT_A_FILE")
    try:
        loaded = await asyncio.to_thread(_load, target, args.offset, args.limit)
    except FileNotFoundError:
        raise ToolFailure(f"There is no file {args.path}.", code="NOT_FOUND") from None
    except OSError as error:
        raise ToolFailure(f"{args.path} cannot be read: {error}") from error
    if loaded is None:
        raise ToolFailure(f"{args.path} does not look like a text file.", code="NOT_TEXT")
    if loaded.total == 0:
        return "(the file is empty)"
    if not loaded.lines:
        return f"[the file has {loaded.total} lines, and the offset skips them all]"
    shown: list[str] = []
    size = 0
    for position, line in enumerate(loaded.lines, start=args.offset + 1):
        numbered = _numbered(position, line)
        size += len(numbered.encode()) + 1
        if shown and size > MAX_BYTES:
            break
        shown.append(numbered)
    last = args.offset + len(shown)
    if last < loaded.total:
        first = args.offset + 1
        shown.append(f"[lines {first} to {last} of {loaded.total}. Pass offset={last} to read on.]")
    return "\n".join(shown)


READ = Tool(
    name="read",
    description=(
        "Read a text file of the working directory. The lines come back with their numbers. "
        f"At most {MAX_LINES} lines are returned at once: pass offset and limit to read a "
        "long file in parts."
    ),
    parameters=ReadArgs,
    execute=_read,
    concurrency_safe=True,
)
"""The read tool."""


def apply(ctx: Context, config: object) -> None:
    """Register the read tool."""
    ctx.use(TOOLS).add(ctx, READ)
