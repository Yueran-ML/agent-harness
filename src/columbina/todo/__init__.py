"""The list of things to do: what a model writes down for itself in a task of many steps.

This module is a plugin. Loading it registers the tool ``todo_write`` and
provides nothing else: what the list holds is read from the log of a session
with :data:`TODOS`.

A model that works on something long loses track of what it meant to do. So
it is given a place to write the steps down, and to tick them off. It writes
the whole list every time, and the new list takes the place of the old one:
there is nothing to add to and nothing to take out of, and so nothing that
can go out of step with what the model believes the list to be.

The list is the model's own. Nothing in the program goes by it. It is kept in
the log, so it is there when a session is taken up again, and it is shown to
the person who watches.

A model sees its list in the call that wrote it. A conversation that grows
too long has its beginning summed up, and the call may be in the part that
goes. Then the list is told to the model again, as context: without that, a
model would go on with a list that it can no longer read.

Which call wrote the list is told by where it stands in the log, not by its
id: a model may use an id again in a later reply.
"""

from typing import ClassVar, Literal

from pydantic import BaseModel, Field

from columbina.agent import AGENTS, Agent, ContextSource
from columbina.data import Data
from columbina.kernel import Context
from columbina.session import (
    SESSIONS,
    AssistantMessageEvent,
    EventData,
    Projection,
    Session,
    SessionEvent,
)
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "MAX_CHARACTERS",
    "MAX_ITEMS",
    "TODOS",
    "TOOL",
    "Item",
    "Status",
    "TodoArgs",
    "TodoWriteEvent",
    "Todos",
    "apply",
    "inject",
    "name",
    "shown",
    "todo_tool",
]

name = "todo"
inject = (TOOLS, SESSIONS, AGENTS)

TOOL = "todo_write"
"""The name of the tool."""

MAX_ITEMS = 100
"""How many things a list holds at most."""

MAX_CHARACTERS = 500
"""How long the words for one thing are at most."""

Status = Literal["pending", "in_progress", "completed"]
"""How far one thing has come."""

_MARKS: dict[Status, str] = {"pending": "[ ]", "in_progress": "[>]", "completed": "[x]"}
_INVALID = "INVALID_ARGS"


class Item(Data):
    """One thing to do.

    Attributes:
        content: What is to be done, on one line.
        status: How far it has come.
    """

    content: str
    status: Status = "pending"


class TodoWriteEvent(EventData):
    """The list was written. It takes the place of the list before it.

    Attributes:
        call_id: The call that wrote it. The model sees the list for as long
            as it sees the reply that made that call.
        items: The things to do, in the order in which they were given.
    """

    event_type: ClassVar[str] = "todo/write"
    ignorable: ClassVar[bool] = True

    call_id: str
    items: tuple[Item, ...]


class Todos(Data):
    """The list of a session.

    Attributes:
        items: The things to do. None before the list is written, and none
            after an empty one was written.
        written_at: Where the list was written last: the position of that
            event in the log, or ``None`` if the list was never written.
    """

    items: tuple[Item, ...] = ()
    written_at: int | None = None


def _fold(state: Todos, event: SessionEvent[EventData]) -> Todos:
    data = event.data
    if isinstance(data, TodoWriteEvent):
        return Todos(items=data.items, written_at=event.seq)
    return state


TODOS = Projection("todos", Todos(), _fold)
"""The list of things to do of a session."""


def shown(items: tuple[Item, ...]) -> list[str]:
    """Return a list as lines for a person or a model to read, one for each thing."""
    return [f"{_MARKS[item.status]} {item.content}" for item in items]


class _Thing(BaseModel):
    content: str = Field(description="What is to be done, in a few words.")
    status: Status = Field(
        default="pending",
        description="pending, in_progress or completed. At most one thing is in progress.",
    )


class TodoArgs(BaseModel):
    """The arguments of the tool that writes the list."""

    items: list[_Thing] = Field(
        description=(
            "The whole list, in the order in which the things are to be done. It takes the "
            "place of the list before it. An empty list clears it."
        )
    )


