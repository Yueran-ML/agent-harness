"""What is done with a call: the rules of the three kinds, and what holds when none speaks."""

import itertools

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.permissions import Decided, RuleSet, Splitter, decide, read_rules
from columbina.permissions.commands import split_bash
from columbina.permissions.rules import Place, Rule, covers_command, covers_file, parse_rule
from columbina.sandbox import Mode, Strength
from columbina.tools import Effect

MODES: tuple[Mode, ...] = ("read-only", "workspace-write", "danger-full-access")
STRENGTHS: tuple[Strength, ...] = ("none", "partial", "full")
HELD = "The sandbox holds the command fully, so nobody needs to be asked."
EFFECTS: tuple[Effect, ...] = ("none", "read", "write", "run")


def rules(deny: str = "", ask: str = "", allow: str = "") -> RuleSet:
    """Rules of the three kinds, each written as one line with commas between the rules."""

    def read(line: str) -> tuple[Rule, ...]:
        return tuple(parse_rule(text) for text in line.split(",") if text.strip())

    return RuleSet(read(deny), read(ask), read(allow))


def file(
    held: RuleSet, tool: str, effect: Effect, relative: str, mode: Mode = "workspace-write"
) -> Decided:
    """What is done with a call of a tool that names a file of the working directory."""
    place = Place(relative, f"/work/{relative}")
    return decide(held, tool, effect, relative, place, mode=mode, splitter=None)


def command(
    held: RuleSet,
    text: str,
    mode: Mode = "workspace-write",
    *,
    understood: bool = True,
    strength: Strength = "none",
) -> Decided:
    """What is done with a command that the tool ``shell`` is to run."""
    splitter = split_bash if understood else None
    return decide(held, "shell", "run", text, None, mode=mode, splitter=splitter, held=strength)


# Without a rule


@pytest.mark.parametrize(("effect", "mode"), list(itertools.product(EFFECTS, MODES)))
def test_without_a_rule_only_a_command_in_a_session_without_full_access_is_asked_about(
    effect: Effect, mode: Mode
) -> None:
    target = "git status" if effect == "run" else "notes.txt"
    place = None if effect in ("none", "run") else Place("notes.txt", "/work/notes.txt")

    decided = decide(RuleSet(), "tool", effect, target, place, mode=mode, splitter=split_bash)

    asked = effect == "run" and mode != "danger-full-access"
    assert decided.verdict == ("ask" if asked else "allow")
    assert decided.rule is None


def test_a_call_that_names_nothing_is_decided_like_any_other() -> None:
    assert decide(
        RuleSet(), "todo", "none", None, None, mode="read-only", splitter=None
    ) == Decided("allow", None, "No rule speaks about this call.")
    assert (
        decide(
            rules(deny="todo"), "todo", "none", None, None, mode="read-only", splitter=None
        ).verdict
        == "deny"
    )


# Rules about files


@pytest.mark.parametrize(
    ("held", "tool", "effect", "path", "verdict", "rule"),
    [
        (rules(deny="read(.env)"), "read", "read", ".env", "deny", "read(.env)"),
        (rules(deny="read(.env)"), "grep", "read", "app/.env", "deny", "read(.env)"),
        (rules(deny="read(.env)"), "read", "read", "notes.txt", "allow", None),
        (rules(deny="read(.env)"), "write", "write", ".env", "allow", None),
        (rules(deny="write(docs)"), "edit", "write", "docs/a.md", "deny", "write(docs)"),
        (rules(deny="edit(docs)"), "write", "write", "docs/a.md", "allow", None),
        (rules(ask="write(*.lock)"), "write", "write", "uv.lock", "ask", "write(*.lock)"),
        (rules(allow="write(docs)"), "write", "write", "docs/a.md", "allow", "write(docs)"),
        (rules(deny="grep"), "grep", "read", "src", "deny", "grep"),
        (rules(deny="grep"), "glob", "read", "src", "allow", None),
        (rules(deny="*(secrets)"), "glob", "read", "secrets", "deny", "*(secrets)"),
    ],
)
def test_a_rule_about_files_settles_the_calls_that_it_speaks_about(
    held: RuleSet, tool: str, effect: Effect, path: str, verdict: str, rule: str | None
) -> None:
    decided = file(held, tool, effect, path)

    assert decided.verdict == verdict
    assert (None if decided.rule is None else str(decided.rule)) == rule


def test_a_rule_that_refuses_counts_before_one_that_asks_and_that_before_one_that_allows() -> None:
    every = rules(deny="read(a)", ask="read(a),read(b)", allow="read(a),read(b),read(c)")

    assert [file(every, "read", "read", name).verdict for name in ("a", "b", "c", "d")] == [
        "deny",
        "ask",
        "allow",
        "allow",
    ]


