"""A session in which everything happens once, kept as a sample that interfaces are tried on.

The session is not written by hand. An agent is driven through it. A scripted
model stands in for the service, and a tool that only pretends to run
commands stands in for the shell, so that the session is the same on every
machine. Everything else is what a run has: the loop, the tools that read and
change files, the rules that have a command asked about, the retry of a
request, and the service that makes a conversation shorter. The events and
their order are therefore the ones that a frontend is sent.

What happens, turn by turn:

1. A mistake in a file is found and mended. The model writes down what it
   means to do, the user says something on the way, one request fails and is
   made again, and a command is asked about and allowed. The user says that
   the turn did what was asked.
2. A command is asked about and refused, and the user stops the reply.
3. The program is left and the session taken up again. A question is handed
   to a subagent. Two commands say a lot, and the conversation is made
   shorter by cutting a result down.
4. The user has the conversation summed up, and one more turn follows, in
   which the model is told again what it had written down.

At the end the user has it summed up once more. The summary of before is in
the part that goes this time, and it stands first there although it was
written last.
"""

import asyncio
import json
import shutil
from collections import defaultdict, deque
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

import columbina.approval
import columbina.checkpoints
import columbina.compaction.basic
import columbina.fs
import columbina.hooks
import columbina.permissions
import columbina.retry
import columbina.sandbox
import columbina.subagents
import columbina.subagents.spawn
import columbina.todo
import columbina.token_meter
from columbina import plan, shells, verdicts
from columbina.agent import Agent, AgentSpec, ContextSource
from columbina.approval import APPROVAL_ASK, ApprovalRequest, Outcome
from columbina.apps.common import DEFAULT_PROMPT
from columbina.builtin_tools import edit, read, subagent
from columbina.checkpoints import Checkpoints
from columbina.compaction import COMPACTION, CompactionEndEvent
from columbina.hooks.config import Hook
from columbina.hooks.run import Result
from columbina.llm import LLM, CallConfig, LlmFailure, Usage
from columbina.llm.adapters.scripted import Call, Reply
from columbina.permissions import PERMISSIONS, split_bash
from columbina.session import (
    TurnEndEvent,
    UserMessageEvent,
    decode_header,
    encode_header,
    read_session,
)
from columbina.session.store import SESSION_FILE
from columbina.todo import TOOL as TODO_TOOL
from columbina.tools import Tool, ToolRun, pipeline
from support.loop import Harness, idle
from support.protocol import event_kinds
from support.scenario import REPOSITORY
from support.snapshot import stored

SAMPLE = Path(__file__).parents[1] / "fixtures" / "tour.jsonl"
"""The sample. The tests of the desktop application replay a copy of it."""
MODEL = CallConfig(provider="deepseek", model="deepseek-flash", context_window=3200)
"""A model that takes so little that a few turns are too much for it."""

BEGINS = 1_791_590_400_000
TASK = (
    "check.py fails. Find the mistake in calc.py and fix it, "
    "and make sure that `python check.py` prints ok."
)
BUSY = LlmFailure(
    message="Too many requests. Try again in a moment.",
    code="RATE_LIMIT",
    status=429,
    request_id="req_7f3a",
)
ANSWER = "mean() divided by one too few. I fixed calc.py, and the check prints ok now."
SUMMARY = """\
1. Progress and decisions: The user asked for the mistake in calc.py to be found and fixed. \
mean() divided by len(values) - 1, and it divides by len(values) now. `python check.py` prints ok. \
The user wanted the change kept as small as possible, so nothing else in the file was touched. \
The history of the file shows that the mistake came in with commit 0029d5f.
2. Constraints and preferences: Keep changes small. Committing was not allowed: the user said no \
to `git commit -am "Fix mean()"`.
3. Remaining steps: The fix is in the working tree and is not committed.
4. Key data: calc.py, check.py, `python check.py`, commit 0029d5f."""
SUMMARY_AGAIN = """\
1. Progress and decisions: mean() in calc.py is fixed and `python check.py` prints ok. The \
mistake came in with commit 0029d5f.
2. Constraints and preferences: Keep changes small. Committing was not allowed.
3. Remaining steps: The fix is not committed.
4. Key data: calc.py, commit 0029d5f."""


HOOKS = {
    "hooks": {
        "PostToolUse": [
            {"matcher": "Edit|Write", "hooks": [{"type": "command", "command": "ruff format"}]}
        ]
    }
}


class Clock:
    """A clock that goes on by a fixed step whenever somebody looks at it."""

    def __init__(self) -> None:
        self.now = BEGINS

    def __call__(self) -> int:
        self.now += 180
        return self.now


class Command(BaseModel):
    """What the shell of the sample is given."""

    command: str


