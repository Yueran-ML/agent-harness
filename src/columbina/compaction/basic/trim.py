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
"""

from columbina.llm import TextBlock
from columbina.session import AssistantMessageEvent, Session, ToolResultEvent

__all__ = ["trim"]


def _left_out(count: int) -> str:
    return f"[... {count} characters of this result were left out to shorten the conversation ...]"


def _two_ends(text: str, head: int, tail: int) -> str:
    """Return the two ends of a text, with a line between them that says what is left out.

    The text itself is returned if that would be no shorter.
    """
    count = len(text) - head - tail
    if count <= 0:
        # The two ends reach each other: nothing would be left out.
        return text
    first, last = text[:head], text[len(text) - tail :]
    # The line stands on its own, whether or not the beginning ends a line.
    between = "" if not first or first.endswith("\n") else "\n"
    cut = f"{first}{between}{_left_out(count)}\n{last}"
    return cut if len(cut) < len(text) else text


def trim(session: Session, *, over: int, head: int, tail: int) -> int:
    """Cut down the long results that the model has read, and return how many were cut.

    Each one is replaced where it stands by a result that differs from it in
    its text alone, so the model goes on seeing which call it answers and
    whether the call failed. A result of several blocks of text has each
    block cut by itself.

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
    cut = 0
    for event in read:
        data = event.data
        if not isinstance(data, ToolResultEvent):
            continue
        content = tuple(
            block if len(block.text) <= over else TextBlock(text=_two_ends(block.text, head, tail))
            for block in data.message.content
        )
        if content == data.message.content:
            continue
        message = data.message.model_copy(update={"content": content})
        session.append(data.model_copy(update={"message": message}), replaces=[event.seq])
        cut += 1
    return cut
