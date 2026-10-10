"""The commands that list the skills and hand one to the agent."""

import os
from pathlib import Path
from typing import Any

import pytest

import columbina.commands
import columbina.skills
from columbina.agent import Agent
from columbina.agent.inbox import INBOX
from columbina.commands import COMMANDS, Commands
from columbina.commands import skills as skills_command
from columbina.llm.adapters.scripted import Reply
from columbina.session import TurnStartEvent, UserMessageEvent
from columbina.skills import files
from columbina.skills.files import FILE, Skill
from support.loop import SYSTEM, Harness, context, idle, user

INTRO = (
    "A skill is a set of instructions for one kind of task, written by the people you work "
    "for. These skills are there. When a task is of the kind that a skill is for, call the "
    "tool `skill` with its name before you do anything else, and do the task the way it says."
)
FROM_THERE = "A path in what follows that does not begin at a root is meant from that directory."


def put(
    root: Path,
    name: str,
    description: str = "Do the thing.",
    text: str = "The way to do it.",
    more: str = "",
) -> Path:
    """Write a skill into a place for skills, and return its directory."""
    directory = root / name
    directory.mkdir(parents=True, exist_ok=True)
    content = f"---\nname: {name}\ndescription: {description}\n{more}---\n{text}\n"
    (directory / FILE).write_bytes(content.encode())
    return directory


def of_the_project(harness: Harness) -> Path:
    """The place for skills in the working directory, by the name that the system has for it."""
    return Path(os.path.realpath(harness.cwd)) / ".columbina" / "skills"


async def loaded(harness: Harness, **config: Any) -> Commands:
    await harness.ctx.plugin(columbina.skills, config)
    await harness.ctx.plugin(columbina.commands)
    await harness.ctx.plugin(skills_command)
    return harness.ctx.use(COMMANDS)


def handed(directory: Path, text: str = "The way to do it.") -> str:
    """What a model reads when a person hands it the skill in a directory."""
    return (
        "The user wants you to go by this skill.\n\n"
        f"Skill: {directory.name}\nDirectory of the skill: {directory}\n{FROM_THERE}\n\n{text}"
    )


def turns(agent: Agent) -> int:
    return sum(isinstance(event.data, TurnStartEvent) for event in agent.session.events)


def waiting(agent: Agent) -> list[tuple[str, str]]:
    """What waits for the next step of the agent: the source and the text of each input."""
    return [
        (item.source, "".join(block.text for block in item.message.content))
        for item in INBOX.of(agent.session).next_step
    ]


# The list


async def test_the_skills_are_listed_with_what_they_are_for_and_where_they_were_found(
    harness: Harness, tmp_path: Path
) -> None:
    home = tmp_path / "home"
    put(of_the_project(harness), "deploy", "Deploy\n  the service.")
    put(home, "release-notes", "Write the notes of a release.")
    commands = await loaded(harness, home=home)
    agent = await harness.agent()

    said = await commands.run(agent, "/skills")

    assert said == (
        "deploy         (project)  Deploy the service.\n"
        "release-notes  (home)  Write the notes of a release."
    )
    # The model is told nothing by it.
    assert [
        event for event in agent.session.events if isinstance(event.data, UserMessageEvent)
    ] == []


async def test_the_list_says_who_can_load_a_skill_if_not_everybody_can(harness: Harness) -> None:
    put(of_the_project(harness), "a", more="disable-model-invocation: true\n")
    put(of_the_project(harness), "b", more="user-invocable: false\n")
    put(
        of_the_project(harness), "c", more="disable-model-invocation: true\nuser-invocable: false\n"
    )
    put(of_the_project(harness), "d")
    commands = await loaded(harness)
    agent = await harness.agent()

    assert (await commands.run(agent, "/skills")).split("\n") == [
        "a  (project)  Do the thing.  [only with /skill]",
        "b  (project)  Do the thing.  [only the agent loads it]",
        "c  (project)  Do the thing.  [nobody can load it]",
        "d  (project)  Do the thing.",
    ]


async def test_the_list_names_what_was_passed_over_and_what_cannot_be_used(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    put(of_the_project(harness), "deploy", "The way of the project.")
    passed_over = put(home, "deploy", "The way of the user.")
    broken = of_the_project(harness) / "broken"
    broken.mkdir()
    (broken / FILE).write_text("no dashes\n")
    commands = await loaded(harness, home=home)
    agent = await harness.agent()

    assert (await commands.run(agent, "/skills")).split("\n") == [
        "deploy  (project)  The way of the project.",
        "Passed over, because a place that counts before theirs has a skill of that name:",
        f"  deploy  (home)  {passed_over}",
        "Not usable:",
        f"  {broken}: SKILL.md does not begin with a line of three dashes",
    ]


async def test_where_there_are_no_skills_the_list_says_so(harness: Harness) -> None:
    commands = await loaded(harness)
    agent = await harness.agent()

    assert await commands.run(agent, "/skills") == "There are no skills."


async def test_a_long_word_on_what_a_skill_is_for_is_cut_in_the_list(harness: Harness) -> None:
    put(of_the_project(harness), "deploy", "word " * 200)
    commands = await loaded(harness)
    agent = await harness.agent()

    said = await commands.run(agent, "/skills")

    assert said == "deploy  (project)  " + ("word " * 100)[:499] + "…"


# Handing a skill over


async def test_a_skill_that_is_handed_over_waits_for_what_the_person_says_next(
    harness: Harness,
) -> None:
    directory = put(of_the_project(harness), "deploy", text="Ask before you deploy.")
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="I will ask."))

    said = await commands.run(agent, "/skill deploy")

    assert said == "The agent has the skill deploy now, and reads it with what you say next."
    # Nothing has begun, and the text of the skill waits.
    assert turns(agent) == 0
    assert waiting(agent) == [("skill", handed(directory, "Ask before you deploy."))]

    agent.followup("Deploy the service.")
    await idle(agent)

    assert harness.model.requests[0].messages == (
        SYSTEM,
        context("skills", "\n".join([INTRO, "- deploy: Do the thing."])),
        user(handed(directory, "Ask before you deploy.")),
        user("Deploy the service."),
    )


