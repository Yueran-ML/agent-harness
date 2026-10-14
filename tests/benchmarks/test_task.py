"""Tasks of the benchmark: ``bench.task``."""

from pathlib import Path

import pytest

from bench.task import TaskError, load, tasks


def written(directory: Path, text: str, **files: str) -> Path:
    directory.mkdir(parents=True)
    (directory / "task.yml").write_text(text, encoding="utf-8")
    for name, content in files.items():
        path = directory / name.replace("__", "/")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return directory


GOOD = "id: code-one\ncategory: code\nprompt: Fix the test.\n"


def test_a_task_is_read_from_its_directory(tmp_path: Path) -> None:
    found = load(written(tmp_path / "code-one", GOOD + "max_steps: 12\n", workspace__a="x"))
    assert found.task.id == "code-one"
    assert found.task.category == "code"
    assert found.task.max_steps == 12
    assert found.task.timeout_seconds == 600
    assert found.directory == tmp_path / "code-one"


def test_the_id_of_a_task_is_the_name_of_its_directory(tmp_path: Path) -> None:
    with pytest.raises(TaskError, match="is in a directory called other"):
        load(written(tmp_path / "other", GOOD, workspace__a="x"))


@pytest.mark.parametrize(
    ("text", "said"),
    [
        ("id: code-one\ncategory: art\nprompt: x\n", "category"),
        ("id: code-one\ncategory: code\n", "prompt"),
        (GOOD + "colour: red\n", "colour"),
        ("- a\n", "is not a mapping"),
        ("id: [\n", "is not YAML"),
    ],
)
def test_a_task_that_cannot_be_read_says_why(tmp_path: Path, text: str, said: str) -> None:
    with pytest.raises(TaskError, match=said):
        load(written(tmp_path / "code-one", text, workspace__a="x"))


def test_a_directory_without_a_task_is_said_to_be_such(tmp_path: Path) -> None:
    (tmp_path / "code-one").mkdir()

    with pytest.raises(TaskError, match=r"task\.yml cannot be read"):
        load(tmp_path / "code-one")


def test_a_task_without_a_workspace_is_refused(tmp_path: Path) -> None:
    with pytest.raises(TaskError, match="has no workspace"):
        load(written(tmp_path / "code-one", GOOD))


def test_the_tasks_of_a_directory_are_found_in_order(tmp_path: Path) -> None:
    written(tmp_path / "b" / "code-two", GOOD.replace("one", "two"), workspace__a="x")
    written(tmp_path / "a" / "code-one", GOOD, workspace__a="x")
    assert [found.task.id for found in tasks(tmp_path)] == ["code-one", "code-two"]
