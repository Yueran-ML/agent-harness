"""The tools that work on files, called by a model that the loop runs.

What a model has been shown is a matter of what it had before it when it
wrote a call. Only the loop knows that, so these tests run the loop.
"""

from pathlib import Path

import pytest

import columbina.fs
import columbina.sandbox
from columbina.agent import Agent
from columbina.builtin_tools import edit, read, write
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent
from support.loop import Harness, idle


@pytest.fixture
async def files(harness: Harness) -> Path:
    """The working directory of an agent that can read, write and edit files."""
    ctx = harness.ctx
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(read)
    await ctx.plugin(write)
    await ctx.plugin(edit)
    (harness.cwd / "notes.txt").write_text("what was there\n")
    return harness.cwd


def call(name: str, **arguments: str) -> Call:
    return Call(name=name, arguments=dict(arguments))


def codes(agent: Agent) -> list[str | None]:
    """The error codes of the tool results of a session, ``None`` for a call that succeeded."""
    return [
        None if event.data.error is None else event.data.error.code
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


async def test_a_file_that_is_read_and_written_in_one_reply_is_not_written(
    harness: Harness, files: Path
) -> None:
    # The model wrote the second call before it had seen the result of the first.
    agent = await harness.agent(
        Reply(
            calls=(
                call("read", path="notes.txt"),
                call("write", path="notes.txt", content="a guess\n"),
            )
        ),
        Reply(text="I could not write it."),
    )

    agent.followup("Change the notes.")
    await idle(agent)

    assert codes(agent) == [None, "NOT_READ"]
    assert (files / "notes.txt").read_text() == "what was there\n"


async def test_a_file_that_is_read_and_edited_in_one_reply_is_not_edited(
    harness: Harness, files: Path
) -> None:
    agent = await harness.agent(
        Reply(
            calls=(
                call("read", path="notes.txt"),
                call("edit", path="notes.txt", old="was", new="is"),
            )
        ),
        Reply(text="I could not edit it."),
    )

    agent.followup("Change the notes.")
    await idle(agent)

    assert codes(agent) == [None, "NOT_READ"]
    assert (files / "notes.txt").read_text() == "what was there\n"


async def test_a_file_that_was_read_in_one_reply_is_written_in_the_next(
    harness: Harness, files: Path
) -> None:
    agent = await harness.agent(
        Reply(calls=(call("read", path="notes.txt"),)),
        Reply(calls=(call("write", path="notes.txt", content="after reading\n"),)),
        Reply(text="Done."),
    )

    agent.followup("Change the notes.")
    await idle(agent)

    assert codes(agent) == [None, None]
    assert (files / "notes.txt").read_text() == "after reading\n"


async def test_what_the_model_wrote_it_may_change_again_in_the_same_reply(
    harness: Harness, files: Path
) -> None:
    # It has not seen the file, but it knows what is in it: it wrote that itself.
    agent = await harness.agent(
        Reply(
            calls=(
                call("write", path="new.txt", content="one\ntwo\n"),
                call("edit", path="new.txt", old="two", new="three"),
                call("write", path="new.txt", content="all new\n"),
            )
        ),
        Reply(text="Done."),
    )

    agent.followup("Make a file.")
    await idle(agent)

    assert codes(agent) == [None, None, None]
    assert (files / "new.txt").read_text() == "all new\n"


async def test_a_read_beside_a_write_of_the_model_does_not_undo_what_the_model_knows(
    harness: Harness, files: Path
) -> None:
    # The read shows the file as it was before the write of the same reply. When its
    # result reaches the model, the file is what the model wrote, and that is what counts.
    agent = await harness.agent(
        Reply(calls=(call("read", path="notes.txt"),)),
        Reply(
            calls=(
                call("read", path="notes.txt"),
                call("write", path="notes.txt", content="second\n"),
            )
        ),
        Reply(calls=(call("write", path="notes.txt", content="third\n"),)),
        Reply(text="Done."),
    )

    agent.followup("Change the notes twice.")
    await idle(agent)

    assert codes(agent) == [None, None, None, None]
    assert (files / "notes.txt").read_text() == "third\n"


async def test_a_file_that_was_read_in_one_turn_is_written_in_the_next(
    harness: Harness, files: Path
) -> None:
    agent = await harness.agent(
        Reply(calls=(call("read", path="notes.txt"),)),
        Reply(text="I have read it."),
        Reply(calls=(call("write", path="notes.txt", content="in the next turn\n"),)),
        Reply(text="Done."),
    )

    agent.followup("Read the notes.")
    await idle(agent)
    agent.followup("Now change them.")
    await idle(agent)

    assert codes(agent) == [None, None]
    assert (files / "notes.txt").read_text() == "in the next turn\n"
