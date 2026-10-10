"""Skill files: what counts as a skill, and which of two skills of one name is the skill."""

import itertools
import time
from pathlib import Path
from typing import Any

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina import places
from columbina.fs import bound
from columbina.places import RootError
from columbina.skills import files
from columbina.skills.files import (
    FILE,
    MAX_BYTES,
    Found,
    Invalid,
    Root,
    Scan,
    Skill,
    SkillError,
    body,
    merged,
    parsed,
    roots,
    scan,
)
from support.links import link_directory, link_file


@pytest.fixture
def place(tmp_path: Path) -> Path:
    """A directory for a test, by the name that the system itself has for it.

    A place of a project is taken as it lies there, and a name that the system would
    write differently looks like a link on the way to it.
    """
    return tmp_path.resolve()


def written(name: str, description: str = "Do the thing.", text: str = "The way to do it.") -> str:
    return f"---\nname: {name}\ndescription: {description}\n---\n{text}\n"


def skill(root: Path, name: str, content: str | bytes | None = None) -> Path:
    """Put a skill into a root, and return its directory."""
    directory = root / name
    directory.mkdir(parents=True)
    if content is None:
        content = written(name)
    data = content if isinstance(content, bytes) else content.encode()
    (directory / FILE).write_bytes(data)
    return directory


def own(path: Path) -> Root:
    return Root(path, "home", own=True)


def of_a_project(path: Path) -> Root:
    return Root(path, "project", own=False)


def reasons(found: Scan) -> list[tuple[str, str]]:
    return [(invalid.directory.name, invalid.reason) for invalid in found.invalid]


# One file


def test_a_skill_file_says_what_the_skill_is_for_and_holds_its_text() -> None:
    content = written("release-notes", "Write the notes of a release.", "First line.\n\nLast line.")

    assert parsed(content.encode(), "release-notes") == (
        "Write the notes of a release.",
        True,
        True,
        "First line.\n\nLast line.",
    )


def test_a_skill_may_say_who_can_reach_it() -> None:
    content = (
        "---\nname: deploy\ndescription: Deploy.\n"
        "disable-model-invocation: true\nuser-invocable: false\n---\nDo it.\n"
    )

    assert parsed(content.encode(), "deploy") == ("Deploy.", False, False, "Do it.")


def test_keys_that_other_tools_know_are_passed_over() -> None:
    content = (
        "---\nname: deploy\ndescription: Deploy.\nallowed-tools: [Bash]\n"
        "license: MIT\nmetadata:\n  author: somebody\n---\nDo it.\n"
    )

    assert parsed(content.encode(), "deploy") == ("Deploy.", True, True, "Do it.")


def test_keys_that_are_no_text_are_passed_over_too() -> None:
    content = (
        "---\nname: deploy\ndescription: Deploy.\n1: one\nyes: indeed\n2024-01-01: a day\n---\n"
    )

    assert parsed(content.encode(), "deploy") == ("Deploy.", True, True, "")


def test_a_file_without_text_after_its_first_lines_is_a_skill_all_the_same() -> None:
    assert parsed(b"---\nname: deploy\ndescription: Deploy.\n---", "deploy")[3] == ""
    assert parsed(b"---\nname: deploy\ndescription: Deploy.\n---\n\n\n", "deploy")[3] == ""


def test_a_mark_at_the_beginning_of_a_file_and_the_line_ends_of_windows_do_not_count() -> None:
    content = b"\xef\xbb\xbf---\r\nname: deploy\r\ndescription: Deploy.\r\n---\r\nOne.\r\nTwo.\r\n"

    assert parsed(content, "deploy") == ("Deploy.", True, True, "One.\nTwo.")


def test_spaces_after_the_dashes_do_not_count() -> None:
    content = "---  \nname: deploy\ndescription: Deploy.\n--- \t\nDo it.\n"

    assert parsed(content.encode(), "deploy") == ("Deploy.", True, True, "Do it.")


