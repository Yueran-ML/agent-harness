"""Skills in the loop: what a model is told of them, how it loads one, and what it may read."""

import logging
import os
from pathlib import Path
from typing import Any

import pytest
from pydantic import ValidationError

import columbina.fs
import columbina.sandbox
import columbina.skills
from columbina.agent import Agent
from columbina.builtin_tools import read, write
from columbina.fs import Access, Located, locate
from columbina.llm import LlmRequest
from columbina.llm.adapters.scripted import Call, Reply
from columbina.session import ToolResultEvent, TurnEndEvent, UserMessageEvent
from columbina.skills import (
    MAX_DESCRIPTION,
    MIN_DESCRIPTION,
    SKILLS,
    Config,
    Skills,
    catalog,
    files,
    loaded,
)
from columbina.skills.files import FILE, Root, Skill
from support.links import link_directory
from support.loop import SYSTEM, Harness, context, idle, user

INTRO = (
    "A skill is a set of instructions for one kind of task, written by the people you work "
    "for. These skills are there. When a task is of the kind that a skill is for, call the "
    "tool `skill` with its name before you do anything else, and do the task the way it says."
)
GONE = "No skill is there any more. The skills that you were told of before cannot be loaded."
FROM_THERE = "A path in what follows that does not begin at a root is meant from that directory."
SOMEWHERE = Root(Path("/skills"), "home", own=True)


def instead(text: str) -> str:
    """What a model is told of skills where it still sees what it was told before."""
    return (
        f"This takes the place of what the same source said earlier in the conversation.\n\n{text}"
    )


def listed(*lines: str) -> str:
    """What a model is told: the words before the list, and a line for each skill."""
    return "\n".join([INTRO, *lines])


def left_out(count: int) -> str:
    more = "1 more skill is" if count == 1 else f"{count} more skills are"
    return (
        f"[{more} there and not listed, because the list would be too long. The user sees "
        "all of them with /skills.]"
    )


def a_skill(name: str, description: str = "Do the thing.", *, model: bool = True) -> Skill:
    return Skill(name, description, SOMEWHERE.path / name, SOMEWHERE, model_invocable=model)


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


async def working(harness: Harness, *script: Any, **config: Any) -> Agent:
    """Load the plugin and return an agent in the working directory of the harness."""
    await harness.ctx.plugin(columbina.skills, config)
    return await harness.agent(*(script or (Reply(text="Hello!"),)))


async def turn(agent: Agent, text: str = "Go on.") -> None:
    agent.followup(text)
    await idle(agent)


def load(name: str) -> Reply:
    """A reply of the model that loads a skill."""
    return Reply(calls=(Call(name="skill", arguments={"name": name}),))


def told(agent: Agent) -> list[str]:
    """What the model still sees of what it was told of skills, without the frame around it."""
    session = agent.session
    return [
        "\n".join(data.message.content[0].text.split("\n")[1:-1])
        for seq in session.surface
        if isinstance(data := session.events[seq].data, UserMessageEvent)
        and data.source == "context/skills"
    ]


def times_told(agent: Agent) -> int:
    """How often the log holds word of skills, whether the model still sees it or not."""
    return sum(
        isinstance(event.data, UserMessageEvent) and event.data.source == "context/skills"
        for event in agent.session.events
    )


def results(agent: Agent) -> list[tuple[str, str | None]]:
    """What each call of a tool gave back, and the code of its error if it failed."""
    return [
        (
            "".join(block.text for block in event.data.message.content),
            None if event.data.error is None else event.data.error.code,
        )
        for event in agent.session.events
        if isinstance(event.data, ToolResultEvent)
    ]


def ends(agent: Agent) -> list[str]:
    return [
        event.data.reason for event in agent.session.events if isinstance(event.data, TurnEndEvent)
    ]


def logged(caplog: pytest.LogCaptureFixture, level: int = logging.WARNING) -> list[str]:
    return [
        record.getMessage()
        for record in caplog.records
        if record.name == "columbina.skills" and record.levelno == level
    ]


# What a model is told