def test_what_a_rule_says_is_said_in_words_that_name_it() -> None:
    held = rules(deny="read(a)", ask="read(b)", allow="read(c)")

    assert file(held, "read", "read", "a").why == "The rule read(a) is against it."
    assert (
        file(held, "read", "read", "b").why == "The rule read(b) wants somebody to be asked first."
    )
    assert file(held, "read", "read", "c").why == "The rule read(c) allows it."


# Rules about commands


@pytest.mark.parametrize(
    ("held", "text", "verdict"),
    [
        # A rule that allows has to allow every simple command of the line.
        (rules(allow="shell(git status*)"), "git status", "allow"),
        (rules(allow="shell(git status*)"), "git status --short", "allow"),
        (rules(allow="shell(git status*)"), "git status && git push", "ask"),
        (rules(allow="shell(git status*)"), "git status; rm -rf .", "ask"),
        (rules(allow="shell(git status*)"), "git status | sh", "ask"),
        (rules(allow="shell(git status*),shell(git diff*)"), "git status && git diff", "allow"),
        (rules(allow="run(git status*)"), "git status", "allow"),
        (rules(allow="other(git status*)"), "git status", "ask"),
        # It allows nothing in a line that cannot be taken apart with certainty.
        (rules(allow="shell(git status*)"), "git status $(rm -rf .)", "ask"),
        (rules(allow="shell(echo *)"), "echo `id`", "ask"),
        (rules(allow="shell(*)"), "echo $(id)", "ask"),
        (rules(allow="shell(echo *)"), "echo hi > ~/.profile", "ask"),
        # A rule without a pattern allows the tool, whatever it is given.
        (rules(allow="shell"), "echo $(id) > x", "allow"),
        (rules(allow="run"), "anything & at all", "allow"),
        # A rule that refuses or asks counts for the whole line and for every part of it.
        (rules(deny="shell(rm *)", allow="shell"), "rm -rf .", "deny"),
        (rules(deny="shell(rm *)", allow="shell"), "ls && rm -rf .", "deny"),
        (rules(deny="shell(*rm -rf*)", allow="shell"), "echo $(rm -rf .)", "deny"),
        (rules(deny="shell(rm *)", allow="shell"), "echo $(rm -rf .)", "allow"),
        (rules(ask="shell(git push*)", allow="shell"), "git status && git push", "ask"),
        (rules(ask="shell(git push*)", allow="shell"), "git status", "allow"),
        (rules(deny="shell(git push*)", ask="shell(git *)", allow="shell"), "git push", "deny"),
        (rules(deny="shell"), "ls", "deny"),
        (rules(ask="shell"), "ls", "ask"),
    ],
)
def test_a_rule_about_commands_settles_the_lines_that_it_speaks_about(
    held: RuleSet, text: str, verdict: str
) -> None:
    assert command(held, text).verdict == verdict


def test_full_access_lets_a_command_go_ahead_that_no_rule_speaks_about() -> None:
    held = rules(deny="shell(rm *)", ask="shell(git push*)")

    assert command(held, "make", "danger-full-access").verdict == "allow"
    assert command(held, "make $(x)", "danger-full-access").verdict == "allow"
    assert command(held, "rm x", "danger-full-access").verdict == "deny"
    assert command(held, "git push", "danger-full-access").verdict == "ask"


def test_without_something_that_takes_commands_apart_only_a_whole_tool_can_be_allowed() -> None:
    assert (
        command(rules(allow="shell(git status*)"), "git status", understood=False).verdict == "ask"
    )
    assert command(rules(allow="shell"), "git status", understood=False).verdict == "allow"
    assert command(rules(deny="shell(git *)"), "git status", understood=False).verdict == "deny"


def test_a_command_is_asked_about_in_words_that_say_why() -> None:
    held = rules(allow="shell(git status*)")

    assert command(RuleSet(), "make").why == (
        "No rule allows this command, and nothing holds it here."
    )
    assert command(held, "git push").why == (
        "No rule allows this command, and nothing holds it here."
    )
    assert command(held, "git status $(x)").why.startswith("No rule can allow this command")
    assert command(held, "git status && git status -s").why == (
        "Rules allow every command in the line."
    )
    assert command(RuleSet(), "make", "danger-full-access").why == "The session has full access."


