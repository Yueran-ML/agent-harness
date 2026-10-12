"""The conduct: what the program itself tells every agent about going about its work.

This module is a plugin. Loading it adds sections to the system prompt.

The spec of an agent holds the text of whoever set the agent up: who the
agent is and what it is for. What makes an agent do its work well is mostly
the same whoever it is, and it is not for every profile to write it down
anew. It stands in the file ``conduct.md`` beside this module, as the prose
that it is, and follows the text of the spec.

Each section is about one thing, has a heading and is known by a name. A
profile can leave a section out, which is how the worth of one is measured:
the same tasks are run with it and without it.

The sections say nothing that changes while a conversation goes on. Where
an agent runs and what it may do there are facts that the plugins which know
them tell when a turn begins, in a message of their own, so that the system
prompt stays the same text from the first turn to the last, which is what a
cache of the provider goes by.

What a section says of the program has to be so: that context comes in a
frame, that calls which only read run side by side. The tests hold those
sentences to the program.
"""

import re
from importlib import resources

from pydantic import BaseModel, field_validator

from columbina.kernel import Context
from columbina.system_prompt import SYSTEM_PROMPT, Section

__all__ = ["SECTIONS", "Config", "apply", "cut", "inject", "name"]

name = "system-prompt-conduct"
inject = (SYSTEM_PROMPT,)

_FILE = "conduct.md"
_NAMES = {
    "# How you work": "work",
    "# Changing things": "changes",
    "# Taking care": "care",
    "# Using the tools": "tools",
    "# Messages from the program": "program",
    "# What you say": "words",
}
"""The name of each section, by the heading that it stands under in the file."""
_STEP = 100
"""How far the sections are apart, so that one of another plugin can stand between two."""


def cut(text: str) -> dict[str, Section]:
    """Cut the text of the conduct into its sections, each from its heading to the next.

    Returns:
        The sections by their names, in the order of the text and a step apart.

    Raises:
        ValueError: The text does not hold exactly the sections that have a
            name, in their order. A section that nobody could leave out, or
            one that is gone without a word, is a mistake in the file.
    """
    parts = [part.strip() for part in re.split(r"(?m)^(?=# )", text) if part.strip()]
    headings = [part.splitlines()[0] for part in parts]
    if headings != list(_NAMES):
        raise ValueError(
            f"{_FILE} holds the sections {headings}, and the sections with a name are "
            f"{list(_NAMES)}"
        )
    return {
        _NAMES[heading]: Section(part, order=_STEP * (at + 1))
        for at, (heading, part) in enumerate(zip(headings, parts, strict=True))
    }


_HERE = resources.files("columbina.system_prompt")
SECTIONS = cut(_HERE.joinpath(_FILE).read_text(encoding="utf-8"))
"""The sections of the conduct, by the names that a config leaves them out by, in their order."""


class Config(BaseModel):
    """The config of the conduct.

    Attributes:
        leave_out: The names of the sections that are not added.
    """

    leave_out: tuple[str, ...] = ()

    @field_validator("leave_out")
    @classmethod
    def _names_of_sections(cls, names: tuple[str, ...]) -> tuple[str, ...]:
        unknown = [section for section in names if section not in SECTIONS]
        if unknown:
            raise ValueError(
                f"no section of the conduct is called {', '.join(map(repr, unknown))}; "
                f"the sections are {', '.join(SECTIONS)}"
            )
        return names


def apply(ctx: Context, config: Config) -> None:
    """Add the sections of the conduct that the config does not leave out."""
    sections = ctx.use(SYSTEM_PROMPT).sections
    for section, text in SECTIONS.items():
        if section not in config.leave_out:
            sections.add(ctx, f"conduct/{section}", text)