def test_a_model_is_told_of_each_skill_in_a_line() -> None:
    skills = [a_skill("deploy", "Deploy the service."), a_skill("notes", "Write release notes.")]

    assert catalog(skills, 12000) == listed(
        "- deploy: Deploy the service.", "- notes: Write release notes."
    )


def test_a_skill_that_a_model_may_not_load_is_not_told_of() -> None:
    skills = [a_skill("deploy", model=False), a_skill("notes", "Write release notes.")]

    assert catalog(skills, 12000) == listed("- notes: Write release notes.")
    assert catalog([a_skill("deploy", model=False)], 12000) is None
    assert catalog([], 12000) is None


def test_what_a_skill_is_for_is_told_on_one_line_and_not_at_length() -> None:
    spread = "Deploy\n  the\tservice,\r\n and say so. "
    long = "word " * 200

    told_of = catalog([a_skill("deploy", spread), a_skill("notes", long)], 12000)

    assert told_of is not None
    deploy, notes = told_of.split("\n")[1:]
    assert deploy == "- deploy: Deploy the service, and say so."
    # Five hundred characters, of which the last one says that there was more.
    assert notes == "- notes: " + ("word " * 100)[: MAX_DESCRIPTION - 1] + "…"
    assert len(notes) == len("- notes: ") + MAX_DESCRIPTION


def test_what_a_skill_is_for_is_told_whole_if_it_is_just_short_enough() -> None:
    just = "x" * MAX_DESCRIPTION

    assert catalog([a_skill("deploy", just)], 12000) == listed(f"- deploy: {just}")
    assert catalog([a_skill("deploy", just + "y")], 12000) == listed(f"- deploy: {just[:-1]}…")
    # A list that is just as long as it may be is given as it is.
    whole = listed(f"- deploy: {just}")
    assert catalog([a_skill("deploy", just)], len(whole)) == whole


def lines_at(skills: list[Skill], width: int) -> list[str]:
    """The lines of some skills, each told of in ``width`` characters at most."""
    found: list[str] = []
    for skill in skills:
        description = " ".join(skill.description.split())
        if len(description) > width:
            description = description[: width - 1] + "…"
        found.append(f"- {skill.name}: {description}")
    return found


def test_skills_that_are_too_many_to_be_told_of_in_full_are_all_told_of_more_briefly() -> None:
    skills = [a_skill(f"skill-{number:02}", f"{number} " + "word " * 80) for number in range(30)]
    limit = 6000
    assert len(listed(*lines_at(skills, MAX_DESCRIPTION))) > limit
    # The widest lines that fit, found the slow way.
    width = max(
        width
        for width in range(MIN_DESCRIPTION, MAX_DESCRIPTION)
        if len(listed(*lines_at(skills, width))) <= limit
    )

    told_of = catalog(skills, limit)

    expected = listed(*lines_at(skills, width))
    assert told_of == expected
    assert len(expected) <= limit < len(listed(*lines_at(skills, width + 1)))
    # Every skill is still there, and none gave way more than another.
    assert len(expected.split("\n")) == 1 + len(skills)
    # Lines that fill the room to the last character are as wide as they can be.
    just = listed(*lines_at(skills, 200))
    assert catalog(skills, len(just)) == just


def test_lines_of_just_the_shortest_length_are_still_all_given() -> None:
    skills = [a_skill(f"skill-{number:02}", "word " * 80) for number in range(30)]
    shortest = listed(*lines_at(skills, MIN_DESCRIPTION))

    assert catalog(skills, len(shortest)) == shortest
    # One character less, and the last skill gives way to word of what was left out.
    fewer = catalog(skills, len(shortest) - 1)
    assert fewer is not None
    assert fewer.split("\n")[-1].startswith("[")
    assert len(fewer) <= len(shortest) - 1


def test_skills_that_are_too_many_even_so_are_left_out_from_the_end_and_that_is_said() -> None:
    skills = [a_skill(f"skill-{number:03}", "word " * 80) for number in range(200)]
    limit = 3000
    short = lines_at(skills, MIN_DESCRIPTION)
    # The most skills that fit, found the slow way.
    kept = max(
        count
        for count in range(len(skills))
        if len(listed(*short[:count], left_out(len(skills) - count))) <= limit
    )

    told_of = catalog(skills, limit)

    assert 0 < kept < len(skills) - 1
    expected = listed(*short[:kept], left_out(len(skills) - kept))
    assert told_of == expected
    assert len(expected) <= limit
    assert len(listed(*short[: kept + 1], left_out(len(skills) - kept - 1))) > limit


