"""Attempts at tasks: ``bench.run``."""

import asyncio
import json
from pathlib import Path
from typing import Any

import pytest
import yaml

from bench import database, run
from bench.check import TASKS
from bench.grade import Call, Verdict
from bench.run import (
    VARIANTS,
    Attempt,
    Model,
    RunError,
    attempted,
    calls_of,
    done_already,
    judged_again,
    made,
    patch_of,
    said_of,
)
from bench.task import Loaded, load
from columbina import process

MODEL = Model(provider="deepseek", model="deepseek-flash")
CODE = load(TASKS / "code" / "code-lru-cache-eviction")
SQL = load(TASKS / "sql" / "sql-orders-per-status")
COMPUTE = load(TASKS / "compute" / "compute-01-factory")


def scripted(*replies: dict[str, Any]) -> list[dict[str, Any]]:
    """The change to the profile that has a scripted model give these replies."""
    model = {"provider": "deepseek", "replies": [*replies, {"text": "Done."}]}
    return [{"id": "model", "plugin": "columbina.llm.adapters.scripted", "config": model}]


def call(name: str, **arguments: object) -> dict[str, Any]:
    return {"calls": [{"name": name, "arguments": arguments}]}


# What a run said


def test_the_calls_of_a_run_are_read_with_whether_each_went_through() -> None:
    lines = [
        {"type": "status", "turn": 1, "step": 1},
        {"type": "tool_call", "id": "a", "name": "lookup", "arguments": '{"name": "days"}'},
        {"type": "tool_result", "id": "a", "text": "days = 5", "error": None},
        {"type": "tool_call", "id": "b", "name": "lookup", "arguments": '{"name": "x"}'},
        {"type": "tool_result", "id": "b", "text": "no", "error": "NO_SUCH_NAME"},
        {"type": "tool_call", "id": "c", "name": "submit_answer", "arguments": "{oops"},
        {"type": "tool_call", "id": "d", "name": "read", "arguments": "[1, 2]"},
        {"type": "tool_result", "id": "d", "text": "", "error": None},
    ]

    assert calls_of(lines) == (
        Call("lookup", {"name": "days"}),
        Call("lookup", {"name": "x"}, failed=True),
        Call("submit_answer", {}, failed=True),
        Call("read", {}),
    )


def test_what_is_no_line_of_json_is_passed_over() -> None:
    printed = '{"type": "status"}\n\nnot json\n[1, 2]\n{"type": "final"}\n{"type": "fin'

    assert said_of(printed) == [{"type": "status"}, {"type": "final"}]


def test_the_attempts_that_are_done_already_are_read_from_their_lines(tmp_path: Path) -> None:
    results = tmp_path / "run" / "results.jsonl"
    assert done_already(results) == set()
    assert results.parent.is_dir()
    line = {"task": "t", "model": "p/m", "variant": "all", "repeat": 2, "passed": True}
    results.write_text(json.dumps(line) + "\n\n", encoding="utf-8")

    assert done_already(results) == {("t", "p/m", "all", 2)}


# What an attempt is given


def test_an_attempt_is_held_to_the_steps_of_its_task_and_runs_commands_in_a_container(
    tmp_path: Path,
) -> None:
    operations = patch_of(Attempt(CODE, MODEL, "all", 1), tmp_path, sandbox="docker")

    assert operations == [
        {"id": "turn-budget", "config": {"max_steps": CODE.task.max_steps}},
        {
            "id": "sandbox",
            "config": {"provider": "docker", "docker": {"image": "python:3.12-slim"}},
        },
    ]


def test_without_a_container_nothing_is_said_of_the_sandbox(tmp_path: Path) -> None:
    operations = patch_of(Attempt(CODE, MODEL, "no-todo", 1), tmp_path, sandbox="none")

    assert operations == [
        {"id": "turn-budget", "config": {"max_steps": CODE.task.max_steps}},
        {"id": "todo", "disabled": True},
    ]


def test_a_task_of_sql_gets_its_tools_and_its_database(tmp_path: Path) -> None:
    operations = patch_of(Attempt(SQL, MODEL, "all", 1), tmp_path, sandbox="none")

    (inserted,) = [operation for operation in operations if "insert" in operation]
    (entry,) = inserted["insert"]
    assert entry["plugin"] == "bench.plugins.sqlite"
    assert entry["config"] == {"database": str(tmp_path / "database.sqlite")}
    assert database.read(tmp_path / "changed.sqlite", "SELECT COUNT(*) FROM orders").rows


