"""Kinds of subagent: what counts as one, and which of two kinds of one name is the kind."""

from pathlib import Path
from typing import Any

import pytest

from columbina import places
from columbina.fs import bound
from columbina.places import Root
from columbina.subagents import types
from columbina.subagents.types import (
    MAX_BYTES,
    SUFFIX,
    Found,
    Invalid,
    Scan,
    SubagentType,
    SubagentTypeError,
    merged,
    parsed,
    roots,
    scan,
    usable,
)
from support.links import link_directory, link_file

KEYS = "The keys of a kind of subagent are: name, description, tools, max_steps, model"


@pytest.fixture
def place(tmp_path: Path) -> Path:
    """A directory for a test, by the name that the system itself has for it.

    A place of a project is taken as it lies there, and a name that the system would
    write differently looks like a link on the way to it.
    """
    return tmp_path.resolve()


def own(path: Path) -> Root:
    return Root(path, "home", own=True)


def of_a_project(path: Path) -> Root:
    return Root(path, "project", own=False)


HERE = Path("agents") / "reviewer.md"
ROOT = of_a_project(Path("agents"))


def written(name: str = "reviewer", more: str = "", text: str = "You review changes.") -> str:
    return f"---\nname: {name}\ndescription: Reviews a change.\n{more}---\n{text}\n"


def read(content: str | bytes, name: str = "reviewer") -> SubagentType:
    data = content if isinstance(content, bytes) else content.encode()
    return parsed(data, name, HERE, ROOT)


def kind(root: Path, name: str, content: str | bytes | None = None) -> Path:
    """Put a kind of subagent into a root, and return its file."""
    root.mkdir(parents=True, exist_ok=True)
    file = root / (name + SUFFIX)
    if content is None:
        content = written(name)
    file.write_bytes(content if isinstance(content, bytes) else content.encode())
    return file


def reasons(found: Scan) -> list[tuple[str, str]]:
    return [(invalid.file.name, invalid.reason) for invalid in found.invalid]


def names(found: Scan) -> list[str]:
    return [one.name for one in found.types]


# One file


def test_a_file_says_what_the_kind_is_for_and_what_a_subagent_of_it_works_with() -> None:
    content = (
        "---\n"
        "name: reviewer\n"
        "description: Reads a change and says what is wrong with it.\n"
        "tools: [read, glob, grep]\n"
        "max_steps: 30\n"
        "model: deepseek-v4-pro\n"
        "---\n"
        "\n"
        "You review a change.\n"
        "\n"
        "Change nothing.\n"
        "\n"
    )

    assert read(content) == SubagentType(
        name="reviewer",
        description="Reads a change and says what is wrong with it.",
        prompt="You review a change.\n\nChange nothing.",
        tools=("read", "glob", "grep"),
        max_steps=30,
        model="deepseek-v4-pro",
        file=HERE,
        root=ROOT,
    )


def test_a_file_may_leave_everything_open_but_its_name_and_what_it_is_for() -> None:
    assert read("---\nname: helper\ndescription: Helps.\n---", "helper") == SubagentType(
        "helper", "Helps.", None, None, None, None, HERE, ROOT
    )


@pytest.mark.parametrize("text", ["", "\n\n", "   ", " \n\t\n"])
def test_a_file_without_text_after_its_first_lines_gives_no_system_prompt(text: str) -> None:
    assert read(written(text=text)).prompt is None


def test_the_names_of_the_tools_may_stand_in_one_line_or_one_under_the_other() -> None:
    assert read(written(more="tools: [read, grep]\n")).tools == ("read", "grep")
    assert read(written(more="tools:\n  - read\n  - mcp__github__search\n")).tools == (
        "read",
        "mcp__github__search",
    )
    # A name that stands twice counts once, where it stood first.
    assert read(written(more="tools: [grep, read, grep]\n")).tools == ("grep", "read")


def test_an_empty_list_of_tools_means_none_and_no_list_means_all() -> None:
    assert read(written(more="tools: []\n")).tools == ()
    assert read(written()).tools is None


