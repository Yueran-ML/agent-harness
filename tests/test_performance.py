"""How long the program takes to start and to take up a long session, and the memory it needs.

The budgets are loose. A machine of the CI is slower than a desk and shared with others, and
a budget is there to catch a change that makes something many times slower, not one that
makes it a little slower. What a desk measures is written down in docs/bench.
"""

import os
import subprocess
import sys
import time
import tracemalloc
from pathlib import Path

import pytest
import yaml

import columbina.session
from columbina.kernel import Context
from columbina.session import SESSIONS, Sessions
from columbina.session.projection import USAGE
from support.long_session import STEPS, long_session

# The budgets below say when something is too slow. This is only for a run that hangs.
pytestmark = pytest.mark.timeout(300)

MEGABYTE = 1024 * 1024


@pytest.fixture
async def sessions(ctx: Context, tmp_path: Path) -> Sessions:
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    return ctx.use(SESSIONS)


def _env(home: Path) -> dict[str, str]:
    """The environment of a program of its own: no key, no keyring, and a home of its own."""
    env = {name: value for name, value in os.environ.items() if not name.startswith("COLUMBINA_")}
    env.pop("DEEPSEEK_API_KEY", None)
    return env | {
        "COLUMBINA_HOME": str(home / "home"),
        "COLUMBINA_AGENTS_HOME": str(home / "agents"),
        "PYTHON_KEYRING_BACKEND": "keyring.backends.null.Keyring",
    }


async def test_a_session_of_a_thousand_steps_is_taken_up_within_its_budget(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    long_session(sessions, ctx, tmp_path)

    began = time.perf_counter()
    session = sessions.resume(ctx, tmp_path, "long")
    seconds = time.perf_counter() - began

    # What the model is sent was folded from the log: the task, and a call and its result
    # for every step.
    assert len(session.messages) == 1 + 2 * STEPS
    assert seconds < 20


async def test_a_session_of_a_thousand_steps_is_taken_up_within_its_memory(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    long_session(sessions, ctx, tmp_path)

    tracemalloc.start()
    try:
        session = sessions.resume(ctx, tmp_path, "long")
        _, peak = tracemalloc.get_traced_memory()
    finally:
        tracemalloc.stop()

    assert len(session) > 5 * STEPS
    assert peak < 300 * MEGABYTE


async def test_the_tokens_of_a_thousand_steps_are_added_up_within_their_budget(
    ctx: Context, sessions: Sessions, tmp_path: Path
) -> None:
    long_session(sessions, ctx, tmp_path)
    session = sessions.resume(ctx, tmp_path, "long")

    began = time.perf_counter()
    total = USAGE.of(session)
    seconds = time.perf_counter() - began

    assert total.calls == STEPS
    assert total.usage.output_tokens == 30 * STEPS
    assert seconds < 5


def test_the_program_is_put_together_within_its_budget(tmp_path: Path) -> None:
    began = time.perf_counter()
    done = subprocess.run(
        [sys.executable, "-m", "columbina", "run", "--dump-config", "Nothing."],
        env=_env(tmp_path),
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=240,
        check=False,
    )
    seconds = time.perf_counter() - began

    assert done.returncode == 0, done.stderr
    assert "id: frontend" in done.stdout
    assert seconds < 30


def test_one_small_task_is_done_within_its_budget(tmp_path: Path) -> None:
    patch = tmp_path / "patch.yml"
    model = {"provider": "deepseek", "replies": [{"text": "Done."}]}
    patch.write_text(
        yaml.safe_dump(
            [{"id": "model", "plugin": "columbina.llm.adapters.scripted", "config": model}]
        ),
        encoding="utf-8",
    )

    began = time.perf_counter()
    done = subprocess.run(
        [sys.executable, "-m", "columbina", "run", "--patch", str(patch), "--cwd", str(tmp_path),
         "Say that it is done."],
        env=_env(tmp_path), capture_output=True, text=True, encoding="utf-8", timeout=240,
        check=False,
    )  # fmt: skip
    seconds = time.perf_counter() - began

    assert done.returncode == 0, done.stderr
    assert done.stdout.strip() == "Done."
    assert seconds < 40