async def test_a_task_after_the_name_of_a_skill_is_begun_at_once(harness: Harness) -> None:
    directory = put(of_the_project(harness), "deploy", text="Ask before you deploy.")
    commands = await loaded(harness)
    agent = await harness.agent(Reply(text="I will ask."))

    said = await commands.run(agent, "/skill deploy   Deploy the service,  now. ")
    await idle(agent)

    # The turn that began is the answer.
    assert said == ""
    assert turns(agent) == 1
    assert harness.model.requests[0].messages[-2:] == (
        user(handed(directory, "Ask before you deploy.")),
        user("Deploy the service,  now."),
    )


async def test_a_skill_that_a_model_may_not_load_can_be_handed_over(harness: Harness) -> None:
    directory = put(of_the_project(harness), "deploy", more="disable-model-invocation: true\n")
    commands = await loaded(harness)
    agent = await harness.agent()

    await commands.run(agent, "/skill deploy")

    assert waiting(agent) == [("skill", handed(directory))]


async def test_a_skill_that_only_the_agent_loads_is_not_handed_over(harness: Harness) -> None:
    put(of_the_project(harness), "deploy", more="user-invocable: false\n")
    commands = await loaded(harness)
    agent = await harness.agent()

    said = await commands.run(agent, "/skill deploy Deploy the service.")

    assert said == "The skill deploy is for the agent to load by itself, and is not handed over."
    assert (turns(agent), waiting(agent)) == (0, [])


async def test_the_command_wants_the_name_of_a_skill_that_there_is(harness: Harness) -> None:
    put(of_the_project(harness), "deploy")
    commands = await loaded(harness)
    agent = await harness.agent()

    assert await commands.run(agent, "/skill") == (
        "Say which skill: /skill NAME, and after the name what is to be done by it. "
        "/skills lists them."
    )
    assert await commands.run(agent, "/skill release Do it.") == (
        "There is no skill named 'release'. /skills lists the skills."
    )
    assert (turns(agent), waiting(agent)) == (0, [])


async def test_a_skill_is_read_anew_when_it_is_handed_over(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = put(of_the_project(harness), "deploy", text="The old way.")
    commands = await loaded(harness)
    agent = await harness.agent()
    really = files.body

    def body(skill: Skill) -> tuple[Skill, str]:
        put(of_the_project(harness), "deploy", text="The new way.", more="user-invocable: false\n")
        return really(skill)

    monkeypatch.setattr(files, "body", body)

    # What the file says when it is read counts, and by then it was closed to people.
    assert await commands.run(agent, "/skill deploy") == (
        "The skill deploy is for the agent to load by itself, and is not handed over."
    )
    assert waiting(agent) == []
    assert directory.is_dir()


@pytest.mark.parametrize(
    ("error", "said"),
    [
        (
            files.SkillError("SKILL.md is no longer there"),
            "cannot be used: SKILL.md is no longer there.",
        ),
        (PermissionError(13, "Permission denied"), "cannot be read: Permission denied."),
        (OSError("the disk is gone"), "cannot be read: the disk is gone."),
    ],
)
async def test_a_skill_that_cannot_be_read_is_not_handed_over(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, error: Exception, said: str
) -> None:
    put(of_the_project(harness), "deploy")
    commands = await loaded(harness)
    agent = await harness.agent()

    def body(skill: Skill) -> tuple[Skill, str]:
        raise error

    monkeypatch.setattr(files, "body", body)

    assert await commands.run(agent, "/skill deploy Do it.") == f"The skill deploy {said}"
    assert (turns(agent), waiting(agent)) == (0, [])


async def test_the_commands_are_listed_and_leave_with_their_plugin(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.skills)
    await harness.ctx.plugin(columbina.commands)
    plugin = await harness.ctx.plugin(skills_command)
    commands = harness.ctx.use(COMMANDS)
    agent = await harness.agent()

    listed = await commands.run(agent, "/help")
    assert "/skills  List the skills, and what cannot be used as one." in listed
    assert "/skill   Hand a skill to the agent: /skill NAME [TASK]." in listed

    await plugin.dispose()

    assert (
        await commands.run(agent, "/skills")
        == "There is no command /skills. /help lists the commands."
    )
    assert (
        await commands.run(agent, "/skill deploy")
        == "There is no command /skill. /help lists the commands."
    )