def test_a_tool_that_runs_something_and_names_no_command_is_asked_about_as_a_tool() -> None:
    def of_a_server(held: RuleSet, mode: Mode = "workspace-write") -> Decided:
        return decide(held, "mcp__files__write", "run", None, None, mode=mode, splitter=None)

    # A tool of a server does what its server likes. There is no line to take apart, so the
    # words about commands are not for it, whatever other rules there are.
    for held in (RuleSet(), rules(allow="shell(git status*)")):
        asked = of_a_server(held)
        assert (asked.verdict, asked.rule) == ("ask", None)
        assert asked.why == "No rule allows calls of this tool."
    # A rule that names the tool decides about it, and so does a session with full access.
    assert of_a_server(rules(allow="mcp__files__*")).verdict == "allow"
    assert of_a_server(rules(deny="mcp__files__write")).verdict == "deny"
    assert of_a_server(RuleSet(), "danger-full-access").why == "The session has full access."


def test_a_tool_that_names_no_command_goes_ahead_once_it_was_allowed_for_the_session() -> None:
    def of_a_server(held: RuleSet) -> Decided:
        return decide(
            held,
            "mcp__files__write",
            "run",
            None,
            None,
            mode="workspace-write",
            splitter=None,
            granted=True,
        )

    assert of_a_server(RuleSet()).why == "It was allowed for the rest of this session."
    # Such a yes counts before a rule that asks and after one that refuses, like every yes.
    assert of_a_server(rules(ask="mcp__files__write")).verdict == "allow"
    assert of_a_server(rules(deny="mcp__files__*")).verdict == "deny"


def test_a_command_that_nothing_takes_apart_is_asked_about_although_it_was_allowed() -> None:
    # A tool that runs a command and has nothing that takes commands apart names what it
    # acts on. A yes for one line of it says nothing of what the line does the next time.
    decided = decide(
        RuleSet(),
        "bare",
        "run",
        "git status",
        None,
        mode="workspace-write",
        splitter=None,
        granted=True,
    )

    assert decided.verdict == "ask"


def test_a_tool_for_commands_that_names_none_is_not_allowed_as_a_whole() -> None:
    # A shell that says nothing of what it runs is no tool of another kind. A yes for every
    # call of it would be a yes for every command.
    decided = decide(
        RuleSet(),
        "shell",
        "run",
        None,
        None,
        mode="workspace-write",
        splitter=split_bash,
        granted=True,
    )

    assert decided.verdict == "ask"


# What was allowed for the rest of a session


def test_a_call_that_was_allowed_for_the_session_goes_ahead_although_a_rule_asks() -> None:
    held = rules(ask="write(notes.txt)", allow="read")
    place = Place("notes.txt", "/work/notes.txt")

    asked = decide(held, "write", "write", "notes.txt", place, mode="read-only", splitter=None)
    allowed = decide(
        held, "write", "write", "notes.txt", place, mode="read-only", splitter=None, granted=True
    )

    assert asked.verdict == "ask"
    assert allowed == Decided("allow", None, "It was allowed for the rest of this session.")


def test_a_rule_that_refuses_counts_against_what_was_allowed_for_the_session() -> None:
    held = rules(deny="write(notes.txt)")
    place = Place("notes.txt", "/work/notes.txt")

    decided = decide(
        held, "write", "write", "notes.txt", place, mode="read-only", splitter=None, granted=True
    )

    assert decided.verdict == "deny"


def test_a_command_that_was_allowed_for_the_session_runs_without_a_rule() -> None:
    decided = decide(
        RuleSet(), "shell", "run", "git status", None, mode="read-only", splitter=split_bash
    )
    again = decide(
        RuleSet(),
        "shell",
        "run",
        "git status",
        None,
        mode="read-only",
        splitter=split_bash,
        granted=True,
    )

    assert decided.verdict == "ask"
    assert again == Decided("allow", None, "It was allowed for the rest of this session.")


def test_a_rule_against_a_part_of_a_command_counts_against_what_was_allowed() -> None:
    held = rules(deny="shell(rm *)")

    decided = decide(
        held,
        "shell",
        "run",
        "git status && rm -rf .",
        None,
        mode="read-only",
        splitter=split_bash,
        granted=True,
    )

    assert decided.verdict == "deny"


# A command that the sandbox holds


def test_a_command_that_the_sandbox_holds_fully_goes_ahead_without_a_rule() -> None:
    assert command(rules(), "make test", strength="full") == Decided("allow", None, HELD, held=True)


@pytest.mark.parametrize("strength", ["none", "partial"])
def test_a_command_that_is_held_in_part_or_not_at_all_is_asked_about(strength: Strength) -> None:
    decided = command(rules(), "make test", strength=strength)

    assert (decided.verdict, decided.held) == ("ask", False)


