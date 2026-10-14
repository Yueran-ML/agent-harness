"""The tools of a task of many steps of reckoning: a number looked up by its name, and the answer.

This module is a plugin of the benchmark. The patch ``bench/patches/compute.yml``
loads it for an attempt at a task of the category ``compute``, with the file
that holds the numbers of the task. The file lies outside the working
directory, so a model learns a number only by looking it up here.
"""

import json
from pathlib import Path

from pydantic import BaseModel, Field

from bench.grade import LOOKUP, SUBMIT_ANSWER, named
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = ["Config", "apply", "inject", "name"]

name = "bench-lookup"
inject = (TOOLS,)


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        values: The file of JSON that maps each name to its number.
    """

    values: Path


class _Name(BaseModel):
    name: str = Field(description="The name of the number, as the task writes it in brackets.")


class _Answer(BaseModel):
    value: int = Field(description="The answer, a whole number.")


def apply(ctx: Context, config: Config) -> None:
    """Register the two tools of a task of many steps of reckoning."""
    values: dict[str, int] = json.loads(config.values.read_text(encoding="utf-8"))

    async def lookup(args: _Name, run: ToolRun) -> str:
        key = named(args.name)
        if not isinstance(key, str) or key not in values:
            raise ToolFailure(
                f"There is no number called {args.name!r}. Use a name that the task writes in "
                "brackets, without the brackets.",
                code="NO_SUCH_NAME",
            )
        return f"{key} = {values[key]}"

    async def submit(args: _Answer, run: ToolRun) -> str:
        return f"{args.value} is submitted as the answer."

    tools = ctx.use(TOOLS)
    tools.add(
        ctx,
        Tool(
            name=LOOKUP,
            description="Look up a number of the task by the name that it has in brackets.",
            parameters=_Name,
            execute=lookup,
            concurrency_safe=True,
        ),
    )
    tools.add(
        ctx,
        Tool(
            name=SUBMIT_ANSWER,
            description="Submit the answer of the task. The last answer that is submitted counts.",
            parameters=_Answer,
            execute=submit,
        ),
    )
