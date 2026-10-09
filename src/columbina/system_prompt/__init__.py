"""The system prompt: what stands before a conversation, put together from several hands.

This module is a plugin. Loading it provides :data:`SYSTEM_PROMPT`.

The instructions that open a conversation do not come from one place. The
spec of an agent holds the text of whoever set the agent up. Plugins know
things that belong there too: how a tool of theirs is meant to be used, what
a session may and may not do. Each of them registers a section, and the text
that the model reads is the text of the spec with the sections after it.

Sections come in the order of their numbers, and those with the same number
in the order of their names, so the text does not depend on the order in
which plugins were loaded. A blank line stands between two of them.

A listener of :data:`SYSTEM_PROMPT_ASSEMBLE` is handed the result before it
is used and may put another text in its place.

The loop asks for the text when a turn begins. A text that differs from the
one in the log takes its place there, and the same text is not written
twice, so a session whose sections do not change keeps the beginning of its
conversation as it is, which is what a cache of the provider goes by.
"""

from dataclasses import dataclass
from typing import cast

from columbina.kernel import Context, Registry, ServiceKey, WaterfallEvent

__all__ = [
    "SYSTEM_PROMPT",
    "SYSTEM_PROMPT_ASSEMBLE",
    "Draft",
    "Section",
    "SystemPrompt",
    "apply",
    "name",
]

name = "system-prompt"


@dataclass(frozen=True, slots=True)
class Section:
    """A part of the system prompt that a plugin adds.

    Attributes:
        text: The text.
        order: Where it stands among the sections: lower numbers first.
    """

    text: str
    order: int = 0


@dataclass(frozen=True, slots=True)
class Draft:
    """A system prompt on its way to the model.

    Attributes:
        base: The text of the spec of the agent.
        sections: The sections in their order, each with its name.
        text: All of it put together, which is what is used unless a
            listener answers something else.
    """

    base: str
    sections: tuple[tuple[str, Section], ...]
    text: str


SYSTEM_PROMPT_ASSEMBLE: WaterfallEvent[Draft, str] = WaterfallEvent("system-prompt/assemble")
"""Asked before a system prompt is used. A listener answers the text to use."""


async def _as_it_is(draft: Draft) -> str:
    return draft.text


class SystemPrompt:
    """Puts the system prompt together.

    Attributes:
        sections: The sections, by name.
    """

    __slots__ = ("sections",)

    def __init__(self) -> None:
        self.sections: Registry[Section] = Registry("section of the system prompt")

    async def render(self, ctx: Context, base: str) -> str:
        """Return the system prompt for an agent whose spec holds ``base``.

        Args:
            ctx: The context whose listeners are asked.
            base: The text of the spec. It comes first.

        Raises:
            TypeError: A listener answered something that is no text.
        """
        ordered = tuple(sorted(self.sections.items(), key=lambda item: (item[1].order, item[0])))
        parts = [base.strip(), *(section.text.strip() for _, section in ordered)]
        text = "\n\n".join(part for part in parts if part)
        answer = cast(
            "object",
            await ctx.waterfall(SYSTEM_PROMPT_ASSEMBLE, Draft(base, ordered, text), _as_it_is),
        )
        if not isinstance(answer, str):
            raise TypeError(
                f"a listener of {SYSTEM_PROMPT_ASSEMBLE.name} answered {answer!r}, which is no text"
            )
        return answer


SYSTEM_PROMPT = ServiceKey[SystemPrompt]("system-prompt")
"""The system prompt."""


def apply(ctx: Context, config: object) -> None:
    """Provide the system prompt."""
    ctx.provide(SYSTEM_PROMPT, SystemPrompt())
