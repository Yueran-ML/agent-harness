"""Taking a command of a shell apart, as far as that can be done with certainty."""

from collections.abc import Callable

import pytest
from hypothesis import given
from hypothesis import strategies as st

from columbina.permissions.commands import split_bash, split_powershell

Splitter = Callable[[str], tuple[str, ...] | None]

BOTH: list[Splitter] = [split_bash, split_powershell]
BACKTICK = chr(96)
BACKSLASH = chr(92)


# The plain shape


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize(
    ("command", "apart"),
    [
        ("git status", ("git status",)),
        ("  git   status\t--short  ", ("git status --short",)),
        ("git status && git diff", ("git status", "git diff")),
        ("git status&&git diff", ("git status", "git diff")),
        ("make || echo failed", ("make", "echo failed")),
        ("cd src; ls", ("cd src", "ls")),
        ("cd src;ls;", ("cd src", "ls")),
        ("ls | sort | head -3", ("ls", "sort", "head -3")),
        ("git status\ngit diff\n", ("git status", "git diff")),
        ("git status\r\ngit diff\r\n", ("git status", "git diff")),
        ("\n\n  git status\n\n\n", ("git status",)),
        ("a &&\n  b", ("a", "b")),
        ("a |\n\n b", ("a", "b")),
        ("a ;\n b", ("a", "b")),
        (
            "git commit -m 'fix; do not && split | this'",
            ("git commit -m 'fix; do not && split | this'",),
        ),
        ('git commit -m "a; b && c"', ('git commit -m "a; b && c"',)),
        ("echo 'it'\"s\"", ("echo 'it'\"s\"",)),
        ("echo a'b c'd", ("echo a'b c'd",)),
        ("echo 'a\nb'", ("echo 'a\nb'",)),
        ("echo ''", ("echo ''",)),
        ("ls *.py ?.txt ~/notes", ("ls *.py ?.txt ~/notes",)),
        ("python -m pytest -q tests/unit", ("python -m pytest -q tests/unit",)),
        ("git log --format=%H -n1", ("git log --format=%H -n1",)),
        ("FOO=bar make", ("FOO=bar make",)),
    ],
)
def test_a_plain_command_is_taken_apart_into_its_simple_commands(
    split: Splitter, command: str, apart: tuple[str, ...]
) -> None:
    assert split(command) == apart


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize(
    "command",
    [
        "",
        "   ",
        "\n\n",
        "; ls",
        "&& ls",
        "| sort",
        "ls ;; ls",
        "ls && && ls",
        "ls && ; ls",
        "ls | | sort",
        "ls &&",
        "ls ||",
        "ls |",
        "ls |\n",
        "a\n| b",
        "a\n&& b",
        "echo 'unclosed",
        'echo "unclosed',
        "echo 'a' 'b",
    ],
)
def test_a_command_that_is_not_whole_is_not_understood(split: Splitter, command: str) -> None:
    assert split(command) is None


# What makes a command something else than plain


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize(
    "command",
    [
        "echo $HOME",
        "echo $(whoami)",
        "echo ${HOME}",
        f"echo {BACKTICK}whoami{BACKTICK}",
        "(cd src && ls)",
        "f() { ls; }",
        "{ ls; }",
        "ls > out.txt",
        "ls >> out.txt",
        "sort < in.txt",
        "ls 2>&1",
        "sleep 9 &",
        "sleep 9 & ls",
        "ls &>/dev/null",
        "ls |& sort",
        "ls # and a comment",
        'echo "$HOME"',
        'echo "$(whoami)"',
        f'echo "{BACKTICK}whoami{BACKTICK}"',
        'echo "a" $b',
    ],
)
def test_a_command_with_something_that_a_shell_expands_or_redirects_is_not_understood(
    split: Splitter, command: str
) -> None:
    assert split(command) is None


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize(
    "inside",
    [
        "$HOME",
        "$(whoami)",
        f"{BACKTICK}id{BACKTICK}",
        "a > b",
        "(x)",
        "{x}",
        "a & b",
        "# no comment",
        "a;b",
    ],
)
def test_between_single_quotes_nothing_is_special(split: Splitter, inside: str) -> None:
    assert split(f"echo '{inside}' && ls") == (f"echo '{inside}'", "ls")


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize("inside", ["a > b", "(x)", "{x}", "a & b", "# no comment", "a;b", "a|b"])
def test_between_double_quotes_what_does_not_expand_is_text(split: Splitter, inside: str) -> None:
    assert split(f'echo "{inside}" && ls') == (f'echo "{inside}"', "ls")


