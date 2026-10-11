"""YAML that somebody else wrote: values, and nothing that builds values out of values.

Some files that this program reads lie in a repository that was cloned: the
first lines of a skill or of a kind of subagent, the rules of a project. They
are YAML, and YAML can do more than such a file has a use for. A value can be
given a name and be used again by that name, and a mapping can take in the
keys of others. A file of thirty lines that does this over and over makes the
reader build a value of a thousand million entries.

So what is read here is plain: a file that uses a value again by its name is
refused before anything is built. Whatever else the reader of YAML fails
with ends as one error of this module, with a few words for whoever wrote
the file.
"""

from typing import Any, Literal

import yaml

__all__ = ["Kind", "PlainYamlError", "load"]

Kind = Literal["syntax", "alias", "depth", "value"]
"""What is wrong with a text: it is no YAML, it uses a value again by its name, it is nested
too deeply, or it gives a value that cannot be made."""

_SAID = 300
"""How many characters of what the reader of YAML says are passed on."""


class PlainYamlError(Exception):
    """A text is no YAML that is read here.

    Attributes:
        kind: What is wrong with it.
        said: What the reader of YAML says about it, in one line and not at
            length. Empty where it says nothing of use.
        line: The line that the reader stopped at, counted from one, if it
            names one.
    """

    def __init__(self, kind: Kind, said: str = "", line: int | None = None) -> None:
        super().__init__(said or kind)
        self.kind: Kind = kind
        self.said = said
        self.line = line


class _UsedAgainError(yaml.YAMLError):
    """A value is used again by its name."""


class _Plain(yaml.SafeLoader):
    """Reads YAML as the safe reader does, and stops at a value that is used again."""

    def compose_node(self, parent: Any, index: Any) -> Any:
        if self.check_event(yaml.AliasEvent):
            # Before the value is looked up: nothing is built from it, not even once.
            raise _UsedAgainError
        return super().compose_node(parent, index)


def _whole_number(loader: _Plain, node: yaml.ScalarNode) -> int:
    """Make a whole number, and refuse one that cannot be written out again.

    Python makes a number of any length from digits that count by twos, eights
    or sixteens, and refuses to write a number out by tens beyond a length.
    So a few lines give a number that whoever names it in an error, or writes
    it down, fails on. It is refused here, like a number of as many digits
    that is written by tens, which Python does not make in the first place.
    """
    number = loader.construct_yaml_int(node)
    str(number)
    return number


_Plain.add_constructor("tag:yaml.org,2002:int", _whole_number)


def _one_line(error: BaseException) -> str:
    said = " ".join(str(error).split())
    return said if len(said) <= _SAID else f"{said[:_SAID]}..."


def _line(error: BaseException) -> int | None:
    mark = getattr(error, "problem_mark", None)
    return None if mark is None else mark.line + 1


def load(text: str) -> object:
    """Read YAML that somebody else wrote, and return what it holds.

    Raises:
        PlainYamlError: The text is no YAML, uses a value again by its name,
            is nested too deeply, or gives a value that cannot be made.
    """
    try:
        # A reader that is the safe one with less to it.
        return yaml.load(text, Loader=_Plain)
    except _UsedAgainError:
        raise PlainYamlError("alias") from None
    except RecursionError:
        raise PlainYamlError("depth") from None
    except yaml.YAMLError as error:
        raise PlainYamlError("syntax", _one_line(error), _line(error)) from None
    except Exception as error:
        # The reader makes values of what it reads, and a value that cannot be made fails
        # with whatever the making of it raises: a date that there is none of, a number of
        # thousands of digits, a tag before something that it does not go with.
        raise PlainYamlError("value", _one_line(error)) from None
