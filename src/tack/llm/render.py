"""Requests as text, for people and for snapshots."""

from collections.abc import Sequence

from tack.llm.types import (
    LlmRequest,
    Message,
    ReasoningBlock,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
)


def _name(text: str) -> str:
    """Return a name on one line: what cannot be printed is written as an escape."""
    return "".join(
        char if char.isprintable() and char != "\\" else char.encode("unicode_escape").decode()
        for char in text
    )


def _target(request: LlmRequest) -> str:
    config = request.config
    names = ", ".join(_name(tool.name) for tool in request.tools)
    # Said in other words, so that no tools cannot be taken for a tool of some name.
    tools = f"tools: {names}" if request.tools else "no tools"
    return f"to {_name(config.provider)}/{_name(config.model)}, {tools}"


def _heading(number: int, message: Message) -> str:
    if isinstance(message, ToolMessage):
        outcome = "failed result" if message.is_error else "result"
        return f"{number} tool, {outcome} of {_name(message.call_id)}"
    return f"{number} {message.role}"


def _block(block: TextBlock | ReasoningBlock | ToolCallBlock) -> str:
    if isinstance(block, ReasoningBlock):
        return f"reasoning: {block.text!r}"
    if isinstance(block, TextBlock):
        return f"text: {block.text!r}"
    return f"call {_name(block.name)} {block.arguments!r} as {_name(block.call_id)}"


def _messages(messages: Sequence[Message], skip: int = 0) -> list[str]:
    """Return the lines of the messages after the first ``skip``, numbered as in the whole."""
    lines: list[str] = []
    for number, message in enumerate(messages[skip:], start=skip + 1):
        lines.append(_heading(number, message))
        lines.extend(f"    {_block(block)}" for block in message.content)
    return lines


def render_request(request: LlmRequest) -> str:
    r"""Return a request as lines of text.

    The first line names the model and the tools. Each message then has a line
    with its number and its role, and under it one indented line for each block
    of its content. The text of a block, and the arguments of a call, are
    written the way Python writes a string: in quotes, with a line break as
    ``\n`` and anything else that cannot be printed as an escape. So a block
    never takes more than one line, and what is invisible in a message shows,
    a space at its end included.

    The options of the call, the descriptions of the tools and the replay state
    of reasoning are left out.
    """
    return "\n".join([_target(request), *_messages(request.messages)])


def render_requests(requests: Sequence[LlmRequest]) -> str:
    """Return requests as text, in the order they were made, with an empty line between them.

    Each request is written as :func:`render_request` writes it, after its
    number. But a request whose messages begin with all the messages of the
    one before is written as what it adds: one line says up to which message
    the two are the same, and only the new messages follow. So it shows at a
    glance whether a conversation only grew, which is what a provider needs to
    answer from its cache.
    """
    parts: list[str] = []
    before: tuple[Message, ...] = ()
    for position, request in enumerate(requests, start=1):
        lines = [f"request {position} {_target(request)}"]
        kept = len(before) if request.messages[: len(before)] == before else 0
        if kept:
            lines.append(f"    as request {position - 1} up to message {kept}")
        lines.extend(_messages(request.messages, skip=kept))
        parts.append("\n".join(lines))
        before = request.messages
    return "\n\n".join(parts)
