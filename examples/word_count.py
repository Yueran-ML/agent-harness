"""A plugin that gives the agent one tool: how many words a text file has.

A change to the profile loads it into any program. In a file ``word_count.yml``::

    - insert:
        - id: word-count
          plugin: word_count
          config:
            largest: 1000000

and from the root of the repository, with Python told where the module is::

    PYTHONPATH=examples uv run columbina run --patch word_count.yml "Count the words in README.md."

The tool reaches the file through the seam of the file system, the way the tools that come
with Columbina do. So the permissions decide whether a call may go ahead, a path that leads
out of the working directory is asked about, and what is read is the file that the decision
was about.
"""

import asyncio

from pydantic import BaseModel, Field

from columbina.fs import FS, Access
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

# What the profile calls the plugin, and the services it needs: it loads once both are there.
name = "word-count"
inject = (TOOLS, FS)


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        largest: How many bytes a file may have for its words to be counted.
    """

    largest: int = Field(default=1_000_000, gt=0)


class CountArgs(BaseModel):
    """The arguments of the tool, which the model sees as a JSON schema."""

    path: str = Field(description="The file. A relative path starts at the working directory.")


def apply(ctx: Context, config: Config) -> None:
    """Register the tool."""
    fs = ctx.use(FS)

    def target(args: CountArgs, run: ToolRun) -> str:
        # What the permissions decide about, before the call goes ahead.
        return fs.locate(run.session, run.cwd, args.path).relative

    async def count(args: CountArgs, run: ToolRun) -> str:
        where = fs.locate(run.session, run.cwd, args.path)
        if where.relative != run.target:
            raise ToolFailure(f"{args.path} leads somewhere else now. Try again.", code="MOVED")
        access = Access(run.session, run.cwd, where, "read", "count_words", call_id=run.call_id)
        permit = fs.permit(access)

        def words() -> int:
            with permit.open() as opened:
                data = opened.file.read(config.largest + 1)
            if len(data) > config.largest:
                raise ToolFailure(f"{args.path} is larger than {config.largest} bytes.")
            return len(data.decode("utf-8", errors="replace").split())

        try:
            # Reading blocks, so it is done in a thread of its own.
            found = await asyncio.to_thread(words)
        except FileNotFoundError:
            raise ToolFailure(f"There is no file {args.path}.", code="NOT_FOUND") from None
        except OSError as error:
            raise ToolFailure(f"{args.path} cannot be read: {error}") from error
        return f"{args.path} has {found} words."

    ctx.use(TOOLS).add(
        ctx,
        Tool(
            name="count_words",
            description="Count the words in a text file of the working directory.",
            parameters=CountArgs,
            execute=count,
            concurrency_safe=True,
            effect="read",
            target=target,
        ),
    )
