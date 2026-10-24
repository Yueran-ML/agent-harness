"""The language that a model reasons in, as the user chose it, told when a turn begins.

This module is a plugin. Loading it registers the source of context
``reasoning-language`` with the agent service.

A model reasons in the language of the conversation, or of the files it reads,
and a user who reads along may want another one. The user chooses it with
``columbina reasoning-language``, and the choice is kept in a file that holds
the code of the language alone, such as ``zh``. The file is read when a turn
begins, so a choice holds from the next turn on, in every conversation.

What the model is told is a message of its own and no part of the system
prompt. When the choice changes, the new word follows the conversation, and
its beginning stays as it was: a provider keeps that from one request to the
next, and some bind the thinking of their model to it. Where no language is
chosen any more, the model is told that too.
"""

import asyncio
from pathlib import Path

from pydantic import BaseModel

from columbina.agent import AGENTS, CONTEXT_PREFIX, Agent, ContextSource
from columbina.kernel import Context
from columbina.session import EventData, Projection, SessionEvent, UserMessageEvent

__all__ = ["LANGUAGES", "Config", "apply", "choose", "chosen", "inject", "name"]

name = "context-reasoning-language"
inject = (AGENTS,)

LANGUAGES = {"zh": "Chinese (中文)", "en": "English", "ja": "Japanese (日本語)"}
"""The languages that can be chosen, by their codes, as the model is told of them."""

_NAME = "reasoning-language"
_SOURCE = CONTEXT_PREFIX + _NAME
_FREE = (
    "No language is asked for your reasoning any more. Reason in whatever language suits the task."
)


class Config(BaseModel):
    """The config of the language of reasoning.

    Attributes:
        file: The file that holds the code of the language that was chosen.
            Where there is none, or it holds no code that is known, no
            language is asked for.
    """

    file: Path | None = None


def chosen(path: Path) -> str | None:
    """Return the code of the language that the file names, or ``None`` if it names none."""
    try:
        code = path.read_text(encoding="utf-8").strip()
    except (OSError, UnicodeDecodeError):
        # A file that cannot be read names nothing that the user could have meant.
        return None
    return code if code in LANGUAGES else None


def choose(path: Path, code: str | None) -> None:
    """Keep the code of a language in the file, or take the file away for none.

    Raises:
        ValueError: The code is no language that can be chosen.
        OSError: The file cannot be written or taken away.
    """
    if code is None:
        path.unlink(missing_ok=True)
        return
    if code not in LANGUAGES:
        raise ValueError(f"{code!r} is no language that can be chosen: {', '.join(LANGUAGES)}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"{code}\n", encoding="utf-8")


def _told(language: str) -> str:
    return (
        f"Reason in {language}: write your thinking before you answer in it, whatever language "
        "the conversation, the files or the tools use. Answer the user in the language that they "
        "write in, as before."
    )


def _last_given(state: bool, event: SessionEvent[EventData]) -> bool:
    data = event.data
    return state or (isinstance(data, UserMessageEvent) and data.source == _SOURCE)


_GIVEN: Projection[bool] = Projection("reasoning-language-given", False, _last_given)
"""Whether a language of reasoning was ever told of in a session."""


def apply(ctx: Context, config: Config) -> None:
    """Register the source of context that tells the language of reasoning."""
    file = config.file

    async def render(agent: Agent) -> str | None:
        code = None if file is None else await asyncio.to_thread(chosen, file)
        if code is None:
            return _FREE if _GIVEN.of(agent.session) else None
        return _told(LANGUAGES[code])

    ctx.use(AGENTS).context.add(ctx, _NAME, ContextSource(render, order=10, kind="replace"))