def test_three_dashes_inside_a_value_do_not_end_the_first_lines() -> None:
    content = (
        "---\nname: deploy\ndescription: |\n  First --- half.\n  ---\n  Second half.\n---\n"
        "The text, with\n---\na line of dashes in it.\n"
    )

    description, _, _, text = parsed(content.encode(), "deploy")

    assert description == "First --- half.\n---\nSecond half."
    assert text == "The text, with\n---\na line of dashes in it."


@pytest.mark.parametrize(
    ("content", "why"),
    [
        ("name: deploy\ndescription: Deploy.\n", "does not begin with a line of three dashes"),
        ("\n---\nname: deploy\ndescription: Deploy.\n---\n", "does not begin with a line of"),
        ("---\nname: deploy\ndescription: Deploy.\n", "are not closed by three dashes"),
        ("---\nname: [deploy\n---\n", "are not YAML: "),
        ("---\n- deploy\n---\n", "do not give keys and values"),
        ("---\n---\n", "the key 'name' is missing"),
        ("---\ndescription: Deploy.\n---\n", "the key 'name' is missing"),
        (
            "---\nname: 2024\ndescription: Deploy.\n---\n",
            "the key 'name' gives 2024, which is no text. Put it in quotes",
        ),
        ("---\nname: ''\ndescription: Deploy.\n---\n", "the key 'name' says nothing"),
        ("---\nname:\ndescription: Deploy.\n---\n", "the key 'name' says nothing"),
        ("---\nname: deploy\n---\n", "the key 'description' is missing"),
        ("---\nname: deploy\ndescription: '   '\n---\n", "the key 'description' says nothing"),
        (
            "---\nname: deploy\ndescription: yes\n---\n",
            "the key 'description' gives True, which is no text. Put it in quotes",
        ),
        (
            "---\nname: deploy\ndescription: [Deploy, now]\n---\n",
            "the key 'description' gives a list, which is no text. Put it in quotes",
        ),
        (
            "---\nname: deploy\ndescription: Deploy: now\n---\n",
            "are not YAML: mapping values are not allowed here",
        ),
        (
            "---\nname: deploy\ndescription:\n  Deploy: now\n---\n",
            "the key 'description' gives keys and values of its own, which is no text. Put it",
        ),
        # The reader of YAML makes values of what it reads, and some of them cannot be made.
        # It says so with an error that is not one of its own.
        (
            "---\nname: deploy\ndescription: Deploy.\nreleased: 2024-02-30\n---\n",
            "give a value that cannot be read: day is out of range for month",
        ),
        (
            "---\nname: deploy\ndescription: Deploy.\nflag: !!bool maybe\n---\n",
            "give a value that cannot be read: 'maybe'",
        ),
        pytest.param(
            "---\nname: deploy\ndescription: Deploy.\nbuild: " + "9" * 5000 + "\n---\n",
            "give a value that cannot be read: Exceeds the limit",
            id="a number of thousands of digits",
        ),
        pytest.param(
            "---\nname: deploy\ndescription: Deploy.\nnested: "
            + "[" * 20000
            + "]" * 20000
            + "\n---\n",
            "are nested too deeply",
            id="lists in lists, thousands deep",
        ),
        (
            "---\nname: deploy\ndescription: Deploy.\nuser-invocable: sometimes\n---\n",
            "the key 'user-invocable' takes true or false, not 'sometimes'",
        ),
        (
            "---\nname: deploy\ndescription: Deploy.\ndisable-model-invocation: 1\n---\n",
            "the key 'disable-model-invocation' takes true or false, not 1",
        ),
    ],
)
def test_a_file_that_is_no_skill_file_says_what_is_wrong_with_it(content: str, why: str) -> None:
    with pytest.raises(SkillError, match=why):
        parsed(content.encode(), "deploy")


def test_a_skill_file_that_uses_a_value_again_by_its_name_is_refused() -> None:
    content = b"---\nname: deploy\ndescription: &what Deploy.\nlicense: *what\n---\nDo it.\n"

    with pytest.raises(SkillError) as refused:
        parsed(content, "deploy")

    assert str(refused.value) == (
        "the lines at the beginning of SKILL.md use a value again by its name, which is not "
        "read here. Write the value out where it is used"
    )


