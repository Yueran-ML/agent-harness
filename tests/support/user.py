"""Somebody at a terminal, for tests that go through the command line.

This module is a plugin. The profile of a test loads it. It provides the
terminal that a conversation is held in, with a screen that keeps what it
shows, and it presses keys the way its script says: every step waits for
something, and then types.

The screens of the terminals that were made are kept in :data:`SCREENS`, the
last one last, so that a test can read what was shown after the program has
ended.
"""

import asyncio
import contextlib
from collections.abc import Callable

from prompt_toolkit.input import PipeInput, create_pipe_input
from pydantic import BaseModel

from columbina.agent import AGENTS
from columbina.apps.cli.terminal import TERMINAL, Terminal
from columbina.kernel import Context
from columbina.session import TURN
from support.terminal import Screen, until

__all__ = ["SCREENS", "Config", "Step", "apply", "name"]

name = "test-user"

SCREENS: list[Screen] = []

_ASKED = " asks for a yes"
_ANSWERED = ("(allowed, ", "(not allowed)", "(the question was withdrawn)")


class Step(BaseModel):
    """One thing that the user does, once what it waits for has happened.

    Attributes:
        sees: Text that has to be printed, after everything that the steps
            before this one waited for.
        rests: Whether to wait until a turn has ended and none is running.
        keys: The keys to press then.
    """

    sees: str = ""
    rests: bool = False
    keys: str = ""


class Config(BaseModel):
    """The config of the user of a test.

    Attributes:
        script: What the user does, step by step.
        answer: The key that answers every question, or nothing if the steps
            of the script answer the questions. Where a question does not offer
            this key, the answer is a yes for this once: a real model decides
            what is asked about, and not every call can be allowed for longer.
        seconds: How long a step waits.
        rows: The height of the window of the terminal.
        columns: Its width.
    """

    script: tuple[Step, ...] = ()
    answer: str = ""
    seconds: float = 30.0
    rows: int = 24
    columns: int = 80


def _printed(screen: Screen) -> str:
    return "".join(screen.printed).replace("\r\n", "\n")


def _shows(screen: Screen, text: str, after: int) -> Callable[[], bool]:
    return lambda: text in _printed(screen)[after:]


def _at_rest(ctx: Context) -> bool:
    """Tell whether an agent is there that has had a turn and runs none."""
    agents = ctx.get(AGENTS)
    if agents is None:
        return False
    states = [TURN.of(agent.session) for agent in agents.live.values()]
    return bool(states) and all(state.turn > 0 and not state.in_turn for state in states)


async def _act(ctx: Context, config: Config, keys: PipeInput, screen: Screen) -> None:
    """Do what the script says."""
    after = 0
    for step in config.script:
        if step.sees:
            await until(_shows(screen, step.sees, after), config.seconds)
            after = _printed(screen).index(step.sees, after) + len(step.sees)
        if step.rests:
            await until(lambda: _at_rest(ctx), config.seconds)
        keys.send_text(step.keys)


async def _answer(config: Config, keys: PipeInput, screen: Screen) -> None:
    """Answer every question once it is on the screen.

    A question is printed when the prompt has become the question, so a key
    that is pressed after the text is there is taken for the answer.
    """
    answered = 0
    while True:
        printed = _printed(screen)
        asked = printed.count(_ASKED)
        closed = sum(printed.count(mark) for mark in _ANSWERED)
        if asked > answered and closed >= answered:
            question = printed[printed.rindex(_ASKED) :]
            offered = f"\n  {config.answer}  " in question
            keys.send_text(config.answer if offered else "y")
            answered += 1
        await asyncio.sleep(0.05)


def apply(ctx: Context, config: Config) -> None:
    """Provide a terminal, and sit at it."""
    stack = contextlib.ExitStack()
    keys = stack.enter_context(create_pipe_input())
    screen = Screen(config.rows, config.columns)
    SCREENS.append(screen)
    ctx.provide(TERMINAL, Terminal(keys, screen, colours=False))
    acting = [asyncio.ensure_future(_act(ctx, config, keys, screen))]
    if config.answer:
        acting.append(asyncio.ensure_future(_answer(config, keys, screen)))

    def sit() -> Callable[[], None]:
        def leave() -> None:
            for task in acting:
                task.cancel()
            stack.close()

        return leave

    ctx.effect(sit)
