"""Where the hook files lie, and what is read from them."""

import os
from pathlib import Path
from typing import Any

import pytest

from columbina.fs import bound
from columbina.hooks.config import MAX_FILE_BYTES
from columbina.hooks.files import (
    PROJECT_FILE,
    Found,
    file_in,
    of_the_project,
    of_the_user,
    project_of,
    record_at,
)
from support.links import link_directory, link_file

CONTENT = b'{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "pytest -q"}]}]}}'


@pytest.fixture
def project(tmp_path: Path) -> Path:
    """The root of a project, with every link on the way to it followed."""
    root = Path(os.path.realpath(tmp_path)) / "project"
    (root / ".git").mkdir(parents=True)
    return root


@pytest.fixture
def mine(tmp_path: Path) -> Path:
    """Where the hook file of the user lies. The directory is there, and the file is not."""
    home = tmp_path / "home"
    home.mkdir()
    return home / "hooks.json"


# Where the file of a project lies


def test_the_hook_file_of_a_project_lies_at_its_root_however_deep_the_work_is(
    project: Path,
) -> None:
    deep = project / "src" / "package"
    deep.mkdir(parents=True)

    assert project_of(deep) == project
    assert project_of(project) == project
    assert file_in(project) == project / ".columbina" / "hooks.json"
    assert PROJECT_FILE == ".columbina/hooks.json"


def test_a_directory_in_no_repository_is_a_project_of_its_own(tmp_path: Path) -> None:
    alone = Path(os.path.realpath(tmp_path)) / "alone"
    alone.mkdir()

    assert project_of(alone) == alone


def test_a_project_has_one_root_by_whatever_way_it_was_entered(
    project: Path, tmp_path: Path
) -> None:
    (project / "src").mkdir()
    way = tmp_path / "way"
    link_directory(way, project)

    # The record of trust goes by the place, so the place must not depend on the way.
    assert project_of(way) == project
    assert project_of(way / "src") == project


# What is read


def test_a_hook_file_is_read_as_it_lies_there(project: Path, mine: Path) -> None:
    theirs = file_in(project)
    theirs.parent.mkdir()
    mine.write_bytes(CONTENT)
    theirs.write_bytes(b'{"hooks": {}}')

    assert of_the_user(mine) == Found(mine, CONTENT)
    assert of_the_project(project) == Found(theirs, b'{"hooks": {}}')


def test_a_hook_file_that_is_not_there_is_nothing_to_speak_of(project: Path, mine: Path) -> None:
    assert of_the_user(mine) == Found(mine)
    assert of_the_project(project) == Found(file_in(project))

    # Neither is it when a file lies where its directory would be.
    (project / ".columbina").write_text("no directory", encoding="utf-8")

    assert of_the_project(project) == Found(file_in(project))


def test_no_more_of_a_file_is_read_than_tells_that_it_is_too_large(
    project: Path, mine: Path
) -> None:
    theirs = file_in(project)
    theirs.parent.mkdir()
    allowed = b" " * MAX_FILE_BYTES
    for file in (mine, theirs):
        file.write_bytes(allowed + b"  ")

    assert of_the_user(mine).content == allowed + b" "
    assert of_the_project(project).content == allowed + b" "

    # A file of just the size that is allowed is read whole.
    for file in (mine, theirs):
        file.write_bytes(allowed)

    assert of_the_user(mine).content == allowed
    assert of_the_project(project).content == allowed


def test_what_lies_there_and_cannot_be_read_is_said_with_the_reason(
    project: Path, mine: Path
) -> None:
    # A directory lies where each of the two files should.
    mine.mkdir()
    file_in(project).mkdir(parents=True)

    of_me = of_the_user(mine)
    of_them = of_the_project(project)

    assert (of_me.path, of_me.content) == (mine, None)
    assert of_me.problem
    assert (of_them.path, of_them.content) == (file_in(project), None)
    assert of_them.problem is not None
    assert of_them.problem.startswith(f"{file_in(project)} is ")


# Links


def test_a_link_in_the_place_of_the_directory_of_the_project_is_not_followed(
    project: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    elsewhere = Path(os.path.realpath(tmp_path)) / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "hooks.json").write_bytes(CONTENT)
    link_directory(project / ".columbina", elsewhere)

    gone_to: list[Path] = []
    opening = bound.open_for_reading

    def opens(path: Path) -> Any:
        gone_to.append(path)
        return opening(path)

    monkeypatch.setattr(bound, "open_for_reading", opens)

    found = of_the_project(project)

    # What a repository brings may lead anywhere. Only a file that lies in it is its hook file.
    assert found == Found(
        file_in(project), problem="it lies behind a link, and links are not followed in a project"
    )
    # A link may lead to another machine, and going there already tells it who asks. So the
    # way is looked at first, and nothing on it is opened.
    assert gone_to == []


def test_a_link_that_takes_the_place_of_the_directory_after_it_was_looked_at_is_not_followed(
    project: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    elsewhere = Path(os.path.realpath(tmp_path)) / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "hooks.json").write_bytes(CONTENT)
    link_directory(project / ".columbina", elsewhere)

    def nothing(path: Path) -> Path | None:
        # No link was on the way when it was looked at. One is there when the file is opened.
        return None

    monkeypatch.setattr(bound, "link_on_the_way", nothing)

    found = of_the_project(project)

    # The file that was opened is asked where it lies, and it lies somewhere else.
    assert found == Found(
        file_in(project), problem=f"{file_in(project)} was somewhere else when it was opened"
    )


def test_a_link_in_the_place_of_the_hook_file_of_a_project_is_not_followed(
    project: Path, tmp_path: Path
) -> None:
    elsewhere = tmp_path / "elsewhere.json"
    elsewhere.write_bytes(CONTENT)
    file_in(project).parent.mkdir()
    link_file(file_in(project), elsewhere)

    found = of_the_project(project)

    assert found.content is None
    assert found.problem


def test_the_hook_file_of_the_user_may_lie_behind_a_link(tmp_path: Path) -> None:
    kept = tmp_path / "dotfiles"
    kept.mkdir()
    (kept / "hooks.json").write_bytes(CONTENT)
    home = tmp_path / "home"
    link_directory(home, kept)

    # The home of the user is the business of the user, and so is how it is laid out.
    assert of_the_user(home / "hooks.json") == Found(home / "hooks.json", CONTENT)


# The record of trust


def test_the_record_of_trust_is_read_if_there_is_one(tmp_path: Path) -> None:
    record = tmp_path / "trusted-hooks.json"

    assert record_at(None) is None
    assert record_at(record) is None

    record.write_bytes(b"whatever it holds")

    assert record_at(record) == b"whatever it holds"


def test_a_record_of_trust_that_cannot_be_read_is_none(tmp_path: Path) -> None:
    record = tmp_path / "trusted-hooks.json"
    record.mkdir()

    assert record_at(record) is None
