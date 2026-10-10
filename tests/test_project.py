"""Where a project begins: the nearest directory, upwards, that holds a repository."""

from pathlib import Path

from columbina.project import root_of, way_down
from support.links import link_directory


def test_the_root_of_a_project_is_the_nearest_directory_above_that_holds_a_repository(
    tmp_path: Path,
) -> None:
    outer = tmp_path / "outer"
    inner = outer / "packages" / "inner"
    deep = inner / "src" / "module"
    deep.mkdir(parents=True)
    (outer / ".git").mkdir()
    (inner / ".git").mkdir()

    assert root_of(deep) == inner
    assert way_down(deep) == [inner, inner / "src", deep]
    # A directory that holds the repository itself is the root, and the whole of the way.
    assert way_down(inner) == [inner]
    assert root_of(outer / "packages") == outer


def test_a_work_tree_that_was_added_to_a_repository_is_a_project_too(tmp_path: Path) -> None:
    tree = tmp_path / "tree"
    (tree / "src").mkdir(parents=True)
    # Such a tree holds a file of that name, which says where the repository is.
    (tree / ".git").write_text("gitdir: /somewhere/else\n")

    assert root_of(tree / "src") == tree


def test_a_directory_in_no_repository_is_a_project_by_itself(tmp_path: Path) -> None:
    here = tmp_path / "a" / "b"
    here.mkdir(parents=True)

    assert way_down(here) == [here]
    assert root_of(here) == here


def test_what_marks_a_repository_counts_as_it_lies_there_and_is_not_followed(
    tmp_path: Path,
) -> None:
    project = tmp_path / "project"
    (project / "src").mkdir(parents=True)
    gone = tmp_path / "gone"
    gone.mkdir()
    link_directory(project / ".git", gone)
    gone.rmdir()

    # The link leads nowhere any more. Something of that name lies there all the same.
    assert root_of(project / "src") == project