@pytest.mark.parametrize(
    ("given", "why"),
    [
        ("read, grep", "the key 'tools' gives 'read, grep', which is no list of the names"),
        ("read", "the key 'tools' gives 'read', which is no list of the names"),
        ("3", "the key 'tools' gives 3, which is no list of the names"),
        ("", "the key 'tools' gives None, which is no list of the names"),
        ("{read: true}", "the key 'tools' gives keys and values of its own, which is no list"),
        ("[read, 3]", "the key 'tools' holds 3, which is no name of a tool"),
        ("[read, [grep]]", "the key 'tools' holds a list, which is no name of a tool"),
        ("[read, '']", "the key 'tools' holds '', which is no name of a tool"),
        ("[read, '  ']", "the key 'tools' holds '  ', which is no name of a tool"),
        ("[read, null]", "the key 'tools' holds None, which is no name of a tool"),
        ("[yes]", "the key 'tools' holds True, which is no name of a tool"),
    ],
)
def test_tools_that_are_no_list_of_names_make_a_file_unusable(given: str, why: str) -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more=f"tools: {given}\n"))

    assert str(refused.value).startswith(why)


def test_whoever_writes_the_tools_wrongly_is_told_how_they_are_written() -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more="tools: read, grep\n"))

    assert str(refused.value) == (
        "the key 'tools' gives 'read, grep', which is no list of the names of tools. Write it "
        "like [read, grep], or leave the key out for all tools"
    )


@pytest.mark.parametrize(("given", "steps"), [("1", 1), ("30", 30), ("100000", 100000)])
def test_a_kind_may_say_how_many_steps_a_subagent_of_it_takes(given: str, steps: int) -> None:
    assert read(written(more=f"max_steps: {given}\n")).max_steps == steps


@pytest.mark.parametrize(
    ("given", "shown"),
    [
        ("0", "0"),
        ("-1", "-1"),
        ("1.5", "1.5"),
        ("'30'", "'30'"),
        ("true", "True"),
        ("", "None"),
        ("[30]", "a list"),
    ],
)
def test_steps_that_are_no_whole_number_above_zero_make_a_file_unusable(
    given: str, shown: str
) -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more=f"max_steps: {given}\n"))

    assert str(refused.value) == (
        f"the key 'max_steps' gives {shown}, which is no whole number above zero"
    )


def test_a_kind_may_name_the_model_that_a_subagent_of_it_asks() -> None:
    assert read(written(more="model: deepseek-v4-flash\n")).model == "deepseek-v4-flash"
    assert read(written()).model is None


@pytest.mark.parametrize(
    ("given", "why"),
    [
        ("", "the key 'model' says nothing"),
        ("'  '", "the key 'model' says nothing"),
        ("4", "the key 'model' gives 4, which is no text. Put it in quotes"),
        ("[a, b]", "the key 'model' gives a list, which is no text. Put it in quotes"),
    ],
)
def test_a_model_that_is_no_name_makes_a_file_unusable(given: str, why: str) -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more=f"model: {given}\n"))

    assert str(refused.value) == why


@pytest.mark.parametrize(
    ("key", "shown"),
    [
        ("color", "'color'"),
        ("permissionMode", "'permissionMode'"),
        ("license", "'license'"),
        ("1", "1"),
        ("yes", "True"),
        ("x" * 80, "'" + "x" * 59 + "..."),
    ],
)
def test_a_key_that_is_not_known_makes_a_file_unusable(key: str, shown: str) -> None:
    # Passed over, a key that was meant to keep a subagent small would leave it large.
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more=f"{key}: something\n"))

    assert str(refused.value) == f"the key {shown} is not known. {KEYS}"


@pytest.mark.parametrize(
    ("key", "meant"),
    [
        ("tool", "tools"),
        ("Tools", "tools"),
        ("TOOLS", "tools"),
        ("allowed-tools", "tools"),
        ("allowed_tools", "tools"),
        ("allowedTools", "tools"),
        ("max-steps", "max_steps"),
        ("maxSteps", "max_steps"),
        ("MAX_STEPS", "max_steps"),
        ("max_turns", "max_steps"),
        ("maxTurns", "max_steps"),
        ("Model", "model"),
        ("Description", "description"),
        ("Name", "name"),
    ],
)
def test_a_key_that_is_nearly_one_of_them_says_which_one_was_meant(key: str, meant: str) -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written(more=f"{key}: [read]\n"))

    assert str(refused.value) == (
        f"the key {key!r} is not known. What is meant is spelled {meant!r}"
    )