def history() -> str:
    """What `git log` says about a file that was changed often: more than anybody reads twice."""
    subjects = (
        "Tidy the calculator",
        "Say what mean() returns",
        "Take the length once",
        "Name the argument values",
        "Leave the empty list to the caller",
    )
    entries = [
        f"commit {number * 7919 + 10_007:07x}\n"
        f"Author: dev <dev@example.com>\n"
        f"Date:   2026-09-{28 - number:02d}\n"
        f"\n"
        f"    {subjects[number % len(subjects)]}\n"
        f"\n"
        f" calc.py | {number % 4 + 2} {'+' * (number % 3 + 1)}{'-' * (number % 2 + 1)}\n"
        f" 1 file changed\n"
        for number in range(27)
    ]
    return "\n".join(entries) + "[exit code 0]"


CHANGE = """\
commit 0029d5f
Author: dev <dev@example.com>
Date:   2026-09-26

    Take the length once

diff --git a/calc.py b/calc.py
--- a/calc.py
+++ b/calc.py
@@ -3,5 +3,6 @@
 def mean(values: list[float]) -> float:
     \"\"\"Return the mean of the values.\"\"\"
-    return sum(values) / len(values)
+    count = len(values) - 1
+    return sum(values) / count
[exit code 0]"""

SAID: dict[str, tuple[str, int]] = {
    "python check.py": ("ok\n[exit code 0]", 640),
    "git log --stat -- calc.py": (history(), 310),
    "git show 0029d5f -- calc.py": (CHANGE, 120),
}
"""What the shell of the sample says to each command, and how long it takes over it."""


def spent(fresh: int, written: int, cached: int = 0) -> Usage:
    return Usage(input_tokens=fresh, output_tokens=written, cache_read_tokens=cached)


