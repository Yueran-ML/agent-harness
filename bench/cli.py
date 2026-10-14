"""The commands of ``python -m bench``.

python -m bench check [DIRECTORY]       whether every task can be judged
python -m bench generate [DIRECTORY]    make the tasks of SQL and of reckoning again
python -m bench run --provider P --model M --out DIR [...]
                                        make attempts at the tasks, see run --help
python -m bench table KIND RESULTS... [--model M] [--out FILE]
                                        a table of results: acceptance, variants or matrix
python -m bench judge RUN               judge the attempts of a run again from what they left
"""

import argparse
import asyncio
import json
import sys
from collections.abc import Callable
from dataclasses import replace
from pathlib import Path
from typing import Any

import yaml

from bench import check, generate, table
from bench.check import TASKS
from bench.ollama import MODELS, Ollama
from bench.run import (
    VARIANTS,
    Attempt,
    Model,
    RunError,
    judged_again,
    made,
    outside_repositories,
)
from bench.task import tasks

__all__ = ["COMMANDS", "MATRIX", "USAGE", "attempts_of", "main", "run_parser"]

MATRIX = Path(__file__).resolve().parent / "subsets" / "matrix.txt"
"""The tasks of the matrix of local models."""

USAGE = "usage: python -m bench {check,generate,judge,run,table} ..."


def run_parser() -> argparse.ArgumentParser:
    """Return the parser of the arguments of ``run``."""
    parser = argparse.ArgumentParser(
        prog="python -m bench run", description="Make attempts at the tasks."
    )
    parser.add_argument(
        "--provider", required=True, help="the service of the model, as columbina run takes it"
    )
    parser.add_argument("--model", required=True, help="the model")
    parser.add_argument("--base-url", help="another address of the service")
    parser.add_argument("--context-window", type=int, help="how many tokens the model takes")
    parser.add_argument(
        "--ollama",
        action="store_true",
        help=(
            "start an Ollama of its own for the run, with the models of --ollama-models and "
            "the context window as its context length, 16384 by default; it takes the place "
            "of --base-url"
        ),
    )
    parser.add_argument(
        "--ollama-models",
        type=Path,
        default=MODELS,
        help=f"where the Ollama of the run finds its models; {MODELS.as_posix()} by default",
    )
    parser.add_argument(
        "--variant",
        action="append",
        choices=sorted(VARIANTS),
        help="may be given several times; all by default",
    )
    parser.add_argument(
        "--category", action="append", choices=table.CATEGORIES, help="may be given several times"
    )
    parser.add_argument(
        "--task", action="append", help="the id of a task; may be given several times"
    )
    parser.add_argument(
        "--tasks-file",
        type=Path,
        help="a file of ids of tasks, one a line, with # before what is no id",
    )
    parser.add_argument("--repeats", type=int, default=1, help="how many attempts at each task")
    parser.add_argument("--jobs", type=int, default=4, help="how many attempts at a time")
    parser.add_argument("--sandbox", choices=("docker", "none"), default="docker")
    parser.add_argument("--patch", type=Path, help="a YAML list of more changes to the profile")
    parser.add_argument("--out", type=Path, required=True, help="the directory of the run")
    return parser


def _named(file: Path) -> set[str]:
    """Read the ids of tasks that a file names, one a line, passing over what follows a ``#``."""
    lines = file.read_text(encoding="utf-8").splitlines()
    return {line.split("#", 1)[0].strip() for line in lines} - {""}


def attempts_of(arguments: argparse.Namespace, model: Model) -> list[Attempt]:
    """Return the attempts that the arguments of ``run`` ask for, task by task.

    Raises:
        SystemExit: A task is named that there is none of.
    """
    named = set(arguments.task or ())
    if arguments.tasks_file is not None:
        named |= _named(arguments.tasks_file)
    known = tasks(TASKS)
    unknown = named - {loaded.task.id for loaded in known}
    if unknown:
        raise SystemExit(f"there is no task {', '.join(sorted(unknown))}")
    chosen = [
        loaded
        for loaded in known
        if (not arguments.category or loaded.task.category in arguments.category)
        and (not named or loaded.task.id in named)
    ]
    return [
        Attempt(loaded, model, variant, repeat)
        for variant in arguments.variant or ["all"]
        for loaded in chosen
        for repeat in range(1, arguments.repeats + 1)
    ]