def test_a_kind_is_called_what_its_file_is_called() -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(written("reviewer"), "review")

    assert str(refused.value) == (
        "the file calls the kind 'reviewer', and by its own name it would be 'review'. The "
        "two have to be the same"
    )


@pytest.mark.parametrize(
    ("content", "why"),
    [
        ("name: reviewer\n", "reviewer.md does not begin with a line of three dashes"),
        (
            "---\nname: reviewer\n",
            "the lines at the beginning of reviewer.md are not closed by three dashes",
        ),
        ("---\n- a\n- b\n---\n", "the lines at the beginning of reviewer.md do not give keys"),
        ("---\n---\n", "the key 'name' is missing"),
        ("---\nname: reviewer\n---\n", "the key 'description' is missing"),
        ("---\nname: reviewer\ndescription:\n---\n", "the key 'description' says nothing"),
        ("---\nname: Reviewer\ndescription: x\n---\n", "the name 'Reviewer' cannot be used: "),
        ("---\nname: 7\ndescription: x\n---\n", "the key 'name' gives 7, which is no text."),
        (
            "---\nname: [\ndescription: x\n---\n",
            "the lines at the beginning of reviewer.md are not YAML: ",
        ),
        (
            "---\nname: reviewer\ndescription: x\ntools: &few [read]\nmodel: *few\n---\n",
            "the lines at the beginning of reviewer.md use a value again by its name, which is "
            "not read here. Write the value out where it is used",
        ),
    ],
)
def test_a_file_that_is_no_file_of_a_kind_says_what_is_wrong_with_it(
    content: str, why: str
) -> None:
    with pytest.raises(SubagentTypeError) as refused:
        read(content)

    assert str(refused.value).startswith(why)
    assert "\n" not in str(refused.value)


def test_a_file_that_is_no_text_is_no_file_of_a_kind() -> None:
    with pytest.raises(SubagentTypeError, match=r"^reviewer\.md is not written in UTF-8$"):
        read(b"---\nname: reviewer\ndescription: \xff\xfe\n---\n")


def test_a_mark_at_the_beginning_of_a_file_and_the_line_ends_of_windows_do_not_count() -> None:
    content = (
        b"\xef\xbb\xbf---\r\nname: reviewer\r\ndescription: Reviews.\r\n---\r\nLook.\r\nSay.\r\n"
    )

    found = read(content)

    assert (found.description, found.prompt) == ("Reviews.", "Look.\nSay.")


# One root


def test_the_files_of_a_root_that_end_like_markdown_are_its_kinds(place: Path) -> None:
    root = own(place / "agents")
    kind(root.path, "reviewer")
    kind(root.path, "explorer", written("explorer", "tools: [read]\n", ""))
    # What else lies there is nobody's kind of subagent.
    (root.path / "notes.txt").write_text(written("notes"), encoding="utf-8")
    (root.path / "tester.MD").write_text(written("tester"), encoding="utf-8")
    (root.path / "folder.md").mkdir()
    (root.path / ".hidden.md").write_text("an editor left this", encoding="utf-8")

    found = scan(root)

    assert found == Scan(
        root,
        (
            SubagentType(
                "explorer",
                "Reviews a change.",
                None,
                ("read",),
                None,
                None,
                root.path / "explorer.md",
                root,
            ),
            SubagentType(
                "reviewer",
                "Reviews a change.",
                "You review changes.",
                None,
                None,
                None,
                root.path / "reviewer.md",
                root,
            ),
        ),
    )


def test_a_root_that_is_not_there_holds_nothing(place: Path) -> None:
    for root in (own(place / "nowhere"), of_a_project(place / "nowhere")):
        assert scan(root) == Scan(root)

    (place / "a-file").write_text("no directory", encoding="utf-8")
    for root in (own(place / "a-file"), of_a_project(place / "a-file")):
        assert scan(root) == Scan(root)


