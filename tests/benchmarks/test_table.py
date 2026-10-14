"""The tables of results: ``bench.table``."""

import json
from pathlib import Path
from typing import Any

import pytest

from bench.table import acceptance, matrix, newcombe, read, variants, wilson


def line(
    task: str = "code-one",
    *,
    category: str = "code",
    model: str = "deepseek/deepseek-flash",
    variant: str = "all",
    passed: bool = True,
    put: int = 1000,
    out: int = 100,
    cached: int = 0,
    written: int = 0,
) -> dict[str, Any]:
    # As Columbina counts them: what was put in and not read from a cache, apart from the rest.
    usage = {
        "input_tokens": put,
        "output_tokens": out,
        "cache_read_tokens": cached,
        "cache_write_tokens": written,
    }
    return {
        "task": task,
        "category": category,
        "model": model,
        "variant": variant,
        "passed": passed,
        "usage": usage,
    }


def test_the_interval_of_wilson_is_the_one_that_was_published() -> None:
    # Newcombe (1998), Two-sided confidence intervals for the single proportion, 81 of 263.
    assert wilson(81, 263) == pytest.approx((0.2553, 0.3662), abs=1e-4)
    assert wilson(5, 10) == pytest.approx((0.2366, 0.7634), abs=1e-4)
    assert wilson(10, 10) == pytest.approx((0.7225, 1.0), abs=1e-4)
    assert wilson(0, 10)[0] == pytest.approx(0.0)


def test_a_rate_of_nothing_could_be_anything() -> None:
    assert wilson(0, 0) == (0.0, 1.0)


def test_the_interval_of_a_difference_is_the_one_that_was_published() -> None:
    # Newcombe (1998), Interval estimation for the difference between independent
    # proportions, 56 of 70 against 48 of 80, by the method of score without correction.
    assert newcombe((56, 70), (48, 80)) == pytest.approx((0.0524, 0.3339), abs=1e-4)
    assert newcombe((8, 10), (5, 10)) == pytest.approx((-0.1067, 0.5999), abs=1e-4)


def test_the_lines_of_several_runs_are_read_together(tmp_path: Path) -> None:
    (tmp_path / "a.jsonl").write_text(json.dumps(line("a")) + "\n\n", encoding="utf-8")
    (tmp_path / "b.jsonl").write_text(json.dumps(line("b")) + "\n", encoding="utf-8")

    lines = read([tmp_path / "a.jsonl", tmp_path / "b.jsonl"])

    assert [read_line["task"] for read_line in lines] == ["a", "b"]


def test_the_acceptance_table_has_a_row_for_each_model_and_a_column_for_each_category() -> None:
    lines = [
        line(passed=True, put=1000, cached=500),
        line(passed=False, put=4000, cached=0),
        line("sql-one", category="sql", passed=True, put=2000, cached=400),
        line(model="deepseek/deepseek-pro", passed=True, written=500),
        line(variant="no-todo", passed=False),
        line(model="local/none", passed=False, put=0, out=0),
    ]

    assert acceptance(lines).splitlines() == [
        "| Model | code | files | sql | compute | All "
        "| Input tokens | Output tokens | From cache |",
        "|---|---|---|---|---|---|---|---|---|",
        "| deepseek/deepseek-flash | 1/2 = 50% [9%, 91%] | - | 1/1 = 100% [21%, 100%] | - "
        "| 2/3 = 67% [21%, 94%] | 2400 | 100 | 11% |",
        "| deepseek/deepseek-pro | 1/1 = 100% [21%, 100%] | - | - | - "
        "| 1/1 = 100% [21%, 100%] | 1500 | 100 | 0% |",
        "| local/none | 0/1 = 0% [0%, 79%] | - | - | - | 0/1 = 0% [0%, 79%] | 0 | 0 | - |",
    ]


def test_the_table_of_variants_sets_each_against_the_whole_program() -> None:
    lines = [
        *(line(f"t{n}", passed=n < 8) for n in range(10)),
        *(line(f"t{n}", variant="no-todo", passed=n < 5) for n in range(10)),
        line("other", model="deepseek/deepseek-pro", variant="no-todo"),
    ]

    rows = variants(lines, "deepseek/deepseek-flash").splitlines()

    assert rows[2] == "| all | 8/10 = 80% [49%, 94%] | | 1000 | 100 |"
    assert rows[3] == "| no-todo | 5/10 = 50% [24%, 76%] | -30% [-60%, +11%] | 1000 | 100 |"
    assert len(rows) == 4


def test_a_variant_without_attempts_of_the_whole_program_has_no_difference() -> None:
    rows = variants([line(variant="no-todo")], "deepseek/deepseek-flash").splitlines()

    assert rows[2] == "| all | - | | 0 | 0 |"
    assert rows[3] == "| no-todo | 1/1 = 100% [21%, 100%] | - | 1000 | 100 |"


def test_the_matrix_marks_a_model_usable_from_half_its_attempts_on() -> None:
    lines = [
        line(model="ollama/qwen-q8", passed=True),
        line("b", model="ollama/qwen-q8", passed=False),
        line("c", model="ollama/qwen-q8", variant="no-todo", passed=False),
        line(model="ollama/qwen-q2", passed=False),
        line("b", model="ollama/qwen-q2", passed=False),
    ]

    rows = matrix(lines).splitlines()

    assert rows[2].endswith("| 0/2 = 0% [0%, 66%] | no |")
    assert rows[2].startswith("| ollama/qwen-q2 |")
    assert rows[3].endswith("| 1/2 = 50% [9%, 91%] | yes |")
