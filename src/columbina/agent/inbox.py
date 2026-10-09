"""The inbox: input that waits for an agent.

The inbox is not kept beside the session log, it is in it. Every change is an
event, and the queues are a projection of those events, so input that was
waiting when a run ended is still waiting when the session is resumed.
"""

from typing import ClassVar, Literal

from columbina.data import Data
from columbina.llm.types import UserMessage
from columbina.session import EventData, Projection, Session, SessionEvent

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


def claim_for_step(session: Session) -> tuple[InboxItem, ...]:
    """Take all input for the next step. Every step of a turn begins with this."""
    inbox = INBOX.of(session)
    if inbox.next_step:
        session.append(InboxSplicedEvent(queue="next-step", removed=len(inbox.next_step)))
    return inbox.next_step


def claim_task(session: Session) -> tuple[InboxItem, ...]:
    """Take the task that has waited longest, if one waits.

    A turn begins with the input for its first step and then with one task.
    The two are taken by two calls, because taking the first writes an event,
    and a listener of that event may have changed what waits. It may also have
    cancelled the agent, which the caller has to look at before it goes on.
    """
    task = INBOX.of(session).next_turn[:1]
    if task:
        session.append(InboxSplicedEvent(queue="next-turn", removed=1))
    return task


def clear_inbox(session: Session) -> None:
    """Drop everything that waits at this moment.

    Dropping from the first queue writes an event, and a listener of that
    event may take tasks away or give new ones. Of the tasks, only those are
    dropped that waited when this began and still do. What is given after a
    cancellation has begun is not lost to it.
    """
    waiting = INBOX.of(session)
    if waiting.next_step:
        session.append(InboxSplicedEvent(queue="next-step", removed=len(waiting.next_step)))
    tasks = _still_first(INBOX.of(session).next_turn, waiting.next_turn)
    if tasks:
        session.append(InboxSplicedEvent(queue="next-turn", removed=tasks))


def _still_first(now: tuple[InboxItem, ...], before: tuple[InboxItem, ...]) -> int:
    """Count the items at the head of a queue that were already in it ``before``.

    Input leaves a queue at its head and enters at its end, so what is left
    of the earlier items stands first. Items are told apart by identity,
    because the same words may be given twice.
    """
    earlier = {id(item) for item in before}
    count = 0
    for item in now:
        if id(item) not in earlier:
            break
        count += 1
    return count