@pytest.mark.parametrize("split", BOTH)
@pytest.mark.parametrize(
    "point",
    [
        0x00,
        0x07,
        0x0B,
        0x0C,
        0x0D,
        0x1B,
        0x7F,
        0x85,
        0xA0,
        0x2000,
        0x2028,
        0x2029,
        0x200B,
        0x200D,
        0x202E,
        0x2066,
        0xFEFF,
        0x3000,
    ],
)
def test_a_character_that_is_not_what_it_looks_like_makes_a_command_not_understood(
    split: Splitter, point: int
) -> None:
    odd = chr(point)

    assert split(f"git{odd}status") is None
    assert split(f"echo '{odd}'") is None
    assert split(f"git status{odd}") is None


@pytest.mark.parametrize("split", BOTH)
def test_letters_of_any_script_are_plain(split: Splitter) -> None:
    moon = chr(0x6708) + chr(0x795E)

    assert split(f"echo {moon} && type {moon}.txt") == (f"echo {moon}", f"type {moon}.txt")


# Where the two shells differ


def test_a_backslash_is_an_escape_for_bash_and_a_part_of_a_path_for_powershell() -> None:
    command = f"type src{BACKSLASH}main.py"

    assert split_bash(command) is None
    assert split_powershell(command) == (command,)
    assert split_bash(f'echo "a{BACKSLASH}"b"') is None
    assert split_bash(f"echo a{BACKSLASH}; rm x") is None


@pytest.mark.parametrize(
    "command",
    [
        "[System.IO.File]::ReadAllText('x')",
        "Get-ChildItem [ab]*",
        "echo @args",
        "echo @'\ntext\n'@",
        "cmd --% /c dir & del x",
        "git --% status",
    ],
)
def test_what_powershell_reads_as_more_than_text_is_not_understood(command: str) -> None:
    assert split_powershell(command) is None


def test_brackets_and_an_at_sign_are_plain_for_bash() -> None:
    assert split_bash("ls [ab]*.txt user@host") == ("ls [ab]*.txt user@host",)


@pytest.mark.parametrize("point", range(0x2018, 0x2020))
def test_a_curved_quotation_mark_makes_a_command_for_powershell_not_understood(point: int) -> None:
    # PowerShell takes them for quotes: a straight one could open and a curved one close.
    curved = chr(point)

    assert split_powershell(f"echo 'a{curved} ; del x ; 'b'") is None
    assert split_powershell(f"echo {curved}a{curved}") is None


def test_a_curved_quotation_mark_is_a_letter_for_bash() -> None:
    curved = chr(0x2019)

    assert split_bash(f"echo it{curved}s") == (f"echo it{curved}s",)


def test_the_word_that_stops_the_reading_of_a_line_is_only_that_word() -> None:
    assert split_powershell("echo a--%b") == ("echo a--%b",)
    assert split_bash("git --% status") == ("git --% status",)


# Whatever the command


# No percent sign: with two dashes before it, it is a word that PowerShell reads specially.
WORDS = st.text(alphabet="abcXYZ019-_./=:,+", min_size=1, max_size=6)
QUOTED = st.text(alphabet="ab ;|&$()<>#", max_size=6).map(lambda inner: f"'{inner}'")
SIMPLE = st.lists(st.one_of(WORDS, QUOTED), min_size=1, max_size=4).map(" ".join)
JOINTS = st.sampled_from(["&&", "||", ";", "|", "\n"])


@st.composite
def plain_commands(draw: st.DrawFn) -> tuple[str, tuple[str, ...]]:
    """A command of the plain shape, and the simple commands that it was made of."""
    simple = draw(st.lists(SIMPLE, min_size=1, max_size=5))
    joints = draw(st.lists(JOINTS, min_size=len(simple) - 1, max_size=len(simple) - 1))
    pad = draw(st.sampled_from(["", " ", "  "]))
    command = simple[0]
    for joint, following in zip(joints, simple[1:], strict=True):
        command += f"{pad}{joint}{pad}{following}"
    return command, tuple(simple)


@pytest.mark.parametrize("split", BOTH)
@given(plain_commands())
def test_a_command_made_of_simple_commands_comes_apart_into_them(
    split: Splitter, made: tuple[str, tuple[str, ...]]
) -> None:
    command, simple = made

    assert split(command) == simple


@pytest.mark.parametrize(
    ("split", "special"),
    [(split_bash, "$`\\(){}<>#"), (split_powershell, "$`(){}[]<>@#")],
)
@given(made=plain_commands(), data=st.data())
def test_a_special_character_outside_quotes_makes_any_command_not_understood(
    split: Splitter, special: str, made: tuple[str, tuple[str, ...]], data: st.DataObject
) -> None:
    command, _ = made
    outside = [index for index in range(len(command) + 1) if command[:index].count("'") % 2 == 0]
    at = data.draw(st.sampled_from(outside))
    letter = data.draw(st.sampled_from(special))

    assert split(command[:at] + letter + command[at:]) is None
