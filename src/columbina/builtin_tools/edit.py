"""The edit tool: put new text in the place of a piece of a file.

This module is a plugin. Loading it registers the tool ``edit``.

The piece to replace is given word for word. It has to occur in the file, and
only once unless every occurrence is to be replaced, so that the model cannot
change a place it did not mean. Nothing else in the file is touched.

A file whose lines all end the way Windows ends them keeps that: line breaks
in the old and the new text are taken to mean the line break of the file. A
model writes a plain line feed whatever the file uses, and an edit that put
plain line feeds into such a file would leave it with two kinds.

The file is read and written through the seam of the file system. It has to
be one that the model has read, in the state in which it read it.
"""

import threading

from pydantic import BaseModel, Field

from columbina.fs import FS, Access, Fs, Permit, carry_out
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = ["EditArgs", "apply", "edit_tool", "inject", "name"]

name = "tool-edit"
inject = (TOOLS, FS)

_MIB = 1024 * 1024
_MAX_BYTES = 16 * _MIB
_SNIFF_BYTES = 8192


class EditArgs(BaseModel):
    """The arguments of the edit tool."""

    path: str = Field(
        description="The file to edit. A relative path starts at the working directory."
    )
    old: str = Field(
        min_length=1,
        description=(
            "The text to replace, exactly as it is in the file. It has to occur once, "
            "unless replace_all is set."
        ),
    )
    new: str = Field(description="The text to put in its place.")
    replace_all: bool = Field(
        default=False, description="Replace every occurrence instead of the only one."
    )


def _ends_lines_like_windows(text: str) -> bool:
    """Tell whether a text has line breaks, and every one of them is a return and a feed."""
    return "\r\n" in text and "\n" not in text.replace("\r\n", "")


def _like_windows(piece: str) -> str:
    return piece.replace("\r\n", "\n").replace("\n", "\r\n")


def _change(text: str, args: EditArgs) -> tuple[str, int]:
    """Return the text with the old piece replaced, and how often it was replaced."""
    old, new = args.old, args.new
    if _ends_lines_like_windows(text):
        old, new = _like_windows(old), _like_windows(new)
    count = text.count(old)
    if count == 0:
        raise ToolFailure(
            f"The old text does not occur in {args.path}. It has to match the file exactly, "
            "with its spaces and line breaks. Read the file again to see what is there.",
            code="NO_MATCH",
        )
    if count > 1 and not args.replace_all:
        raise ToolFailure(
            f"The old text occurs {count} times in {args.path}. Give more of the text around "
            "the place you mean, so that it occurs once, or set replace_all.",
            code="AMBIGUOUS",
        )
    return text.replace(old, new), count


def _edit(
    fs: Fs, reading: Permit, writing: Permit, args: EditArgs, given_up: threading.Event
) -> int:
    """Read the file, change it and write it back. Return how often the piece was replaced."""
    with reading.open() as opened:
        raw = opened.file.read(_MAX_BYTES + 1)
        fingerprint = opened.fingerprint
    # First of all: is this a file that the model has seen as it is?
    fs.must_have_shown(reading, fingerprint)
    if len(raw) > _MAX_BYTES:
        raise ToolFailure(
            f"{args.path} is larger than {_MAX_BYTES // _MIB} MiB, which is more than an "
            "edit takes.",
            code="TOO_LARGE",
        )
    try:
        if bytes(1) in raw[:_SNIFF_BYTES]:
            raise ValueError("the file holds bytes that no text holds")
        text = raw.decode("utf-8")
    except ValueError:
        raise ToolFailure(
            f"{args.path} is not a text file in UTF-8, so it cannot be edited.", code="NOT_TEXT"
        ) from None
    changed, count = _change(text, args)
    # The new content takes the place of the content that it was made from, and of no other.
    writing.replace(changed.encode("utf-8"), made_from=fingerprint, given_up=given_up)
    return count


def edit_tool(fs: Fs) -> Tool[EditArgs, str]:
    """Make the edit tool, which reaches files through ``fs``."""

    def target(args: EditArgs, run: ToolRun) -> str:
        return fs.locate(run.session, run.cwd, args.path).relative

    async def edit(args: EditArgs, run: ToolRun) -> str:
        where = fs.locate(run.session, run.cwd, args.path)
        if where.relative != run.target:
            raise ToolFailure(
                f"{args.path} leads somewhere else than it did when the call was checked, "
                "so nothing was changed. Try again.",
                code="MOVED",
            )
        if args.old == args.new:
            raise ToolFailure(
                "The old text and the new text are the same, so there is nothing to change.",
                code="NO_CHANGE",
            )
        reading = fs.permit(Access(run.session, run.cwd, where, "read", "edit"))
        writing = fs.permit(Access(run.session, run.cwd, where, "write", "edit"))
        try:
            count = await carry_out(lambda given_up: _edit(fs, reading, writing, args, given_up))
        except FileNotFoundError:
            raise ToolFailure(
                f"There is no file {args.path}. To make one, write it.", code="NOT_FOUND"
            ) from None
        except OSError as error:
            raise ToolFailure(f"{args.path} cannot be edited: {error}") from error
        times = "1 occurrence" if count == 1 else f"{count} occurrences"
        return f"Replaced {times} in {args.path}."

    return Tool(
        name="edit",
        description=(
            "Change a part of a text file: put new text in the place of old text. The old "
            "text has to be in the file exactly as you give it, and only once, unless "
            "replace_all is set. The file has to be read first."
        ),
        parameters=EditArgs,
        execute=edit,
        effect="write",
        target=target,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the edit tool."""
    ctx.use(TOOLS).add(ctx, edit_tool(ctx.use(FS)))
