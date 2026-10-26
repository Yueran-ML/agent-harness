"""The command that says what the conversation is made of, and how much room is left.

This module is a plugin. Loading it registers the command ``/context``.

A model takes so much and no more, and a conversation that comes near that is
made shorter. Before it gets there, a person may want to see where the room
goes: the instructions and facts of the program, the tools that are declared,
what they wrote, what the model answered and what the tools gave back. The
command says what each of these comes to, as the meter of tokens estimates
it, how far that is from what the model takes, and which results of tools
are the largest, since those are what a summary or a cut takes first.

The parts are estimated alone, and the rate at which the provider counted
otherwise is applied to each, so that they add up to the size of the whole.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.compaction import SUMMARY_SOURCE
from columbina.kernel import Context
from columbina.session import (
    RequestHeaderEvent,
    SystemMessageEvent,
    ToolCallEvent,
    ToolResultEvent,
    UserMessageEvent,
)
from columbina.token_meter import TOKEN_METER, TokenMeter, estimate_message, estimate_tools

__all__ = ["apply", "inject", "name"]

name = "command-context"
inject = (COMMANDS, TOKEN_METER)

_LARGEST = 3
"""How many of the largest results of tools are named."""

_PARTS = (
    ("system", "what the program tells the model of its work"),
    ("context", "instructions of the project and of the user, and facts of this run"),
    ("tools", "the tools that are declared"),
    ("summary", "the summary of the earlier part"),
    ("user", "what was written to the model"),
    ("assistant", "what the model wrote"),
    ("results", "what tools gave back"),
)


def _part(data: object) -> str:
    """Return the part of the conversation that the message of an event belongs to."""
    if isinstance(data, SystemMessageEvent):
        return "system"
    if isinstance(data, UserMessageEvent):
        if data.source == SUMMARY_SOURCE:
            return "summary"
        return "context" if data.source.startswith("context/") else "user"
    if isinstance(data, ToolResultEvent):
        return "results"
    return "assistant"


def _thousands(tokens: int) -> str:
    return f"{tokens:,}"


def described(agent: Agent, meter: TokenMeter) -> str:
    """Say what the conversation of an agent is made of, and how much room is left."""
    session = agent.session
    measured = meter.measure(session)
    rate = measured.rate

    def scale(tokens: int) -> int:
        return tokens if rate is None else round(tokens * rate)

    sizes = dict.fromkeys((key for key, _ in _PARTS), 0)
    names: dict[str, str] = {}
    results: list[tuple[int, str]] = []
    header: RequestHeaderEvent | None = None
    surface = set(session.surface)
    for event in session.events:
        data = event.data
        if isinstance(data, RequestHeaderEvent):
            header = data
        elif isinstance(data, ToolCallEvent):
            names[data.call_id] = data.name
        if event.seq not in surface:
            continue
        message = data.for_model()
        # What stands on the surface is there for its message.
        assert message is not None
        tokens = estimate_message(message)
        part = _part(data)
        sizes[part] += tokens
        if isinstance(data, ToolResultEvent):
            results.append((tokens, names.get(data.message.call_id, "a tool")))
    tools = () if header is None else header.tools
    sizes["tools"] = estimate_tools(tools)
    window = (header.config.context_window if header is not None else None) or (
        agent.spec.model.context_window
    )
    total = measured.tokens
    if window:
        lines = [
            f"The conversation comes to about {_thousands(total)} tokens of the "
            f"{_thousands(window)} that the model takes ({round(100 * total / window)}%)."
        ]
    else:
        lines = [
            f"The conversation comes to about {_thousands(total)} tokens. How many the model "
            "takes is not known."
        ]
    for key, said in _PARTS:
        if sizes[key]:
            counted = f" ({len(tools)})" if key == "tools" else ""
            lines.append(f"- {said}{counted}: {_thousands(scale(sizes[key]))}")
    largest = sorted(results, key=lambda result: -result[0])[:_LARGEST]
    if largest:
        named = ", ".join(f"{tool} {_thousands(scale(tokens))}" for tokens, tool in largest)
        lines.append(f"The largest results of tools: {named}.")
    return "\n".join(lines)


def apply(ctx: Context, config: object) -> None:
    """Register the command that says what the conversation is made of."""
    meter = ctx.use(TOKEN_METER)

    async def run(agent: Agent, rest: str) -> str:
        return described(agent, meter)

    ctx.use(COMMANDS).registry.add(
        ctx,
        "context",
        Command(run, "Say what the conversation is made of, and how much room is left."),
    )