def test_a_rule_that_refuses_or_asks_counts_against_a_command_that_is_held() -> None:
    refused = command(rules(deny="shell(rm *)"), "rm -rf .", strength="full")
    asked = command(rules(ask="shell(git push *)"), "git push origin", strength="full")
    in_a_line = command(rules(ask="shell(git push *)"), "make && git push origin", strength="full")

    assert (refused.verdict, refused.held) == ("deny", False)
    assert (asked.verdict, asked.held) == ("ask", False)
    assert (in_a_line.verdict, in_a_line.held) == ("ask", False)


def test_what_lets_a_command_go_ahead_anyway_is_said_before_the_hold_is() -> None:
    # Only a call that goes ahead for the hold and for nothing else is looked at again
    # when its command is run.
    by_rule = command(rules(allow="shell(make *)"), "make test", strength="full")
    with_access = command(rules(), "make test", "danger-full-access", strength="full")
    granted = decide(
        rules(), "shell", "run", "make test", None,
        mode="workspace-write", splitter=split_bash, granted=True, held="full",
    )  # fmt: skip

    assert (by_rule.verdict, by_rule.held) == ("allow", False)
    assert str(by_rule.rule) == "shell(make *)"
    assert (with_access.why, with_access.held) == ("The session has full access.", False)
    assert (granted.why, granted.held) == ("It was allowed for the rest of this session.", False)


def test_a_command_that_cannot_be_taken_apart_goes_ahead_if_it_is_held_fully() -> None:
    odd = "echo $(cat notes.txt)"

    assert command(rules(allow="shell(echo *)"), odd).verdict == "ask"
    assert command(rules(allow="shell(echo *)"), odd, strength="full") == Decided(
        "allow", None, HELD, held=True
    )
    assert command(rules(), odd, understood=False, strength="full").held


@pytest.mark.parametrize("splitter", [None, split_bash])
def test_a_command_that_cannot_be_taken_apart_is_asked_about_every_time(
    splitter: Splitter | None,
) -> None:
    # Whatever said yes to it once: a line with a variable in it may do another thing next time.
    decided = decide(
        RuleSet(),
        "shell",
        "run",
        "rm $TARGET",
        None,
        mode="read-only",
        splitter=splitter,
        granted=True,
    )

    assert decided.verdict == "ask"


# Against the definition


TOOLS = {"read": "read", "grep": "read", "write": "write", "edit": "write", "todo": "none"}
FILE_RULES = st.builds(
    Rule,
    st.sampled_from(["read", "write", "grep", "edit", "todo", "*", "run"]),
    st.sampled_from([None, "a", "docs", "docs/a", "./a", "*.md", "**"]),
)
PATHS = st.sampled_from(["a", "docs/a", "docs/b.md", "x/a", "b.md", "."])


def some(rules_of_a_kind: tuple[Rule, ...], speaks: "list[bool]") -> Rule | None:
    return next((rule for rule, yes in zip(rules_of_a_kind, speaks, strict=True) if yes), None)


@given(
    st.lists(FILE_RULES, max_size=3),
    st.lists(FILE_RULES, max_size=3),
    st.lists(FILE_RULES, max_size=3),
    st.sampled_from(sorted(TOOLS)),
    PATHS,
    st.sampled_from(MODES),
    st.booleans(),
)
def test_a_call_about_a_file_is_decided_as_the_order_of_the_kinds_says(
    deny: list[Rule],
    ask: list[Rule],
    allow: list[Rule],
    tool: str,
    path: str,
    mode: Mode,
    granted: bool,
) -> None:
    effect: Effect = TOOLS[tool]  # pyright: ignore[reportAssignmentType]
    place = Place(path, f"/work/{path}")
    held = RuleSet(tuple(deny), tuple(ask), tuple(allow))

    decided = decide(held, tool, effect, path, place, mode=mode, splitter=None, granted=granted)

    def speaks(kind: list[Rule]) -> bool:
        return any(covers_file(rule, tool, effect, place) for rule in kind)

    # What was allowed for the session stands between the rules that refuse and those that ask.
    if speaks(deny):
        expected = "deny"
    elif granted:
        expected = "allow"
    else:
        expected = "ask" if speaks(ask) else "allow"
    assert decided.verdict == expected