def made_up(size: int) -> str:
    """What stands in for the part of the id of a session that nobody can know ahead."""
    return "5eed" * (size // 2)


def shell(command: str) -> Call:
    return Call(name="bash", arguments={"command": command})


def noted(mended: str, checked: str) -> Call:
    """The model writes down how far the two things have come that it means to do."""
    return Call(
        name=TODO_TOOL,
        arguments={
            "items": [
                {"content": "Mend mean() in calc.py", "status": mended},
                {"content": "Run the check", "status": checked},
            ]
        },
    )


async def toured(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> tuple[str, list[ApprovalRequest]]:
    """Drive an agent through the session, and return its file and what was asked about."""
    ctx = harness.ctx
    clock = Clock()
    harness.sessions.clock = clock
    ctx.use(LLM).adapters.add(ctx, MODEL.provider, harness.model)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    await ctx.plugin(columbina.permissions)
    await ctx.plugin(columbina.retry, {"base_delay_seconds": 0.02, "jitter": 0})
    await ctx.plugin(columbina.token_meter)
    await ctx.plugin(
        columbina.compaction.basic, {"trim_over": 2000, "trim_head": 800, "trim_tail": 400}
    )
    await ctx.plugin(read)
    await ctx.plugin(edit)
    await ctx.plugin(columbina.todo)
    await ctx.plugin(columbina.subagents)
    await ctx.plugin(columbina.subagents.spawn)
    await ctx.plugin(subagent)
    await ctx.plugin(verdicts)
    await ctx.plugin(plan)

    # The files are kept before each turn. git is not run: what it would answer depends on
    # the moment, and the sample is the same at every moment.
    async def kept(checkpoints: Checkpoints, cwd: Path) -> str:
        return "c0ffee" * 6 + "c0ff"

    monkeypatch.setattr(Checkpoints, "keep", kept)
    await ctx.plugin(columbina.checkpoints, {"home": str(harness.cwd.parent / "checkpoints")})
    # The session of a subagent gets an id that is made up. Here it is the same every time.
    monkeypatch.setattr("columbina.session.store.secrets.token_hex", made_up)
    # The user has one hook, the most common one: a formatter after every change to a file.
    # It is not run here. What stands in for it takes a third of a second of the clock.
    hooks = harness.cwd.parent / "hooks.json"
    hooks.write_text(json.dumps(HOOKS))
    # A run of a tool is timed by what it takes itself. Calls that run beside each other go
    # by the one clock of the session, and each would count what the others took.
    took_by: defaultdict[asyncio.Task[Any] | None, int] = defaultdict(int)

    def taking(milliseconds: int) -> None:
        clock.now += milliseconds
        took_by[asyncio.current_task()] += milliseconds

    async def formatted(hook: Hook, event: str, told: bytes, **where: object) -> Result:
        taking(310)
        return Result(exit_code=0)

    monkeypatch.setattr(shells, "find", lambda: "bash")
    monkeypatch.setattr(columbina.hooks, "run", formatted)
    monkeypatch.setattr(columbina.hooks, "_now", lambda: clock.now / 1000)
    monkeypatch.setattr(pipeline, "_now", lambda: took_by[asyncio.current_task()] / 1000)
    await ctx.plugin(columbina.hooks, {"user_file": hooks})
    for name in ("calc.py", "check.py"):
        shutil.copy(REPOSITORY / name, harness.cwd / name)

    async def ran(args: Command, run: ToolRun) -> str:
        said, took = SAID[args.command]
        taking(took)
        return said

    def command_of(args: Command, run: ToolRun) -> str:
        return args.command

    harness.tools.add(
        ctx,
        Tool(
            name="bash",
            description="Run a command in bash and return what it printed, with its exit code.",
            parameters=Command,
            execute=ran,
            effect="run",
            target=command_of,
        ),
    )
    ctx.use(PERMISSIONS).splitters.add(ctx, "bash", split_bash)

    known = ["The project is a small calculator. Its check is run with `python check.py`."]

    async def project(agent: Agent) -> str:
        return known[0]

    harness.agents.context.add(ctx, "project", ContextSource(render=project, kind="replace"))

    answers: deque[Outcome] = deque(["allowed-once", "rejected", "allowed-once", "allowed-once"])
    asked: list[ApprovalRequest] = []

    def answer(request: ApprovalRequest) -> Outcome:
        asked.append(request)
        # Somebody reads the command before saying yes or no.
        clock.now += 4200
        return answers.popleft()

    ctx.on(APPROVAL_ASK, answer)
    spec = AgentSpec(cwd=harness.cwd, model=MODEL, system_prompt=DEFAULT_PROMPT)
    model = harness.model
    agent = await harness.agents.create(ctx, spec, session_id="tour")

    # 1. A mistake is found and mended.
    model.add(
        Reply(
            reasoning="The check fails, so I look at the check and at the code before I change "
            "anything.",
            text="I will read the check and the calculator.",
            calls=(
                Call(name="read", arguments={"path": "check.py"}),
                Call(name="read", arguments={"path": "calc.py"}),
            ),
            usage=spent(412, 58),
            hold=True,
        ),
        Reply(text="mean() divides by", usage=spent(96, 4, 704), failure=BUSY),
        Reply(
            reasoning="check.py expects mean([1.0, 2.0, 3.0]) to be 2.0. calc.py divides by "
            "len(values) - 1. One expression has to change, and nothing else.",
            text="mean() divides by one too few. I change that expression and nothing else.",
            calls=(
                noted("in_progress", "pending"),
                Call(
                    name="edit",
                    arguments={"path": "calc.py", "old": "(len(values) - 1)", "new": "len(values)"},
                ),
            ),
            usage=spent(96, 131, 704),
        ),
        Reply(
            calls=(noted("completed", "in_progress"), shell("python check.py")),
            usage=spent(84, 58, 896),
        ),
        Reply(calls=(noted("completed", "completed"),), usage=spent(52, 41, 1024)),
        Reply(text=ANSWER, usage=spent(48, 24, 1088)),
    )
    agent.followup(TASK)
    await asyncio.wait_for(model.held(), timeout=5)
    agent.steer("Keep the change as small as you can.")
    model.release()
    await idle(agent)
    verdicts.judge(agent.session, True, "The mean is right now.")
    # The user looks before the next task: plan mode, on and off again.
    plan.set_planning(agent.session, True)
    plan.set_planning(agent.session, False)

    # 2. A command is refused, and the reply is stopped.
    known[0] += " The fix of mean() is in the working tree."
    model.add(
        Reply(
            text="I will commit the change.",
            calls=(shell('git commit -am "Fix mean()"'),),
            usage=spent(83, 27, 1152),
        ),
        Reply(
            text="The commit was not allowed, so the fix is in the working tree and is not "
            "committed. If you want it committed, you",
            usage=spent(74, 31, 1216),
            hold=True,
        ),
    )
    agent.followup("Commit the fix.")
    await asyncio.wait_for(model.held(), timeout=5)
    agent.cancel("user")
    await idle(agent)

    # 3. The session is taken up again, and a long result is cut down.
    await agent.close()
    clock.now += 95 * 60 * 1000
    agent = await harness.agents.resume(ctx, spec, "tour")
    model.add(
        Reply(
            text="While I go through the history, the uses of mean() can be looked up.",
            calls=(
                Call(
                    name="subagent",
                    arguments={
                        "description": "find uses of mean",
                        "prompt": "In the project in the working directory, find every place "
                        "where the function mean() of calc.py is called. Give the files and "
                        "the lines, and nothing else.",
                    },
                ),
            ),
            usage=spent(131, 64, 1280),
        ),
        # What the subagent answers. Its own steps are in its own session.
        Reply(text="mean() is called in one place: check.py, line 14.", usage=spent(612, 17)),
        Reply(calls=(shell("git log --stat -- calc.py"),), usage=spent(92, 22, 1408)),
        Reply(
            text="The file was changed often. The commit that takes the length once is the "
            "likely one.",
            calls=(shell("git show 0029d5f -- calc.py"),),
            usage=spent(1287, 41, 1344),
        ),
        Reply(
            text="The mistake came in with commit 0029d5f, which took the length once and "
            "took one off it.",
            usage=spent(1016, 29, 1280),
        ),
    )
    agent.followup("How did calc.py come to be like this? Look at its history.")
    await idle(agent)

    # 4. The conversation is summed up, and goes on.
    model.add(Reply(text=SUMMARY, usage=spent(88, 171, 2240)))
    compaction = ctx.use(COMPACTION)
    await asyncio.wait_for(
        agent.run_maintenance(lambda: compaction.compact(agent, "manual")), timeout=5
    )
    model.add(
        Reply(
            text="Only the commit. The fix is in the working tree, and committing it was not "
            "allowed.",
            usage=spent(398, 23, 256),
        )
    )
    agent.followup("What is left to do?")
    await idle(agent)

    # And once more: the summary of before goes with what followed it.
    model.add(Reply(text=SUMMARY_AGAIN, usage=spent(79, 68, 640)))
    await asyncio.wait_for(
        agent.run_maintenance(lambda: compaction.compact(agent, "manual")), timeout=5
    )
    assert model.remaining == 0

    path = harness.sessions.directory(harness.cwd, "tour") / SESSION_FILE
    await agent.close()
    head, _, events = path.read_text(encoding="utf-8").partition("\n")
    # The real path names the machine and its user.
    header = decode_header(head).model_copy(update={"cwd": "/work"})
    return f"{encode_header(header)}\n{events}", asked


def events_of(text: str) -> list[dict[str, object]]:
    return [json.loads(line) for line in text.splitlines()[1:]]


async def test_the_sample_is_what_such_a_session_leaves(
    harness: Harness, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    text, asked = await toured(harness, monkeypatch)

    assert tmp_path.name not in text
    assert text == stored(SAMPLE, text)
    assert [(request.tool, request.target, request.standing) for request in asked] == [
        ("bash", "python check.py", "this command, word for word"),
        ("bash", 'git commit -am "Fix mean()"', "this command, word for word"),
        ("bash", "git log --stat -- calc.py", "this command, word for word"),
        ("bash", "git show 0029d5f -- calc.py", "this command, word for word"),
    ]


def test_everything_that_can_happen_happens_in_the_sample(tmp_path: Path) -> None:
    kinds = event_kinds()
    shutil.copy(SAMPLE, tmp_path / SESSION_FILE)
    # Read the way a session is read that is taken up again: the numbering has no gap, and
    # what takes the place of messages fits the conversation as it stood.
    session = read_session(tmp_path, kinds)
    events = session.events

    assert session.header.cwd == "/work"
    assert {event.type for event in events} == set(kinds)
    ends = [event.data for event in events if isinstance(event.data, TurnEndEvent)]
    assert [(end.reason, end.detail) for end in ends] == [
        ("completed", None),
        ("aborted", "user"),
        ("completed", None),
        ("completed", None),
    ]
    said = [event.data.source for event in events if isinstance(event.data, UserMessageEvent)]
    assert set(said) == {
        "user",
        "steer",
        "context/project",
        "context/interruption",
        "context/todo",
        "context/plan",
        "compaction",
    }
    shorter = [event.data.outcome for event in events if isinstance(event.data, CompactionEndEvent)]
    assert [(made.trimmed, made.summarised > 0, made.error) for made in shorter] == [
        (1, False, None),
        (0, True, None),
        (0, True, None),
    ]
    # Each way in which something takes the place of something else.
    replacing = {event.type for event in events if event.replaces}
    assert replacing == {"tool/result", "user/message"}
    # What is replaced is listed in the order of the conversation. The second summary takes
    # the place of the first, which was written after the messages that follow it.
    listed = [event.replaces for event in events if len(event.replaces) > 1]
    assert [list(seqs) == sorted(seqs) for seqs in listed] == [True, False]
    first, second = (
        event.seq
        for event in events
        if isinstance(event.data, UserMessageEvent) and event.data.source == "compaction"
    )
    assert listed[1][0] == first
    assert first > max(listed[1][1:])
    assert second > first
