"""The write tool: make a file, or put new content in the place of all that a file holds.

This module is a plugin. Loading it registers the tool ``write``.

The file is found, allowed and written through the seam of the file system,
which decides where a session may write, makes sure that an existing file is
one that the model has read, and puts the new content in place in one step.
The content is stored as the model gave it, as UTF-8, with the line breaks it
came with.
"""

from pydantic import BaseModel, Field

from columbina.fs import FS, Access, Fs, carry_out
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = ["WriteArgs", "apply", "inject", "name", "write_tool"]

name = "tool-write"
inject = (TOOLS, FS)


class WriteArgs(BaseModel):
    """The arguments of the write tool."""

    path: str = Field(
        description="The file to write. A relative path starts at the working directory."
    )
    content: str = Field(description="All that the file is to hold.")


def _lines(content: str) -> str:
    count = content.count("\n") + (1 if content and not content.endswith("\n") else 0)
    return "1 line" if count == 1 else f"{count} lines"


def write_tool(fs: Fs) -> Tool[WriteArgs, str]:
    """Make the write tool, which reaches files through ``fs``."""

    def target(args: WriteArgs, run: ToolRun) -> str:
        return fs.locate(run.session, run.cwd, args.path).relative

    async def write(args: WriteArgs, run: ToolRun) -> str:
        where = fs.locate(run.session, run.cwd, args.path)
        if where.relative != run.target:
            raise ToolFailure(
                f"{args.path} leads somewhere else than it did when the call was checked, "
                "so nothing was written. Try again.",
                code="MOVED",
            )
        permit = fs.permit(Access(run.session, run.cwd, where, "write", "write"))
        # Arguments are read as JSON in UTF-8, so the content is text that can be stored.
        data = args.content.encode("utf-8")
        was_there = where.path.exists()
        try:
            await carry_out(lambda given_up: permit.replace(data, given_up=given_up))
        except OSError as error:
            raise ToolFailure(f"{args.path} cannot be written: {error}") from error
        deed = "Replaced" if was_there else "Created"
        return f"{deed} {args.path}: {_lines(args.content)}."

    return Tool(
        name="write",
        description=(
            "Write a file: make it, or put new content in the place of all that it holds. "
            "A file that is there has to be read first. Directories that are missing are "
            "made. To change a part of a file, edit it instead."
        ),
        parameters=WriteArgs,
        execute=write,
        effect="write",
        target=target,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the write tool."""
    ctx.use(TOOLS).add(ctx, write_tool(ctx.use(FS)))
