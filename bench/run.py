"""Attempts at tasks: each in a directory of its own, made by ``columbina run``, and judged.

An attempt has a directory below the directory of its run::

    <task>/<model>/<variant>/<repeat>/
        workspace/     a copy of the workspace of the task
        home/          the home of Columbina for this attempt alone, with its service
        private/       what the tools of the task read: the databases, the numbers
        patch.yml      the changes to the profile for this attempt
        out.jsonl      what ``columbina run --json`` said
        err.txt        what it said on its error stream
        result.json    the line of results of the attempt

``columbina run`` runs in a process of its own, with the commands of the
model in a container, and is stopped with all that it started when its time
is up. What it said is read for the calls of tools and the use of tokens, and
the attempt is judged by the category of its task.

An attempt has what its task gives it and nothing of this machine: a home of
its own, no skills or agents of the user, and nothing that the environment
says of Columbina. A run is not made inside a repository, since every attempt
would read what that holds for its own work: the files of instructions, the
skills, the hooks.

One line goes into ``results.jsonl`` for each attempt. An attempt that has a
line there already is not made again, so a run that was broken off is taken
up by running it again.
"""

import asyncio
import json
import os
import re
import shutil
import sys
import time
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, cast

import yaml

from bench import database
from bench.check import TASKS, judged
from bench.grade import Call
from bench.task import Loaded, load
from columbina import process

__all__ = [
    "IMAGE",
    "VARIANTS",
    "Attempt",
    "Model",
    "RunError",
    "attempted",
    "calls_of",
    "done_already",
    "judged_again",
    "made",
    "outside_repositories",
    "patch_of",
    "said_of",
]

IMAGE = "python:3.12-slim"
"""The image that the commands of a model, and the hidden tests of a code task, run in."""

REPO = Path(__file__).resolve().parent.parent
"""The root of the repository, where Python finds the plugins of the benchmark."""

VARIANTS: dict[str, list[dict[str, Any]]] = {
    "all": [],
    "no-compaction": [{"id": "compaction", "disabled": True}],
    "no-todo": [{"id": "todo", "disabled": True}],
    "no-spill": [{"id": "spill-policy", "disabled": True}],
    "no-conduct": [{"id": "system-prompt-conduct", "disabled": True}],
}
"""What is switched off in each variant of the program: all of it is on in ``all``."""

_KEPT = 32 * 1024 * 1024
"""How much of each stream of a run is kept. An attempt says far less than this."""


class RunError(Exception):
    """A run cannot be made. The words say why."""


@dataclass(frozen=True)
class Model:
    """The model that attempts are made with.

    Attributes:
        provider: The service, as ``columbina run --provider`` takes it.
        model: The model, as ``--model`` takes it.
        base_url: Another address of the service, for one on this machine.
        context_window: How many tokens the model takes, where the service
            does not say.
    """

    provider: str
    model: str
    base_url: str | None = None
    context_window: int | None = None

    @property
    def label(self) -> str:
        """The service and the model, as the results name them."""
        return f"{self.provider}/{self.model}"


@dataclass(frozen=True)
class Attempt:
    """One attempt at one task.

    Attributes:
        loaded: The task.
        model: The model that it is made with.
        variant: What is switched off, a key of :data:`VARIANTS`.
        repeat: Which attempt of the same kind this is, from 1.
    """

    loaded: Loaded
    model: Model
    variant: str
    repeat: int

    @property
    def key(self) -> tuple[str, str, str, int]:
        """What tells this attempt from every other in the results."""
        return (self.loaded.task.id, self.model.label, self.variant, self.repeat)

    @property
    def place(self) -> Path:
        """Where its directory lies, below the directory of a run."""
        model = re.sub(r"[^A-Za-z0-9._-]+", "_", self.model.label)
        return Path(self.loaded.task.id) / model / self.variant / str(self.repeat)


def calls_of(lines: Sequence[dict[str, Any]]) -> tuple[Call, ...]:
    """Return the calls of tools that a run reported, each with whether it failed.

    A call that never got a result failed: the run ended before it did.
    Arguments that are no object of JSON are taken for none.
    """
    failed = {
        line.get("id"): line.get("error") is not None
        for line in lines
        if line.get("type") == "tool_result"
    }
    calls: list[Call] = []
    for line in lines:
        if line.get("type") != "tool_call":
            continue
        try:
            read = json.loads(line.get("arguments") or "{}")
        except (TypeError, json.JSONDecodeError):
            read = None
        arguments: dict[str, Any] = cast("dict[str, Any]", read) if isinstance(read, dict) else {}
        calls.append(Call(str(line.get("name")), arguments, failed.get(line.get("id"), True)))
    return tuple(calls)


