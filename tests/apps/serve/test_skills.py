"""The service process: the skills that there are for the session that is open."""

import asyncio
import os
import threading
from pathlib import Path

import pytest

import columbina.skills
from columbina.apps.serve.rpc import NO_SESSION
from columbina.fs import Access, locate
from columbina.session import Session
from columbina.skills import MAX_DESCRIPTION, SKILLS, Skills
from columbina.skills.files import FILE, Found
from support.loop import Harness
from support.served import Served

NO_SESSION_OPEN = (NO_SESSION, "No session is open. session/open opens one.")


def put(harness: Harness, name: str, description: str, more: str = "") -> Path:
    """Write a skill into the place that the project has for skills, and return its file."""
    directory = Path(os.path.realpath(harness.cwd)) / ".columbina" / "skills" / name
    directory.mkdir(parents=True, exist_ok=True)
    content = f"---\nname: {name}\ndescription: {description}\n{more}---\nThe way to do it.\n"
    (directory / FILE).write_bytes(content.encode())
    return directory / FILE


def may_read(harness: Harness, session: Session, file: Path) -> bool:
    """Tell whether a session was told of the skill that a file belongs to."""
    cwd = Path(harness.cwd)
    where = locate(cwd, str(file))
    return harness.ctx.use(SKILLS).grants(Access(session, cwd, where, "read", "read"))


async def test_the_skills_of_the_session_are_listed_with_who_may_load_each(served: Served) -> None:
    harness = served.harness
    await harness.ctx.plugin(columbina.skills)
    put(harness, "review", "Reads a change\n  and says what is wrong with it.")
    put(harness, "release", "Cuts a release.", "disable-model-invocation: true\n")
    put(harness, "style", "The style of the house.", "user-invocable: false\n")
    await served.opened()

    # By name, each with what it is for on one line.
    assert await served.far.result("session/skills") == {
        "skills": [
            {
                "name": "release",
                "description": "Cuts a release.",
                "user_invocable": True,
                "model_invocable": False,
            },
            {
                "name": "review",
                "description": "Reads a change and says what is wrong with it.",
                "user_invocable": True,
                "model_invocable": True,
            },
            {
                "name": "style",
                "description": "The style of the house.",
                "user_invocable": False,
                "model_invocable": True,
            },
        ]
    }


async def test_the_skills_are_looked_for_anew_each_time(served: Served) -> None:
    harness = served.harness
    await harness.ctx.plugin(columbina.skills)
    await served.opened()
    assert await served.far.result("session/skills") == {"skills": []}

    put(harness, "review", "Reads a change.")

    listed = await served.far.result("session/skills")
    assert [skill["name"] for skill in listed["skills"]] == ["review"]


async def test_what_a_skill_is_for_is_told_in_no_more_than_a_line_of_some_length(
    served: Served,
) -> None:
    harness = served.harness
    await harness.ctx.plugin(columbina.skills)
    put(harness, "long", "word " * 400)
    await served.opened()

    (skill,) = (await served.far.result("session/skills"))["skills"]
    assert len(skill["description"]) == MAX_DESCRIPTION
    assert skill["description"].endswith("…")


async def test_a_program_without_skills_lists_none(served: Served) -> None:
    await served.opened()

    assert await served.far.result("session/skills") == {"skills": []}


async def test_the_skills_are_those_of_a_session_so_one_has_to_be_open(served: Served) -> None:
    await served.harness.ctx.plugin(columbina.skills)
    await served.greeted()

    assert await served.far.refusal("session/skills") == NO_SESSION_OPEN


async def test_looking_for_skills_does_not_hold_up_what_is_asked_for_after_it(
    served: Served, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness = served.harness
    await harness.ctx.plugin(columbina.skills)
    file = put(harness, "review", "Reads a change.")
    await served.opened("first")
    first = served.agent().session
    looking = threading.Event()
    let_go = threading.Event()
    look = Skills.found

    async def slowly(self: Skills, session: object, cwd: Path) -> Found:
        looking.set()
        await asyncio.to_thread(let_go.wait, 10)
        return await look(self, session, cwd)  # pyright: ignore[reportArgumentType]

    monkeypatch.setattr(Skills, "found", slowly)

    slow = served.far.ask("session/skills")
    await asyncio.to_thread(looking.wait, 10)
    # Another session is opened meanwhile. The answer is of the session that was asked about.
    opened = await served.far.result("session/open", {"session_id": "second"})
    before = served.far.answers(slow)
    let_go.set()
    answer = await served.far.answer(slow)

    assert opened["session_id"] == "second"
    assert before == []
    assert [skill["name"] for skill in answer["result"]["skills"]] == ["review"]
    # The skills were looked for as those of the first session. The second was told of none yet.
    assert may_read(harness, first, file) is True
    assert may_read(harness, served.agent().session, file) is False
