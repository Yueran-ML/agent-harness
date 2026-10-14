"""The tasks that are made from a seed: ``bench.generate``."""

import json
import re
from pathlib import Path

import pytest

from bench import database
from bench.check import check
from bench.generate import COMPUTE_COUNT, compute, main, sql
from bench.task import load, tasks

# Reckoning


def test_the_same_seed_makes_the_same_problems() -> None:
    assert compute.made(7, 20) == compute.made(7, 20)
    assert compute.made(7, 20) != compute.made(8, 20)


@pytest.mark.parametrize("seed", [1, 2, 3])
def test_no_number_is_written_in_a_problem_and_each_is_named_once(seed: int) -> None:
    for problem in compute.made(seed, 60):
        assert not re.search(r"\d", problem.text), problem.text
        named = re.findall(r"\[(\w+)\]", problem.text)
        assert sorted(named) == sorted(problem.values), problem.text
        assert compute.LOWEST <= problem.answer <= compute.HIGHEST


def test_the_problems_go_round_the_places_and_the_lengths() -> None:
    problems = compute.made(5, 12)

    places = [problem.id.split("-")[2] for problem in problems]
    assert places[:3] == ["factory", "commerce", "logistics"]
    assert [problem.id for problem in problems[:2]] == ["compute-01-factory", "compute-02-commerce"]


def test_each_step_does_what_its_words_say() -> None:
    steps: list[compute.Step] = [
        ("start", {"r": 10, "h": 5}),
        ("mul_days", {"d": 3}),
        ("pct_loss", {"pct": 20}),
        ("div_group", {"g": 4}),
        ("add_batch", {"b": 15}),
        ("sub_loss", {"s": 5}),
        ("mul_value", {"p": 3}),
    ]

    assert compute.run(steps) == [50, 150, 120, 30, 45, 40, 120]


@pytest.mark.parametrize(
    "step",
    [("pct_loss", {"pct": 25}), ("div_group", {"g": 7})],
    ids=["a share that is not whole", "a split that is not even"],
)
def test_a_step_that_does_not_come_out_whole_ends_the_program(
    step: tuple[str, dict[str, int]],
) -> None:
    assert compute.run([("start", {"r": 3, "h": 5}), step]) is None


def test_steps_before_the_days_happen_each_day_and_the_ones_after_once() -> None:
    steps: list[compute.Step] = [
        ("start", {"r": 10, "h": 5}),
        ("add_batch", {"b": 15}),
        ("mul_days", {"d": 3}),
        ("sub_loss", {"s": 5}),
    ]

    assert compute.text(steps, "commerce", "the store") == (
        "Each day, the store handles [hourly_rate] boxes per hour for [hours_per_day] hours. "
        "Each day, an extra rush order adds [rush_order] more boxes. This daily routine repeats "
        "for [days] days in a row, and all subsequent operations apply to the combined total. "
        "Then [damaged] boxes are damaged in handling and written off. How many boxes are left "
        "in total at the end?"
    )


def test_without_days_everything_happens_once_and_a_price_asks_for_dollars() -> None:
    steps: list[compute.Step] = [
        ("start", {"r": 10, "h": 5}),
        ("div_group", {"g": 5}),
        ("mul_value", {"p": 4}),
    ]

    assert compute.text(steps, "logistics", "the warehouse") == (
        "On one day, the warehouse handles [hourly_rate] packages per hour for [hours_per_day] "
        "hours. The packages remaining at that point are then split evenly into [shipments] "
        "equal shipments, and only one shipment is kept. Each remaining package brings in "
        "[price] dollars when sold. How many dollars does this bring in total?"
    )


def test_the_days_as_the_last_step_say_no_more_than_that() -> None:
    steps: list[compute.Step] = [("start", {"r": 10, "h": 5}), ("mul_days", {"d": 4})]

    assert compute.text(steps, "factory", "the plant").endswith(
        "This daily routine repeats for [days] days in a row. How many parts are left in total "
        "at the end?"
    )


