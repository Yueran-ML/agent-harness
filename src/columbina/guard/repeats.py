"""Calls that are made again: the same call, with nothing changed since it was last made.

This module is a plugin. Loading it has every call of a tool compared with
the calls that came before it in the same turn.

A model that does not get what it wants may ask for it again, word for word:
the same file read a fifth time, the same search, the same command that
failed. Nothing has changed since the first time, so the call gives what it
gave then, and the turn spends its steps going round. The model is told when
a call comes a third time, beside its result, and the fifth one does not run.

Two calls are the same where the tool and its arguments are: the arguments
are compared as the tool read them, so the order of their keys and the
spaces between them do not count. What counts as a change is a call that
went through and may have changed something: one that writes clears what
was counted, and so does one that runs a command, for every call but itself.
A command that is run again, with nothing written in between, is counted
like any other call. A new turn begins with nothing counted, since the user
may ask for the same thing again.

Only calls that the checks let through are counted. A call that is refused
here, or by a check after this one, is not.
"""

import json
import weakref

from pydantic import BaseModel, Field, model_validator

from columbina.kernel import Context, Next
from columbina.session import SESSION_EVENT, Committed, Session, TurnStartEvent
from columbina.tools import (
    POST_EXECUTE,
    PRE_EXECUTE,
    Accept,
    Allow,
    Decision,
    Deny,
    Executed,
    Invocation,
    Verdict,
)

__all__ = ["REPEATED_CALL", "Config", "apply", "name", "refusal", "warning"]

name = "repeats"

REPEATED_CALL = "REPEATED_CALL"
"""The code of a call that was refused because it was made too often."""


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        warn_at: The time a call comes, counted with itself, at which the
            model is told beside its result.
        refuse_at: The time a call comes at which it no longer runs.
    """

    warn_at: int = Field(default=3, ge=2, strict=True)
    refuse_at: int = Field(default=5, ge=2, strict=True)

    @model_validator(mode="after")
    def _warned_first(self) -> "Config":
        if self.refuse_at <= self.warn_at:
            raise ValueError("refuse_at must come after warn_at")
        return self


def _same(invocation: Invocation) -> str:
    """Return what two calls share where they are the same: the tool and its arguments."""
    arguments = invocation.args.model_dump(mode="json")
    return json.dumps([invocation.tool.name, arguments], sort_keys=True, ensure_ascii=False)


def warning(times: int) -> str:
    """Return what the model is told beside the result of a call that it made again."""
    return (
        f"(This is the same call as before, made {times} times in this turn, and nothing was "
        "changed in between, so it gives what it gave before. Use what you have, or try "
        "another way.)"
    )


def refusal(times: int) -> str:
    """Return why a call that was made too often does not run."""
    return (
        f"it is the same call, made {times - 1} times in this turn already, and nothing was "
        "changed since: it would give what it gave before. Use what you have, or try another "
        "way"
    )


class _Counts:
    """How often each call has been made in the turn of each session since the last change."""

    __slots__ = ("_config", "_made")

    def __init__(self, config: Config) -> None:
        self._config = config
        self._made: weakref.WeakKeyDictionary[Session, dict[str, int]] = weakref.WeakKeyDictionary()

    def heard(self, committed: Committed) -> None:
        """Count afresh where a turn begins."""
        if isinstance(committed.event.data, TurnStartEvent):
            self._made.pop(committed.session, None)

    async def before_a_call(
        self, invocation: Invocation, proceed: Next[Invocation, Decision]
    ) -> Decision:
        """Refuse a call that was made too often, and count the others that are let through."""
        made = self._made.setdefault(invocation.run.session, {})
        same = _same(invocation)
        times = made.get(same, 0) + 1
        if times >= self._config.refuse_at:
            return Deny(refusal(times), REPEATED_CALL)
        decision = await proceed(invocation)
        if isinstance(decision, Allow):
            made[same] = times
        return decision

    async def after_a_call(self, executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        """Tell the model of a call that it made again, and forget what a change has changed."""
        verdict = await proceed(executed)
        invocation = executed.invocation
        made = self._made.setdefault(invocation.run.session, {})
        same = _same(invocation)
        times = made.get(same, 0)
        effect = invocation.tool.effect
        if effect == "write":
            made.clear()
        elif effect == "run":
            made.clear()
            made[same] = times
        if isinstance(verdict, Accept) and times >= self._config.warn_at:
            between = "" if verdict.text.endswith("\n") else "\n"
            return Accept(text=f"{verdict.text}{between}{warning(times)}")
        return verdict


def apply(ctx: Context, config: Config) -> None:
    """Have every call compared with those before it, before any other check is asked."""
    counts = _Counts(config)
    ctx.on(SESSION_EVENT, counts.heard)
    ctx.on(PRE_EXECUTE, counts.before_a_call, prepend=True)
    ctx.on(POST_EXECUTE, counts.after_a_call)
