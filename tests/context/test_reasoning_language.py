"""The language that a model reasons in: chosen in a file, told when a turn begins."""

from pathlib import Path

import pytest

from columbina.agent import Agent
from columbina.context import reasoning_language
from columbina.context.reasoning_language import choose, chosen
from columbina.llm.adapters.scripted import Reply
from columbina.session import UserMessageEvent
from support.loop import SYSTEM, Harness, context, idle, user

INSTEAD = "This takes the place of what the same source said earlier in the conversation."
CHINESE = (
    "Reason in Chinese (中文): write your thinking before you answer in it, whatever language "
    "the conversation, the files or the tools use. Answer the user in the language that they "
    "write in, as before."
)
FREE = (
    "No language is asked for your reasoning any more. Reason in whatever language suits the task."
)


async def with_a_choice(harness: Harness, file: Path | None, *replies: Reply) -> Agent:
    await harness.ctx.plugin(reasoning_language, {"file": file})
    return await harness.agent(*replies)


async def turn(agent: Agent, text: str) -> None:
    agent.followup(text)
    await idle(agent)


def told(agent: Agent) -> list[str]:
    """What the model was told of the language, without the frame around it."""
    return [
        "\n".join(event.data.message.content[0].text.split("\n")[1:-1])
        for event in agent.session.events
        if isinstance(event.data, UserMessageEvent)
        and event.data.source == "context/reasoning-language"
    ]


# What the file holds


@pytest.mark.parametrize("code", ["zh", "en", "ja"])
def test_a_language_that_is_chosen_is_kept_by_its_code(tmp_path: Path, code: str) -> None:
    file = tmp_path / "home" / "reasoning-language"

    choose(file, code)

    assert file.read_text(encoding="utf-8") == f"{code}\n"
    assert chosen(file) == code


def test_no_language_takes_the_file_away(tmp_path: Path) -> None:
    file = tmp_path / "reasoning-language"
    choose(file, "zh")

    choose(file, None)
    choose(file, None)

    assert not file.exists()
    assert chosen(file) is None


def test_a_language_that_cannot_be_chosen_is_refused(tmp_path: Path) -> None:
    file = tmp_path / "reasoning-language"

    with pytest.raises(ValueError, match="'fr' is no language that can be chosen: zh, en, ja"):
        choose(file, "fr")

    assert not file.exists()


@pytest.mark.parametrize("held", [b"fr\n", b"", b"\xff\xfe", b" zh \r\n"])
def test_a_file_names_a_language_only_by_a_code_that_is_known(tmp_path: Path, held: bytes) -> None:
    file = tmp_path / "reasoning-language"
    file.write_bytes(held)

    assert chosen(file) == ("zh" if held.strip() == b"zh" else None)


def test_a_file_that_cannot_be_read_names_none(tmp_path: Path) -> None:
    assert chosen(tmp_path) is None
    assert chosen(tmp_path / "nothing" / "here") is None


# What the model is told


async def test_the_model_is_told_the_language_before_the_input_of_the_turn(
    harness: Harness, tmp_path: Path
) -> None:
    file = tmp_path / "reasoning-language"
    choose(file, "zh")
    agent = await with_a_choice(harness, file, Reply(text="好。"))

    await turn(agent, "Hello.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        context("reasoning-language", CHINESE),
        user("Hello."),
    )


async def test_without_a_choice_nothing_is_said(harness: Harness, tmp_path: Path) -> None:
    agent = await with_a_choice(harness, tmp_path / "reasoning-language", Reply(text="Hi."))

    await turn(agent, "Hello.")

    assert told(agent) == []
    assert harness.model.requests[0].messages == (SYSTEM, user("Hello."))


async def test_without_a_file_nothing_is_said(harness: Harness) -> None:
    agent = await with_a_choice(harness, None, Reply(text="Hi."))

    await turn(agent, "Hello.")

    assert told(agent) == []


async def test_a_new_choice_follows_the_conversation_and_leaves_its_beginning_as_it_was(
    harness: Harness, tmp_path: Path
) -> None:
    file = tmp_path / "reasoning-language"
    choose(file, "zh")
    replies = (Reply(text="One."), Reply(text="Two."), Reply(text="Three."), Reply(text="Four."))
    agent = await with_a_choice(harness, file, *replies)
    await turn(agent, "First.")

    choose(file, "en")
    await turn(agent, "Second.")
    await turn(agent, "Third.")
    choose(file, None)
    await turn(agent, "Fourth.")

    english = CHINESE.replace("Chinese (中文)", "English")
    assert told(agent) == [CHINESE, f"{INSTEAD}\n\n{english}", f"{INSTEAD}\n\n{FREE}"]
    first, *_, last = harness.model.requests
    # The model reads the conversation as it began, which a service keeps from request to request.
    assert last.messages[: len(first.messages)] == first.messages


async def test_a_language_that_is_chosen_after_none_was_is_told_as_any_other(
    harness: Harness, tmp_path: Path
) -> None:
    file = tmp_path / "reasoning-language"
    agent = await with_a_choice(harness, file, Reply(text="One."), Reply(text="Two."))
    await turn(agent, "First.")

    choose(file, "ja")
    await turn(agent, "Second.")

    assert told(agent) == [CHINESE.replace("Chinese (中文)", "Japanese (日本語)")]
