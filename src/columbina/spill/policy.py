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
  a second time. It ends with the line that says where the whole is, even if
  it is small: its text has a hole in it. The tool may have put that line
  there itself, and then it is not said twice. Where such a result is cut,
  the line between its ends gives no number. How much the tool had left out
  before is not known here.

The check comes last. It measures what the other checks of a result have
made of it.
"""

import asyncio
import logging
import math

from pydantic import BaseModel, Field

from columbina.kernel import Context, Next
from columbina.session import EventData, Projection, RequestHeaderEvent, Session, SessionEvent
from columbina.spill import LIMIT, SPILL, Saved
from columbina.token_meter import estimate
from columbina.tools import POST_EXECUTE, Accept, Executed, Verdict

__all__ = ["Config", "allowed", "apply", "name", "two_ends"]

name = "spill-policy"

logger = logging.getLogger(__name__)

_NOWHERE = "(The whole result was not kept: this program has no place to save it.)"
_UNCOUNTED = "[... a part is left out here ...]"
"""The line between two ends where nobody knows how much is left out."""
_SPACE_AT_MOST = 7
"""No kind of character comes to less than a seventh of a token."""


class Config(BaseModel):
    """The config of the limit on results.

    Attributes:
        max_tokens: How many tokens a result may come to, as they are
            estimated.
        window_ratio: The part of the window of its model that a result may
            fill, where that is less.
    """

    max_tokens: int = Field(default=12_500, gt=0)
    window_ratio: float = Field(default=0.25, gt=0, le=1)


def _asked_last(state: int | None, event: SessionEvent[EventData]) -> int | None:
    data = event.data
    return data.config.context_window if isinstance(data, RequestHeaderEvent) else state


_WINDOW: Projection[int | None] = Projection("result-limit-window", None, _asked_last)
"""How many tokens the model takes that a session asked last, if anything says so."""

_LEAST = 256
"""How many tokens a result may come to at the least, however little its model takes.

What takes the place of a result that is cut says in a line what was left
out and in another where the whole is. Those two come to some tens of
tokens. A result that might hold little more than them would be longer cut
than it was whole.
"""


def allowed(config: Config, window: int | None) -> int:
    """Return how many tokens a result may come to for a model that takes so many.

    The limit of the config was chosen for models that take a hundred
    thousand tokens and more. A model that takes ten thousand would have its
    conversation filled by one result, and what was said before would have
    to go to make room for it. So a result may fill a part of the window and
    no more. Where nothing says how much a model takes, the limit of the
    config stands.

    The part of a window does not go below a least, which is there for a
    window that is written down wrongly: no model that this program can work
    with takes so little. The limit of the config is never raised by it.
    """
    if window is None:
        return config.max_tokens
    part = max(math.floor(window * config.window_ratio), _LEAST)
    return min(config.max_tokens, part)


class _Limit:
    """Says how large a result may be in a session, by the model that the session asks."""

    __slots__ = ("_config",)

    def __init__(self, config: Config) -> None:
        self._config = config

    def tokens(self, session: Session) -> int:
        return allowed(self._config, _WINDOW.of(session))


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


def two_ends(text: str, tokens: int, last: str, *, counted: bool = True) -> str:
    """Return the two ends of a text and a last line, in no more than so many tokens.

    Between the ends stands a line that says how many characters are left
    out, or only that some are if they are not to be counted. No part of the
    text is shown twice: the end is taken from what the beginning has left.
    """
    # The line between the ends is priced as if everything were left out.
    widest = _left_out(len(text)) if counted else _UNCOUNTED
    room = tokens - estimate(last) - estimate(widest) - 2
    half = max(room // 2, 0)
    head = _beginning(text, half)
    tail = _end(text[len(head) :], half)
    between = "" if not head or head.endswith("\n") else "\n"
    after = "" if not tail or tail.endswith("\n") else "\n"
    gap = _left_out(len(text) - len(head) - len(tail)) if counted else _UNCOUNTED
    return f"{head}{between}{gap}\n{tail}{after}{last}"


def apply(ctx: Context, config: Config) -> None:
    """Listen for results, and keep each within the limit. Say what the limit is."""
    limit = _Limit(config)

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
        return saved.told()

    async def within_the_limit(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        verdict = await proceed(executed)
        if not isinstance(verdict, Accept) or executed.invocation.tool.paged:
            return verdict
        text = verdict.text
        complete: object = getattr(executed.value, "complete", None)
        tokens = limit.tokens(executed.invocation.run.session)
        too_large = estimate(text) > tokens
        counted = True
        if isinstance(complete, Saved):
            last = complete.told()
            # The tool may have said so itself. It is said once, and it is said last.
            text = text.removesuffix(last)
            # What the tool saved may be more than its text ever held.
            counted = False
        elif too_large:
            last = await saved_somewhere(executed, text)
        else:
            return verdict
        if not too_large:
            between = "" if text.endswith("\n") else "\n"
            return Accept(text=f"{text}{between}{last}")
        return Accept(text=two_ends(text, tokens, last, counted=counted))

    ctx.on(POST_EXECUTE, within_the_limit, prepend=True)
    ctx.provide(LIMIT, limit)