def test_one_skill_that_is_left_out_is_one_skill() -> None:
    # Names so long that a line takes more room than the word of what was left out.
    names = [f"{which}-skill-with-a-name-that-fills-half-of-its-line" for which in "abc"]
    skills = [a_skill(name, "word " * 80) for name in names]
    short = lines_at(skills, MIN_DESCRIPTION)
    limit = len(listed(short[0], short[1], left_out(1)))
    assert limit < len(listed(*short))

    assert catalog(skills, limit) == listed(short[0], short[1], left_out(1))
    assert left_out(1).startswith("[1 more skill is there and not listed")
    assert left_out(2).startswith("[2 more skills are there and not listed")


def test_the_words_before_the_list_are_given_however_little_room_there_is() -> None:
    skills = [a_skill("first", "word " * 80), a_skill("last", "word " * 80)]

    assert catalog(skills, 10) == listed(left_out(2))


# In the loop


async def test_the_skills_stand_before_the_first_input_and_their_text_does_not(
    harness: Harness,
) -> None:
    put(of_the_project(harness), "notes", "Write release notes.", "Begin with the codename.")
    put(of_the_project(harness), "deploy", "Deploy the service.", "Ask before you deploy.")
    agent = await working(harness)

    await turn(agent, "Say hello.")

    assert harness.model.requests[0].messages == (
        SYSTEM,
        context("skills", listed("- deploy: Deploy the service.", "- notes: Write release notes.")),
        user("Say hello."),
    )
    # The list is told after the facts of the run, and before the list of things to do.
    source = harness.agents.context["skills"]
    assert (source.order, source.kind) == (120, "replace")


async def test_without_skills_nothing_is_told(harness: Harness) -> None:
    put(of_the_project(harness), "deploy", more="disable-model-invocation: true\n")
    agent = await working(harness, Reply(text="Hello!"), Reply(text="Hello again!"))

    await turn(agent, "Say hello.")
    await turn(agent, "Say it again.")

    assert harness.model.requests[0].messages == (SYSTEM, user("Say hello."))
    # Nor is a model told that skills are gone of which it never heard.
    assert times_told(agent) == 0


async def test_skills_are_found_in_all_their_places_and_the_first_place_counts(
    harness: Harness, tmp_path: Path
) -> None:
    home, shared, extra = tmp_path / "home", tmp_path / "shared", tmp_path / "extra"
    put(of_the_project(harness), "deploy", "The way of the project.")
    put(harness.cwd / ".agents" / "skills", "review", "Review a change.")
    put(home, "deploy", "The way of the user.")
    put(home, "notes", "Write notes.")
    put(shared, "notes", "The notes of the shared place.")
    put(shared, "translate", "Translate a text.")
    put(extra, "translate", "The translation of the further place.")
    put(extra, "audit", "Audit a change.")
    agent = await working(harness, home=home, user=shared, extra=[extra])

    await turn(agent)

    assert told(agent) == [
        listed(
            "- audit: Audit a change.",
            "- deploy: The way of the project.",
            "- notes: Write notes.",
            "- review: Review a change.",
            "- translate: Translate a text.",
        )
    ]


async def test_the_skills_of_a_project_are_those_at_its_root(
    harness: Harness, tmp_path: Path
) -> None:
    project = tmp_path / "project"
    (project / ".git").mkdir(parents=True)
    (project / "src").mkdir()
    put(project / ".columbina" / "skills", "deploy", "Deploy the service.")
    await harness.ctx.plugin(columbina.skills)
    harness.model.add(Reply(text="Hello!"))
    agent = await harness.agents.create(harness.ctx, harness.spec(cwd=project / "src"))

    await turn(agent)

    assert told(agent) == [listed("- deploy: Deploy the service.")]