def test_a_task_of_reckoning_gets_its_tools_and_its_numbers(tmp_path: Path) -> None:
    operations = patch_of(Attempt(COMPUTE, MODEL, "all", 1), tmp_path, sandbox="none")

    (inserted,) = [operation for operation in operations if "insert" in operation]
    (entry,) = inserted["insert"]
    assert entry == {
        "id": "bench-lookup",
        "plugin": "bench.plugins.lookup",
        "config": {"values": str(tmp_path / "values.json")},
    }
    assert json.loads((tmp_path / "values.json").read_text("utf-8")) == json.loads(
        (COMPUTE.directory / "private" / "values.json").read_text("utf-8")
    )


def test_every_variant_switches_off_parts_that_are_in_the_program() -> None:
    switched = {operation["id"] for operations in VARIANTS.values() for operation in operations}

    assert switched == {"compaction", "todo", "spill-policy", "system-prompt-conduct"}
    assert VARIANTS["all"] == []


def test_the_directory_of_an_attempt_names_its_task_model_variant_and_repeat() -> None:
    attempt = Attempt(CODE, Model("ollama", "qwen2.5:7b-q4"), "no-todo", 2)

    assert attempt.place == Path("code-lru-cache-eviction/ollama_qwen2.5_7b-q4/no-todo/2")
    assert attempt.key == ("code-lru-cache-eviction", "ollama/qwen2.5:7b-q4", "no-todo", 2)


async def test_an_attempt_is_given_nothing_of_this_machine(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_PROVIDER", "openai")
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(tmp_path / "mine"))
    handed: dict[str, Any] = {}

    async def ran(argv: list[str], **choices: Any) -> process.Completed:
        handed.update(choices, argv=argv)
        return process.Completed(0, False, "", 0, 0, 0)

    monkeypatch.setattr(process, "run", ran)
    attempt = Attempt(SQL, MODEL, "all", 1)

    await attempted(attempt, tmp_path / "run", sandbox="none", container=None)

    place = tmp_path / "run" / attempt.place
    env: dict[str, str] = handed["env"]
    assert {name for name in env if name.startswith("COLUMBINA_")} == {
        "COLUMBINA_HOME",
        "COLUMBINA_AGENTS_HOME",
    }
    assert env["COLUMBINA_HOME"] == str(place / "home")
    assert env["COLUMBINA_AGENTS_HOME"] == str(place / "agents")
    assert not (place / "agents").exists()
    assert handed["cwd"] == place
    assert handed["timeout_seconds"] == SQL.task.timeout_seconds


@pytest.mark.parametrize("kind", ["directory", "file"])
async def test_a_run_is_not_made_inside_a_repository(tmp_path: Path, kind: str) -> None:
    marker = tmp_path / "repository" / ".git"
    marker.parent.mkdir()
    if kind == "directory":
        marker.mkdir()
    else:
        marker.write_text("gitdir: elsewhere\n", encoding="utf-8")
    root = tmp_path / "repository" / "runs" / "one"

    with pytest.raises(RunError, match="lies in the repository"):
        await made([Attempt(CODE, MODEL, "all", 1)], root, jobs=1)

    assert not root.exists()


async def test_no_more_attempts_are_made_at_a_time_than_there_are_jobs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    running: list[int] = []
    most = 0
    handed: list[dict[str, Any]] = []

    async def attempted(attempt: Attempt, root: Path, **choices: Any) -> dict[str, Any]:
        nonlocal most
        running.append(attempt.repeat)
        most = max(most, len(running))
        handed.append(choices)
        await asyncio.sleep(0.05)
        running.remove(attempt.repeat)
        key = dict(zip(("task", "model", "variant", "repeat"), attempt.key, strict=True))
        return {**key, "passed": True, "reason": "completed"}

    monkeypatch.setattr(run, "attempted", attempted)
    attempts = [Attempt(CODE, MODEL, "all", repeat) for repeat in range(1, 6)]
    extra = [{"id": "todo", "disabled": True}]
    told: list[str] = []

    lines = await made(
        attempts, tmp_path, jobs=2, sandbox="none", container="image", extra=extra, told=told.append
    )

    assert most == 2
    assert sorted(line["repeat"] for line in lines) == [1, 2, 3, 4, 5]
    assert handed == [{"sandbox": "none", "container": "image", "extra": extra}] * 5


# Whole attempts, with a scripted model


