"""Taking a command of a shell apart, as far as that can be done with certainty.

A rule that allows a command allows a text. ``git status*`` is meant to allow
a look at a repository, and it would also allow ``git status; rm -rf .`` if
it were asked about the whole line. So a command is taken apart into the
simple commands that it runs, and each of them has to be allowed.

A shell has many ways to run something that do not look like a command: a
substitution, a variable that holds one, a block, a redirection that writes
a file. To follow them all would take a shell. This does the opposite. It
knows one plain shape, and of everything else it says that it does not
understand it:

    simple commands, joined by ``&&``, ``||``, ``;``, ``|`` or a line break,
    each made of plain words and of quoted strings in which nothing is
    expanded.

A command of that shape is returned as its simple commands, each with one
space between its words. For any other command the answer is ``None``, and
whoever asks then lets no rule allow it. That is the safe side: what is not
understood is asked about.

What makes a command something else than plain differs between the shells,
and each has its own list here. Characters that cannot be seen, or that pass
for a space or turn the direction of the text, are refused in both: a person
who is asked about a command has to see what will run.
"""

import unicodedata
from collections.abc import Callable

__all__ = ["Splitter", "split_bash", "split_powershell"]

Splitter = Callable[[str], tuple[str, ...] | None]
"""Takes a command apart into its simple commands, or returns ``None`` if it cannot."""

_HIDDEN = frozenset({"Cc", "Cf", "Zl", "Zp", "Zs"})
"""The kinds of characters that are not what they look like: controls, marks, odd spaces."""


def _split(
    command: str, plain_breaks: str, in_double_quotes: str, quotes_of_other_kinds: str
) -> tuple[str, ...] | None:
    """Take a command apart, or return ``None`` if it is not of the plain shape.

    Args:
        command: The command.
        plain_breaks: The characters that, outside quotes, make a command
            something else than plain.
        in_double_quotes: The characters that make a string in double quotes
            something else than the text between them.
        quotes_of_other_kinds: Characters that the shell takes for quotes
            though they are not the two that this knows.
    """
    text = command.replace("\r\n", "\n")
    for letter in text:
        if letter in quotes_of_other_kinds:
            return None
        if letter not in " \n\t" and unicodedata.category(letter) in _HIDDEN:
            return None
    commands: list[str] = []
    words: list[str] = []
    word = ""
    owed = False  # whether a joint was seen that needs a command after it
    at = 0
    while at < len(text):
        letter = text[at]
        if letter in "'\"":
            end = text.find(letter, at + 1)
            if end < 0:
                return None
            if letter == '"' and any(inner in in_double_quotes for inner in text[at + 1 : end]):
                return None
            word += text[at : end + 1]
            at = end + 1
            continue
        joint = text[at : at + 2] if text[at : at + 2] in ("&&", "||") else letter
        if letter in " \t" or joint in ("&&", "||", ";", "|", "\n"):
            if word:
                words.append(word)
                word = ""
            if letter not in " \t":
                if words:
                    commands.append(" ".join(words))
                    words = []
                    owed = False
                elif joint != "\n":
                    # A joint with nothing before it.
                    return None
                owed = owed or joint in ("&&", "||", "|")
            at += len(joint)
            continue
        if letter in plain_breaks:
            return None
        word += letter
        at += 1
    if word:
        words.append(word)
    if words:
        commands.append(" ".join(words))
        owed = False
    if owed or not commands:
        return None
    return tuple(commands)


def split_bash(command: str) -> tuple[str, ...] | None:
    """Take a command for bash apart, or return ``None`` if it is not of the plain shape.

    Outside quotes these make a command something else: ``$`` and the
    backtick, which put the output of a command or the content of a variable
    into the line; the backslash, which takes the meaning from the character
    after it; parentheses and braces, which make groups and functions; ``<``
    and ``>``, which read and write files; a single ``&``, which lets a
    command run on beside the next; and ``#``, which begins a comment. In
    double quotes, ``$``, the backtick and the backslash still work.
    """
    return _split(command, "$`\\(){}<>&#", "$`\\", "")


def split_powershell(command: str) -> tuple[str, ...] | None:
    """Take a command for PowerShell apart, or return ``None`` if it is not of the plain shape.

    Outside quotes these make a command something else: ``$``, which begins
    a variable or an expression; the backtick, which takes the meaning from
    the character after it; parentheses, braces and brackets, which make
    expressions, blocks and types; ``<`` and ``>``, which redirect; a single
    ``&``, which calls what follows; ``@``, which begins a here-string or
    spreads a table over the parameters; and ``#``, which begins a comment.
    In double quotes, ``$`` and the backtick still work. The arguments
    ``--%`` switch the reading of the rest of the line off, so a line with
    them is not plain either.

    PowerShell takes the curved quotation marks of a word processor for
    quotes. A string could be opened by a straight one and closed by a
    curved one, and what follows would run. So a command with such a mark in
    it is not plain.
    """
    curved = "".join(chr(point) for point in range(0x2018, 0x2020))
    apart = _split(command, "$`(){}[]<>&@#", "$`", curved)
    if apart is None or any("--%" in simple.split(" ") for simple in apart):
        return None
    return apart