async def test_the_list_is_cut_to_the_length_that_the_config_gives(harness: Harness) -> None:
    for number in range(40):
        put(of_the_project(harness), f"skill-{number:02}", "word " * 60)
    agent = await working(harness, catalog_max_chars=1000)

    await turn(agent)

    (said,) = told(agent)
    assert len(said) <= 1000
    assert said.split("\n")[-1] == left_out(40 - (len(said.split("\n")) - 2))


async def test_a_change_of_the_skills_is_told_in_the_place_of_what_was_told_before(
    harness: Harness,
) -> None:
    put(of_the_project(harness), "deploy", "Deploy the service.")
    agent = await working(harness, Reply(text="One."), Reply(text="Two."), Reply(text="Three."))

    await turn(agent)
    put(of_the_project(harness), "notes", "Write release notes.")
    await turn(agent)

    # The model sees the new list after the old one, said to hold in its place.
    assert told(agent) == [
        listed("- deploy: Deploy the service."),
        instead(listed("- deploy: Deploy the service.", "- notes: Write release notes.")),
    ]
    assert times_told(agent) == 2

    # Another text of a skill is no other list.
    put(of_the_project(harness), "notes", "Write release notes.", "Begin with the codename.")
    await turn(agent)

    assert times_told(agent) == 2


async def test_a_model_is_told_when_its_skills_are_gone(harness: Harness) -> None:
    directory = put(of_the_project(harness), "deploy", "Deploy the service.")
    agent = await working(harness, Reply(text="One."), Reply(text="Two."), Reply(text="Three."))

    await turn(agent)
    (directory / FILE).unlink()
    await turn(agent)

    assert told(agent) == [listed("- deploy: Deploy the service."), instead(GONE)]

    # They may come back.
    put(of_the_project(harness), "deploy", "Deploy the service.")
    await turn(agent)

    assert told(agent)[-1] == instead(listed("- deploy: Deploy the service."))


async def test_what_a_summary_took_from_the_model_is_told_again(harness: Harness) -> None:
    put(of_the_project(harness), "deploy", "Deploy the service.")
    agent = await working(harness, Reply(text="One."), Reply(text="Two."))
    await turn(agent)
    session = agent.session
    gone = [
        seq
        for seq in session.surface
        if isinstance(data := session.events[seq].data, UserMessageEvent)
        and data.source in ("context/skills", "user")
    ]
    summary = UserMessageEvent(message=user("A summary of what was said."), source="compaction")
    session.append(summary, replaces=gone)
    assert told(agent) == []

    await turn(agent)

    assert told(agent) == [listed("- deploy: Deploy the service.")]
    assert times_told(agent) == 2


# A place that cannot be looked into


def failing(monkeypatch: pytest.MonkeyPatch, label: str, error: Exception) -> list[bool]:
    """Make the look into the places of one label fail for as long as the switch is on."""
    switch = [True]
    really = files.scan

    def scan(root: Root) -> files.Scan:
        if switch[0] and root.label == label:
            raise error
        return really(root)

    monkeypatch.setattr(files, "scan", scan)
    return switch


