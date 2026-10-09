"""The glob tool: the files whose paths fit a pattern, the newest first.

This module is a plugin. Loading it registers the tool ``glob``.

A pattern is made of names with ``/`` between them. In a name, ``*`` stands
for any run of characters, ``?`` for one, and ``[...]`` for one of a set. A
name that is ``**`` stands for any number of directories, also none. A
pattern of one name alone is looked for at every depth, since that is what
somebody who writes ``*.py`` means.

The tree is walked through the seam of the file system, which is asked about
every entry. What the tool may not see, it does not list, and it says how
much that was.
"""

import fnmatch
import re
import sys
from collections.abc import Callable
from functools import cache

from pydantic import BaseModel, Field

from columbina.fs import FS, Access, Fs
from columbina.fs.walk import Found, Tally, remarks, walk
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "MAX_ENTRIES",
    "MAX_RESULTS",
    "GlobArgs",
    "apply",
    "glob_tool",
    "inject",
    "matcher",
    "name",
]

name = "tool-glob"
inject = (TOOLS, FS)

MAX_RESULTS = 100
MAX_ENTRIES = 20_000

IGNORES_CASE = sys.platform in ("win32", "darwin")
"""Whether the disks of this platform usually take two names for one that differ in case."""


class GlobArgs(BaseModel):
    """The arguments of the glob tool."""

    pattern: str = Field(
        min_length=1,
        description=(
            "The pattern that the path of a file has to fit, from the directory that is "
            "searched: for example **/*.py, src/**/test_*.py or *.md. A pattern without a "
            "slash is looked for at every depth."
        ),
    )
    path: str = Field(
        default=".",
        description="The directory to search. A relative path starts at the working directory.",
    )


def matcher(pattern: str) -> Callable[[tuple[str, ...]], bool]:
    """Return a function that tells whether the names of a path fit a pattern.

    Raises:
        ToolFailure: The pattern does not name files below the directory that
            is searched. The code is ``INVALID_PATTERN``.
    """
    spelled = pattern.replace("\\", "/")
    names = [part for part in spelled.split("/") if part not in ("", ".")]
    if spelled.startswith("/") or (names and names[0].endswith(":")):
        raise ToolFailure(
            "The pattern has to start at the directory that is searched. Give that "
            "directory as path, and the rest as pattern.",
            code="INVALID_PATTERN",
        )
    if ".." in names:
        raise ToolFailure(
            "The pattern cannot step out of the directory that is searched.",
            code="INVALID_PATTERN",
        )
    if not names:
        raise ToolFailure("The pattern names nothing.", code="INVALID_PATTERN")
    if len(names) == 1:
        names = ["**", *names]
    flags = re.IGNORECASE if IGNORES_CASE else 0
    fits = [None if part == "**" else re.compile(fnmatch.translate(part), flags) for part in names]

    def matches(path: tuple[str, ...]) -> bool:
        @cache
        def from_here(at: int, part: int) -> bool:
            if part == len(fits):
                return at == len(path)
            fit = fits[part]
            if fit is None:
                # Any number of directories, also none.
                return from_here(at, part + 1) or (at < len(path) and from_here(at + 1, part))
            if at == len(path) or fit.match(path[at]) is None:
                return False
            return from_here(at + 1, part + 1)

        return from_here(0, 0)

    return matches


def _said(matched: list[Found], tally: Tally, args: GlobArgs) -> str:
    """Return what the model reads: the paths, and what was not looked at."""
    newest = sorted(matched, key=lambda found: (-found.modified_ns, found.where.relative))
    lines = [found.where.relative for found in newest[:MAX_RESULTS]]
    if not matched:
        lines.append(f"No file fits {args.pattern} in {args.path}.")
    if len(matched) > MAX_RESULTS:
        lines.append(
            f"[These are the {MAX_RESULTS} files that were changed last, of {len(matched)} "
            "that fit. Narrow the pattern to see the others.]"
        )
    return "\n".join([*lines, *remarks(tally)])


def glob_tool(fs: Fs) -> Tool[GlobArgs, str]:
    """Make the glob tool, which reaches files through ``fs``."""

    def target(args: GlobArgs, run: ToolRun) -> str:
        return fs.locate(run.session, run.cwd, args.path).relative

    async def glob(args: GlobArgs, run: ToolRun) -> str:
        matches = matcher(args.pattern)
        root = fs.locate(run.session, run.cwd, args.path)
        if root.relative != run.target:
            raise ToolFailure(
                f"{args.path} leads somewhere else than it did when the call was checked, "
                "so it was not searched. Try again.",
                code="MOVED",
            )
        fs.permit(Access(run.session, run.cwd, root, "read", "glob"))
        if not root.path.is_dir():
            if root.path.exists():
                raise ToolFailure(
                    f"{args.path} is a file. Give the directory to search.",
                    code="NOT_A_DIRECTORY",
                )
            raise ToolFailure(f"There is no directory {args.path}.", code="NOT_FOUND")
        tally = Tally()
        matched = [
            found
            async for found in walk(
                fs, run.session, run.cwd, root, "glob", limit=MAX_ENTRIES, tally=tally
            )
            if matches(found.below)
        ]
        return _said(matched, tally, args)

    return Tool(
        name="glob",
        description=(
            "Find files by the pattern of their paths, for example **/*.py or src/**/*.md. "
            f"The files that were changed last come first, at most {MAX_RESULTS} of them. "
            "Directories named .git are passed over, and links are not followed."
        ),
        parameters=GlobArgs,
        execute=glob,
        concurrency_safe=True,
        effect="read",
        target=target,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the glob tool."""
    ctx.use(TOOLS).add(ctx, glob_tool(ctx.use(FS)))
