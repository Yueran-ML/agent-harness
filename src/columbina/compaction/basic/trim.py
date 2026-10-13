"""Old results of tools, cut down to their two ends.

Most of what a long conversation holds is what tools gave back: files that
were read, the output of commands. The model has read each of them once and
has gone on. What it needs of them later is seldom more than how they began
and how they ended. So before anything is summed up, the long results that
the model has read are cut down. That costs no request to a model, and often
it is enough.

A result that the model has not read yet is not cut with those. The results
that follow its last reply are such: the request that shows them to it is
still to come. They give way only where cutting what was read is not enough,
and only as far as together they fill more of the window of the model than
the results of one reply should. How far is counted in tokens, and not in
characters as with what was read: a text in Chinese costs a model several
times what one in English of the same length does, and a model that takes
little is filled by results that are not long. See :func:`level` and
:func:`capped`.

A result that was saved somewhere because it was too large says so in its
last line. That line is at the end, and the end is kept.

What is cut out of a result is gone from the conversation, and a model may
need it again: the middle of a long output cannot always be made a second
time. So whoever cuts a result may save the whole of it first, and have the
line that says where it is put after the line that says what is left out.
See :func:`whole` and :func:`located`.

Finding what would be cut and cutting it are two moves. Whoever makes a
conversation shorter looks first whether this alone would be enough. If it
is not, the oldest part is summed up as it stands, and what is cut
afterwards is only what the summary has left.
"""

from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass

from columbina.llm import TextBlock
from columbina.session import (
    AssistantMessageEvent,
    EventData,
    Session,
    SessionEvent,
    ToolResultEvent,
)
from columbina.spill import Saved

__all__ = ["Cut", "apply", "capped", "cuts", "level", "located", "trim", "unread", "whole"]

_WORTH = 512
"""By how many characters a block of text has to get shorter for its result to give way.

A cut that says where the whole of a result is has two lines more than the
result had. One that took fewer characters away than those come to would
leave a model no less to read, and no room for the line about the whole.
"""


@dataclass(frozen=True, slots=True)
class Cut:
    """A result that can be replaced by a shorter one.

    Attributes:
        seq: Where the result stands in the log.
        whole: The result as it is.
        shorter: What would take its place.
        kept: How many characters of each block of its text are kept at
            most, if it was cut to fit a cap: see :func:`capped`. ``None``
            if it was cut by the numbers that are the same for every result.
    """

    seq: int
    whole: ToolResultEvent
    shorter: ToolResultEvent
    kept: int | None = None


def _left_out(count: int) -> str:
    return f"[... {count} characters of this result were left out to shorten the conversation ...]"


def _two_ends(text: str, over: int, head: int, tail: int, where: str = "") -> str:
    """Return the two ends of a long text, with a line between them that says what is left out.

    The text has more than `over` characters. It is returned as it is if what would take its
    place would be long itself: that would be cut again the next time, and the line in it
    would count wrongly. So whatever takes the place of a text is shorter than the text.

    A line that says where the whole is stands after the line about what is left out, if
    one is given.
    """
    count = len(text) - head - tail
    if count <= 0:
        # The two ends reach each other: nothing would be left out.
        return text
    first, last = text[:head], text[len(text) - tail :]
    # The line stands on its own, whether or not the beginning ends a line.
    between = "" if not first or first.endswith("\n") else "\n"
    gap = f"{_left_out(count)}\n{where}" if where else _left_out(count)
    cut = f"{first}{between}{gap}\n{last}"
    return cut if len(cut) <= over else text


def _parted(
    session: Session,
) -> tuple[list[SessionEvent[EventData]], list[SessionEvent[EventData]]]:
    """Return what the model has read of a session, and what follows its last reply."""
    events = [session.events[seq] for seq in session.surface]
    replies = [
        at for at, event in enumerate(events) if isinstance(event.data, AssistantMessageEvent)
    ]
    read = events[: replies[-1]] if replies else []
    return read, events[len(read) :]


def unread(session: Session) -> list[ToolResultEvent]:
    """Return the results that the model has not read yet: those that follow its last reply."""
    results = (event.data for event in _parted(session)[1])
    return [data for data in results if isinstance(data, ToolResultEvent)]


def cuts(session: Session, *, over: int, head: int, tail: int) -> list[Cut]:
    """Find the long results that the model has read, and what each would be cut down to.

    The shorter result differs from the whole one in its text alone, so the
    model goes on seeing which call it answers and whether the call failed.
    A result of several blocks of text has each block cut by itself.

    Args:
        session: The session whose messages are looked through.
        over: How many characters a block of text may have without being
            cut.
        head: How many characters of its beginning are kept.
        tail: How many characters of its end are kept.
    """
    found: list[Cut] = []
    for event in _parted(session)[0]:
        data = event.data
        if not isinstance(data, ToolResultEvent):
            continue
        content = tuple(
            block
            if len(block.text) <= over
            else TextBlock(text=_two_ends(block.text, over, head, tail))
            for block in data.message.content
        )
        if content == data.message.content:
            continue
        message = data.message.model_copy(update={"content": content})
        found.append(Cut(event.seq, data, data.model_copy(update={"message": message})))
    return found