def test_a_problem_is_written_as_a_task_with_its_numbers_kept_apart(tmp_path: Path) -> None:
    (problem,) = compute.made(3, 1)
    compute.written([problem], tmp_path)

    loaded = load(tmp_path / problem.id)
    assert loaded.task.category == "compute"
    assert loaded.task.prompt.startswith(problem.text)
    values = json.loads((loaded.directory / "private" / "values.json").read_text("utf-8"))
    assert values == problem.values
    answer = json.loads((loaded.directory / "reference" / "answer.json").read_text("utf-8"))
    assert answer == {"value": problem.answer, "lookups": list(problem.values)}
    assert check(loaded, tmp_path / "scratch") == []


# SQL


def test_the_same_seed_makes_the_same_shop() -> None:
    assert sql.dump(4) == sql.dump(4)
    assert sql.dump(4) != sql.dump(5)


def test_every_question_tells_the_two_shops_of_the_benchmark_apart(tmp_path: Path) -> None:
    sql.written(sql.QUESTIONS, tmp_path, seed=11, changed=12)

    for question in sql.QUESTIONS:
        loaded = load(tmp_path / question.id)
        built = database.built(loaded, tmp_path / "built" / question.id)
        first = database.read(built / "database.sqlite", question.sql)
        second = database.read(built / "changed.sqlite", question.sql)
        assert first.rows, question.id
        assert second.rows, question.id
        assert first.rows != second.rows, question.id


def test_a_question_that_does_not_tell_the_shops_apart_is_refused(tmp_path: Path) -> None:
    same = sql.Question(
        "sql-products", "How many products are there?", "SELECT COUNT(*) FROM products"
    )

    with pytest.raises(ValueError, match="sql-products does not tell the two shops apart"):
        sql.written((same,), tmp_path, seed=11, changed=12)
    assert not (tmp_path / "sql-products").exists()


def test_a_question_that_finds_nothing_is_refused(tmp_path: Path) -> None:
    empty = sql.Question(
        "sql-none", "Who joined in 1999?", "SELECT name FROM customers WHERE joined < '2000'"
    )

    with pytest.raises(ValueError, match="sql-none"):
        sql.written((empty,), tmp_path, seed=11, changed=12)


def test_the_prompt_of_a_task_is_written_as_a_block_and_read_back_whole(tmp_path: Path) -> None:
    sql.written(sql.QUESTIONS[:1], tmp_path, seed=11, changed=12)
    text = (tmp_path / sql.QUESTIONS[0].id / "task.yml").read_text(encoding="utf-8")

    assert "prompt: |\n" in text
    assert load(tmp_path / sql.QUESTIONS[0].id).task.prompt == sql.PROMPT.format(
        text=sql.QUESTIONS[0].text
    )


# Both


def test_the_tasks_are_made_again_in_place_of_the_old_ones(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    stale = tmp_path / "compute" / "compute-99-old"
    stale.mkdir(parents=True)

    assert main([str(tmp_path)]) == 0

    assert not stale.exists()
    made = tasks(tmp_path)
    assert len(made) == len(sql.QUESTIONS) + COMPUTE_COUNT
    assert capsys.readouterr().out.startswith(f"{len(sql.QUESTIONS)} tasks of SQL and")


def test_the_tasks_in_the_repository_are_the_ones_the_seeds_make(tmp_path: Path) -> None:
    # Whoever changes a generator writes the tasks again with it.
    main([str(tmp_path)])
    here = Path(__file__).resolve().parents[2] / "bench" / "tasks"

    for category in ("sql", "compute"):
        made = sorted(path.relative_to(tmp_path) for path in (tmp_path / category).rglob("*"))
        kept = sorted(path.relative_to(here) for path in (here / category).rglob("*"))
        assert made == kept
        for name in made:
            if (tmp_path / name).is_file():
                assert (tmp_path / name).read_bytes() == (here / name).read_bytes(), name
