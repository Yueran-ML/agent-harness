"""The facts about where an agent runs, told to the model when a turn begins.

This module is a plugin. Loading it registers the source of context
``runtime`` with the agent service.

A model works better when it knows where it is: in which directory, what the
sandbox lets it write, whether somebody can be asked. Facts like these change
in the middle of a conversation, so they are not a part of the system prompt,
which would be another one after every change, and with it everything a
provider has kept of the conversation would be lost. They are a message of
their own, and a new one follows the conversation when a fact has changed.

The working directory is a fact that this plugin knows. Every other fact
comes from the plugin that it belongs to, which listens to
:data:`columbina.agent.RUNTIME_FACTS` and adds a line. A plugin that is not
loaded adds none, and a plugin that leaves takes its line with it.

The date is not among the facts: it would make the conversation of every new
day begin differently.
"""

from columbina.agent import AGENTS, RUNTIME_FACTS, Agent, ContextSource, Facts
from columbina.kernel import Context

__all__ = ["apply", "inject", "name"]

name = "context-runtime"
inject = (AGENTS,)


async def _runtime(agent: Agent) -> str | None:
    """Gather the facts about where an agent runs, one to a line, in the order of their names."""
    facts = Facts(agent, {"working directory": str(agent.cwd)})
    agent.ctx.emit(RUNTIME_FACTS, facts)
    return "\n".join(f"{name}: {value}" for name, value in sorted(facts.lines.items()))


def apply(ctx: Context, config: object) -> None:
    """Register the source of context that gathers the facts."""
    ctx.use(AGENTS).context.add(ctx, "runtime", ContextSource(_runtime, order=100, kind="append"))
