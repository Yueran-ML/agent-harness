"""Hook files: what is read from them, which tools a group is for, and which files are trusted."""

import json
from pathlib import Path
from typing import Any

import pytest

from columbina.hooks.config import (
    DEFAULT_TIMEOUT,
    EVENTS,
    MAX_FILE_BYTES,
    MAX_TIMEOUT,
    Group,
    Hook,
    HookFileError,
    Hooks,
    chooses_tools,
    digest,
    distrust,
    matches,
    names_of,
    parsed,
    trust,
    trusted,
)


def file(hooks: Any) -> bytes:
    return json.dumps({"hooks": hooks}).encode()


def command(text: str = "ruff format", **more: Any) -> dict[str, Any]:
    return {"type": "command", "command": text, **more}


# What a file holds


def test_a_file_gives_the_groups_of_each_moment_in_their_order() -> None:
    content = file(
        {
            "PostToolUse": [
                {"matcher": "Edit|Write", "hooks": [command("ruff format", timeout=30)]},
                {"hooks": [command("echo done"), command("echo again")]},
            ],
            "Stop": [{"hooks": [{"command": "pytest -q"}]}],
            "PreToolUse": [
                {"matcher": "*", "hooks": [command("echo all")]},
                {"matcher": "", "hooks": [command("echo too")]},
            ],
        }
    )

    assert parsed(content) == Hooks(
        {
            # A star is no pattern. It is kept as it was written, like every matcher.
            "PreToolUse": (
                Group("*", (Hook("echo all"),)),
                Group("", (Hook("echo too"),)),
            ),
            "PostToolUse": (
                Group("Edit|Write", (Hook("ruff format", 30.0),)),
                Group(
                    None, (Hook("echo done", DEFAULT_TIMEOUT), Hook("echo again", DEFAULT_TIMEOUT))
                ),
            ),
            # A hook that names no type is a command.
            "Stop": (Group(None, (Hook("pytest -q", DEFAULT_TIMEOUT),)),),
        }
    )
    assert (DEFAULT_TIMEOUT, MAX_TIMEOUT) == (60.0, 600.0)
    assert EVENTS == ("SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop")


def test_a_file_without_hooks_holds_none() -> None:
    assert parsed(b"{}") == Hooks()
    assert parsed(b'{"hooks": {}}') == Hooks()
    assert parsed(b'\xef\xbb\xbf{"hooks": {"Stop": []}}') == Hooks()
    # Whatever else stands at the top is somebody else's.
    assert parsed(b'{"permissions": {"allow": ["Bash"]}, "hooks": {}}') == Hooks()


@pytest.mark.parametrize(
    ("content", "why"),
    [
        (b"not json", "it is not JSON: "),
        (b'{"hooks": ', "it is not JSON: "),
        (b"[]", "it holds a list and no object"),
        (b'"hooks"', 'it holds "hooks" and no object'),
        (b'{"hooks": []}', 'its "hooks" is a list and no object'),
        (b'{"hooks": 3}', 'its "hooks" is 3 and no object'),
        (b"\xff\xfe", "it is not written in UTF-8"),
        pytest.param(b"[" * 200_000, "it is nested too deeply", id="lists in lists"),
        pytest.param(
            b'{"hooks": {}, "pad": "' + b"x" * MAX_FILE_BYTES + b'"}',
            f"it has more than {MAX_FILE_BYTES} bytes",
            id="too large",
        ),
    ],
)
def test_a_file_that_is_no_hook_file_is_not_used(content: bytes, why: str) -> None:
    with pytest.raises(HookFileError) as raised:
        parsed(content)

    assert str(raised.value).startswith(why)


def test_a_file_of_just_the_size_that_is_allowed_is_read() -> None:
    head = b'{"hooks": {}, "pad": "'
    content = head + b"x" * (MAX_FILE_BYTES - len(head) - 2) + b'"}'

    assert len(content) == MAX_FILE_BYTES
    assert parsed(content) == Hooks()


