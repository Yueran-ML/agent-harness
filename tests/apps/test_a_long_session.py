"""Where stage 5 ends: a conversation that outgrows its model is made shorter, and goes on.

The program is the one that is shipped, run by its command line. Two things
are other than in a real run. The model is scripted. And the profile says
that the model takes little, so that five files are more than it takes.

Three runs are told here. In the first the conversation comes near the limit
while the model is at work, and its oldest part is summed up between two
steps. In the second the model refuses a request as too long, the
conversation is made shorter, and the same step is taken again. In the third
there is nothing that could go, and the turn ends with the refusal.
"""

from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.compaction import (
    EVENTS,
    CompactionEndEvent,
    CompactionStartEvent,
    CompactionSummaryEvent,
)
from columbina.compaction.basic.summary import INSTRUCTION, carried
from columbina.llm import AssistantMessage, LlmRequest, Message, TextBlock, ToolMessage, UserMessage
from columbina.llm.adapters import scripted
from columbina.session import (
    AsideEvent,
    AssistantAttemptEvent,
    StepStartEvent,
    StoredSession,
    TurnEndEvent,
    read_session,
)
from columbina.session.store import workspace_key
from columbina.todo import TodoWriteEvent
from columbina.token_meter import estimate
from support.launch import KINDS, profile

WINDOW = 15_000
"""How much the model of the first run takes: 11250 tokens may be filled.

What the model is sent beside the conversation, its instructions and the
tools, comes to some two thousand tokens. A window has to be several times
that for the numbers of a real model to hold in small: the part that is
summed up is then more than half of what the model takes, as it is when a
real conversation comes near a real limit.
"""

TREES = ("alder", "birch", "cedar", "elm", "fir", "hazel")
TASK = "Read the six files about trees, and say which trees they are about."
SUMMARY = (
    "1. Progress and decisions: The user asked which trees six files are about. alder.txt is "
    "about the alder, birch.txt about the birch and cedar.txt about the cedar.\n"
    "2. Constraints and preferences: none.\n"
    "3. Remaining steps: read elm.txt, fir.txt and hazel.txt, then answer.\n"
    "4. Key data: alder.txt, birch.txt, cedar.txt, elm.txt, fir.txt, hazel.txt."
)
ANSWER = "They are about the alder, the birch, the cedar, the elm, the fir and the hazel."
ALL_KINDS = {**KINDS, **{kind.event_type: kind for kind in (*EVENTS, TodoWriteEvent)}}


def call(name: str, **arguments: object) -> dict[str, Any]:
    return {"calls": [{"name": name, "arguments": arguments}]}


def refused(window: int, requested: int) -> dict[str, Any]:
    """A reply of the service that refuses a request as too long, the way DeepSeek words it."""
    message = (
        f"This model's maximum context length is {window} tokens. However, you requested "
        f"{requested} tokens ({requested} in the messages, 0 in the completion)."
    )
    return {"failure": {"message": message, "code": "CONTEXT_WINDOW_EXCEEDED", "status": 400}}


def about(tree: str) -> str:
    """What a file says about a tree: enough to count, and too little to be cut down."""
    text = "".join(
        f"The {tree} is a tree, and line {number} of this file says so once more.\n"
        for number in range(1, 111)
    )
    assert len(text) < 8000
    assert 1500 < estimate(text) < 2200
    return text


@pytest.fixture
def woods(tmp_path: Path) -> Path:
    """A working directory with a file about each of six trees."""
    directory = tmp_path / "woods"
    directory.mkdir()
    for tree in TREES:
        (directory / f"{tree}.txt").write_text(about(tree), encoding="utf-8", newline="\n")
    return directory


@pytest.fixture
def models(monkeypatch: pytest.MonkeyPatch) -> list[scripted.Scripted]:
    """The scripted models that the runs of a test create, for what they were sent."""
    made: list[scripted.Scripted] = []

    class Watched(scripted.Scripted):
        def __init__(self, *args: Any, **options: Any) -> None:
            super().__init__(*args, **options)
            made.append(self)

    monkeypatch.setattr(scripted, "Scripted", Watched)
    return made