def test_a_skill_file_that_would_build_a_value_without_end_is_refused_at_once(
    place: Path,
) -> None:
    # Each key takes in the one before it twice. Built, the last would have 2**40 entries.
    lines = ["a0: &a0 {x: 1}"]
    lines.extend(f"a{n}: &a{n} {{<<: [*a{n - 1}, *a{n - 1}]}}" for n in range(1, 41))
    front = "\n".join(lines)
    # Were a value that is used again built at all, the file would be the end of this
    # run. So a small one is tried first, and it stops the test if it is taken.
    with pytest.raises(SkillError):
        parsed(b"---\nname: small\ndescription: &d Small.\nother: *d\n---\n", "small")
    root = of_a_project(place / "skills")
    skill(root.path, "bomb", f"---\nname: bomb\ndescription: Boom.\n{front}\n---\n")
    skill(root.path, "deploy")
    began = time.monotonic()

    found = scan(root)

    assert time.monotonic() - began < 5
    assert [one.name for one in found.skills] == ["deploy"]
    assert [reason[:60] for _, reason in reasons(found)] == [
        "the lines at the beginning of SKILL.md use a value again by "
    ]


@pytest.mark.parametrize(
    ("slip", "meant"),
    [
        ("disable_model_invocation", "disable-model-invocation"),
        ("disableModelInvocation", "disable-model-invocation"),
        ("model-invocable", "disable-model-invocation"),
        ("model_invocable", "disable-model-invocation"),
        ("modelInvocable", "disable-model-invocation"),
        ("user_invocable", "user-invocable"),
        ("userInvocable", "user-invocable"),
        ("Disable-Model-Invocation", "disable-model-invocation"),
        ("USER_INVOCABLE", "user-invocable"),
        ("User-Invocable", "user-invocable"),
        ("userinvocable", "user-invocable"),
        ("user-invokable", "user-invocable"),
        ("disable-model-invokation", "disable-model-invocation"),
    ],
)
def test_a_key_that_is_nearly_one_that_says_who_can_reach_a_skill_is_not_passed_over(
    slip: str, meant: str
) -> None:
    # Passed over, it would change who can reach the skill without a word.
    content = f"---\nname: deploy\ndescription: Deploy.\n{slip}: true\n---\n"

    with pytest.raises(SkillError) as raised:
        parsed(content.encode(), "deploy")

    assert str(raised.value) == f"the key {slip!r} is not known. What is meant is spelled {meant!r}"


def test_what_a_file_gives_is_never_shown_at_length() -> None:
    # Lists that hold one another take a few lines in a file, and millions of them written out.
    letters = "abcdefg"
    shared = "a: &a [x, x, x, x, x, x, x, x, x]\n" + "".join(
        f"{letter}: &{letter} [{', '.join(['*' + before] * 9)}]\n"
        for before, letter in itertools.pairwise(letters)
    )
    long = "x" * 100_000
    for lines in (
        f"{shared}name: *g\ndescription: Deploy.\n",
        f"{shared}name: deploy\ndescription: Deploy.\nuser-invocable: *g\n",
        f"name: deploy\ndescription: Deploy.\nuser-invocable: {long}\n",
        f"name: {long}\ndescription: Deploy.\n",
        # A key may have a thousand characters, and YAML allows no more.
        f"name: deploy\ndescription: Deploy.\n{'_' * 1000}user_invocable: true\n",
        f"name: deploy\ndescription: Deploy.\nflag: !!bool {long}\n",
        f"name: deploy\ndescription: Deploy.\n{long}\n",
        # The reader of YAML repeats a name that stands for nothing, however long it is.
        f"name: deploy\ndescription: *{long}\n",
    ):
        with pytest.raises(SkillError) as raised:
            parsed(f"---\n{lines}---\n".encode(), "deploy")

        assert len(str(raised.value)) < 400, str(raised.value)[:400]


