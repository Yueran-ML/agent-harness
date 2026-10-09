"""How large the result of a tool may be, and what is done with one that is larger.

This module is a plugin. It listens to ``tools/post-execute``.

A result goes into the conversation and stays there, to be sent again with
every request that follows. One search with thousands of hits can cost more
than the rest of a session. So a result has a limit, counted in tokens the
way :mod:`columbina.token_meter` estimates them. Of a result beyond it, the
model gets the beginning and the end, with a line between them that says how
much is left out, and a last line that says where the whole of it is and how
to read it. The whole is saved through :data:`columbina.spill.SPILL`.

The limit is kept also where nothing can be saved: the program has no such
place, or the place fails. The model is then told that the rest is gone. A
conversation that no longer fits its model is the worse of the two losses.

Two kinds of results are other than the rest.

- A tool that hands out what it has in pages bounds its result by itself and
  says how to ask for more. It is left alone: see ``Tool.paged``.
- A tool may have saved the whole of its output already, as it came, because
  it was never held in memory. It says so by a ``complete`` on what it
  returns, which is what the place said about it. Such a result is not saved
  a second time, and it gets the last line even if it is small: its text has
  a hole in it.

The check comes last. It measures what the other checks of a result have
made of it.
"""

import asyncio
import logging

from pydantic import BaseModel, Field

from columbina.kernel import Context, Next
from columbina.spill import SPILL, Saved
from columbina.token_meter import estimate
from columbina.tools import POST_EXECUTE, Accept, Executed, Verdict

__all__ = ["Config", "apply", "name", "two_ends"]

name = "spill-policy"

logger = logging.getLogger(__name__)

_NOWHERE = "(The whole result was not kept: this program has no place to save it.)"
_SPACE_AT_MOST = 7
"""No kind of character comes to less than a seventh of a token."""


class Config(BaseModel):
    """The config of the limit on results.

    Attributes:
        max_tokens: How many tokens a result may come to, as they are
            estimated.
    """

    max_tokens: int = Field(default=12_500, gt=0)


def _whole(saved: Saved) -> str:
    return f"(The whole result has {saved.bytes} bytes and is in {saved.locator}. {saved.hint})"


def _left_out(count: int) -> str:
    return f"[... {count} characters are left out here ...]"


def _beginning(text: str, tokens: int) -> str:
    """Return the longest beginning of a text that comes to no more than so many tokens.

    It ends with a line if a line ends in its last fifth.
    """
    low, high = 0, min(len(text), tokens * _SPACE_AT_MOST)
    while low < high:
        middle = (low + high + 1) // 2
        if estimate(text[:middle]) <= tokens:
            low = middle
        else:
            high = middle - 1
    broken = text.rfind("\n", low - low // 5, low)
    return text[: broken + 1] if broken >= 0 else text[:low]


def _end(text: str, tokens: int) -> str:
    """Return the longest end of a text that comes to no more than so many tokens.

    It begins with a line if a line begins in its first fifth.
    """
    low, high = 0, min(len(text), tokens * _SPACE_AT_MOST)
    while low < high:
        middle = (low + high + 1) // 2
        if estimate(text[len(text) - middle :]) <= tokens:
            low = middle
        else:
            high = middle - 1
    start = len(text) - low
    broken = text.find("\n", start, start + low // 5)
    return text[broken + 1 :] if broken >= 0 else text[start:]


def two_ends(text: str, tokens: int, last: str) -> str:
    """Return the two ends of a text and a last line, in no more than so many tokens.

    Between the ends stands a line that says how many characters are left
    out. No part of the text is shown twice: the end is taken from what the
    beginning has left.
    """
    # The line between the ends is priced as if everything were left out.
    room = tokens - estimate(last) - estimate(_left_out(len(text))) - 2
    half = max(room // 2, 0)
    head = _beginning(text, half)
    tail = _end(text[len(head) :], half)
    between = "" if not head or head.endswith("\n") else "\n"
    after = "" if not tail or tail.endswith("\n") else "\n"
    gap = _left_out(len(text) - len(head) - len(tail))
    return f"{head}{between}{gap}\n{tail}{after}{last}"


def apply(ctx: Context, config: Config) -> None:
    """Listen for results, and keep each within the limit."""

    async def saved_somewhere(executed: Executed, text: str) -> str:
        """Save a text, and return the line that says where it is, or that it is gone."""
        store = ctx.get(SPILL)
        if store is None:
            return _NOWHERE
        run = executed.invocation.run
        named = f"{executed.invocation.tool.name}-result.txt"
        try:
            saved = await asyncio.to_thread(store.save_text, run.session, run.cwd, named, text)
        except OSError as error:
            logger.error("a result of %s could not be saved: %s", named, error)
            return f"(The whole result could not be saved: {error.strerror or error}.)"
        return _whole(saved)

    async def within_the_limit(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        verdict = await proceed(executed)
        if not isinstance(verdict, Accept) or executed.invocation.tool.paged:
            return verdict
        text = verdict.text
        complete: object = getattr(executed.value, "complete", None)
        too_large = estimate(text) > config.max_tokens
        if isinstance(complete, Saved):
            last = _whole(complete)
        elif too_large:
            last = await saved_somewhere(executed, text)
        else:
            return verdict
        if not too_large:
            between = "" if text.endswith("\n") else "\n"
            return Accept(text=f"{text}{between}{last}")
        return Accept(text=two_ends(text, config.max_tokens, last))

    ctx.on(POST_EXECUTE, within_the_limit, prepend=True)