COMMAND_RULES = st.builds(
    Rule,
    st.sampled_from(["shell", "run", "other", "*"]),
    st.sampled_from([None, "a", "a *", "b*", "*"]),
)
SIMPLE = st.sampled_from(["a", "a x", "b", "bb y", "c"])
JOINED = st.lists(SIMPLE, min_size=1, max_size=3).flatmap(
    lambda simple: st.sampled_from([" && ", " ; ", " | "]).map(lambda joint: joint.join(simple))
)
ODD = SIMPLE.map(lambda simple: f"{simple} $(c)")


@given(
    st.lists(COMMAND_RULES, max_size=3),
    st.lists(COMMAND_RULES, max_size=3),
    st.lists(COMMAND_RULES, max_size=3),
    st.one_of(JOINED, ODD),
    st.sampled_from(MODES),
    st.booleans(),
    st.sampled_from(STRENGTHS),
)
def test_a_command_is_decided_as_the_definition_says(
    deny: list[Rule],
    ask: list[Rule],
    allow: list[Rule],
    text: str,
    mode: Mode,
    granted: bool,
    strength: Strength,
) -> None:
    held = RuleSet(tuple(deny), tuple(ask), tuple(allow))
    apart = split_bash(text)
    every = [text, *(apart or ())]

    def speaks(kind: list[Rule], of: list[str]) -> bool:
        return any(covers_command(rule, "shell", part) for rule in kind for part in of)

    whole_tool = any(rule.pattern is None and covers_command(rule, "shell", text) for rule in allow)
    every_part = apart is not None and all(speaks(allow, [part]) for part in apart)
    allowed = whole_tool or every_part or mode == "danger-full-access"
    for_the_hold = False
    if speaks(deny, every):
        expected = "deny"
    elif granted and apart is not None:
        expected = "allow"
    elif speaks(ask, every):
        expected = "ask"
    elif allowed:
        expected = "allow"
    else:
        for_the_hold = strength == "full"
        expected = "allow" if for_the_hold else "ask"

    decided = decide(
        held, "shell", "run", text, None,
        mode=mode, splitter=split_bash, granted=granted, held=strength,
    )  # fmt: skip
    assert (decided.verdict, decided.held) == (expected, for_the_hold)


# Rules in a file


def test_a_file_holds_rules_of_the_three_kinds() -> None:
    held = read_rules(
        "deny:\n  - read(.env)\n  - write(.github)\nask:\n  - shell(git push*)\nallow:\n  - grep\n",
        "the rules",
    )

    assert held == rules(deny="read(.env),write(.github)", ask="shell(git push*)", allow="grep")


@pytest.mark.parametrize("text", ["", "# nothing but a comment\n", "deny:\nask: []\n", "{}"])
def test_a_file_without_rules_holds_none(text: str) -> None:
    assert read_rules(text, "the rules") == RuleSet()


@pytest.mark.parametrize(
    ("text", "wrong"),
    [
        ("- read(.env)\n", "it has to hold the lists deny, ask and allow"),
        ("just text", "it has to hold the lists deny, ask and allow"),
        ("deny: read(.env)\n", "deny has to be a list of rules"),
        ("allow:\n  key: value\n", "allow has to be a list of rules"),
        ("deny:\n  - 5\n", "a rule under deny is not text"),
        ("ask:\n  - [a, b]\n", "a rule under ask is not text"),
        ("deny:\n  - 'read('\n", "does not close it"),
        ("forbid:\n  - read\n", "it holds forbid, and only deny, ask and allow are known"),
        ("deny: [a\n", "it is not YAML that can be read at line"),
        ("deny:\n\t- a\n", "it is not YAML that can be read at line 2"),
        (
            "few: &few [read]\ndeny: *few\n",
            "it uses a value again by its name, which is not read here",
        ),
        pytest.param("deny: " + "[" * 100_000, "it is nested too deeply", id="lists in lists"),
        ("deny: 2024-13-45\n", "it gives a value that cannot be read"),
    ],
)
def test_a_file_that_does_not_hold_rules_is_refused_in_words_that_name_it(
    text: str, wrong: str
) -> None:
    with pytest.raises(ValueError, match=r"the rules in x\.yml: ") as refused:
        read_rules(text, "the rules in x.yml")

    assert wrong in str(refused.value)


def test_what_a_file_holds_is_not_repeated_when_it_cannot_be_read() -> None:
    secret = "password: hunter2\n  nested: [unclosed\n"

    with pytest.raises(ValueError, match="not YAML") as refused:
        read_rules(secret, "the rules")

    assert "hunter2" not in str(refused.value)


def test_rules_of_two_places_are_all_rules() -> None:
    assert rules(deny="a", allow="b") + rules(deny="c", ask="d") == rules(
        deny="a,c", ask="d", allow="b"
    )
