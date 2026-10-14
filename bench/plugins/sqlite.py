"""The tools of a task that asks a database: its tables, a query that only reads, and the answer.

This module is a plugin of the benchmark. The patch ``bench/patches/sql.yml``
loads it for an attempt at a task of the category ``sql``, with the database
of the task. The database lies outside the working directory, so a model
reaches it through these tools alone, and every query runs read-only, see
:mod:`bench.database`.

The tools say that they do nothing to the world outside the conversation: the
database is a copy that belongs to the attempt, and nothing in it changes.
"""

import asyncio
from pathlib import Path

from pydantic import BaseModel, Field

from bench import database
from bench.grade import SUBMIT_SQL
from columbina.kernel import Context
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = ["Config", "apply", "inject", "name", "shown"]

name = "bench-sqlite"
inject = (TOOLS,)

_FAILED = "QUERY_FAILED"
_SCHEMA = "SELECT name, sql FROM sqlite_master WHERE type = 'table' ORDER BY name"


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        database: The file of the database.
        max_rows: How many rows of a query a model is shown.
    """

    database: Path
    max_rows: int = Field(default=50, gt=0)


class _Nothing(BaseModel):
    """No arguments."""


class _Query(BaseModel):
    sql: str = Field(description="One SQL statement of SQLite that reads, such as a SELECT.")


class _Answer(BaseModel):
    sql: str = Field(
        description="The query whose rows answer the question: one statement that reads."
    )


def shown(rows: database.Rows) -> str:
    """Show rows as lines of values apart by tabs, the names of the columns first."""
    lines = ["\t".join(rows.columns)]
    lines.extend(
        "\t".join("NULL" if value is None else str(value) for value in row) for row in rows.rows
    )
    count = len(rows.rows)
    if rows.more:
        lines.append(f"(the first {count} rows; there are more)")
    else:
        lines.append(f"({count} row{'' if count == 1 else 's'})")
    return "\n".join(lines)


def apply(ctx: Context, config: Config) -> None:
    """Register the three tools of a task that asks a database."""

    async def read(sql: str, max_rows: int | None) -> database.Rows:
        try:
            return await asyncio.to_thread(database.read, config.database, sql, max_rows=max_rows)
        except database.QueryError as error:
            raise ToolFailure(f"The query cannot be run: {error}", code=_FAILED) from error

    async def tables(args: _Nothing, run: ToolRun) -> str:
        rows = await read(_SCHEMA, None)
        return "\n\n".join(f"{name}:\n{sql}" for name, sql in rows.rows)

    async def query(args: _Query, run: ToolRun) -> str:
        return shown(await read(args.sql, config.max_rows))

    async def submit(args: _Answer, run: ToolRun) -> str:
        count = len((await read(args.sql, None)).rows)
        rows = "1 row" if count == 1 else f"{count} rows"
        return f"The query is submitted as the answer. It returns {rows}."

    tools = ctx.use(TOOLS)
    tools.add(
        ctx,
        Tool(
            name="list_tables",
            description="Show the tables of the database and how each was made.",
            parameters=_Nothing,
            execute=tables,
            concurrency_safe=True,
        ),
    )
    tools.add(
        ctx,
        Tool(
            name="run_query",
            description=(
                "Run a query that reads the database, and see its rows. At most "
                f"{config.max_rows} rows are shown."
            ),
            parameters=_Query,
            execute=query,
            concurrency_safe=True,
        ),
    )
    tools.add(
        ctx,
        Tool(
            name=SUBMIT_SQL,
            description=(
                "Submit the query whose rows answer the question. The last query that is "
                "submitted counts."
            ),
            parameters=_Answer,
            execute=submit,
        ),
    )