@pytest.mark.parametrize(
    ("given", "shown"),
    [
        # What is shown of a long value is its beginning.
        pytest.param("x" * 100_000, f"'{'x' * 59}...", id="very long"),
        pytest.param("x" * 59, f"'{'x' * 59}...", id="one character too long"),
        # In its quotes this one is just as long as a value may be shown.
        pytest.param("x" * 58, f"'{'x' * 58}'", id="as long as may be shown"),
    ],
)
def test_a_value_that_is_not_long_is_shown_whole(given: str, shown: str) -> None:
    lines = f"name: deploy\ndescription: Deploy.\nuser-invocable: {given}\n"

    with pytest.raises(SkillError) as raised:
        parsed(f"---\n{lines}---\n".encode(), "deploy")

    assert str(raised.value) == f"the key 'user-invocable' takes true or false, not {shown}"


def test_what_the_reader_of_yaml_says_is_passed_on_in_one_line() -> None:
    with pytest.raises(SkillError) as raised:
        parsed(b"---\nname: deploy\ndescription: Deploy: now\n---\n", "deploy")

    said = str(raised.value)
    assert said.startswith("the lines at the beginning of SKILL.md are not YAML: mapping values")
    # Where in the lines, which the reader says on a line of its own.
    assert "line 2, column 20" in said
    assert "\n" not in said


@pytest.mark.parametrize(
    "name",
    ["Deploy", "deploy_now", "-deploy", "deploy-", "de--ploy", "dé", "de ploy", "a" * 65],
)
def test_a_name_that_cannot_be_used_is_refused(name: str) -> None:
    content = f"---\nname: {name}\ndescription: Deploy.\n---\n"

    with pytest.raises(SkillError, match="cannot be used: a name is made of lower-case letters"):
        parsed(content.encode(), name)


@pytest.mark.parametrize("name", ["a", "deploy", "deploy-2", "2", "a-b-c", "a" * 64])
def test_lower_case_letters_digits_and_single_dashes_make_a_name(name: str) -> None:
    # Digits alone are a number to YAML, so such a name stands in quotes.
    content = f"---\nname: '{name}'\ndescription: Deploy.\n---\n"

    assert parsed(content.encode(), name)[0] == "Deploy."


def test_a_skill_is_called_what_its_directory_is_called() -> None:
    with pytest.raises(SkillError) as raised:
        parsed(written("deploy").encode(), "release")

    assert str(raised.value) == (
        "the file calls the skill 'deploy' and its directory is called 'release'. "
        "The two have to be the same"
    )


def test_a_file_that_is_no_text_is_no_skill_file() -> None:
    with pytest.raises(SkillError, match=r"SKILL\.md is not written in UTF-8"):
        parsed(b"---\nname: deploy\ndescription: \xff\xfe\n---\n", "deploy")


NAMES = st.from_regex(r"[a-z0-9]{1,8}(-[a-z0-9]{1,8}){0,3}", fullmatch=True)
WORDS = st.text(
    alphabet=st.characters(codec="utf-8", categories=("L", "N", "Zs"), exclude_characters=" "),
    min_size=1,
    max_size=40,
)


@given(name=NAMES, words=st.lists(WORDS, min_size=1, max_size=6))
def test_whatever_a_description_says_comes_back_as_it_was_written(
    name: str, words: list[str]
) -> None:
    description = " ".join(words)
    quoted = "'" + description.replace("'", "''") + "'"
    content = f"---\nname: '{name}'\ndescription: {quoted}\n---\nText.\n"

    assert parsed(content.encode(), name) == (description, True, True, "Text.")


# One root


def test_the_directories_of_a_root_that_hold_a_skill_file_are_its_skills(place: Path) -> None:
    root = own(place / "skills")
    skill(root.path, "release-notes", written("release-notes", "Write notes."))
    skill(root.path, "deploy", written("deploy", "Deploy."))
    (root.path / "notes").mkdir()
    (root.path / "notes" / "README.md").write_text("Not a skill.")
    (root.path / "README.md").write_text("Nor is this.")

    found = scan(root)

    assert found.skills == (
        Skill("deploy", "Deploy.", root.path.resolve() / "deploy", root),
        Skill("release-notes", "Write notes.", root.path.resolve() / "release-notes", root),
    )
    assert found.invalid == ()


def test_a_root_that_is_not_there_holds_nothing(place: Path) -> None:
    assert scan(own(place / "nowhere")) == Scan(own(place / "nowhere"))
    assert scan(of_a_project(place / "nowhere")) == Scan(of_a_project(place / "nowhere"))
    (place / "file").write_text("not a directory")
    assert scan(own(place / "file")).skills == ()


