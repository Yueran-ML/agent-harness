"""Old results of tools, cut down to their two ends.

Most of what a long conversation holds is what tools gave back: files that
were read, the output of commands. The model has read each of them once and
has gone on. What it needs of them later is seldom more than how they began
and how they ended. So before anything is summed up, the long results that
the model has read are cut down. That costs no request to a model, and often
it is enough.

A result that the model has not read yet is not cut. The results that follow
its last reply are such: the request that shows them to it is still to come.

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

from collections.abc import Iterable
from dataclasses import dataclass

from columbina.llm import TextBlock
from columbina.session import AssistantMessageEvent, Session, ToolResultEvent

__all__ = ["Cut", "apply", "cuts", "located", "trim", "whole"]


@dataclass(frozen=True, slots=True)
class Cut:
    """A result that can be replaced by a shorter one.

    Attributes:
        seq: Where the result stands in the log.
        whole: The result as it is.
        shorter: What would take its place.
    """

    seq: int
    whole: ToolResultEvent
    shorter: ToolResultEvent


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
    events = [session.events[seq] for seq in session.surface]
    replies = [
        at for at, event in enumerate(events) if isinstance(event.data, AssistantMessageEvent)
    ]
    read = events[: replies[-1]] if replies else []
    found: list[Cut] = []
    for event in read:
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
            cut. The three numbers are those that the cut was found with.
        head: How many characters of its beginning are kept.
        tail: How many characters of its end are kept.
    """
    content: list[TextBlock] = []
    for before, after in zip(cut.whole.message.content, cut.shorter.message.content, strict=True):
        told = after
        if after != before:
            longer = _two_ends(before.text, over, head, tail, where)
            if longer != before.text:
                told = TextBlock(text=longer)
        content.append(told)
    message = cut.shorter.message.model_copy(update={"content": tuple(content)})
    return Cut(cut.seq, cut.whole, cut.shorter.model_copy(update={"message": message}))


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