async def test_an_attempt_at_a_code_task_that_fixes_it_passes(tmp_path: Path) -> None:
    fixed = (CODE.directory / "reference" / "lru.py").read_text(encoding="utf-8")
    attempt = Attempt(CODE, MODEL, "all", 1)

    result = await attempted(
        attempt,
        tmp_path,
        sandbox="none",
        container=None,
        extra=scripted(call("read", path="lru.py"), call("write", path="lru.py", content=fixed)),
    )

    assert result["passed"], result
    assert result["reason"] == "completed"
    assert result["tool_calls"] == 2
    assert result["requests"] == 3
    place = tmp_path / attempt.place
    assert (place / "workspace" / "lru.py").read_text(encoding="utf-8") == fixed
    assert json.loads((place / "result.json").read_text("utf-8")) == result
    kept = (place / "out.jsonl").read_text(encoding="utf-8").splitlines()
    assert json.loads(kept[-1])["type"] == "final"
    assert all(json.loads(line) for line in kept)
    assert (CODE.workspace / "lru.py").read_text(encoding="utf-8") != fixed


async def test_an_attempt_at_a_sql_task_is_judged_by_the_query_it_submitted(tmp_path: Path) -> None:
    query = (SQL.directory / "reference" / "answer.sql").read_text(encoding="utf-8")

    result = await attempted(
        Attempt(SQL, MODEL, "all", 1),
        tmp_path,
        sandbox="none",
        container=None,
        extra=scripted(call("list_tables"), call("submit_sql", sql=query)),
    )

    assert result["passed"], result
    assert result["tool_calls"] == 2


@pytest.mark.parametrize(
    ("model", "stored"),
    [
        (MODEL, {"model": "deepseek-flash"}),
        (
            Model("ollama", "qwen2.5:7b", "http://127.0.0.1:11435/v1", 16384),
            {
                "model": "qwen2.5:7b",
                "base_url": "http://127.0.0.1:11435/v1",
                "context_window": 16384,
            },
        ),
    ],
    ids=["a service that is known", "a service of this machine"],
)
async def test_the_service_is_chosen_in_the_home_of_the_attempt(
    tmp_path: Path, model: Model, stored: dict[str, Any]
) -> None:
    attempt = Attempt(CODE, model, "all", 1)

    await attempted(attempt, tmp_path, sandbox="none", container=None, extra=scripted())

    home = tmp_path / attempt.place / "home"
    chosen = yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8"))
    assert chosen == {"use": model.provider, "services": {model.provider: stored}}


async def test_an_attempt_begins_afresh_where_one_was_made_before(tmp_path: Path) -> None:
    attempt = Attempt(CODE, MODEL, "all", 1)
    left = tmp_path / attempt.place / "workspace" / "left.txt"
    left.parent.mkdir(parents=True)
    left.write_text("from before", encoding="utf-8")

    await attempted(attempt, tmp_path, sandbox="none", container=None, extra=scripted())

    assert not left.exists()
    assert (tmp_path / attempt.place / "workspace" / "lru.py").is_file()


async def test_the_hidden_tests_are_run_where_the_attempt_says(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    where: list[str | None] = []

    def judged(*given: object, container: str | None, private: Path) -> Verdict:
        where.append(container)
        return Verdict(False, "not looked at")

    monkeypatch.setattr(run, "judged", judged)

    result = await attempted(
        Attempt(CODE, MODEL, "all", 1),
        tmp_path,
        sandbox="none",
        container="image",
        extra=scripted(),
    )

    assert where == ["image"]
    assert result["said"] == "not looked at"

    again = judged_again(result)

    assert where == ["image", "python:3.12-slim"]
    assert again == {**result, "passed": False, "said": "not looked at"}


async def test_an_attempt_that_takes_too_long_is_stopped_and_fails(tmp_path: Path) -> None:
    slow = Attempt(
        Loaded(CODE.task.model_copy(update={"timeout_seconds": 0.1}), CODE.directory),
        MODEL,
        "all",
        1,
    )

    result = await attempted(slow, tmp_path, sandbox="none", container=None, extra=scripted())

    assert result["timed_out"]
    assert not result["passed"]
    assert result["reason"] == "timeout"


async def test_attempts_that_are_done_already_are_not_made_again(tmp_path: Path) -> None:
    first = Attempt(CODE, MODEL, "all", 1)
    second = Attempt(CODE, MODEL, "all", 2)
    line = {"task": CODE.task.id, "model": MODEL.label, "variant": "all", "repeat": 1}
    (tmp_path / "results.jsonl").write_text(json.dumps(line) + "\n", encoding="utf-8")
    told: list[str] = []

    lines = await made(
        [first, second],
        tmp_path,
        jobs=2,
        sandbox="none",
        container=None,
        extra=scripted(),
        told=told.append,
    )

    assert [line["repeat"] for line in lines] == [2]
    assert told[0] == "1 of 2 attempts are done already"
    assert told[1].startswith("1/1 code-lru-cache-eviction #2: failed")
    written = (tmp_path / "results.jsonl").read_text(encoding="utf-8").splitlines()
    assert len(written) == 2
    assert not (tmp_path / first.place).exists()