def test_only_the_files_of_a_root_are_looked_at_and_nothing_deeper(place: Path) -> None:
    root = of_a_project(place / "agents")
    kind(root.path / "team", "reviewer")

    assert scan(root) == Scan(root)


def test_a_kind_that_cannot_be_used_does_not_hide_the_others(place: Path) -> None:
    root = of_a_project(place / "agents")
    kind(root.path, "good")
    kind(root.path, "no-dashes", "name: no-dashes\n")
    kind(root.path, "other-name", written("another"))
    kind(root.path, "too-long", written("too-long", text="x" * MAX_BYTES))
    kind(root.path, "wide", written("wide", "tool: [read]\n"))

    found = scan(root)

    assert names(found) == ["good"]
    assert reasons(found) == [
        ("no-dashes.md", "no-dashes.md does not begin with a line of three dashes"),
        (
            "other-name.md",
            "the file calls the kind 'another', and by its own name it would be 'other-name'. "
            "The two have to be the same",
        ),
        ("too-long.md", f"it has more than {MAX_BYTES} bytes"),
        ("wide.md", "the key 'tool' is not known. What is meant is spelled 'tools'"),
    ]


def test_a_file_of_just_the_length_that_is_allowed_is_read(place: Path) -> None:
    root = of_a_project(place / "agents")
    head = written("long", text="")
    kind(root.path, "long", head + "x" * (MAX_BYTES - len(head)))

    found = scan(root)

    assert names(found) == ["long"]
    assert found.invalid == ()


