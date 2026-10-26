"""The command that writes the conversation into a page, to read or to hand on.

This module is a plugin. Loading it registers the command ``/export``:
``/export`` writes a page of HTML, and ``/export md`` one of Markdown, into
``exports`` in the home of the program, under the id of the session, and says
where it is. What the page holds is told in :mod:`columbina.export`.
"""

import asyncio
from pathlib import Path

from pydantic import BaseModel

from columbina import files
from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.export import About, exported
from columbina.kernel import Context

__all__ = ["Config", "apply", "inject", "name"]

name = "command-export"
inject = (COMMANDS,)

_USAGE = "Say /export for a page of HTML, or /export md for one of Markdown."


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        home: The home of the program. The pages go into ``exports`` there.
    """

    home: Path


def apply(ctx: Context, config: Config) -> None:
    """Register the command that writes the conversation into a page."""

    async def run(agent: Agent, rest: str) -> str:
        word = rest.strip().lower()
        if word not in ("", "html", "md"):
            return _USAGE
        form = "md" if word == "md" else "html"
        session = agent.session
        about = About(session.id, session.created_at, str(agent.spec.cwd))
        page = exported(about, list(session.events), form)
        where = config.home / "exports" / f"{session.id}.{form}"
        try:
            await asyncio.to_thread(files.replaced, where, page.encode("utf-8"))
        except OSError as error:
            return f"The page could not be written to {where}: {error}"
        return f"The conversation is written to {where}"

    ctx.use(COMMANDS).registry.add(
        ctx,
        "export",
        Command(run, "Write the conversation into a page of HTML, or of Markdown: /export [md]."),
    )
