"""The commands of the benchmark: ``bench.cli`` and ``python -m bench``."""

import argparse
import asyncio
import functools
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

from bench import cli, table
from bench.check import TASKS
from bench.cli import MATRIX, USAGE, attempts_of, main, run_parser
from bench.ollama import Ollama
from bench.run import Attempt, Model
from support.ollama import FAKE, asked, free_port

REPO = Path(__file__).resolve().parents[2]
MODEL = Model("p", "m")


@pytest.mark.parametrize("argv", [[], ["race"]], ids=["nothing", "no such command"])
def test_without_a_command_it_says_how_it_is_called(
    argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(argv) == 2
    assert capsys.readouterr().err == USAGE + "\n"


def test_a_command_is_run_with_the_rest_of_the_arguments(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["generate", str(tmp_path)]) == 0
    assert main(["check", str(tmp_path / "compute")]) == 0
    assert capsys.readouterr().out.rstrip().endswith("30 tasks, all of them good")


def test_the_benchmark_runs_as_a_module(tmp_path: Path) -> None:
    done = subprocess.run(
        [sys.executable, "-m", "bench", "generate", str(tmp_path)],
        cwd=REPO,
        env={**os.environ, "PYTHONPATH": str(REPO)},
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )

    assert done.returncode == 0, done.stderr
    assert "tasks of SQL" in done.stdout


# Which attempts


def arguments(*argv: str) -> argparse.Namespace:
    return run_parser().parse_args(["--provider", "p", "--model", "m", "--out", "x", *argv])


def test_the_attempts_go_task_by_task_and_repeat_by_repeat() -> None:
    chosen = attempts_of(arguments("--task", "sql-orders-per-status", "--repeats", "2"), MODEL)

    assert [(attempt.loaded.task.id, attempt.variant, attempt.repeat) for attempt in chosen] == [
        ("sql-orders-per-status", "all", 1),
        ("sql-orders-per-status", "all", 2),
    ]


def test_the_attempts_are_chosen_by_category_by_file_and_by_variant(tmp_path: Path) -> None:
    named = tmp_path / "some.txt"
    named.write_text(
        "# a few\ncode-lru-cache-eviction  # one line\n\nsql-top-products\n", encoding="utf-8"
    )

    chosen = attempts_of(
        arguments("--tasks-file", str(named), "--category", "sql", "--variant", "no-todo"), MODEL
    )

    assert [(attempt.loaded.task.id, attempt.variant) for attempt in chosen] == [
        ("sql-top-products", "no-todo")
    ]


def test_without_a_choice_every_task_is_attempted() -> None:
    assert len(attempts_of(arguments(), MODEL)) == 72


def test_a_task_that_there_is_none_of_is_said_to_be_so() -> None:
    with pytest.raises(SystemExit, match="there is no task code-none, sql-none"):
        attempts_of(arguments("--task", "sql-none", "--task", "code-none"), MODEL)


def test_the_tasks_of_the_matrix_are_five_of_each_category() -> None:
    chosen = attempts_of(arguments("--tasks-file", str(MATRIX)), MODEL)

    categories = [attempt.loaded.task.category for attempt in chosen]
    assert sorted(categories) == sorted(["code", "files", "sql", "compute"] * 5)


# A run and its tables


def scripted(directory: Path) -> Path:
    """Write a change to the profile that has a scripted model solve the task of SQL."""
    query = (TASKS / "sql" / "sql-orders-per-status" / "reference" / "answer.sql").read_text(
        encoding="utf-8"
    )
    replies = [
        {"calls": [{"name": "submit_sql", "arguments": {"sql": query}}]},
        {"text": "Done."},
    ]
    model = {"provider": "deepseek", "replies": replies}
    patch = directory / "patch.yml"
    patch.write_text(
        yaml.safe_dump(
            [{"id": "model", "plugin": "columbina.llm.adapters.scripted", "config": model}]
        ),
        encoding="utf-8",
    )
    return patch


def test_a_run_makes_its_attempts_and_writes_their_lines(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    run = tmp_path / "run"

    code = main(
        ["run", "--provider", "deepseek", "--model", "deepseek-flash",
         "--task", "sql-orders-per-status", "--sandbox", "none", "--patch", str(scripted(tmp_path)),
         "--out", str(run)]
    )  # fmt: skip

    assert code == 0
    (line,) = table.read([run / "results.jsonl"])
    assert line["passed"], line
    assert "1/1 sql-orders-per-status #1: passed" in capsys.readouterr().out
    assert main(["table", "acceptance", str(run / "results.jsonl")]) == 0
    assert "| deepseek/deepseek-flash | - | - | 1/1 = 100%" in capsys.readouterr().out


def test_a_run_with_an_ollama_of_its_own_points_its_attempts_at_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    port = free_port()
    started: list[int] = []

    def ollama(*, context: int) -> Ollama:
        started.append(context)
        return Ollama(tmp_path / "models", port=port, context=context, command=FAKE)

    monkeypatch.setattr(cli, "Ollama", ollama)
    run = tmp_path / "run"

    code = main(
        ["run", "--provider", "ollama", "--model", "qwen", "--ollama", "--context-window", "8192",
         "--task", "sql-orders-per-status", "--sandbox", "none", "--patch", str(scripted(tmp_path)),
         "--out", str(run)]
    )  # fmt: skip

    assert code == 0
    assert started == [8192]
    (line,) = table.read([run / "results.jsonl"])
    assert line["model"] == "ollama/qwen"
    chosen = yaml.safe_load((Path(line["record"]) / "home" / "provider.yml").read_text("utf-8"))
    own = {"model": "qwen", "base_url": f"http://127.0.0.1:{port}/v1", "context_window": 8192}
    assert chosen == {"use": "ollama", "services": {"ollama": own}}
    assert asyncio.run(asked(port)) is None


def test_the_attempts_take_turns_at_an_ollama_of_its_own_while_it_answers(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    port = free_port()
    monkeypatch.setattr(
        cli, "Ollama", functools.partial(Ollama, tmp_path / "models", port=port, command=FAKE)
    )
    handed: dict[str, Any] = {}

    async def made(attempts: list[Attempt], root: Path, **choices: object) -> list[object]:
        handed.update(choices, attempts=attempts, told=await asked(port))
        return []

    monkeypatch.setattr(cli, "made", made)

    code = main(
        ["run", "--provider", "ollama", "--model", "qwen", "--ollama", "--jobs", "4",
         "--task", "sql-top-products", "--out", str(tmp_path / "run")]
    )  # fmt: skip

    assert code == 0
    assert handed["jobs"] == 1
    assert handed["told"]["context"] == "16384"
    local = Model("ollama", "qwen", f"http://127.0.0.1:{port}/v1", 16384)
    assert [attempt.model for attempt in handed["attempts"]] == [local]


@pytest.mark.parametrize("patch", [None, ""], ids=["no patch", "an empty patch"])
def test_a_run_hands_its_choices_to_the_attempts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, patch: str | None
) -> None:
    handed: dict[str, Any] = {}

    async def made(attempts: list[Attempt], root: Path, **choices: object) -> list[object]:
        handed.update(choices, attempts=attempts, root=root)
        return []

    monkeypatch.setattr(cli, "made", made)
    more: list[str] = []
    if patch is not None:
        (tmp_path / "patch.yml").write_text(patch, encoding="utf-8")
        more = ["--patch", str(tmp_path / "patch.yml")]

    code = main(
        ["run", "--provider", "p", "--model", "m", "--task", "sql-top-products", "--jobs", "3",
         "--out", str(tmp_path / "run"), *more]
    )  # fmt: skip

    assert code == 0
    assert handed == {
        "attempts": attempts_of(arguments("--task", "sql-top-products"), MODEL),
        "root": tmp_path / "run",
        "jobs": 3,
        "sandbox": "docker",
        "extra": [],
    }


def test_a_run_that_names_a_task_there_is_none_of_starts_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(cli, "Ollama", None)

    with pytest.raises(SystemExit, match="there is no task none-such"):
        main(
            ["run", "--provider", "p", "--model", "m", "--ollama", "--task", "none-such",
             "--out", str(tmp_path / "run")]
        )  # fmt: skip

    assert not (tmp_path / "run").exists()


def test_a_run_inside_a_repository_starts_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(cli, "Ollama", None)
    (tmp_path / ".git").mkdir()

    with pytest.raises(SystemExit, match="lies in the repository"):
        main(
            ["run", "--provider", "p", "--model", "m", "--ollama", "--task", "sql-top-products",
             "--out", str(tmp_path / "run")]
        )  # fmt: skip

    assert not (tmp_path / "run").exists()


def test_a_table_is_written_to_a_file_where_one_is_named(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    results = tmp_path / "results.jsonl"
    usage = {"input_tokens": 10, "output_tokens": 1}
    line = {"task": "t", "category": "code", "variant": "all", "usage": usage}
    results.write_text(
        json.dumps({**line, "model": "ollama/q", "passed": True})
        + "\n"
        + json.dumps({**line, "model": "ollama/r", "passed": False})
        + "\n",
        encoding="utf-8",
    )

    assert main(["table", "matrix", str(results), "--out", str(tmp_path / "m.md")]) == 0
    assert "| ollama/q |" in (tmp_path / "m.md").read_text(encoding="utf-8")
    assert main(["table", "variants", str(results), "--out", str(tmp_path / "v.md")]) == 0
    assert (tmp_path / "v.md").read_text(encoding="utf-8").splitlines()[2].startswith("| all | 1/1")
    assert main(["table", "variants", str(results), "--model", "ollama/r"]) == 0
    assert capsys.readouterr().out.splitlines()[2].startswith("| all | 0/1")