def patch_of(attempt: Attempt, private: Path, *, sandbox: str) -> list[dict[str, Any]]:
    """Return the changes to the profile for an attempt, and make what its tools read.

    The steps of a turn are limited to those of the task. Commands run in a
    container of :data:`IMAGE` where the sandbox is ``docker``. A task of SQL
    gets its tools and its databases, made in ``private``; a task of
    reckoning gets its tools and its numbers. Last come the parts of the
    program that the variant switches off.
    """
    loaded = attempt.loaded
    operations: list[dict[str, Any]] = [
        {"id": "turn-budget", "config": {"max_steps": loaded.task.max_steps}},
    ]
    if sandbox == "docker":
        operations.append(
            {"id": "sandbox", "config": {"provider": "docker", "docker": {"image": IMAGE}}}
        )
    if loaded.task.category == "sql":
        built = database.built(loaded, private)
        entry = {
            "id": "bench-sqlite",
            "plugin": "bench.plugins.sqlite",
            "config": {"database": str(built / database.DATABASES[0])},
        }
        operations.append({"insert": [entry]})
    elif loaded.task.category == "compute":
        values = private / "values.json"
        shutil.copyfile(loaded.directory / "private" / "values.json", values)
        entry = {
            "id": "bench-lookup",
            "plugin": "bench.plugins.lookup",
            "config": {"values": str(values)},
        }
        operations.append({"insert": [entry]})
    return [*operations, *VARIANTS[attempt.variant]]


def said_of(text: str) -> list[dict[str, Any]]:
    """Read the lines of JSON that a run printed, passing over what is not one.

    The last line of a run that was stopped may have been cut short.
    """
    lines: list[dict[str, Any]] = []
    for line in text.splitlines():
        try:
            read = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(read, dict):
            lines.append(cast("dict[str, Any]", read))
    return lines


async def attempted(
    attempt: Attempt,
    root: Path,
    *,
    sandbox: str = "docker",
    container: str | None = IMAGE,
    extra: Sequence[dict[str, Any]] = (),
) -> dict[str, Any]:
    """Make one attempt, judge it, and return its line of results.

    Args:
        attempt: What is attempted.
        root: The directory of the run. What the directory of the attempt
            held before is gone.
        sandbox: What runs the commands of the model: ``docker`` or ``none``.
        container: The image that the hidden tests of a code task run in, or
            ``None`` to run them with this Python.
        extra: More changes to the profile, after all the others.
    """
    loaded = attempt.loaded
    place = root / attempt.place
    await asyncio.to_thread(_prepared, attempt, place, sandbox, extra)
    argv = [
        sys.executable, "-m", "columbina", "run", "--json",
        "--profile", "headless", "--patch", str(place / "patch.yml"),
        "--cwd", str(place / "workspace"), "--model", attempt.model.model, "--sandbox", sandbox,
        loaded.task.prompt,
    ]  # fmt: skip
    env = {
        **{name: value for name, value in os.environ.items() if not name.startswith("COLUMBINA_")},
        "COLUMBINA_HOME": str(place / "home"),
        # A directory that is not there: the skills and agents of the user are not looked at.
        "COLUMBINA_AGENTS_HOME": str(place / "agents"),
        "PYTHONPATH": str(REPO),
        "PYTHONUTF8": "1",
        # A key comes from the environment. The keyring of the user is left alone.
        "PYTHON_KEYRING_BACKEND": "keyring.backends.null.Keyring",
    }
    began = time.monotonic()
    done = await process.run(
        argv,
        cwd=place,
        env=env,
        timeout_seconds=loaded.task.timeout_seconds,
        max_output_bytes=_KEPT,
        streams="apart",
    )
    seconds = round(time.monotonic() - began, 1)
    return await asyncio.to_thread(_finished, attempt, place, done, seconds, container)


def _prepared(attempt: Attempt, place: Path, sandbox: str, extra: Sequence[dict[str, Any]]) -> None:
    """Make the directory of an attempt afresh, with its workspace, home and patch. This blocks."""
    loaded, model = attempt.loaded, attempt.model
    shutil.rmtree(place, ignore_errors=True)
    shutil.copytree(loaded.workspace, place / "workspace")
    home, private = place / "home", place / "private"
    home.mkdir()
    private.mkdir()
    operations = [*patch_of(attempt, private, sandbox=sandbox), *extra]
    (place / "patch.yml").write_text(
        yaml.safe_dump({"patch": operations}, sort_keys=False), encoding="utf-8"
    )
    # The service is chosen as ``columbina setup`` chooses it, in the home of the attempt.
    # A profile whose model talks in another way, as a scripted one does, is left as it is.
    own: dict[str, Any] = {"model": model.model}
    if model.base_url is not None:
        own["base_url"] = model.base_url
    if model.context_window is not None:
        own["context_window"] = model.context_window
    (home / "provider.yml").write_text(
        yaml.safe_dump({"use": model.provider, "services": {model.provider: own}}),
        encoding="utf-8",
    )