def run(argv: list[str]) -> int:
    """Make the attempts that the arguments ask for. Return 0.

    Raises:
        SystemExit: A task is named that there is none of, or the directory
            of the run lies in a repository. Nothing was started then.
    """
    arguments = run_parser().parse_args(argv)
    model = Model(arguments.provider, arguments.model, arguments.base_url, arguments.context_window)
    chosen = attempts_of(arguments, model)
    try:
        outside_repositories(arguments.out)
    except RunError as error:
        raise SystemExit(str(error)) from None
    extra: list[dict[str, Any]] = []
    if arguments.patch is not None:
        extra = yaml.safe_load(arguments.patch.read_text(encoding="utf-8")) or []

    async def attempts() -> None:
        if not arguments.ollama:
            await made(
                chosen, arguments.out, jobs=arguments.jobs, sandbox=arguments.sandbox, extra=extra
            )
            return
        context = arguments.context_window or 16384
        async with Ollama(arguments.ollama_models, context=context) as server:
            # What the server was given is the window of the model.
            local = replace(model, base_url=server.base_url, context_window=context)
            # One model answers at a time on this machine, so the attempts take turns.
            await made(
                [replace(attempt, model=local) for attempt in chosen],
                arguments.out,
                jobs=1,
                sandbox=arguments.sandbox,
                extra=extra,
            )

    asyncio.run(attempts())
    return 0


def table_command(argv: list[str]) -> int:
    """Print a table of results, or write it to a file. Return 0."""
    parser = argparse.ArgumentParser(prog="python -m bench table")
    parser.add_argument("kind", choices=("acceptance", "variants", "matrix"))
    parser.add_argument("results", type=Path, nargs="+")
    parser.add_argument("--model", help="the model of a table of variants")
    parser.add_argument("--out", type=Path)
    arguments = parser.parse_args(argv)
    lines = table.read(arguments.results)
    if arguments.kind == "acceptance":
        made_table = table.acceptance(lines)
    elif arguments.kind == "variants":
        made_table = table.variants(lines, arguments.model or lines[0]["model"])
    else:
        made_table = table.matrix(lines)
    if arguments.out is None:
        print(made_table, end="")
    else:
        arguments.out.write_text(made_table, encoding="utf-8")
    return 0


def judge_command(argv: list[str]) -> int:
    """Judge the attempts of a run again, write their lines anew, and say what changed. Return 0."""
    parser = argparse.ArgumentParser(
        prog="python -m bench judge",
        description="Judge the attempts of a run again from what they left, as the judge is now.",
    )
    parser.add_argument("run", type=Path, help="the directory of the run")
    arguments = parser.parse_args(argv)
    results = arguments.run / "results.jsonl"
    before = table.read([results])
    after = [judged_again(line) for line in before]
    results.write_text("".join(json.dumps(line) + "\n" for line in after), encoding="utf-8")
    for old, new in zip(before, after, strict=True):
        if old["passed"] != new["passed"]:
            verdict = "passed" if new["passed"] else f"failed: {new['said']}"
            print(f"{new['task']} {new['variant']} #{new['repeat']}: now {verdict}")
    changed = sum(old["passed"] != new["passed"] for old, new in zip(before, after, strict=True))
    print(f"{len(after)} attempts judged again, {changed} with another verdict")
    return 0


COMMANDS: dict[str, Callable[[list[str]], int]] = {
    "check": check.main,
    "generate": generate.main,
    "judge": judge_command,
    "run": run,
    "table": table_command,
}
"""Each command, by its name."""


def main(argv: list[str]) -> int:
    """Run a command of the benchmark. Return what it returns, or 2 if there is no such command."""
    if not argv or argv[0] not in COMMANDS:
        print(USAGE, file=sys.stderr)
        return 2
    return COMMANDS[argv[0]](argv[1:])
