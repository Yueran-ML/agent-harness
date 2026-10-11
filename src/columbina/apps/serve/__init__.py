"""The service process: a conversation with an agent, held by another program over a pipe.

This module is a plugin. It provides the frontend that ``columbina serve``
runs.

A desktop application, or whatever else wants to show a conversation, starts
this program and talks to it over its standard input and output, in the
messages of ``docs/protocol``. The program holds the whole state of the
conversation: it asks the model, runs the tools and writes the log. The other
side shows what it is told and sends what a person does.

The three parts are :mod:`columbina.apps.serve.rpc`, which knows what a line
means, :mod:`columbina.apps.serve.wire`, which reads and writes the lines of
a real process, and :mod:`columbina.apps.serve.server`, which does what is
asked.

The command ``/model`` is brought by this plugin, as the interface at the
terminal brings its own: which model answers is a matter of whoever holds the
conversation.
"""

from columbina.agent import AGENTS
from columbina.apps import FRONTEND
from columbina.apps.serve.server import Config, Server
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.session import SESSIONS

__all__ = ["Config", "Server", "apply", "inject", "name"]

name = "serve"
inject = (AGENTS, SESSIONS, COMMANDS)


def apply(ctx: Context, config: Config) -> None:
    """Provide the frontend that serves another program over a wire."""
    commands = ctx.use(COMMANDS)
    server = Server(ctx, config, ctx.use(AGENTS), ctx.use(SESSIONS), commands)
    commands.registry.add(ctx, "model", Command(server.model, "Show the model, or name another."))
    ctx.provide(FRONTEND, server)
