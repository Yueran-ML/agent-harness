"""Whether a turn did what was asked, as the person who asked says.

This module is a plugin. Loading it has the logs of sessions take
:class:`VerdictEvent`. The command ``/verdict`` is a plugin of its own, see
:mod:`columbina.commands.verdict`.

How a turn ended says whether the model came to an end, not whether what it
did was right: a turn that ends well may have changed the wrong file. Only the
person who asked can say that, and ``/verdict right`` or ``/verdict wrong``,
with a note after it if they like, writes it into the log for the last turn
that came to its end. ``columbina sessions stats`` counts what was said. A
verdict that is said again for the same turn takes the place of the one
before.
"""

from typing import ClassVar

from columbina.kernel import Context
from columbina.session import SESSIONS, EventData, Session, TurnEndEvent

__all__ = ["VerdictEvent", "apply", "inject", "judge", "last_turn", "name"]

name = "verdicts"
inject = (SESSIONS,)


class VerdictEvent(EventData):
    """What the person who asked says of a turn.

    Attributes:
        turn: The number of the turn.
        right: Whether it did what was asked.
        note: What they said beside, if anything.
    """

    event_type: ClassVar[str] = "verdict"
    ignorable: ClassVar[bool] = True

    turn: int
    right: bool
    note: str | None = None


def last_turn(session: Session) -> int | None:
    """Return the number of the last turn of a session that came to its end, if there is one."""
    for event in reversed(session.events):
        if isinstance(event.data, TurnEndEvent):
            return event.data.turn
    return None


def judge(session: Session, right: bool, note: str | None = None, turn: int | None = None) -> int:
    """Write down what a person says of a turn, the last that ended unless one is named.

    Returns:
        The number of the turn.

    Raises:
        ValueError: No turn has ended, or the one that was named has not.
    """
    ended = last_turn(session)
    if ended is None:
        raise ValueError("no turn has come to its end yet")
    if turn is None:
        turn = ended
    elif not 1 <= turn <= ended:
        raise ValueError(f"turn {turn} has not come to its end")
    session.append(VerdictEvent(turn=turn, right=right, note=note or None))
    return turn


def apply(ctx: Context, config: object) -> None:
    """Have the logs take verdicts."""
    ctx.use(SESSIONS).declare(ctx, VerdictEvent)
