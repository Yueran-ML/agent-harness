"""A desk for the tests of the tools that work on files."""

import json
from pathlib import Path
from typing import Any

from columbina.fs import Fs
from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.sandbox import Sandbox
from columbina.session import Session
from columbina.tools import PendingCall, ToolOutcome, ToolRun, Tools

__all__ = ["Desk"]


class Desk:
    """A working directory, a session in it, and the tools that work on its files.

    Attributes:
        ctx: The context that the tools are called through.
        tools: The tool service.
        fs: The seam of the file system.
        sandbox: The sandbox, which holds the mode of the session.
        session: The session that the calls belong to.
        work: The working directory.
    """

    def __init__(
        self, ctx: Context, tools: Tools, fs: Fs, sandbox: Sandbox, session: Session, work: Path
    ) -> None:
        self.ctx = ctx
        self.tools = tools
        self.fs = fs
        self.sandbox = sandbox
        self.session = session
        self.work = work
        self._calls = 0

    def begin(self, name: str, **arguments: Any) -> PendingCall:
        """Take up a call of a tool the way the loop does."""
        self._calls += 1
        call_id = f"call-{self._calls}"
        asked = ToolCallBlock(call_id=call_id, name=name, arguments=json.dumps(arguments))
        return self.tools.begin(self.ctx, asked, ToolRun(call_id, self.work, self.session))

    async def call(self, name: str, **arguments: Any) -> ToolOutcome:
        """Call a tool the way a model does, and return how the call ended."""
        return await self.begin(name, **arguments).execute()

    async def have_read(self, path: str) -> None:
        """Read a file with the read tool, so that the model has been shown it."""
        outcome = await self.call("read", path=path)
        assert outcome.error is None, outcome.text
        # What a call has read counts once its result is before the model.
        self.fs.delivered(self.session)

    def names(self, directory: str = ".") -> list[str]:
        """The names of what is in a directory of the desk."""
        return sorted(entry.name for entry in (self.work / directory).iterdir())
