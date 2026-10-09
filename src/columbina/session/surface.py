"""The message surface: the part of a session log that the model sees."""

from collections.abc import Sequence

from columbina.llm.types import Message
from columbina.session.errors import InvalidEventError
from columbina.session.events import EventData, SessionEvent


class Surface:
    """The messages that the model sees, as an ordered list of nodes.

    A node is an event that carries a message. A new message becomes the last
    node. Compaction and trimming put one new node in the place of a run of
    neighbouring nodes. The events that such a node shadows stay in the log,
    so the surface can always be rebuilt by applying the events in order.
    """

    __slots__ = ("_messages", "_seqs")

    def __init__(self) -> None:
        self._seqs: list[int] = []
        self._messages: list[Message] = []

    @property
    def seqs(self) -> tuple[int, ...]:
        """The positions in the log of the nodes, in the order the model sees them."""
        return tuple(self._seqs)

    @property
    def messages(self) -> tuple[Message, ...]:
        """The messages of the nodes, in the order the model sees them."""
        return tuple(self._messages)

    def locate(self, replaces: Sequence[int]) -> int:
        """Return the index of the first of the nodes ``replaces``.

        Raises:
            InvalidEventError: The events are not a run of neighbouring nodes.
        """
        for seq in replaces:
            if seq not in self._seqs:
                raise InvalidEventError(f"event {seq} is not on the message surface")
        start = self._seqs.index(replaces[0])
        if self._seqs[start : start + len(replaces)] != list(replaces):
            listed = ", ".join(str(seq) for seq in replaces)
            raise InvalidEventError(
                f"the events {listed} are not neighbours on the message surface"
            )
        return start

    def apply(self, event: SessionEvent[EventData]) -> None:
        """Take in the next event of the log.

        Raises:
            InvalidEventError: The event does not fit the surface as it stands.
        """
        if not event.on_surface:
            if event.replaces:
                raise InvalidEventError(f"event {event.seq} replaces messages without being one")
            return
        message = event.data.for_model()
        if message is None:
            raise InvalidEventError(f"event {event.seq} is marked as a message but holds none")
        if not event.replaces:
            self._seqs.append(event.seq)
            self._messages.append(message)
            return
        start = self.locate(event.replaces)
        stop = start + len(event.replaces)
        self._seqs[start:stop] = [event.seq]
        self._messages[start:stop] = [message]