def _finished(
    attempt: Attempt,
    place: Path,
    done: process.Completed,
    seconds: float,
    container: str | None,
) -> dict[str, Any]:
    """Keep what a run said, judge the attempt, and write its line. This blocks."""
    loaded, model = attempt.loaded, attempt.model
    # As it was said: on Windows the lines end in CR LF already, and are not to get another CR.
    (place / "out.jsonl").write_text(done.output, encoding="utf-8", newline="")
    (place / "err.txt").write_text(done.error_output, encoding="utf-8", newline="")
    said = said_of(done.output)
    finals = [line for line in said if line.get("type") == "final"]
    final: dict[str, Any] = finals[-1] if finals else {}
    calls = calls_of(said)
    verdict = judged(
        loaded, place / "workspace", calls, container=container, private=place / "private"
    )
    result = {
        "task": loaded.task.id,
        "category": loaded.task.category,
        "model": model.label,
        "variant": attempt.variant,
        "repeat": attempt.repeat,
        "passed": verdict.passed,
        "said": verdict.said,
        "reason": final.get("reason", "timeout" if done.timed_out else "none"),
        "code": final.get("code"),
        "requests": final.get("calls", 0),
        "tool_calls": len(calls),
        "usage": final.get("usage", {}),
        "seconds": seconds,
        "timed_out": done.timed_out,
        "exit_code": done.exit_code,
        "record": str(place),
        "at": datetime.now().astimezone().isoformat(timespec="seconds"),
    }
    (place / "result.json").write_text(json.dumps(result, indent=1) + "\n", encoding="utf-8")
    return result


def judged_again(line: dict[str, Any], *, container: str | None = IMAGE) -> dict[str, Any]:
    """Judge an attempt again from what it left, and return its line. This blocks.

    The calls come from what the run said, and the workspace and the databases are those
    that the attempt left. So a judge that was set right is applied without asking a model
    anything. The line keeps all but its verdict, and ``result.json`` is written anew.
    """
    place = Path(line["record"])
    loaded = load(TASKS / line["category"] / line["task"])
    calls = calls_of(said_of((place / "out.jsonl").read_text(encoding="utf-8")))
    verdict = judged(
        loaded, place / "workspace", calls, container=container, private=place / "private"
    )
    again = {**line, "passed": verdict.passed, "said": verdict.said}
    (place / "result.json").write_text(json.dumps(again, indent=1) + "\n", encoding="utf-8")
    return again


def _appended(results: Path, line: dict[str, Any]) -> None:
    """Add a line to a file of results. This blocks."""
    with results.open("a", encoding="utf-8") as file:
        file.write(json.dumps(line) + "\n")


def outside_repositories(root: Path) -> None:
    """Make sure that the directory of a run lies in no repository.

    Raises:
        RunError: It lies in one, or is one.
    """
    here = root.resolve()
    for above in (here, *here.parents):
        if os.path.lexists(above / ".git"):
            raise RunError(
                f"{root} lies in the repository {above}, and every attempt would read what that "
                "holds for its own work. Make the run somewhere else"
            )


def done_already(results: Path) -> set[tuple[str, str, str, int]]:
    """Return the attempts that a file of results has lines of. Make its directory; this blocks."""
    results.parent.mkdir(parents=True, exist_ok=True)
    if not results.exists():
        return set()
    keys: set[tuple[str, str, str, int]] = set()
    for line in results.read_text(encoding="utf-8").splitlines():
        if line.strip():
            read = json.loads(line)
            keys.add((read["task"], read["model"], read["variant"], read["repeat"]))
    return keys


async def made(
    attempts: Sequence[Attempt],
    root: Path,
    *,
    jobs: int,
    sandbox: str = "docker",
    container: str | None = IMAGE,
    extra: Sequence[dict[str, Any]] = (),
    told: Callable[[str], None] = print,
) -> list[dict[str, Any]]:
    """Make the attempts that ``root/results.jsonl`` has no line of, ``jobs`` at a time.

    Each line is written as soon as its attempt is judged, so what was done
    is kept when the run is broken off. Return the lines of this run.

    Raises:
        RunError: The directory of the run lies in a repository.
    """
    outside_repositories(root)
    results = root / "results.jsonl"
    done = await asyncio.to_thread(done_already, results)
    left = [attempt for attempt in attempts if attempt.key not in done]
    told(f"{len(attempts) - len(left)} of {len(attempts)} attempts are done already")
    gate = asyncio.Semaphore(jobs)
    lock = asyncio.Lock()
    lines: list[dict[str, Any]] = []

    async def one(attempt: Attempt) -> None:
        async with gate:
            line = await attempted(attempt, root, sandbox=sandbox, container=container, extra=extra)
        async with lock:
            await asyncio.to_thread(_appended, results, line)
            lines.append(line)
            verdict = "passed" if line["passed"] else f"failed ({line['reason']})"
            told(f"{len(lines)}/{len(left)} {attempt.loaded.task.id} #{attempt.repeat}: {verdict}")

    await asyncio.gather(*(one(attempt) for attempt in left))
    return lines
