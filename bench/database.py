"""Queries that only read a database of SQLite, within a limit of time and of rows.

A model writes the queries of a task of the category ``sql``, and a query of
a model must neither change the database nor reach another one. So the file
is opened read-only, the connection is told to refuse writing, and an
authorizer lets a statement read and do nothing else: no ``ATTACH``, no
``PRAGMA``, no change. A query that runs too long is broken off.

A task keeps its databases as text, the statements that make them, so that
what a task holds can be read and compared. They are made into files for
each attempt and each check, :func:`built`.
"""

import re
import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path

from bench.task import Loaded

__all__ = ["DATABASES", "STATEMENTS", "QueryError", "Rows", "built", "ordered", "read", "same"]

STATEMENTS = ("database.sql", "changed.sql")
"""The statements that make the two databases of a task of SQL, in its ``private/``: the one
that the model asks, and a copy with other rows, which a query is run on as well."""

DATABASES = ("database.sqlite", "changed.sqlite")
"""The files that the statements make, in the same order."""

_READING = frozenset(
    {sqlite3.SQLITE_SELECT, sqlite3.SQLITE_READ, sqlite3.SQLITE_FUNCTION, sqlite3.SQLITE_RECURSIVE}
)
"""What a statement may do: read rows, and call functions over them."""

_CHECK_EVERY = 10_000
"""After how many steps of the engine of SQLite the clock is looked at."""


class QueryError(Exception):
    """A query cannot be run, or was refused. The words are those of SQLite, or say why."""


@dataclass(frozen=True)
class Rows:
    """What a query returned.

    Attributes:
        columns: The names of its columns.
        rows: Its rows, as many as were kept.
        more: Whether it returned more rows than were kept.
    """

    columns: tuple[str, ...]
    rows: tuple[tuple[object, ...], ...]
    more: bool = False


def _allowed(action: int, *_: str | None) -> int:
    return sqlite3.SQLITE_OK if action in _READING else sqlite3.SQLITE_DENY


def read(database: Path, sql: str, *, max_rows: int | None = None, seconds: float = 10.0) -> Rows:
    """Run a query that only reads, and return what it returned.

    Args:
        database: The file of the database.
        sql: One statement.
        max_rows: How many rows are kept, or ``None`` to keep them all.
        seconds: How long the query may run.

    Raises:
        QueryError: The query is no single statement that reads, it failed,
            or it ran too long.
    """
    try:
        connection = sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)
    except sqlite3.Error as error:
        raise QueryError(f"the database cannot be opened: {error}") from error
    try:
        connection.execute("PRAGMA query_only = ON")
        connection.set_authorizer(_allowed)
        deadline = time.monotonic() + seconds
        connection.set_progress_handler(lambda: time.monotonic() > deadline, _CHECK_EVERY)
        try:
            cursor = connection.execute(sql)
            if cursor.description is None:
                raise QueryError("it returns no rows. Only a query that reads is run")
            columns = tuple(str(column[0]) for column in cursor.description)
            rows = cursor.fetchall() if max_rows is None else cursor.fetchmany(max_rows + 1)
        except sqlite3.OperationalError as error:
            if time.monotonic() > deadline:
                raise QueryError(f"it ran for more than {seconds:g} seconds") from error
            raise QueryError(str(error)) from error
        except (sqlite3.Error, sqlite3.Warning) as error:
            raise QueryError(str(error)) from error
    finally:
        connection.close()
    more = max_rows is not None and len(rows) > max_rows
    kept = rows if max_rows is None else rows[:max_rows]
    return Rows(columns, tuple(tuple(row) for row in kept), more)


def ordered(sql: str) -> bool:
    """Tell whether a query puts its rows in an order: it has an ``ORDER BY`` outside brackets.

    One inside brackets orders a part of the query, or the rows that a window
    function sees, and not what the query returns.
    """
    depth = 0
    for found in re.finditer(r"\(|\)|\border\s+by\b", sql, re.IGNORECASE):
        if found.group() == "(":
            depth += 1
        elif found.group() == ")":
            depth -= 1
        elif depth == 0:
            return True
    return False


def built(loaded: Loaded, target: Path) -> Path:
    """Make the databases of a task of SQL from their statements, in a directory, and return it.

    The statements are ``private/database.sql`` and ``private/changed.sql``
    of the task, and the files are made afresh: what was there before is
    gone.
    """
    target.mkdir(parents=True, exist_ok=True)
    for statements, name in zip(STATEMENTS, DATABASES, strict=True):
        file = target / name
        file.unlink(missing_ok=True)
        connection = sqlite3.connect(file)
        try:
            # One transaction: a statement that is a transaction of its own waits for the disk.
            made = (loaded.directory / "private" / statements).read_text(encoding="utf-8")
            connection.executescript(f"BEGIN;\n{made}\nCOMMIT;")
        finally:
            connection.close()
    return target


def _plain(value: object) -> object:
    return round(value, 6) if isinstance(value, float) else value


def same(want: Rows, got: Rows, *, in_order: bool) -> bool:
    """Tell whether two queries returned the same rows: in the same order where order counts.

    The names of the columns count for nothing. Numbers that are not whole
    are compared to six places.
    """
    wanted = [tuple(_plain(value) for value in row) for row in want.rows]
    gotten = [tuple(_plain(value) for value in row) for row in got.rows]
    if in_order:
        return wanted == gotten
    return sorted(wanted, key=repr) == sorted(gotten, key=repr)
