"""A run that was recorded against the real service happens again, without the service.

Every directory under ``tests/fixtures/recorded`` holds a session that
``columbina run`` wrote while it talked to DeepSeek. Here the replies of that
session are played to the loop again, with the plugins that the run had. The
same input must lead to the same log, event by event, and the model must be
sent exactly the requests of ``requests.txt``. How such a recording is made
is written down next to them.

The tools really run again: files are read and changed, and commands are
executed, in a copy of the working directory as it was before the run.
"""

import os
import re
import sys
from pathlib import Path
from typing import Any, cast

import pytest
import yaml

from columbina.agent import AGENTS, AgentSpec
from columbina.kernel import Context, flatten, mount, parse_layer
from columbina.llm import LLM, render_requests
from columbina.llm.adapters import replay
from columbina.session import (
    SESSIONS,
    EventData,
    RequestHeaderEvent,
    SessionEvent,
    SystemMessageEvent,
    UserMessageEvent,
    read_session,
)
from columbina.tools import TOOLS
from support.launch import bundle
from support.loop import from_a_source, idle
from support.neutral import copy_of, neutral, neutral_requests
from support.snapshot import stored

RECORDED = Path(__file__).parent.parent / "fixtures" / "recorded"
RECORDINGS = sorted(path.name for path in RECORDED.iterdir() if path.is_dir())
WITHOUT_A_MODEL = parse_layer("- id: model\n  disabled: true\n", source="the replay")

Essence = tuple[int, str, Any, bool, tuple[int, ...]]


def how_it_was_made(name: str) -> dict[str, Any]:
    """Read what a recording says about the run: its profile, and its platform if it has one."""
    return yaml.safe_load((RECORDED / name / "recording.yml").read_text(encoding="utf-8"))


def timeless(data: dict[str, Any]) -> dict[str, Any]:
    """Return what an event holds without how long a tool ran, which differs from run to run."""
    meta: object = data.get("meta")
    if not isinstance(meta, dict) or "duration_ms" not in meta:
        return data
    told = cast("dict[str, Any]", meta)
    rest = {key: value for key, value in told.items() if key != "duration_ms"}
    return {**data, "meta": rest or None}


def essence(events: list[SessionEvent[EventData]], work: Path | None = None) -> list[Essence]:
    """What each event is, apart from the moment and the place it was written at.

    How long a tool ran is a moment too: the recordings were made before it was written
    down, and it is another number every time.
    """
    plain = [(e, timeless(e.data.model_dump(mode="json"))) for e in events]
    return [
        (e.seq, e.type, data if work is None else neutral(data, work), e.on_surface, e.replaces)
        for e, data in plain
    ]


def test_there_are_recordings() -> None:
    assert RECORDINGS == ["deepseek-fix", "deepseek-read"]


@pytest.mark.parametrize("name", RECORDINGS)
async def test_a_recorded_run_happens_again_event_by_event(
    name: str, ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    made = how_it_was_made(name)
    if made.get("platform", sys.platform) != sys.platform:
        pytest.skip(f"the run calls a tool that only {made['platform']} has")
    monkeypatch.setenv("PYTHONDONTWRITEBYTECODE", "1")
    work = copy_of(RECORDED / name / "work", tmp_path / "work")
    profile = parse_layer(yaml.safe_dump(made["profile"]), source=f"{name}/recording.yml")
    # The replies come from the recording, which can only be read once the plugins are there.
    entries = flatten([profile, WITHOUT_A_MODEL], lookup=bundle)
    places = {
        "COLUMBINA_HOME": str(tmp_path / "home"),
        "COLUMBINA_AGENTS_HOME": str(tmp_path / "agents"),
    }
    composition = mount(ctx, entries, env={**os.environ, **places})
    await ctx.settled()
    assert composition.problems() == []
    recorded = read_session(RECORDED / name, ctx.use(SESSIONS).kinds).events
    header = next(e.data for e in recorded if isinstance(e.data, RequestHeaderEvent))
    system = next(e.data for e in recorded if isinstance(e.data, SystemMessageEvent))
    tasks = [
        e.data
        for e in recorded
        if isinstance(e.data, UserMessageEvent) and not from_a_source(e.data)
    ]
    prompt = system.message.content[0].text
    spec = AgentSpec(cwd=work, model=header.config, system_prompt=prompt)
    await ctx.plugin(replay, {"provider": header.config.provider, "directory": RECORDED / name})

    agent = await ctx.use(AGENTS).create(ctx, spec, session_id="again")
    for task in tasks:
        agent.followup(task.message)
        await idle(agent)

    # The tools are described by the code of today, the rest is as it was recorded.
    today = RequestHeaderEvent(config=header.config, tools=ctx.use(TOOLS).schemas())
    expected = [
        (seq, kind, today.model_dump(mode="json") if kind == "request/header" else data, *rest)
        for seq, kind, data, *rest in essence(recorded)
    ]
    assert essence(list(agent.session.events), work) == expected

    adapter = ctx.use(LLM).adapters[header.config.provider]
    assert isinstance(adapter, replay.Replay)
    seen = render_requests(neutral_requests(adapter.requests, work)) + "\n"
    assert seen == stored(RECORDED / name / "requests.txt", seen)


@pytest.mark.parametrize("name", RECORDINGS)
def test_a_recording_holds_nothing_of_the_machine_it_was_made_on(name: str) -> None:
    session = (RECORDED / name / "session.v1.jsonl").read_text(encoding="utf-8")
    everything = "".join(
        path.read_text(encoding="utf-8")
        for path in sorted((RECORDED / name).rglob("*"))
        if path.is_file() and "__pycache__" not in path.parts
    )

    assert '"cwd":"/work"' in session.splitlines()[0]
    assert re.search(r"sk-[A-Za-z0-9_-]{16,}", everything) is None
    # No key, no home of a user, no drive of a disk.
    assert re.search(r"Bearer|Users|/home/|[A-Za-z]:\\\\", everything) is None
