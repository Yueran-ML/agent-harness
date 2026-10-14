"""The real DeepSeek service in a conversation at a terminal. Skipped unless the key is set.

The conversation is the one of ``tests/apps/cli``: the program that
``columbina`` starts without a command, with a script at the terminal. Here
the model is the real one, so the script cannot know what will be on the
screen and when. It gives the task and says yes to every question, for the
rest of the session where that is offered and for this once where it is
not. It leaves when the agent is at rest.

The run costs a few thousand tokens. Run it with::

    uv run pytest tests/live -m live
"""

import os
from pathlib import Path

import pytest
import yaml

from columbina.apps.launcher import main
from support import user as at_the_terminal
from support.scenario import TASK, check, fresh

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(300),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

ENTER = "\r"


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A copy of the repository, and a data directory that is thrown away."""
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("COLUMBINA_AGENTS_HOME", str(tmp_path / "agents"))
    return fresh(tmp_path / "repository", monkeypatch)


def test_a_repository_is_mended_in_a_conversation(repository: Path, tmp_path: Path) -> None:
    script = [
        {"sees": "/help lists the commands.", "keys": TASK + ENTER},
        {"rests": True, "keys": "/exit" + ENTER},
    ]
    somebody = {
        "id": "user",
        "plugin": "support.user",
        "config": {"script": script, "answer": "a", "seconds": 240},
    }
    patch = tmp_path / "user.yml"
    patch.write_text(yaml.safe_dump([{"insert": [somebody]}]), encoding="utf-8")

    code = main(["--cwd", str(repository), "--patch", str(patch)])

    shown = "".join(at_the_terminal.SCREENS[-1].printed).replace("\r\n", "\n")
    assert code == 0
    assert check(repository) == (0, "ok\n")
    # Which command the model runs to see that the check passes is up to the model. Nobody
    # had allowed any, so it was asked about, and the answer was a yes.
    assert " asks for a yes:\n" in shown
    assert "(allowed, " in shown
    assert "(not allowed)" not in shown