def test_what_is_wrong_in_a_file_is_passed_over_and_the_rest_counts() -> None:
    content = file(
        {
            "Notification": [{"hooks": [command("echo")]}],
            "PreToolUse": "ruff",
            "PostToolUse": [
                "ruff format",
                {"matcher": 3, "hooks": [command("echo a")]},
                {"matcher": "Edit(", "hooks": [command("echo b")]},
                {"matcher": "Edit", "hooks": "echo c"},
                {
                    "matcher": "Write",
                    "hooks": [
                        "echo d",
                        {"type": "prompt", "prompt": "Is this safe?"},
                        {"type": "command"},
                        command("   "),
                        command("echo e"),
                    ],
                },
                # A group of which nothing is left is no group.
                {"hooks": [{"type": "http", "url": "https://example.com"}]},
            ],
            # One group, written without the list around it.
            "Stop": {"hooks": [command("echo f")]},
        }
    )

    found = parsed(content)

    assert found.groups == {"PostToolUse": (Group("Write", (Hook("echo e"),)),)}
    assert list(found.warnings) == [
        '"Notification" is no moment at which hooks run here, and is passed over. The moments '
        "are: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop",
        'PreToolUse holds "ruff" and no list of groups, and is passed over',
        'PostToolUse, group 1 is "ruff format" and no object, and is passed over',
        "PostToolUse, group 2 has the matcher 3, which is no text. It is passed over",
        'PostToolUse, group 3 has the matcher "Edit(", which is no regular expression '
        "(missing ), unterminated subpattern at position 4). It is passed over",
        "PostToolUse, group 4 has no list of hooks, and is passed over",
        'PostToolUse, group 5, hook 1 is "echo d" and no object, and is passed over',
        'PostToolUse, group 5, hook 2 is of the type "prompt". Only hooks of the type '
        '"command" are run, and this one is passed over',
        "PostToolUse, group 5, hook 3 names no command, and is passed over",
        "PostToolUse, group 5, hook 4 names no command, and is passed over",
        'PostToolUse, group 6, hook 1 is of the type "http". Only hooks of the type '
        '"command" are run, and this one is passed over',
        "Stop holds an object and no list of groups, and is passed over",
    ]


@pytest.mark.parametrize(
    ("timeout", "taken"),
    [(30, 30.0), (0.5, 0.5), (600, 600.0), (1, 1.0)],
)
def test_a_hook_may_take_the_time_that_its_file_gives_it(timeout: float, taken: float) -> None:
    found = parsed(file({"Stop": [{"hooks": [command(timeout=timeout)]}]}))

    assert found == Hooks({"Stop": (Group(None, (Hook("ruff format", taken),)),)})


def test_a_hook_that_asks_for_too_long_gets_the_longest_that_there_is() -> None:
    found = parsed(file({"Stop": [{"hooks": [command(timeout=601)]}]}))

    assert found.groups == {"Stop": (Group(None, (Hook("ruff format", MAX_TIMEOUT),)),)}
    assert list(found.warnings) == [
        "Stop, group 1, hook 1 has the timeout 601. A hook may take 600 seconds at most, and "
        "that is what this one gets"
    ]


@pytest.mark.parametrize(
    ("timeout", "shown"),
    [(0, "0"), (-5, "-5"), ("30", '"30"'), (True, "true"), (None, "null"), ([30], "a list")],
)
def test_a_time_that_is_no_number_of_seconds_makes_a_hook_no_hook(
    timeout: object, shown: str
) -> None:
    found = parsed(file({"Stop": [{"hooks": [command(timeout=timeout), command("echo kept")]}]}))

    assert found.groups == {"Stop": (Group(None, (Hook("echo kept"),)),)}
    assert list(found.warnings) == [
        f"Stop, group 1, hook 1 has the timeout {shown}, which is no number of seconds above "
        "zero. The hook is passed over"
    ]