def ran(tmp_path: Path, woods: Path, *replies: dict[str, Any], window: int | None) -> int:
    """Run the task with a model that says these things and takes so many tokens."""
    model = {"provider": "deepseek", "model": "deepseek-flash", "context_window": window}
    frontend = {"id": "frontend", "config": {"model": model}}
    chosen = profile(tmp_path, *replies, extra=[frontend])
    return main(["run", TASK, "--profile", chosen, "--cwd", str(woods), "--session-id", "s-1"])


def left(home: Path, woods: Path) -> StoredSession:
    return read_session(home / "sessions" / workspace_key(woods) / "s-1", ALL_KINDS)


def text_of(message: Message) -> str:
    return "".join(block.text for block in message.content if isinstance(block, TextBlock))


def asked_for_a_summary(request: LlmRequest) -> bool:
    return request.messages[-1] == UserMessage(content=(TextBlock(text=INSTRUCTION),))


def test_a_conversation_that_outgrows_its_model_is_summed_up_and_goes_on(
    home: Path,
    woods: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    things = [{"content": "Read the six files", "status": "in_progress"}, {"content": "Answer"}]

    code = ran(
        tmp_path,
        woods,
        call("todo_write", items=things),
        *[call("read", path=f"{tree}.txt") for tree in TREES[:5]],
        {"text": SUMMARY},
        call("read", path="hazel.txt"),
        {"text": ANSWER},
        window=WINDOW,
    )

    captured = capsys.readouterr()
    assert (code, captured.out, captured.err) == (0, f"{ANSWER}\n", "")
    events = left(home, woods).events
    assert [event.data for event in events if isinstance(event.data, TurnEndEvent)] == [
        TurnEndEvent(turn=1, reason="completed")
    ]

    # After the fifth file the conversation had come to more than it may fill, and its
    # oldest part was summed up before the next step: the task, the list of things to do,
    # and the first three files. The two files that were read last stay as they are.
    at = {event.type: event.seq for event in events if event.type.startswith("compaction/")}
    assert list(at) == ["compaction/start", "compaction/summary", "compaction/end"]
    begun, summed, over = (events[seq].data for seq in at.values())
    assert isinstance(begun, CompactionStartEvent)
    assert isinstance(summed, CompactionSummaryEvent)
    assert isinstance(over, CompactionEndEvent)
    assert (begun.why, over.why) == ("pressure", "pressure")
    assert begun.tokens > 11_250
    assert (over.outcome.trimmed, over.outcome.summarised, over.outcome.error) == (0, 9, None)
    assert over.outcome.before == begun.tokens
    assert over.outcome.after < 11_250 // 2 + 1000
    steps = [event.seq for event in events if isinstance(event.data, StepStartEvent)]
    assert len(steps) == 8
    assert steps[5] < at["compaction/start"] < at["compaction/end"] < steps[6]
    # What the summary cost is written down, beside the conversation.
    asides = [event.data for event in events if isinstance(event.data, AsideEvent)]
    assert [(aside.purpose, aside.failure) for aside in asides] == [("summary", None)]
    assert summed.summary == SUMMARY

    (model,) = models
    assert len(model.requests) == 9
    assert [asked_for_a_summary(request) for request in model.requests].index(True) == 6
    assert sum(asked_for_a_summary(request) for request in model.requests) == 1
    # The model that summed up was shown the part that went, as the model had seen it
    # before, and the tools as they were declared before.
    summing = model.requests[6]
    assert summing.messages[:-1] == model.requests[5].messages[: len(summing.messages) - 1]
    assert summing.tools == model.requests[5].tools
    shown = "\n".join(text_of(message) for message in summing.messages)
    assert all(f"The {tree} is a tree" in shown for tree in TREES[:3])
    assert not any(f"The {tree} is a tree" in shown for tree in TREES[3:])

    # From there on a request holds the instructions, what the model was told about the
    # run, the summary, the two calls that stayed with their results, and the list of
    # things to do, which the model is told again because the call that wrote it is
    # summed up.
    after = model.requests[7].messages
    assert after[0].role == "system"
    told = after[1:-6]
    assert told
    assert all(text_of(message).startswith("<context source=") for message in told)
    summary, *kept, recalled = after[-6:]
    assert summary == carried(SUMMARY)
    assert [type(message) for message in kept] == [
        AssistantMessage,
        ToolMessage,
        AssistantMessage,
        ToolMessage,
    ]
    assert ["The elm is a tree" in text_of(kept[1]), "The fir is a tree" in text_of(kept[3])] == [
        True,
        True,
    ]
    assert text_of(recalled).startswith('<context source="todo">')
    assert "[>] Read the six files\n[ ] Answer" in text_of(recalled)
    sent = "\n".join(text_of(message) for message in after)
    assert not any(f"The {tree} is a tree" in sent for tree in TREES[:3])
    assert TASK not in sent
    # The conversation goes on from there like any other: the last file is read.
    last = model.requests[8].messages
    assert last[: len(after)] == after
    assert "The hazel is a tree" in text_of(last[-1])


def test_a_conversation_that_the_model_refuses_is_made_shorter_and_the_step_is_taken_again(
    home: Path,
    woods: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    # Nothing says how much this model takes, so nothing is done before it refuses.
    code = ran(
        tmp_path,
        woods,
        call("read", path="alder.txt"),
        call("read", path="birch.txt"),
        refused(5000, 5600),
        {"text": SUMMARY},
        {"text": ANSWER},
        window=None,
    )

    captured = capsys.readouterr()
    assert (code, captured.out) == (0, f"{ANSWER}\n")
    events = left(home, woods).events
    kinds = [event.type for event in events]
    refusal = kinds.index("assistant/attempt")
    failed = events[refusal].data
    assert isinstance(failed, AssistantAttemptEvent)
    assert failed.failure.code == "CONTEXT_WINDOW_EXCEEDED"
    # The conversation is made shorter by as much as the service said was too much, and the
    # room that a reply needs: the task and the first file give way to the summary.
    assert kinds[refusal + 1 : refusal + 6] == [
        "compaction/start",
        "request/aside",
        "compaction/summary",
        "user/message",
        "compaction/end",
    ]
    over = events[refusal + 5].data
    assert isinstance(over, CompactionEndEvent)
    assert (over.why, over.outcome.summarised, over.outcome.error) == ("overflow", 3, None)
    # The same step is taken again, and this time the model answers.
    assert kinds[refusal + 6] == "assistant/message"
    assert sum(isinstance(event.data, StepStartEvent) for event in events) == 3
    assert [event.data for event in events if isinstance(event.data, TurnEndEvent)] == [
        TurnEndEvent(turn=1, reason="completed")
    ]
    (model,) = models
    again = "\n".join(text_of(message) for message in model.requests[4].messages)
    assert "The alder is a tree" not in again
    assert "The birch is a tree" in again


def test_a_refusal_ends_the_turn_where_nothing_of_the_conversation_can_go(
    home: Path,
    woods: Path,
    tmp_path: Path,
    models: list[scripted.Scripted],
    capsys: pytest.CaptureFixture[str],
) -> None:
    code = ran(tmp_path, woods, refused(5000, 5600), {"text": ANSWER}, window=None)

    captured = capsys.readouterr()
    assert (code, captured.out) == (1, "")
    assert "the turn ended with an error: CONTEXT_WINDOW_EXCEEDED" in captured.err
    events = left(home, woods).events
    (over,) = [event.data for event in events if isinstance(event.data, CompactionEndEvent)]
    assert over.why == "overflow"
    assert (over.outcome.trimmed, over.outcome.summarised) == (0, 0)
    assert over.outcome.error == "nothing of this conversation can go"
    (end,) = [event.data for event in events if isinstance(event.data, TurnEndEvent)]
    assert end.reason == "error"
    # The request was not made a second time: the model would have refused it again.
    (model,) = models
    assert len(model.requests) == 1
