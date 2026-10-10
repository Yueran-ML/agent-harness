"""The real DeepSeek service sums up a conversation that has outgrown its window.

Skipped unless DEEPSEEK_API_KEY is set. The program is told that its model
takes fifteen thousand tokens, which the real one takes many times over. The
user hands it eight notes, one in each turn, and each note names somebody.
Before the last note the conversation is more than may be filled, and the
oldest turns are summed up by the model itself. Then the user asks for the
name in the first note. The note is gone from what the model is sent: only
the summary can tell.

No tool is needed for any of it, and all tools are declared as in any run. So
the request for the summary is the one that a real conversation makes, with
tools on offer that the model is asked not to call.

The whole file sends about a hundred thousand tokens, most of them from the
cache of the service. Run it with::

    uv run pytest tests/live -m live
"""

import json
import os
from pathlib import Path
from typing import Any

import pytest
import yaml

from columbina.apps.launcher import main
from columbina.compaction import EVENTS, CompactionEndEvent, CompactionSummaryEvent
from columbina.session import StoredSession, read_session
from columbina.session.store import workspace_key
from columbina.token_meter import estimate
from support.launch import KINDS

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(600),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

KEEPERS = {
    "alder": "Marta",
    "birch": "Ines",
    "cedar": "Tobias",
    "elm": "Greta",
    "fir": "Anselm",
    "hazel": "Odile",
    "larch": "Corin",
    "maple": "Sunniva",
}
ALL_KINDS = {**KINDS, **{kind.event_type: kind for kind in EVENTS}}


def note(tree: str) -> str:
    """A note that names who keeps a tree, and goes on for long enough to count."""
    lines = [f"The keeper of the {tree} is called {KEEPERS[tree]}."]
    lines.extend(
        f"Entry {number} of the log of the {tree}: looked at, measured, nothing to report."
        for number in range(1, 96)
    )
    text = "\n".join(lines)
    assert 1500 < estimate(text) < 2300
    return text


@pytest.fixture
def work(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """An empty working directory, a home that is thrown away, and a model that takes little."""
    monkeypatch.setenv("COLUMBINA_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("COLUMBINA_CONTEXT_WINDOW", "15000")
    directory = tmp_path / "work"
    directory.mkdir()
    return directory


def said(work: Path, text: str, capsys: pytest.CaptureFixture[str]) -> list[dict[str, Any]]:
    """Say something in the session, and return what the run reported."""
    code = main(["run", text, "--cwd", str(work), "--session-id", "live", "--json"])
    captured = capsys.readouterr()
    assert (code, captured.err) == (0, "")
    return [json.loads(line) for line in captured.out.splitlines()]


def log(work: Path) -> StoredSession:
    home = Path(os.environ["COLUMBINA_HOME"])
    return read_session(home / "sessions" / workspace_key(work) / "live", ALL_KINDS)


def test_a_fact_from_the_part_that_was_summed_up_is_still_known(
    work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    for tree in KEEPERS:
        lines = said(
            work,
            f"Here is a note about the {tree}. Read it, and answer with one short sentence "
            f"that names the keeper.\n\n{note(tree)}",
            capsys,
        )
        assert KEEPERS[tree] in lines[-1]["text"]

    events = log(work).events
    ends = [event.data for event in events if isinstance(event.data, CompactionEndEvent)]
    summed = [event.data for event in events if isinstance(event.data, CompactionSummaryEvent)]
    # The conversation was made shorter by a summary that the model wrote, without help.
    assert ends, "the conversation never came near what the model is said to take"
    assert [made.outcome.error for made in ends] == [None] * len(ends)
    assert sum(made.outcome.summarised for made in ends) >= 4
    assert summed
    assert all(made.why == "pressure" for made in ends)
    # The first note is gone from what the model sees. Its text is in no message any more.
    seen = read_surface(work)
    assert "Entry 5 of the log of the alder" not in seen
    assert "Entry 5 of the log of the maple" in seen

    lines = said(
        work,
        "Use no tool. From what you know of this conversation: who is the keeper of the "
        "alder? Answer with the name only.",
        capsys,
    )

    assert not [line for line in lines if line["type"] == "tool_call"]
    assert "Marta" in lines[-1]["text"]


def read_surface(work: Path) -> str:
    """Return the text of every message that the model of the session would be sent now."""
    stored = log(work)
    replaced = {seq for event in stored.events for seq in event.replaces}
    texts: list[str] = []
    for event in stored.events:
        message = event.data.for_model() if event.on_surface else None
        if message is None or event.seq in replaced:
            continue
        texts.extend(getattr(block, "text", "") for block in message.content)
    return "\n".join(texts)


def test_the_service_takes_the_room_that_a_summary_is_given(
    work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # A request for a summary may be answered with as many tokens as are kept free for a
    # reply: 65536 with the window that the model really has. The service has to take a
    # request that allows so many.
    model = {
        "provider": "deepseek",
        "model": "deepseek-flash",
        "context_window": 1_048_576,
        "max_output_tokens": 65_536,
    }
    patch = tmp_path / "patch.yml"
    patch.write_text(
        yaml.safe_dump([{"id": "frontend", "config": {"model": model}}]), encoding="utf-8"
    )

    where = ["--cwd", str(work), "--patch", str(patch), "--json"]
    code = main(["run", "Reply with the single word: ready", *where])

    captured = capsys.readouterr()
    lines = [json.loads(line) for line in captured.out.splitlines()]
    assert (code, captured.err) == (0, "")
    assert "ready" in lines[-1]["text"].lower()