@pytest.mark.parametrize(
    ("matcher", "why"),
    [
        pytest.param("a{99999999999999999999}", "the repetition number is too large", id="count"),
        pytest.param(
            "(" * 20_000 + "a" + ")" * 20_000, "maximum recursion depth exceeded", id="depth"
        ),
    ],
)
def test_a_matcher_that_the_reader_of_patterns_gives_up_on_is_passed_over(
    matcher: str, why: str
) -> None:
    found = parsed(
        file(
            {
                "PreToolUse": [
                    {"matcher": matcher, "hooks": [command("echo a")]},
                    {"matcher": "Bash", "hooks": [command("echo b")]},
                ]
            }
        )
    )

    assert found.groups == {"PreToolUse": (Group("Bash", (Hook("echo b"),)),)}
    (warning,) = found.warnings
    assert warning.startswith("PreToolUse, group 1 has the matcher ")
    # How the reader of patterns words its giving up depends on where it gave up.
    assert f", which is no regular expression ({why}" in warning
    assert warning.endswith("). It is passed over")


def test_a_command_with_half_a_character_in_it_is_passed_over() -> None:
    content = (
        rb'{"hooks": {"Stop": [{"hooks": ['
        rb'{"command": "echo \ud83d"}, {"command": "echo \ud83d'
        rb'\ude00"}]}]}}'
    )

    hooks = parsed(content)

    # No shell can be handed half a character, and no log can hold one.
    assert hooks.groups == {"Stop": (Group(None, (Hook("echo \U0001f600"),)),)}
    assert hooks.warnings == (
        "Stop, group 1, hook 1 has a command with half a character in it, which no shell can "
        "be handed. It is passed over",
    )


def test_what_a_file_holds_is_never_shown_at_length() -> None:
    long = "x" * 20_000
    content = file(
        {
            long: [],
            "Stop": [long, {"matcher": [long], "hooks": []}, {"hooks": [{"type": long}]}],
            "PreToolUse": [{"matcher": "(" + long, "hooks": [command()]}],
        }
    )

    found = parsed(content)

    assert len(found.warnings) == 5
    assert max(len(warning) for warning in found.warnings) < 400
    assert found.warnings[1] == (
        f'Stop, group 1 is "{"x" * 59}... and no object, and is passed over'
    )


# Which tools a group is for


@pytest.mark.parametrize("matcher", [None, "", "*"])
def test_a_group_that_names_no_tool_is_for_every_tool(matcher: str | None) -> None:
    assert matches(matcher, ("read", "Read")) is True
    assert matches(matcher, ("mcp__github__create_issue",)) is True


@pytest.mark.parametrize(
    ("matcher", "chooses"),
    [(None, False), ("", False), ("*", False), ("Edit|Write", True), (".*", True), (" ", True)],
)
def test_a_matcher_chooses_among_the_tools_unless_it_is_for_all_of_them(
    matcher: str | None, chooses: bool
) -> None:
    assert chooses_tools(matcher) is chooses


def test_names_with_bars_between_them_mean_just_these_tools() -> None:
    assert matches("Edit|Write", ("edit", "Edit")) is True
    assert matches("Edit|Write", ("write", "Write")) is True
    assert matches("Edit|Write", ("read", "Read")) is False
    assert matches("Read", ("read", "Read")) is True
    # A name is the whole name. Tools that only hold these letters are other tools.
    assert matches("Read", ("read_all",)) is False
    assert matches("Read", ("Thread",)) is False
    assert matches("read|", ("edit", "Edit")) is False
    # The name that the tool has here does as well as what others call it.
    assert matches("todo_write", ("todo_write", "TodoWrite")) is True


def test_any_other_matcher_is_a_pattern_for_the_whole_name() -> None:
    assert matches("mcp__.*", ("mcp__github__create_issue",)) is True
    assert matches("mcp__github__.*", ("mcp__gitlab__create_issue",)) is False
    assert matches("Edit.*", ("edit", "Edit")) is True
    # The pattern has to cover the name from its first letter to its last.
    assert matches("ea.", ("read", "Read")) is False
    assert matches(".ea", ("read", "Read")) is False
    assert matches("(Read)|(Write)", ("write", "Write")) is True


def test_a_tool_goes_by_its_own_name_and_by_what_others_call_it() -> None:
    assert names_of("read") == ("read", "Read")
    assert names_of("bash") == ("bash", "Bash")
    assert names_of("powershell") == ("powershell", "Bash")
    assert names_of("todo_write") == ("todo_write", "TodoWrite")
    assert names_of("subagent") == ("subagent", "Task")
    assert names_of("skill") == ("skill", "Skill")
    # A tool that nobody else has goes by its own name alone.
    assert names_of("mcp__github__create_issue") == ("mcp__github__create_issue",)
    for ours in ("write", "edit", "glob", "grep"):
        assert names_of(ours) == (ours, ours.capitalize())


