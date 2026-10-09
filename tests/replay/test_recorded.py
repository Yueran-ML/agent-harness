"""A run that was recorded against the real service happens again, without the service.

The session under ``tests/fixtures/recorded/deepseek-read`` was written by
``columbina run`` talking to DeepSeek. Here the replies of that session are played
to the loop again. The same input must lead to the same log, event by event,
and the model must be sent exactly the requests of ``requests.txt``. How such
a recording is made is written down next to it.
"""

import re
import shutil
from pathlib import Path

import columbina.agent
import columbina.agent_loop
import columbina.llm
import columbina.retry
import columbina.session
import columbina.tools
from columbina.agent import AGENTS, AgentSpec
from columbina.builtin_tools import read
from columbina.kernel import Context
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
from support.loop import idle
from support.snapshot import stored

RECORDED = Path(__file__).parent.parent / "fixtures" / "recorded" / "deepseek-read"

Essence = tuple[int, str, EventData, bool, tuple[int, ...]]


def essence(events: list[SessionEvent[EventData]]) -> list[Essence]:
    """What each event is, apart from the moment it was written."""
    return [(e.seq, e.type, e.data, e.on_surface, e.replaces) for e in events]


async def test_a_recorded_run_happens_again_event_by_event(ctx: Context, tmp_path: Path) -> None:
    work = tmp_path / "work"
    shutil.copytree(RECORDED / "work", work)
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.llm)
    await ctx.plugin(columbina.tools)
    await ctx.plugin(read)
    await ctx.plugin(columbina.agent)
    await ctx.plugin(columbina.agent_loop)
    await ctx.plugin(columbina.retry)
    recorded = read_session(RECORDED, ctx.use(SESSIONS).kinds).events
    header = next(e.data for e in recorded if isinstance(e.data, RequestHeaderEvent))
    system = next(e.data for e in recorded if isinstance(e.data, SystemMessageEvent))
    tasks = [e.data for e in recorded if isinstance(e.data, UserMessageEvent)]
    prompt = system.message.content[0].text
    spec = AgentSpec(cwd=work, model=header.config, system_prompt=prompt)
    await ctx.plugin(replay, {"provider": header.config.provider, "directory": RECORDED})

    agent = await ctx.use(AGENTS).create(ctx, spec, session_id="again")
    for task in tasks:
        agent.followup(task.message)
        await idle(agent)

    # The tools are described by the code of today, the rest is as it was recorded.
    today = RequestHeaderEvent(config=header.config, tools=ctx.use(TOOLS).schemas())
    expected = [
        (seq, kind, today if isinstance(data, RequestHeaderEvent) else data, surface, replaces)
        for seq, kind, data, surface, replaces in essence(recorded)
    ]
    assert essence(list(agent.session.events)) == expected

    adapter = ctx.use(LLM).adapters[header.config.provider]
    assert isinstance(adapter, replay.Replay)
    seen = render_requests(adapter.requests) + "\n"
    assert seen == stored(RECORDED / "requests.txt", seen)


def test_a_recording_holds_nothing_of_the_machine_it_was_made_on() -> None:
    session = (RECORDED / "session.v1.jsonl").read_text(encoding="utf-8")
    everything = "".join(
        path.read_text(encoding="utf-8") for path in sorted(RECORDED.rglob("*")) if path.is_file()
    )

    assert '"cwd":"/work"' in session.splitlines()[0]
    assert re.search(r"sk-[A-Za-z0-9_-]{16,}", everything) is None
    assert re.search(r"Bearer|Users|/home/", everything) is None