def test_only_the_directories_of_a_root_are_looked_into_and_no_deeper(place: Path) -> None:
    root = own(place / "skills")
    skill(root.path / "group", "deploy")
    (root.path / FILE).write_text(written("skills"))

    assert scan(root) == Scan(root)


def test_a_skill_that_cannot_be_used_does_not_hide_the_others(place: Path) -> None:
    root = own(place / "skills")
    skill(root.path, "good")
    skill(root.path, "wrong-name", written("another"))
    skill(root.path, "no-yaml", "just text\n")
    (root.path / "a-directory" / FILE).mkdir(parents=True)
    skill(root.path, "too-long", written("too-long", text="x" * MAX_BYTES))

    found = scan(root)

    assert [one.name for one in found.skills] == ["good"]
    assert reasons(found) == [
        ("a-directory", "SKILL.md is not a file"),
        ("no-yaml", "SKILL.md does not begin with a line of three dashes"),
        ("too-long", f"SKILL.md has more than {MAX_BYTES} bytes"),
        (
            "wrong-name",
            "the file calls the skill 'another' and its directory is called 'wrong-name'. "
            "The two have to be the same",
        ),
    ]


def test_a_skill_file_of_just_the_length_that_is_allowed_is_read(place: Path) -> None:
    root = own(place / "skills")
    head = written("long", text="")
    skill(root.path, "long", head + "x" * (MAX_BYTES - len(head)))

    assert [one.name for one in scan(root).skills] == ["long"]