def level(sizes: Sequence[int], room: int) -> int:
    """Return the most that each of some results may come to, so that together they have room.

    The results share their room. One that comes to less than its share
    stays as it is, and what it leaves goes to the others. So the largest
    give way first, and none further than is needed: with every result that
    comes to more cut down to what is returned, they come to no more than
    the room together, and with one token more for each they would.

    Where all of them have room as they are, what is returned is what the
    largest comes to: none comes to more.
    """
    left, waiting = room, len(sizes)
    for one in sorted(sizes):
        if one * waiting > left:
            return left // waiting
        left -= one
        waiting -= 1
    return max(sizes, default=0)


def _named(text: str) -> int:
    """Return how many characters at the end of a text are a line about where its whole is.

    A result that was too large when it was made ends with such a line. A
    text that does not has none.
    """
    if not Saved.spoken_of_in(text):
        return 0
    return len(text) - (text.rstrip().rfind("\n") + 1)


def _to(text: str, kept: int, where: str = "") -> str:
    """Return a text of which no more than so many characters of its two ends are kept.

    Four fifths of them are of its beginning. A last line about where the
    whole is is kept however few they are: it is what a model finds the rest
    by.

    The text comes back as it is unless it gets shorter by enough to be
    worth it. Once a line is given that says where the whole is, it need
    only get shorter: that line was reckoned with when it was found worth
    it.
    """
    tail = max(kept // 5, _named(text))
    head = max(kept - tail, 0)
    return _two_ends(text, len(text) - (1 if where else _WORTH), head, tail, where)


def _kept(data: ToolResultEvent, kept: int) -> ToolResultEvent:
    """Return a result with each block of its text cut to so many characters at most."""
    cut = [(block, _to(block.text, kept)) for block in data.message.content]
    content = tuple(block if text == block.text else TextBlock(text=text) for block, text in cut)
    # Its text alone changes: which call it answers and whether that failed stay as they are.
    shorter = data.message.model_copy(update={"content": content})
    return data.model_copy(update={"message": shorter})


def capped(session: Session, *, cap: int, size: Callable[[ToolResultEvent], int]) -> list[Cut]:
    """Find the unread results that come to more than a cap, and what each would be cut to.

    A result is cut to its two ends, as in :func:`cuts`. How much of them is
    kept is no number that is the same for every result. It is as many
    characters as come to the cap, which is counted the way the rest of the
    conversation is: so a result gives way by what it costs, whatever is
    written in it. Each block of text keeps the same number at most.

    A result that was cut before is passed over. It stands in the
    conversation as two ends with a line between them about where its whole
    is, and cut again it would lose that line with its middle.

    Args:
        session: The session whose messages are looked through.
        cap: What a result may come to.
        size: Says what a result comes to.
    """
    found: list[Cut] = []
    for event in _parted(session)[1]:
        data = event.data
        if not isinstance(data, ToolResultEvent) or event.replaces or size(data) <= cap:
            continue
        # The most characters with which it comes to no more than the cap. Where none
        # does, nothing of its ends is kept.
        low, high = 0, max((len(block.text) for block in data.message.content), default=0)
        while low < high:
            middle = (low + high + 1) // 2
            if size(_kept(data, middle)) <= cap:
                low = middle
            else:
                high = middle - 1
        shorter = _kept(data, low)
        if shorter != data:
            found.append(Cut(event.seq, data, shorter, low))
    return found


def whole(cut: Cut) -> str:
    """Return all the text of a result as it was before it was cut, block after block."""
    return "\n".join(block.text for block in cut.whole.message.content)


def located(cut: Cut, where: str, *, over: int, head: int, tail: int) -> Cut:
    """Return a cut whose shorter result also says where the whole one is.

    The line stands after the one about what is left out, in every block
    that is cut. A block that would be long itself with one line more keeps
    the plain line: it would be cut again the next time.

    Args:
        cut: The cut, as :func:`cuts` found it.
        where: The line that says where the whole is.
        over: How many characters a block of text may have without being
            cut. The three numbers are those that the cut was found with. A
            cut that was found by a cap has its own number, and these do not
            count for it.
        head: How many characters of its beginning are kept.
        tail: How many characters of its end are kept.
    """
    content: list[TextBlock] = []
    for before, after in zip(cut.whole.message.content, cut.shorter.message.content, strict=True):
        told = after
        if after != before:
            if cut.kept is None:
                longer = _two_ends(before.text, over, head, tail, where)
            else:
                longer = _to(before.text, cut.kept, where)
            if longer != before.text:
                told = TextBlock(text=longer)
        content.append(told)
    message = cut.shorter.message.model_copy(update={"content": tuple(content)})
    return Cut(cut.seq, cut.whole, cut.shorter.model_copy(update={"message": message}), cut.kept)


def apply(session: Session, found: Iterable[Cut]) -> int:
    """Put each shorter result in the place of its whole one, and return how many were put.

    A result that the model no longer sees is passed over: a summary may
    have taken its place since it was found.
    """
    seen = set(session.surface)
    put = 0
    for cut in found:
        if cut.seq in seen:
            session.append(cut.shorter, replaces=[cut.seq])
            put += 1
    return put


def trim(session: Session, *, over: int, head: int, tail: int) -> int:
    """Cut down the long results that the model has read, and return how many were cut."""
    return apply(session, cuts(session, over=over, head=head, tail=tail))
