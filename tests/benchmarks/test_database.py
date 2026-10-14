"""Queries that only read: ``bench.database``."""

import sqlite3
from pathlib import Path

import pytest

from bench.database import QueryError, Rows, ordered, read, same


@pytest.fixture
def shop(tmp_path: Path) -> Path:
    file = tmp_path / "shop.sqlite"
    with sqlite3.connect(file) as made:
        made.execute("CREATE TABLE items (name TEXT, price REAL, stock INTEGER)")
        made.executemany(
            "INSERT INTO items VALUES (?, ?, ?)",
            [("pen", 1.5, 10), ("ink", 3.25, 0), ("pad", 2.0, 4)],
        )
    made.close()
    return file


def test_a_query_that_reads_returns_its_columns_and_rows(shop: Path) -> None:
    found = read(shop, "SELECT name, stock FROM items WHERE stock > 0 ORDER BY name")

    assert found == Rows(("name", "stock"), (("pad", 4), ("pen", 10)))


def test_only_as_many_rows_are_kept_as_are_asked_for(shop: Path) -> None:
    assert read(shop, "SELECT name FROM items ORDER BY name", max_rows=2) == Rows(
        ("name",), (("ink",), ("pad",)), more=True
    )
    assert read(shop, "SELECT name FROM items", max_rows=3).more is False


@pytest.mark.parametrize(
    "sql",
    [
        "DELETE FROM items",
        "UPDATE items SET stock = 0",
        "INSERT INTO items VALUES ('cap', 1, 1)",
        "DROP TABLE items",
        "CREATE TABLE more (x)",
        "ATTACH DATABASE 'other.sqlite' AS other",
        "PRAGMA table_info(items)",
    ],
)
def test_a_statement_that_would_do_more_than_read_is_refused(
    shop: Path, sql: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A database that is attached is made where the program runs, should the guard fail.
    monkeypatch.chdir(shop.parent)

    with pytest.raises(QueryError):
        read(shop, sql)
    assert len(read(shop, "SELECT * FROM items").rows) == 3
    assert not (shop.parent / "other.sqlite").exists()


def test_a_statement_that_returns_no_rows_is_said_to_be_no_query(shop: Path) -> None:
    with pytest.raises(QueryError, match="it returns no rows"):
        read(shop, "-- nothing but a comment")


def test_two_statements_are_not_run(shop: Path) -> None:
    with pytest.raises(QueryError, match="one statement"):
        read(shop, "SELECT 1; DELETE FROM items")


def test_a_query_that_fails_says_what_sqlite_says(shop: Path) -> None:
    with pytest.raises(QueryError, match="no such table: orders"):
        read(shop, "SELECT * FROM orders")


def test_a_query_that_runs_too_long_is_broken_off(shop: Path) -> None:
    endless = (
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT count(*) FROM n"
    )

    with pytest.raises(QueryError, match=r"ran for more than 0\.2 seconds"):
        read(shop, endless, seconds=0.2)


def test_a_database_that_is_not_there_is_said_to_be_so(tmp_path: Path) -> None:
    with pytest.raises(QueryError, match="cannot be opened"):
        read(tmp_path / "gone.sqlite", "SELECT 1")


@pytest.mark.parametrize(
    ("sql", "expected"),
    [
        ("SELECT a FROM t ORDER BY a", True),
        ("select a from t order  by a desc", True),
        ("SELECT a FROM (SELECT a FROM t ORDER BY a) LIMIT 3", False),
        ("SELECT a, SUM(b) FROM t GROUP BY a ORDER BY SUM(b) DESC", True),
        ("SELECT a, ROW_NUMBER() OVER (ORDER BY b) FROM t", False),
        ("SELECT a FROM t", False),
        ("SELECT border_byte FROM t", False),
    ],
)
def test_a_query_whose_last_clause_orders_its_rows_is_ordered(sql: str, expected: bool) -> None:
    assert ordered(sql) is expected


def test_rows_are_the_same_in_any_order_unless_order_counts() -> None:
    one = Rows(("a",), ((1,), (2,)))
    other = Rows(("b",), ((2,), (1,)))

    assert same(one, other, in_order=False)
    assert not same(one, other, in_order=True)
    assert same(one, one, in_order=True)


def test_numbers_that_are_not_whole_are_compared_to_six_places() -> None:
    assert same(Rows(("a",), ((0.1 + 0.2,),)), Rows(("a",), ((0.3,),)), in_order=True)
    assert not same(Rows(("a",), ((0.3001,),)), Rows(("a",), ((0.3,),)), in_order=True)
    assert not same(Rows(("a",), ((1,), (1,))), Rows(("a",), ((1,),)), in_order=False)
