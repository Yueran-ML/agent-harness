"""What is done with a call: the rules of the three kinds, and what holds when none speaks."""

import itertools

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.permissions import Decided, RuleSet, decide, read_rules
from columbina.permissions.commands import split_bash
from columbina.permissions.rules import Place, Rule, covers_command, covers_file, parse_rule
from columbina.sandbox import Mode
from columbina.tools import Effect

MODES: tuple[Mode, ...] = ("read-only", "workspace-write", "danger-full-access")
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
    held: RuleSet, text: str, mode: Mode = "workspace-write", *, understood: bool = True
) -> Decided:
    """What is done with a command that the tool ``shell`` is to run."""
    splitter = split_bash if understood else None
    return decide(held, "shell", "run", text, None, mode=mode, splitter=splitter)


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
)
def test_a_call_about_a_file_is_decided_as_the_order_of_the_kinds_says(
    deny: list[Rule], ask: list[Rule], allow: list[Rule], tool: str, path: str, mode: Mode
) -> None:
    effect: Effect = TOOLS[tool]  # pyright: ignore[reportAssignmentType]
    place = Place(path, f"/work/{path}")
    held = RuleSet(tuple(deny), tuple(ask), tuple(allow))

    decided = decide(held, tool, effect, path, place, mode=mode, splitter=None)

    expected = "allow"
    for verdict, kind in (("allow", allow), ("ask", ask), ("deny", deny)):
        if any(covers_file(rule, tool, effect, place) for rule in kind):
            expected = verdict
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
)
def test_a_command_is_decided_as_the_definition_says(
    deny: list[Rule], ask: list[Rule], allow: list[Rule], text: str, mode: Mode
) -> None:
    held = RuleSet(tuple(deny), tuple(ask), tuple(allow))
    apart = split_bash(text)
    every = [text, *(apart or ())]

    def speaks(kind: list[Rule], of: list[str]) -> bool:
        return any(covers_command(rule, "shell", part) for rule in kind for part in of)

    whole_tool = any(rule.pattern is None and covers_command(rule, "shell", text) for rule in allow)
    every_part = apart is not None and all(speaks(allow, [part]) for part in apart)
    allowed = whole_tool or every_part or mode == "danger-full-access"
    if speaks(deny, every):
        expected = "deny"
    elif speaks(ask, every):
        expected = "ask"
    else:
        expected = "allow" if allowed else "ask"

    assert command(held, text, mode).verdict == expected


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