# Which files are trusted


PROJECT = Path("/work/project/.columbina/hooks.json")
CONTENT = file({"Stop": [{"hooks": [command("pytest -q")]}]})


def test_a_file_is_trusted_once_the_user_has_said_so_and_for_as_long_as_it_stays_the_same() -> None:
    assert trusted(None, PROJECT, CONTENT) is False

    record = trust(None, PROJECT, CONTENT)

    assert trusted(record, PROJECT, CONTENT) is True
    assert json.loads(record) == {"version": 1, "files": {str(PROJECT): digest(CONTENT)}}
    assert record.endswith(b"\n")
    # One byte more, and what was trusted is not what is there.
    assert trusted(record, PROJECT, CONTENT + b" ") is False
    # The same words in another place are other commands.
    assert trusted(record, PROJECT.parent.parent / "other" / "hooks.json", CONTENT) is False


def test_the_record_keeps_what_it_held_and_knows_one_content_of_each_file() -> None:
    other = Path("/work/other/.columbina/hooks.json")
    record = trust(trust(None, PROJECT, CONTENT), other, b"{}")

    assert trusted(record, PROJECT, CONTENT) is True
    assert trusted(record, other, b"{}") is True

    # The file changed and was looked at again.
    again = trust(record, PROJECT, CONTENT + b" ")

    assert trusted(again, PROJECT, CONTENT) is False
    assert trusted(again, PROJECT, CONTENT + b" ") is True
    assert trusted(again, other, b"{}") is True
    assert list(json.loads(again)["files"]) == sorted([str(PROJECT), str(other)])


@pytest.mark.parametrize(
    "record",
    [
        b"",
        b"not json",
        b"[]",
        b'{"files": {}}',
        b"\xff\xfe",
        pytest.param(b"[" * 200_000, id="lists in lists"),
    ],
)
def test_a_record_that_cannot_be_read_trusts_nothing_and_is_begun_anew(record: bytes) -> None:
    assert trusted(record, PROJECT, CONTENT) is False

    assert json.loads(trust(record, PROJECT, CONTENT)) == {
        "version": 1,
        "files": {str(PROJECT): digest(CONTENT)},
    }


@pytest.mark.parametrize(
    "written",
    [
        {"version": 2, "files": {str(PROJECT): digest(CONTENT)}},
        {"version": 1, "files": [str(PROJECT)]},
        {"version": 1, "files": {str(PROJECT): 3}},
        {"version": 1, "files": {str(PROJECT): digest(CONTENT).upper()}},
    ],
)
def test_a_record_of_another_shape_trusts_nothing(written: dict[str, Any]) -> None:
    assert trusted(json.dumps(written).encode(), PROJECT, CONTENT) is False


def test_trust_that_was_given_can_be_taken_back() -> None:
    other = Path("/work/other/.columbina/hooks.json")
    record = trust(trust(None, PROJECT, CONTENT), other, b"{}")

    left = distrust(record, PROJECT)

    assert left is not None
    assert trusted(left, PROJECT, CONTENT) is False
    # What the record says about another file stays.
    assert trusted(left, other, b"{}") is True
    assert json.loads(left) == {"version": 1, "files": {str(other): digest(b"{}")}}
    assert left.endswith(b"\n")

    # The last file goes, and a record stays that trusts none.
    none = distrust(left, other)

    assert none is not None
    assert json.loads(none) == {"version": 1, "files": {}}


def test_a_file_that_was_not_trusted_leaves_the_record_as_it_is() -> None:
    record = trust(None, PROJECT, CONTENT)

    assert distrust(record, PROJECT.parent.parent / "other" / "hooks.json") is None
    assert distrust(None, PROJECT) is None
    # A record that cannot be read trusts nothing, so there is nothing to take out of it.
    assert distrust(b"not json", PROJECT) is None


def test_the_mark_of_a_content_is_its_sha_256() -> None:
    assert digest(b"") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    assert digest(CONTENT) != digest(CONTENT + b" ")