def test_a_root_that_cannot_be_looked_into_is_an_error_and_not_an_empty_root(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = own(place / "agents")
    kind(root.path, "reviewer")

    def refuses(path: Path, at_most: int) -> Any:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(bound, "listing", refuses)

    # Whoever asks may have been told of kinds before, and decides what to go by meanwhile.
    with pytest.raises(PermissionError):
        scan(root)


def test_a_file_that_cannot_be_read_does_not_hide_the_others(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = own(place / "agents")
    kind(root.path, "locked")
    kind(root.path, "reviewer")
    opening = bound.open_for_reading

    def refuses(path: Path) -> Any:
        if path.name == "locked.md":
            raise PermissionError(13, "Permission denied")
        return opening(path)

    monkeypatch.setattr(bound, "open_for_reading", refuses)

    found = scan(root)

    assert names(found) == ["reviewer"]
    assert reasons(found) == [("locked.md", "it cannot be read: Permission denied")]


def test_a_file_that_is_gone_when_it_is_opened_is_nothing_to_speak_of(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = of_a_project(place / "agents")
    kind(root.path, "gone")
    kind(root.path, "reviewer")
    opening = bound.open_for_reading

    def misses(path: Path) -> Any:
        if path.name == "gone.md":
            raise FileNotFoundError(2, "No such file or directory")
        return opening(path)

    monkeypatch.setattr(bound, "open_for_reading", misses)

    found = scan(root)

    assert names(found) == ["reviewer"]
    assert found.invalid == ()


def test_a_root_with_very_many_entries_is_looked_at_as_far_as_that_goes(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(types, "_MAX_ENTRIES", 2)
    root = of_a_project(place / "agents")
    for name in ("a", "b", "c"):
        kind(root.path, name)

    found = scan(root)

    assert len(found.types) == 2
    assert found.invalid == (
        Invalid(root.path, root, "it holds more than 2 entries, and the rest is not looked at"),
    )


# Links


def test_a_place_of_the_user_may_itself_be_a_link(place: Path) -> None:
    kept = place / "dotfiles" / "agents"
    kind(kept, "reviewer")
    root = own(place / "home" / "agents")
    root.path.parent.mkdir()
    link_directory(root.path, kept)

    found = scan(root)

    assert names(found) == ["reviewer"]
    # The file is told where it lies, and not by the way that led to it.
    assert found.types[0].file == kept / "reviewer.md"


def test_a_file_of_the_user_that_is_a_link_is_read_where_it_leads(place: Path) -> None:
    kept = place / "dotfiles"
    kind(kept, "other-name", written("reviewer"))
    root = own(place / "agents")
    root.path.mkdir()
    link_file(root.path / "reviewer.md", kept / "other-name.md")

    found = scan(root)

    # It is called by the name of the link. What the link leads to may be called anything.
    assert names(found) == ["reviewer"]
    assert found.types[0].file == kept / "other-name.md"
    assert found.invalid == ()


def test_a_kind_is_called_what_its_entry_is_called_wherever_the_entry_leads(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The test above, for a user who may make no link to a file: the entry is said to lead
    # somewhere else, as a link would.
    kept = place / "dotfiles"
    kind(kept, "other-name", written("reviewer", text="Read where it leads."))
    root = own(place / "agents")
    kind(root.path, "reviewer", written("reviewer", text="Read where it lies."))

    def leads_away(root: Root, there: Path, entry: bound.Listed) -> Path:
        return kept / "other-name.md"

    monkeypatch.setattr(places, "target", leads_away)

    found = scan(root)

    assert names(found) == ["reviewer"]
    assert found.types[0].prompt == "Read where it leads."
    assert found.types[0].file == kept / "other-name.md"
    assert found.invalid == ()


def test_a_link_of_the_user_that_leads_to_no_file_is_said_to_do_so(place: Path) -> None:
    root = own(place / "agents")
    root.path.mkdir()
    (place / "a-directory").mkdir()
    (place / "gone").mkdir()
    link_directory(root.path / "folder.md", place / "a-directory")
    link_directory(root.path / "gone.md", place / "gone")
    (place / "gone").rmdir()
    kind(root.path, "reviewer")

    found = scan(root)

    assert names(found) == ["reviewer"]
    assert reasons(found) == [
        ("folder.md", "it is not a file"),
        ("gone.md", "it is a link that leads nowhere"),
    ]


def test_a_link_in_a_project_is_not_followed(place: Path) -> None:
    elsewhere = place / "elsewhere"
    kind(elsewhere, "reviewer")
    root = of_a_project(place / "agents")
    root.path.mkdir()
    link_directory(root.path / "team.md", elsewhere)
    kind(root.path, "explorer")

    found = scan(root)

    assert names(found) == ["explorer"]
    assert reasons(found) == [
        ("team.md", "it lies behind a link, and links are not followed in a project")
    ]


def test_a_file_of_a_project_that_is_a_link_is_not_read(place: Path) -> None:
    elsewhere = place / "elsewhere"
    kind(elsewhere, "reviewer")
    root = of_a_project(place / "agents")
    root.path.mkdir()
    link_file(root.path / "reviewer.md", elsewhere / "reviewer.md")

    found = scan(root)

    assert found.types == ()
    assert reasons(found) == [
        ("reviewer.md", "it lies behind a link, and links are not followed in a project")
    ]


def test_a_project_whose_place_for_kinds_is_a_link_has_no_kinds(place: Path) -> None:
    elsewhere = place / "elsewhere"
    kind(elsewhere, "reviewer")
    root = of_a_project(place / "project" / "agents")
    root.path.parent.mkdir()
    link_directory(root.path, elsewhere)

    assert scan(root) == Scan(
        root,
        invalid=(
            Invalid(
                root.path, root, "it lies behind a link, and links are not followed in a project"
            ),
        ),
    )


def test_nobody_goes_where_the_place_for_kinds_of_a_project_leads(
    place: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    elsewhere = place / "elsewhere"
    kind(elsewhere, "reviewer")
    root = of_a_project(place / "project" / "agents")
    root.path.parent.mkdir()
    link_directory(root.path, elsewhere)
    gone_to: list[Path] = []
    listing = bound.listing

    def lists(path: Path, at_most: int) -> Any:
        gone_to.append(path)
        return listing(path, at_most)

    monkeypatch.setattr(bound, "listing", lists)

    scan(root)

    # Going where a link leads may already tell another machine who asks.
    assert gone_to == []


# Tools that are not there


def test_a_kind_that_names_a_tool_which_is_not_there_cannot_be_used(place: Path) -> None:
    root = of_a_project(place / "agents")
    kind(root.path, "all", written("all"))
    kind(root.path, "none", written("none", "tools: []\n"))
    kind(root.path, "reader", written("reader", "tools: [read, grep]\n"))
    kind(root.path, "searcher", written("searcher", "tools: [read, web_search]\n"))
    kind(root.path, "wide", written("wide", "tools: [Read, bash, web_search, read]\n"))
    found = scan(root)

    left = usable(found, {"read", "grep", "edit"})

    assert names(left) == ["all", "none", "reader"]
    assert left.root == root
    assert reasons(left) == [
        ("searcher.md", "it names a tool that is not there: 'web_search'"),
        ("wide.md", "it names tools that are not there: 'Read', 'bash', 'web_search'"),
    ]
    # With every tool that is named there, every kind counts.
    assert usable(found, {"read", "grep", "web_search", "Read", "bash"}) == found


def test_what_could_not_be_used_before_stays_told(place: Path) -> None:
    root = of_a_project(place / "agents")
    kind(root.path, "broken", "no dashes")
    kind(root.path, "searcher", written("searcher", "tools: [web_search]\n"))

    left = usable(scan(root), {"read"})

    assert left.types == ()
    assert reasons(left) == [
        ("broken.md", "broken.md does not begin with a line of three dashes"),
        ("searcher.md", "it names a tool that is not there: 'web_search'"),
    ]


def test_of_many_tools_that_are_not_there_a_few_are_named_and_none_at_length(
    place: Path,
) -> None:
    root = of_a_project(place / "agents")
    many = ", ".join(f"tool_{number}" for number in range(1, 8))
    kind(root.path, "many", written("many", f"tools: [{many}]\n"))
    kind(root.path, "five", written("five", "tools: [a, b, c, d, e]\n"))
    kind(root.path, "long", written("long", f"tools: [{'x' * 100}]\n"))

    left = usable(scan(root), set())

    assert dict(reasons(left)) == {
        "many.md": (
            "it names tools that are not there: 'tool_1', 'tool_2', 'tool_3', 'tool_4', "
            "'tool_5', and 2 more"
        ),
        "five.md": "it names tools that are not there: 'a', 'b', 'c', 'd', 'e'",
        "long.md": f"it names a tool that is not there: '{'x' * 59}...",
    }


# Several roots


def test_of_two_kinds_of_one_name_the_one_of_the_project_is_the_kind(place: Path) -> None:
    project, home = roots(place / "project", place / "home" / "agents")
    kind(project.path, "reviewer", written("reviewer", text="Of the project."))
    kind(project.path, "broken", "no dashes")
    kind(project.path, "searcher", written("searcher", "tools: [web_search]\n"))
    kind(home.path, "reviewer", written("reviewer", text="Of the user."))
    kind(home.path, "broken", written("broken", text="Of the user."))
    kind(home.path, "searcher", written("searcher", text="Of the user."))
    kind(home.path, "explorer")

    found = merged([usable(scan(root), {"read"}) for root in (project, home)])

    assert list(found.types) == ["broken", "explorer", "reviewer", "searcher"]
    assert found.types["reviewer"].prompt == "Of the project."
    assert [(one.name, one.root.label) for one in found.shadowed] == [("reviewer", "home")]
    # A kind that cannot be used stands in nobody's way, not even in that of its namesake.
    assert found.types["broken"].prompt == "Of the user."
    assert found.types["searcher"].prompt == "Of the user."
    assert [(invalid.file.name, invalid.root.label) for invalid in found.invalid] == [
        ("broken.md", "project"),
        ("searcher.md", "project"),
    ]


def test_roots_that_hold_nothing_come_to_nothing() -> None:
    assert merged([]) == Found({})
    assert merged([Scan(ROOT), Scan(ROOT)]) == Found({})


def test_the_places_where_kinds_are_looked_for_come_in_a_fixed_order(place: Path) -> None:
    assert roots(place / "project", place / "home" / "agents") == (
        Root(place / "project" / ".columbina" / "agents", "project", own=False),
        Root(place / "home" / "agents", "home", own=True),
    )
    assert roots(place / "project", None) == (
        Root(place / "project" / ".columbina" / "agents", "project", own=False),
    )
