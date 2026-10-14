"""The tools that the tasks of SQL and of reckoning offer: ``bench.plugins``."""

import json
import sqlite3
from pathlib import Path

import pytest
from pydantic import JsonValue

from bench.plugins import lookup as lookup_plugin
from bench.plugins import sqlite as sqlite_plugin
from columbina.agent import Agent
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent
from support.loop import Harness, idle


def results(agent: Agent) -> list[tuple[str, str | None]]:
    """What each call of a tool gave back, and the code of its error if it failed."""
    return [
        (
            "".join(block.text for block in event.data.message.content),
            None if event.data.error is None else event.data.error.code,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


async def calling(harness: Harness, *calls: tuple[str, dict[str, JsonValue]]) -> Agent:
    agent = await harness.agent(
        *(Reply(calls=(Call(name=name, arguments=arguments),)) for name, arguments in calls),
        Reply(text="Done."),
    )
    agent.followup("Go.")
    await idle(agent)
    return agent


@pytest.fixture
def shop(tmp_path: Path) -> Path:
    file = tmp_path / "shop.sqlite"
    made = sqlite3.connect(file)
    with made:
        made.execute("CREATE TABLE items (name TEXT, price REAL)")
        made.executemany(
            "INSERT INTO items VALUES (?, ?)", [("pen", 1.5), ("ink", None), ("pad", 2.0)]
        )
    made.close()
    return file


async def test_the_tables_of_the_database_are_shown(harness: Harness, shop: Path) -> None:
    await harness.ctx.plugin(sqlite_plugin, {"database": str(shop)})

    agent = await calling(harness, ("list_tables", {}))

    assert results(agent) == [("items:\nCREATE TABLE items (name TEXT, price REAL)", None)]


async def test_a_query_shows_its_rows_up_to_a_limit(harness: Harness, shop: Path) -> None:
    await harness.ctx.plugin(sqlite_plugin, {"database": str(shop), "max_rows": 2})

    agent = await calling(
        harness,
        ("run_query", {"sql": "SELECT name, price FROM items ORDER BY name"}),
        ("run_query", {"sql": "SELECT name FROM items WHERE price > 1.9"}),
    )

    assert results(agent) == [
        ("name\tprice\nink\tNULL\npad\t2.0\n(the first 2 rows; there are more)", None),
        ("name\npad\n(1 row)", None),
    ]


async def test_a_query_that_cannot_be_run_says_why(harness: Harness, shop: Path) -> None:
    await harness.ctx.plugin(sqlite_plugin, {"database": str(shop)})

    agent = await calling(
        harness,
        ("run_query", {"sql": "DELETE FROM items"}),
        ("submit_sql", {"sql": "SELECT * FROM orders"}),
    )

    (deleted, code_one), (missing, code_two) = results(agent)
    assert deleted.startswith("The query cannot be run: ")
    assert missing == "The query cannot be run: no such table: orders"
    assert code_one == code_two == "QUERY_FAILED"


async def test_a_query_that_is_submitted_is_run_first(harness: Harness, shop: Path) -> None:
    await harness.ctx.plugin(sqlite_plugin, {"database": str(shop)})

    agent = await calling(harness, ("submit_sql", {"sql": "SELECT name FROM items"}))

    assert results(agent) == [("The query is submitted as the answer. It returns 3 rows.", None)]


@pytest.fixture
def values(tmp_path: Path) -> Path:
    file = tmp_path / "values.json"
    file.write_text(json.dumps({"hourly_rate": 12, "days": 5}), encoding="utf-8")
    return file


async def test_a_number_is_looked_up_by_its_name_with_or_without_brackets(
    harness: Harness, values: Path
) -> None:
    await harness.ctx.plugin(lookup_plugin, {"values": str(values)})

    agent = await calling(
        harness,
        ("lookup", {"name": "hourly_rate"}),
        ("lookup", {"name": " [days] "}),
        ("lookup", {"name": "price"}),
        ("submit_answer", {"value": 60}),
    )

    assert results(agent) == [
        ("hourly_rate = 12", None),
        ("days = 5", None),
        (
            "There is no number called 'price'. Use a name that the task writes in brackets, "
            "without the brackets.",
            "NO_SUCH_NAME",
        ),
        ("60 is submitted as the answer.", None),
    ]