async def test_a_place_that_cannot_be_looked_into_is_taken_as_it_was_found_before(
    harness: Harness,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    home = tmp_path / "home"
    put(home, "notes", "Write release notes.")
    put(of_the_project(harness), "deploy", "Deploy the service.")
    agent = await working(harness, *(Reply(text="Go.") for _ in range(4)), home=home)
    await turn(agent)
    whole = listed("- deploy: Deploy the service.", "- notes: Write release notes.")
    assert told(agent) == [whole]

    switch = failing(monkeypatch, "home", PermissionError(13, "Permission denied"))
    await turn(agent)
    await turn(agent)

    # One look that failed does not make the skills of that place go away.
    assert told(agent) == [whole]
    assert times_told(agent) == 1
    assert ends(agent) == ["completed"] * 3
    # It is said once, and not at every turn for as long as it lasts.
    assert logged(caplog) == [
        f"the skills in {home} cannot be looked for now, so what was found there before is "
        "kept: [Errno 13] Permission denied"
    ]
    # A disk that says no is no fault of the program.
    assert logged(caplog, logging.ERROR) == []

    # Once it can be looked into again, what is there counts.
    switch[0] = False
    put(home, "audit", "Audit a change.")
    await turn(agent)

    assert told(agent) == [
        whole,
        instead(
            listed(
                "- audit: Audit a change.",
                "- deploy: Deploy the service.",
                "- notes: Write release notes.",
            )
        ),
    ]


async def test_a_place_that_could_never_be_looked_into_holds_nothing(
    harness: Harness,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    home = tmp_path / "home"
    put(home, "notes", "Write release notes.")
    put(of_the_project(harness), "deploy", "Deploy the service.")
    failing(monkeypatch, "home", PermissionError(13, "Permission denied"))
    agent = await working(harness, home=home)

    await turn(agent)

    assert told(agent) == [listed("- deploy: Deploy the service.")]
    assert len(logged(caplog)) == 1


@pytest.mark.parametrize(
    ("error", "said"),
    [(RuntimeError("something else"), "something else"), (RuntimeError(), "RuntimeError")],
)
async def test_a_look_that_fails_in_a_way_that_nobody_foresaw_ends_no_turn(
    harness: Harness,
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
    error: Exception,
    said: str,
) -> None:
    put(of_the_project(harness), "deploy", "Deploy the service.")
    failing(monkeypatch, "project", error)
    agent = await working(harness)

    await turn(agent)

    assert ends(agent) == ["completed"]
    assert told(agent) == []
    assert logged(caplog) == [
        f"the skills in {of_the_project(harness)} cannot be looked for now, so what was found "
        f"there before is kept: {said}"
    ]
    # What went wrong is kept for whoever looks after the program.
    assert logged(caplog, logging.ERROR) == [
        f"looking for skills in {of_the_project(harness)} failed"
    ]
    assert [
        record.exc_info is not None for record in caplog.records if record.levelno == logging.ERROR
    ] == [True]


async def test_a_skill_that_cannot_be_used_is_said_once(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    broken = of_the_project(harness) / "broken"
    broken.mkdir(parents=True)
    (broken / FILE).write_text("no dashes\n")
    put(of_the_project(harness), "deploy", "Deploy the service.")
    agent = await working(harness, *(Reply(text="Go.") for _ in range(4)))

    await turn(agent)
    await turn(agent)

    wrong = "SKILL.md does not begin with a line of three dashes"
    said = f"{broken} is no skill that can be used: {wrong}"
    assert logged(caplog) == [said]
    assert told(agent) == [listed("- deploy: Deploy the service.")]

    # Mended, and broken again: that is news again.
    put(of_the_project(harness), "broken", "Mended.")
    await turn(agent)
    (broken / FILE).write_text("no dashes\n")
    await turn(agent)

    assert logged(caplog) == [said, said]


async def test_a_skill_that_is_passed_over_is_written_down(
    harness: Harness, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO, logger="columbina.skills")
    home = tmp_path / "home"
    put(of_the_project(harness), "deploy", "The way of the project.")
    passed_over = put(home, "deploy", "The way of the user.")
    agent = await working(harness, Reply(text="One."), Reply(text="Two."), home=home)

    await turn(agent)
    await turn(agent)

    assert logged(caplog, logging.INFO) == [
        f"the skill in {passed_over.resolve()} is passed over for one of the same name"
    ]
    assert logged(caplog) == []


# Loading a skill


async def test_a_model_loads_a_skill_and_is_given_its_text_and_where_it_lies(
    harness: Harness,
) -> None:
    directory = put(
        of_the_project(harness), "notes", "Write release notes.", "Begin with the codename.\n\nEnd."
    )
    agent = await working(harness, load("notes"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        (
            f"Skill: notes\nDirectory of the skill: {directory}\n{FROM_THERE}\n\n"
            "Begin with the codename.\n\nEnd.",
            None,
        )
    ]
    # The lines at the beginning of the file are for the program, and the model gets none.
    assert "description" not in results(agent)[0][0]


def test_a_skill_without_text_is_given_as_what_it_is_and_where_it_lies() -> None:
    skill = a_skill("notes")

    assert (
        loaded(skill, "")
        == f"Skill: notes\nDirectory of the skill: {skill.directory}\n{FROM_THERE}"
    )
    assert loaded(skill, "Do it.").endswith(f"{FROM_THERE}\n\nDo it.")


async def test_a_skill_is_read_anew_every_time_that_it_is_loaded(harness: Harness) -> None:
    put(of_the_project(harness), "notes", text="The old way.")

    def again(request: LlmRequest) -> Reply:
        put(of_the_project(harness), "notes", text="The new way.")
        return load("notes")

    agent = await working(harness, load("notes"), again, Reply(text="Done."))

    await turn(agent)

    assert [text.split("\n\n")[1] for text, _ in results(agent)] == ["The old way.", "The new way."]


async def test_a_skill_that_was_written_during_a_turn_can_be_loaded_in_it(
    harness: Harness,
) -> None:
    def first(request: LlmRequest) -> Reply:
        put(of_the_project(harness), "notes", text="The way.")
        return load("notes")

    agent = await working(harness, first, Reply(text="Done."))

    await turn(agent)

    assert [code for _, code in results(agent)] == [None]


async def test_a_name_that_no_skill_has_is_answered_with_the_names_that_there_are(
    harness: Harness,
) -> None:
    put(of_the_project(harness), "notes")
    put(of_the_project(harness), "deploy")
    put(of_the_project(harness), "secret", more="disable-model-invocation: true\n")
    agent = await working(harness, load("release"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        ("There is no skill named 'release'. The skills are: deploy, notes.", "UNKNOWN_SKILL")
    ]


async def test_where_there_are_no_skills_a_model_that_asks_for_one_is_told_so(
    harness: Harness,
) -> None:
    agent = await working(harness, load("release"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        ("There is no skill named 'release'. There are no skills.", "UNKNOWN_SKILL")
    ]


async def test_a_skill_that_only_a_person_hands_over_is_not_loaded_by_a_model(
    harness: Harness,
) -> None:
    put(of_the_project(harness), "deploy", more="disable-model-invocation: true\n")
    agent = await working(harness, load("deploy"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        (
            "The skill deploy is not loaded by a model itself. The user can hand it to you "
            "with /skill deploy.",
            "NOT_FOR_A_MODEL",
        )
    ]


def meanwhile(monkeypatch: pytest.MonkeyPatch, change: Any) -> None:
    """Have something done between the look for a skill and the reading of its text."""
    really = files.body

    def body(skill: Skill) -> tuple[Skill, str]:
        change()
        return really(skill)

    monkeypatch.setattr(files, "body", body)


async def test_a_skill_that_was_closed_to_models_while_it_was_loaded_is_not_given(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    put(of_the_project(harness), "deploy", text="The way.")
    meanwhile(
        monkeypatch,
        lambda: put(of_the_project(harness), "deploy", more="disable-model-invocation: true\n"),
    )
    agent = await working(harness, load("deploy"), Reply(text="Done."))

    await turn(agent)

    assert [code for _, code in results(agent)] == ["NOT_FOR_A_MODEL"]
    assert "The way." not in results(agent)[0][0]


async def test_a_skill_that_was_broken_while_it_was_loaded_says_what_is_wrong_with_it(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = put(of_the_project(harness), "deploy")
    meanwhile(monkeypatch, lambda: (directory / FILE).write_text("no dashes\n"))
    agent = await working(harness, load("deploy"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [
        (
            "The skill deploy cannot be used: SKILL.md does not begin with a line of three dashes.",
            "INVALID_SKILL",
        )
    ]


@pytest.mark.parametrize(
    ("error", "said"),
    [
        (PermissionError(13, "Permission denied"), "Permission denied"),
        (OSError("the disk is gone"), "the disk is gone"),
    ],
)
async def test_a_skill_whose_file_cannot_be_read_says_so(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, error: OSError, said: str
) -> None:
    put(of_the_project(harness), "deploy")

    def fails() -> None:
        raise error

    meanwhile(monkeypatch, fails)
    agent = await working(harness, load("deploy"), Reply(text="Done."))

    await turn(agent)

    assert results(agent) == [(f"The skill deploy cannot be read: {said}.", "UNREADABLE_SKILL")]


async def test_skills_can_be_loaded_side_by_side(harness: Harness) -> None:
    await harness.ctx.plugin(columbina.skills)

    tool = harness.tools.registry["skill"]
    assert (tool.concurrency_safe, tool.effect) == (True, "none")


# The files of a skill


async def with_files(harness: Harness, home: Path, *script: Any) -> Agent:
    """An agent that can read and write files, with skills of the user in ``home``."""
    await harness.ctx.plugin(columbina.fs)
    await harness.ctx.plugin(columbina.sandbox)
    await harness.ctx.plugin(read)
    await harness.ctx.plugin(write)
    return await working(harness, *script, home=home)


def reads(path: Path) -> Reply:
    return Reply(calls=(Call(name="read", arguments={"path": str(path)}),))


def writes(path: Path) -> Reply:
    return Reply(calls=(Call(name="write", arguments={"path": str(path), "content": "mine"}),))


async def test_the_files_of_a_skill_can_be_read_wherever_the_skill_lies(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    directory = put(home, "notes", text="The codenames are in codenames.txt.")
    (directory / "codenames.txt").write_text("1.4: Larkspur\n")
    (home / "beside.txt").write_text("Nobody's business.\n")
    agent = await with_files(
        harness,
        home,
        load("notes"),
        reads(directory / "codenames.txt"),
        reads(home / "beside.txt"),
        writes(directory / "codenames.txt"),
        writes(directory / "new.txt"),
        Reply(text="Done."),
    )

    await turn(agent)

    loaded_skill, codenames, beside, written, made = results(agent)
    assert loaded_skill[1] is None
    assert (codenames[1], "Larkspur" in codenames[0]) == (None, True)
    # What lies beside the skill is as far away as it was.
    assert beside[1] == "OUTSIDE_WORKING_DIRECTORY"
    # Reading is all that a skill opens.
    assert written[1] == made[1] == "OUTSIDE_WORKING_DIRECTORY"
    assert (directory / "codenames.txt").read_text() == "1.4: Larkspur\n"
    assert not (directory / "new.txt").exists()


async def test_without_the_seam_of_the_file_system_skills_are_told_of_and_loaded_all_the_same(
    harness: Harness, tmp_path: Path
) -> None:
    put(tmp_path / "home", "notes", "Write release notes.")
    agent = await working(harness, load("notes"), Reply(text="Done."), home=tmp_path / "home")

    await turn(agent)

    assert told(agent) == [listed("- notes: Write release notes.")]
    assert [code for _, code in results(agent)] == [None]


async def asked(harness: Harness, home: Path) -> tuple[Skills, Agent]:
    await harness.ctx.plugin(columbina.skills, {"home": home})
    agent = await harness.agent(Reply(text="Hello!"))
    return harness.ctx.use(SKILLS), agent


def access(agent: Agent, path: Path, kind: Any = "read") -> Access:
    return Access(agent.session, agent.cwd, locate(agent.cwd, str(path)), kind, "read")


async def test_a_directory_that_differs_from_that_of_a_skill_by_its_case_is_another_one(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    directory = put(home, "notes")
    skills, agent = await asked(harness, home)
    await skills.found(agent.session, agent.cwd)

    def at(path: Path) -> Access:
        # Where the file lies, as a system would say that tells the two spellings apart.
        where = Located(path, str(path), False, path.as_posix(), agent.cwd)
        return Access(agent.session, agent.cwd, where, "read", "read")

    assert skills.grants(at(directory / "codenames.txt")) is True
    # A disk can hold both, and to Windows the two paths are one and the same.
    assert skills.grants(at(home / "NOTES" / "secret.txt")) is False
    assert skills.grants(at(home / "Notes")) is False
    assert skills.grants(at(Path(str(home).swapcase()) / "notes" / "codenames.txt")) is False


async def test_the_files_of_a_skill_may_be_read_once_its_session_was_told_of_it(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    directory = put(home, "notes")
    (directory / "parts").mkdir()
    skills, agent = await asked(harness, home)
    inside = access(agent, directory / "parts" / "codenames.txt")

    # Nobody has looked for the skills of this session yet.
    assert skills.grants(inside) is False

    await skills.found(agent.session, agent.cwd)

    assert skills.grants(inside) is True
    assert skills.grants(access(agent, directory)) is True
    assert skills.grants(access(agent, directory / FILE)) is True
    assert skills.grants(access(agent, home)) is False
    assert skills.grants(access(agent, home / "notes-2" / "file.txt")) is False
    assert skills.grants(access(agent, directory / "new.txt", "write")) is False
    # Another session was not told of anything.
    other = await harness.agent(Reply(text="Hello!"), session_id="s-2")
    assert skills.grants(access(other, directory / FILE)) is False


async def test_the_files_of_a_skill_that_is_passed_over_stay_closed(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    passed_over = put(home, "notes", "The notes of the user.")
    counts = put(of_the_project(harness), "notes", "The notes of the project.")
    skills, agent = await asked(harness, home)

    await skills.found(agent.session, agent.cwd)

    # A model is never given that skill, so nothing in it is its business.
    assert skills.grants(access(agent, passed_over / FILE)) is False
    assert skills.grants(access(agent, counts / FILE)) is True


async def test_a_link_in_the_directory_of_a_skill_opens_nothing_that_lies_elsewhere(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    directory = put(home, "notes")
    elsewhere = (tmp_path / "elsewhere").resolve()
    elsewhere.mkdir()
    (elsewhere / "secret.txt").write_text("Nobody's business.\n")
    link_directory(directory / "more", elsewhere)
    skills, agent = await asked(harness, home)

    await skills.found(agent.session, agent.cwd)

    # The grant goes by where a file really is.
    assert skills.grants(access(agent, directory / "more" / "secret.txt")) is False


async def test_the_files_of_a_skill_that_is_gone_can_be_read_no_more(
    harness: Harness, tmp_path: Path
) -> None:
    home = (tmp_path / "home").resolve()
    directory = put(home, "notes")
    (directory / "codenames.txt").write_text("1.4: Larkspur\n")
    skills, agent = await asked(harness, home)
    await skills.found(agent.session, agent.cwd)
    assert skills.grants(access(agent, directory / "codenames.txt")) is True

    (directory / FILE).unlink()
    await skills.found(agent.session, agent.cwd)

    assert skills.grants(access(agent, directory / "codenames.txt")) is False


# For whoever else wants to know


async def test_whoever_asks_is_told_of_every_skill_and_of_what_is_wrong_with_the_others(
    harness: Harness, tmp_path: Path
) -> None:
    home = tmp_path / "home"
    put(of_the_project(harness), "deploy", "The way of the project.", "Ask first.")
    put(of_the_project(harness), "secret", more="disable-model-invocation: true\n")
    put(home, "deploy", "The way of the user.")
    (home / "broken").mkdir()
    (home / "broken" / FILE).write_text("no dashes\n")
    skills, agent = await asked(harness, home)

    found = await skills.found(agent.session, agent.cwd)

    assert {name: skill.model_invocable for name, skill in found.skills.items()} == {
        "deploy": True,
        "secret": False,
    }
    assert [(skill.name, skill.root.label) for skill in found.shadowed] == [("deploy", "home")]
    assert [invalid.directory.name for invalid in found.invalid] == ["broken"]
    assert (await skills.body(found.skills["deploy"]))[1] == "Ask first."


# The config


@pytest.mark.parametrize(
    "config",
    [{"home": "skills"}, {"user": "skills"}, {"extra": [str(Path.cwd()), "skills"]}],
)
def test_a_place_for_skills_is_given_by_its_whole_path(config: dict[str, Any]) -> None:
    with pytest.raises(ValidationError, match="skills does not begin at a root"):
        Config.model_validate(config)


def test_the_list_may_not_be_given_too_little_room() -> None:
    assert Config().catalog_max_chars == 12000
    assert Config(catalog_max_chars=1000).catalog_max_chars == 1000
    with pytest.raises(ValidationError):
        Config(catalog_max_chars=999)