def test_a_root_that_cannot_be_looked_into_is_an_error_and_not_an_empty_root(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = own(place / "skills")
    skill(root.path, "deploy")

    def fails(path: Path, at_most: int) -> object:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(bound, "listing", fails)

    # Whoever asks may know better than to believe that the skills are gone.
    with pytest.raises(PermissionError):
        scan(root)


@pytest.mark.parametrize(
    ("error", "said"),
    [
        (PermissionError(13, "Permission denied"), "it cannot be read: Permission denied"),
        (OSError("the disk is gone"), "it cannot be read: the disk is gone"),
    ],
)
def test_a_skill_that_cannot_be_read_does_not_hide_the_others(
    place: Path, monkeypatch: pytest.MonkeyPatch, error: OSError, said: str
) -> None:
    root = own(place / "skills")
    skill(root.path, "good")
    locked = skill(root.path, "locked")
    really = bound.open_for_reading

    def opens(path: Path) -> object:
        if path.parent == locked:
            raise error
        return really(path)

    monkeypatch.setattr(bound, "open_for_reading", opens)

    found = scan(root)

    assert [one.name for one in found.skills] == ["good"]
    assert reasons(found) == [("locked", said)]


def test_a_place_of_the_user_that_is_changed_while_it_is_looked_into_is_an_error(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = own(place / "skills")
    skill(root.path, "deploy")

    def elsewhere(path: Path, at_most: int) -> object:
        raise bound.MovedError(f"{path} was somewhere else when it was opened")

    monkeypatch.setattr(bound, "listing", elsewhere)

    # Its links were followed a moment before, so somebody is at work there, and that passes.
    with pytest.raises(bound.MovedError):
        scan(root)


def test_a_place_of_a_project_that_becomes_a_link_while_it_is_looked_into_says_so(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = of_a_project(place / "skills")
    skill(root.path, "deploy")

    def elsewhere(path: Path, at_most: int) -> object:
        # No link was on the way when it was looked at, and one is there when it is opened.
        raise bound.MovedError(f"{path} was somewhere else when it was opened")

    monkeypatch.setattr(bound, "listing", elsewhere)

    assert scan(root) == Scan(
        root,
        invalid=(
            Invalid(
                root.path, root, "it lies behind a link, and links are not followed in a project"
            ),
        ),
    )


def test_a_skill_file_of_the_user_that_is_changed_while_it_is_opened_cannot_be_read(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = own(place / "skills")
    skill(root.path, "deploy")

    def elsewhere(path: Path) -> object:
        raise bound.MovedError(f"{path.name} was somewhere else when it was opened")

    monkeypatch.setattr(bound, "open_for_reading", elsewhere)

    # No link is to blame for it here, where links are followed.
    assert reasons(scan(root)) == [
        ("deploy", "it cannot be read: SKILL.md was somewhere else when it was opened")
    ]


def test_a_root_with_very_many_entries_is_looked_at_as_far_as_that_goes(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(files, "_MAX_ENTRIES", 3)
    root = own(place / "skills")
    for name in ("a", "b", "c", "d"):
        skill(root.path, name)

    found = scan(root)

    assert len(found.skills) == 3
    assert reasons(found) == [
        ("skills", "it holds more than 3 entries, and the rest is not looked at")
    ]


# Links


def test_links_in_a_place_of_the_user_are_followed(place: Path) -> None:
    elsewhere = place / "elsewhere"
    working_copy = skill(elsewhere, "deploy")
    root = own(place / "home" / "skills")
    root.path.mkdir(parents=True)
    link_directory(root.path / "deploy", working_copy)

    found = scan(root)

    # The skill lies where the link leads, and that is where its files are read.
    assert found.skills == (Skill("deploy", "Do the thing.", working_copy.resolve(), root),)


def test_a_place_of_the_user_may_itself_be_a_link(place: Path) -> None:
    real = place / "real"
    skill(real, "deploy")
    link_directory(place / "skills", real)
    root = own(place / "skills")

    assert scan(root).skills == (Skill("deploy", "Do the thing.", real.resolve() / "deploy", root),)


def test_a_link_in_a_place_of_the_user_is_called_by_the_name_of_the_link(place: Path) -> None:
    # The directory that the link leads to has another name than the skill.
    working_copy = place / "elsewhere" / "deploy-v2"
    working_copy.mkdir(parents=True)
    (working_copy / FILE).write_text(written("deploy"))
    root = own(place / "skills")
    root.path.mkdir()
    link_directory(root.path / "deploy", working_copy)

    found = scan(root)

    assert found.skills == (Skill("deploy", "Do the thing.", working_copy, root),)
    # It is read anew where it lies, and still goes by the name that it has in its place.
    assert body(found.skills[0])[0].name == "deploy"


def test_a_link_to_a_file_in_a_place_of_the_user_is_no_skill(place: Path) -> None:
    notes = place / "notes.md"
    notes.write_text("Not a skill.")
    root = own(place / "skills")
    skill(root.path, "deploy")
    link_file(root.path / "notes", notes)

    assert scan(root) == Scan(root, (Skill("deploy", "Do the thing.", root.path / "deploy", root),))


def test_a_link_that_leads_nowhere_is_said_to_do_so(place: Path) -> None:
    gone = place / "gone"
    gone.mkdir()
    root = own(place / "skills")
    root.path.mkdir()
    link_directory(root.path / "deploy", gone)
    gone.rmdir()

    assert reasons(scan(root)) == [("deploy", "it is a link that leads nowhere")]


def test_links_that_go_in_a_circle_do_not_hide_the_skills_beside_them(place: Path) -> None:
    root = own(place / "skills")
    skill(root.path, "good")
    one, other = root.path / "one", root.path / "other"
    other.mkdir()
    link_directory(one, other)
    other.rmdir()
    link_directory(other, one)

    found = scan(root)

    assert [each.name for each in found.skills] == ["good"]
    assert [name for name, _ in reasons(found)] == ["one", "other"]
    assert all(reason.startswith("it cannot be read: ") for _, reason in reasons(found))
    # A place that is itself such a link cannot be looked into, like one that may not be.
    with pytest.raises(OSError, match=r".") as raised:
        scan(own(one))
    assert not isinstance(raised.value, FileNotFoundError)


def test_a_link_in_a_project_is_not_followed(place: Path) -> None:
    elsewhere = skill(place / "elsewhere", "deploy")
    root = of_a_project(place / "project" / ".columbina" / "skills")
    skill(root.path, "honest")
    link_directory(root.path / "deploy", elsewhere)

    found = scan(root)

    assert [one.name for one in found.skills] == ["honest"]
    assert reasons(found) == [
        ("deploy", "it lies behind a link, and links are not followed in a project")
    ]


def test_a_project_whose_place_for_skills_is_a_link_has_no_skills(place: Path) -> None:
    real = place / "elsewhere"
    skill(real, "deploy")
    project = place / "project" / ".columbina"
    project.mkdir(parents=True)
    link_directory(project / "skills", real)

    found = scan(of_a_project(project / "skills"))

    assert found.skills == ()
    assert [invalid.reason for invalid in found.invalid] == [
        "it lies behind a link, and links are not followed in a project"
    ]


@pytest.mark.parametrize("linked", [".columbina", "skills"])
def test_nobody_goes_where_a_link_in_a_project_leads(
    place: Path, monkeypatch: pytest.MonkeyPatch, linked: str
) -> None:
    real = place / "elsewhere"
    skill(real / "skills", "deploy")
    project = place / "project"
    project.mkdir()
    if linked == ".columbina":
        link_directory(project / ".columbina", real)
    else:
        (project / ".columbina").mkdir()
        link_directory(project / ".columbina" / "skills", real / "skills")
    gone_to: list[Path] = []
    listing = bound.listing
    opening = bound.open_for_reading

    def lists(path: Path, at_most: int) -> Any:
        gone_to.append(path)
        return listing(path, at_most)

    def opens(path: Path) -> Any:
        gone_to.append(path)
        return opening(path)

    monkeypatch.setattr(bound, "listing", lists)
    monkeypatch.setattr(bound, "open_for_reading", opens)
    root = of_a_project(project / ".columbina" / "skills")

    found = scan(root)

    # A link may lead to another machine, and going there already tells it who asks. So the
    # way is looked at first, and nothing on it is opened.
    assert gone_to == []
    assert found == Scan(
        root,
        invalid=(
            Invalid(
                root.path, root, "it lies behind a link, and links are not followed in a project"
            ),
        ),
    )


def test_a_file_of_a_project_behind_a_link_is_not_opened(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    kept = place / "elsewhere"
    skill(kept, "deploy")
    root = of_a_project(place / "skills")
    root.path.mkdir()
    link_directory(root.path / "door", kept / "deploy")
    gone_to: list[Path] = []
    opening = bound.open_for_reading

    def opens(path: Path) -> Any:
        gone_to.append(path)
        return opening(path)

    monkeypatch.setattr(bound, "open_for_reading", opens)

    # The directory that the file is asked for in turns out to be a link.
    with pytest.raises(RootError, match="it lies behind a link"):
        places.opened(root, root.path / "door" / FILE)

    assert gone_to == []
    # In a place of the user's own the same file is read where the link leads.
    opened = places.opened(own(root.path), root.path / "door" / FILE)
    assert opened is not None
    with opened:
        assert opened.read().startswith(b"---\nname: deploy\n")


def test_a_skill_file_of_a_project_that_is_a_link_is_not_read(place: Path) -> None:
    secret = place / "secret.md"
    secret.write_text(written("deploy", "Nobody's business."))
    root = of_a_project(place / "project" / ".columbina" / "skills")
    (root.path / "deploy").mkdir(parents=True)
    link_file(root.path / "deploy" / FILE, secret)

    found = scan(root)

    assert found.skills == ()
    assert reasons(found) == [
        ("deploy", "it lies behind a link, and links are not followed in a project")
    ]


def test_a_skill_file_that_the_system_finds_elsewhere_than_it_was_looked_for_is_not_read(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # This is what a link does where a user may make one. Here it is told to the code by
    # hand, so that the test runs on every system.
    root = of_a_project(place / "skills")
    skill(root.path, "deploy")

    def elsewhere(path: Path) -> object:
        raise bound.MovedError(f"{path} was somewhere else when it was opened")

    monkeypatch.setattr(bound, "open_for_reading", elsewhere)

    found = scan(root)

    assert found.skills == ()
    assert reasons(found) == [
        ("deploy", "it lies behind a link, and links are not followed in a project")
    ]


def test_a_skill_file_of_the_user_that_is_a_link_is_read_where_it_leads(place: Path) -> None:
    shared = place / "shared.md"
    shared.write_text(written("deploy", "Shared."))
    root = own(place / "skills")
    (root.path / "deploy").mkdir(parents=True)
    link_file(root.path / "deploy" / FILE, shared)

    assert [one.description for one in scan(root).skills] == ["Shared."]


# Several roots


def test_of_two_skills_of_one_name_the_one_whose_root_comes_first_is_the_skill(
    place: Path,
) -> None:
    project, home = of_a_project(place / "project"), own(place / "home")
    skill(project.path, "deploy", written("deploy", "The way of this project."))
    skill(project.path, "zip", written("zip", "Zip."))
    skill(home.path, "deploy", written("deploy", "The way of the user."))
    skill(home.path, "notes", written("notes", "Notes."))
    skill(home.path, "broken", "no dashes\n")

    found = merged([scan(project), scan(home)])

    assert {name: one.description for name, one in found.skills.items()} == {
        "deploy": "The way of this project.",
        "notes": "Notes.",
        "zip": "Zip.",
    }
    # They come by name, whatever root each was found in.
    assert list(found.skills) == ["deploy", "notes", "zip"]
    assert [(one.name, one.root.label) for one in found.shadowed] == [("deploy", "home")]
    assert [invalid.directory.name for invalid in found.invalid] == ["broken"]
    # With the roots the other way round, the other one of the two is the skill.
    other = merged([scan(home), scan(project)])
    assert other.skills["deploy"].description == "The way of the user."
    assert [(one.name, one.root.label) for one in other.shadowed] == [("deploy", "project")]
    assert merged([]) == Found({})


def test_the_places_where_skills_are_looked_for_come_in_a_fixed_order(place: Path) -> None:
    project, home, user = place / "project", place / "home", place / "user"
    extra = [place / "one", place / "two"]

    assert roots(project, home, user, extra) == (
        Root(project / ".columbina" / "skills", "project", own=False),
        Root(project / ".agents" / "skills", "project, .agents", own=False),
        Root(home, "home", own=True),
        Root(user, "user, .agents", own=True),
        Root(extra[0], "extra", own=True),
        Root(extra[1], "extra", own=True),
    )
    # A place that there is none of is left out, and the others keep their order.
    assert [root.label for root in roots(project, None, None, [])] == [
        "project",
        "project, .agents",
    ]


# Reading a skill anew


def test_the_text_of_a_skill_is_read_when_it_is_asked_for(place: Path) -> None:
    root = own(place / "skills")
    directory = skill(root.path, "deploy", written("deploy", text="The old way."))
    (found,) = scan(root).skills
    (directory / FILE).write_text(
        "---\nname: deploy\ndescription: Deploy again.\nuser-invocable: false\n---\nThe new way.\n"
    )

    now, text = body(found)

    assert text == "The new way."
    assert (now.description, now.user_invocable) == ("Deploy again.", False)


def test_a_skill_of_the_user_is_read_anew_wherever_the_links_lead_by_then(place: Path) -> None:
    working_copy = skill(place / "elsewhere", "deploy", written("deploy", text="The way."))
    root = own(place / "skills")
    root.path.mkdir()
    link_directory(root.path / "deploy", working_copy)
    # The skill as somebody knows it who never followed the link.
    known = Skill("deploy", "Do the thing.", root.path / "deploy", root)

    assert body(known) == (known, "The way.")


def test_a_skill_that_is_gone_or_broken_when_it_is_asked_for_says_so(place: Path) -> None:
    root = own(place / "skills")
    directory = skill(root.path, "deploy")
    (found,) = scan(root).skills

    (directory / FILE).write_text("no dashes\n")
    with pytest.raises(SkillError, match="does not begin with a line of three dashes"):
        body(found)

    (directory / FILE).unlink()
    with pytest.raises(SkillError, match=r"SKILL\.md is no longer there"):
        body(found)


def test_an_invalid_entry_says_where_it_lies(place: Path) -> None:
    root = own(place / "skills")
    directory = skill(root.path, "broken", "no dashes\n")

    assert scan(root).invalid == (
        Invalid(directory.resolve(), root, "SKILL.md does not begin with a line of three dashes"),
    )
