"""The inbox: input that waits for an agent.

The inbox is not kept beside the session log, it is in it. Every change is an
event, and the queues are a projection of those events, so input that was
waiting when a run ended is still waiting when the session is resumed.
"""

from typing import ClassVar, Literal

from tack.data import Data
from tack.llm.types import UserMessage
from tack.session import EventData, Projection, Session, SessionEvent

Queue = Literal["next-turn", "next-step"]
"""The two queues: tasks that each open a turn, and input for the next step of a turn."""


class InboxItem(Data):
    """One input that waits for an agent.

    Attributes:
        message: What the model will read.
        source: Where the input comes from, as in ``UserMessageEvent``.
    """

    message: UserMessage
    source: str = "user"


class InboxSplicedEvent(EventData):
    """A queue of the inbox changed: items left at its front, or joined at its end."""

    event_type: ClassVar[str] = "agent/inbox/spliced"

    queue: Queue
    removed: int = 0
    added: tuple[InboxItem, ...] = ()


class Inbox(Data):
    """The input that waits for an agent.

    Attributes:
        next_turn: Tasks. Each one opens a turn of its own.
        next_step: Input for the next step. A turn that is running reads it at
            its next step, and a turn that begins reads all of it.
    """

    next_turn: tuple[InboxItem, ...] = ()
    next_step: tuple[InboxItem, ...] = ()

    def __bool__(self) -> bool:
        return bool(self.next_turn or self.next_step)


def _fold(state: Inbox, event: SessionEvent[EventData]) -> Inbox:
    data = event.data
    if not isinstance(data, InboxSplicedEvent):
        return state
    if data.queue == "next-turn":
        return Inbox(
            next_turn=(*state.next_turn[data.removed :], *data.added), next_step=state.next_step
        )
    return Inbox(
        next_turn=state.next_turn, next_step=(*state.next_step[data.removed :], *data.added)
    )


INBOX = Projection("inbox", Inbox(), _fold)
"""The input that waits for the agent of a session."""


def enqueue(session: Session, queue: Queue, item: InboxItem) -> None:
    """Put ``item`` at the end of a queue."""
    session.append(InboxSplicedEvent(queue=queue, added=(item,)))


def claim_for_turn(session: Session) -> tuple[InboxItem, ...]:
    """Take what a turn begins with: all input for the next step, then one task.

    The inbox is looked at anew for the task. Taking the other input writes an
    event, and a listener of that event may have changed what waits: it may
    have cancelled the agent, which drops the tasks.
    """
    for_the_step = claim_for_step(session)
    task = INBOX.of(session).next_turn[:1]
    if task:
        session.append(InboxSplicedEvent(queue="next-turn", removed=1))
    return (*for_the_step, *task)


def claim_for_step(session: Session) -> tuple[InboxItem, ...]:
    """Take what a further step of a turn begins with: all input for the next step."""
    inbox = INBOX.of(session)
    if inbox.next_step:
        session.append(InboxSplicedEvent(queue="next-step", removed=len(inbox.next_step)))
    return inbox.next_step


def clear_inbox(session: Session) -> None:
    """Drop everything that waits.

    Each queue is looked at when its turn comes, for the same reason as in
    :func:`claim_for_turn`.
    """
    for_the_step = len(INBOX.of(session).next_step)
    if for_the_step:
        session.append(InboxSplicedEvent(queue="next-step", removed=for_the_step))
    tasks = len(INBOX.of(session).next_turn)
    if tasks:
        session.append(InboxSplicedEvent(queue="next-turn", removed=tasks))