def _counted(count: int, what: str) -> str:
    return f"{count} {what}"


def _told(items: tuple[Item, ...]) -> str:
    """Say what a list holds, for the model that wrote it."""
    if not items:
        return "The list is empty now."
    counts = {status: sum(item.status == status for item in items) for status in _MARKS}
    things = "1 thing" if len(items) == 1 else f"{len(items)} things"
    return (
        f"The list holds {things}: {_counted(counts['completed'], 'completed')}, "
        f"{_counted(counts['in_progress'], 'in progress')}, "
        f"{_counted(counts['pending'], 'pending')}."
    )


def _checked(args: TodoArgs) -> tuple[Item, ...]:
    """Return the list that a call gives, or say what is wrong with it.

    Raises:
        ToolFailure: The list is too long, a thing has no words or too many,
            or more than one thing is in progress.
    """
    if len(args.items) > MAX_ITEMS:
        raise ToolFailure(
            f"The list has {len(args.items)} things, and it holds {MAX_ITEMS} at most. "
            "Put small steps together.",
            code=_INVALID,
        )
    items: list[Item] = []
    for number, thing in enumerate(args.items, start=1):
        # One thing is one line, for the model as for whoever reads the list.
        content = " ".join(thing.content.split())
        if not content:
            raise ToolFailure(f"Thing {number} of the list says nothing.", code=_INVALID)
        if len(content) > MAX_CHARACTERS:
            raise ToolFailure(
                f"Thing {number} of the list has {len(content)} characters, and one thing has "
                f"{MAX_CHARACTERS} at most. Say it in a few words.",
                code=_INVALID,
            )
        items.append(Item(content=content, status=thing.status))
    doing = [number for number, item in enumerate(items, start=1) if item.status == "in_progress"]
    if len(doing) > 1:
        numbers = ", ".join(str(number) for number in doing)
        raise ToolFailure(
            f"Things {numbers} are all in progress, and one thing is at a time. "
            "Leave one in progress and make the others pending.",
            code=_INVALID,
        )
    return tuple(items)


def todo_tool() -> Tool[TodoArgs, str]:
    """Make the tool that writes the list of things to do."""

    async def write(args: TodoArgs, run: ToolRun) -> str:
        items = _checked(args)
        run.session.append(TodoWriteEvent(call_id=run.call_id, items=items))
        return _told(items)

    return Tool(
        name=TOOL,
        description=(
            "Write down the list of things to do in this task, for yourself to go by. Use it "
            "for work of several steps: write the steps when you have a plan, and write the "
            "list again whenever a step is begun or done. Give the whole list every time. "
            "It takes the place of the list before it. The user sees the list."
        ),
        parameters=TodoArgs,
        execute=write,
    )


def _seen(session: Session, written_at: int) -> bool:
    """Tell whether the model still sees the call that wrote the list.

    The call stands in the reply that came last before the list was written.
    The model sees the call for as long as it sees that reply.
    """
    events = session.events
    reply = next(
        seq
        for seq in range(written_at - 1, -1, -1)
        if isinstance(events[seq].data, AssistantMessageEvent)
    )
    return reply in session.surface


async def _remembered(agent: Agent) -> str | None:
    """Return the list as context, if the model no longer sees the call that wrote it."""
    session = agent.session
    todos = TODOS.of(session)
    if not todos.items or todos.written_at is None or _seen(session, todos.written_at):
        return None
    lines = [
        "This is your list of things to do, as you wrote it last. The call that wrote it is "
        "in the part of the conversation that was summed up.",
        *shown(todos.items),
        f"Write the whole list again with {TOOL} when something on it changes.",
    ]
    return "\n".join(lines)


def apply(ctx: Context, config: object) -> None:
    """Register the tool that writes the list, its event, and the context that recalls it."""
    ctx.use(SESSIONS).declare(ctx, TodoWriteEvent)
    ctx.use(TOOLS).add(ctx, todo_tool())
    # After what is told about the run, and before word of a turn that was stopped.
    ctx.use(AGENTS).context.add(ctx, "todo", ContextSource(_remembered, order=150, kind="replace"))
