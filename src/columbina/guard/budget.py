"""The budget of a turn: how many steps, how many tokens and how much time it may take.

This module is a plugin. Loading it has every step of every agent asked
about before it begins.

The loop counts nothing. It asks the listeners of ``agent/pre-step`` before
each step, and a turn ends as ``blocked`` when one of them refuses. This
plugin is such a listener. It refuses the step that would go over one of
three limits, and the end of the turn says which one.

The steps of a turn are counted by the loop: a step is one request to the
model and the calls that the reply asks for. The tokens of a turn are what
its requests have cost so far, all four counts of the usage together. The
time of a turn is counted from the moment its first step was asked about.

A limit comes from the config of this plugin and holds for every agent. The
spec of an agent may name the steps of its own turns, and that number counts
before the one of the config: a subagent is created with fewer steps than
the conversation that delegates to it.

A turn is not cut off in the middle of a step. The step that is under way
when a limit is reached runs to its end, with its calls, and the next one
does not begin.

The model is told of the steps before they run out, since it cannot count
them itself: when a few are left, and when the step that begins is the last.
It is told in a message of its log that the step begins with, which is read
as context and is no word of the user. So it can save what it found and say
what it did while it still has a step for it.
"""

import time
import weakref
from dataclasses import dataclass

from pydantic import BaseModel, Field

from columbina.agent import CONTEXT_PREFIX, PRE_STEP, Reject, StepDecision, StepInfo
from columbina.kernel import Context, Next
from columbina.llm import TextBlock, Usage, UserMessage
from columbina.session import USAGE, Session, UserMessageEvent

__all__ = ["STEPS_LEFT", "STEP_BUDGET", "TIME_BUDGET", "TOKEN_BUDGET", "Config", "apply", "name"]

name = "turn-budget"

STEP_BUDGET = "STEP_BUDGET"
"""The code of a turn that ended because it had taken its steps."""
TOKEN_BUDGET = "TOKEN_BUDGET"
"""The code of a turn that ended because it had used its tokens."""
TIME_BUDGET = "TIME_BUDGET"
"""The code of a turn that ended because it had run for its time."""

_now = time.monotonic
"""The clock that the time of a turn is read from."""

STEPS_LEFT = CONTEXT_PREFIX + "steps-left"
"""The source of the message that tells the model how many steps its turn has left."""

_FEW = 5
"""How many steps are left, this one among them, when the model is first told."""


def _steps_left(steps: int, step: int) -> str | None:
    """Return what the model is told when a step begins, or ``None`` if nothing."""
    left = steps - step + 1
    if left == 1:
        return (
            f"This is the last step of this turn: it may take {steps}, and this is step {step}. "
            "A tool that you call now runs, but you will not see what it gives back. Give your "
            "answer now: what you did, what you found, and what is left undone."
        )
    if left == _FEW:
        return (
            f"This turn may take {steps} steps, and this is step {step}: {left} are left, this "
            "one among them. Finish what matters most, and save what you found where your task "
            "says, before the steps run out. Keep the last step for your answer."
        )
    return None


class Config(BaseModel):
    """The config of the budget.

    Attributes:
        max_steps: How many steps a turn may take, unless the spec of its
            agent says. Without it the steps are not limited.
        max_tokens: How many tokens the requests of a turn may cost together.
            Without it the tokens are not limited.
        max_seconds: How long a turn may run. Without it the time is not
            limited.
    """

    max_steps: int | None = Field(default=None, ge=1, strict=True)
    max_tokens: int | None = Field(default=None, ge=1, strict=True)
    max_seconds: float | None = Field(default=None, gt=0)


@dataclass(frozen=True, slots=True)
class _Begun:
    """Where a session stood when the turn that is running began.

    Attributes:
        turn: The number of the turn.
        tokens: What the session had cost by then.
        at: When the first step of the turn was asked about.
    """

    turn: int
    tokens: int
    at: float


def _tokens(usage: Usage) -> int:
    """Return what a usage comes to: everything that was sent and received."""
    return (
        usage.input_tokens
        + usage.output_tokens
        + usage.cache_read_tokens
        + usage.cache_write_tokens
    )


class _Budget:
    """Asks about every step whether its turn still has what the step needs."""

    __slots__ = ("_begun", "_config")

    def __init__(self, config: Config) -> None:
        self._config = config
        self._begun: weakref.WeakKeyDictionary[Session, _Begun] = weakref.WeakKeyDictionary()

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Refuse the step that would take its turn over a limit."""
        over = self._over(info)
        if over is not None:
            # Nobody else is asked about a step that does not begin.
            return Reject(*over)
        steps = info.agent.spec.max_steps or self._config.max_steps
        told = None if steps is None else _steps_left(steps, info.step)
        if told is not None:
            source = STEPS_LEFT.removeprefix(CONTEXT_PREFIX)
            framed = f'<context source="{source}">\n{told}\n</context>'
            note = UserMessage(content=(TextBlock(text=framed),))
            info.agent.session.append(UserMessageEvent(message=note, source=STEPS_LEFT))
        return await proceed(info)

    def _over(self, info: StepInfo) -> tuple[str, str] | None:
        """Say which limit a step would go over, or ``None`` if it goes over none.

        It is said in words for a person and in a code for a program.
        """
        config = self._config
        session = info.agent.session
        spent = _tokens(USAGE.of(session).usage)
        begun = self._begun.get(session)
        if begun is None or begun.turn != info.turn:
            begun = self._begun[session] = _Begun(info.turn, spent, _now())
        steps = info.agent.spec.max_steps or config.max_steps
        if steps is not None and info.step > steps:
            return f"the turn has taken its {steps} step{'' if steps == 1 else 's'}", STEP_BUDGET
        used = spent - begun.tokens
        if config.max_tokens is not None and used >= config.max_tokens:
            return f"the turn has used {used} tokens, and it has {config.max_tokens}", TOKEN_BUDGET
        passed = _now() - begun.at
        if config.max_seconds is not None and passed >= config.max_seconds:
            told = f"the turn has run for {passed:.0f} seconds, and it has {config.max_seconds:g}"
            return told, TIME_BUDGET
        return None


def apply(ctx: Context, config: Config) -> None:
    """Have every step asked about before it begins, and before anybody else is asked."""
    ctx.on(PRE_STEP, _Budget(config).before_a_step, prepend=True)
