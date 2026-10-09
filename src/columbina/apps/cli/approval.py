"""Asking the person at the terminal before something is done.

Whatever decides that a call needs a yes puts the question through the
approval service, and this is who answers there when somebody sits at a
terminal. The question shows the tool, the whole of what the call acts on,
and why it is asked: nobody can say yes to a command of which only the
beginning is seen. Below that stand the keys and what each of them means.

One key answers. A yes is for this one call. Where the question says that a
longer yes can be given, a third key gives it: the call, as it stands, is
then written down as allowed for the rest of the session, where the rules
are kept, and the answer to the question is still a yes for this one call.

Nothing but one of the keys that say yes is a yes. An answer that is none of
the keys that were offered should not come back from the prompt, and if one
does, it is a no.
"""

import asyncio
from typing import Protocol

from rich.text import Text

from columbina.approval import ApprovalRequest, Outcome
from columbina.apps.cli.prompt import Prompt
from columbina.apps.terminal_text import spelled_out
from columbina.session import Session

__all__ = ["Granting", "Responder"]

_NOTE = "dim"
_NAME = "bold"


class Granting(Protocol):
    """Where a yes for the rest of a session is written down."""

    def grant(self, session: Session, tool: str, target: str | None) -> None:
        """Allow one call, as it stands, for the rest of a session.

        Raises:
            ValueError: No such yes can be given for this call.
        """
        ...


class Responder:
    """Answers the questions of approval by asking the person at the terminal.

    Attributes:
        prompt: Where the question is shown and the key is pressed.
    """

    __slots__ = ("_granting", "prompt")

    def __init__(self, prompt: Prompt, granting: Granting | None) -> None:
        self.prompt = prompt
        self._granting = granting

    async def __call__(self, request: ApprovalRequest) -> Outcome:
        """Put one question to the person and return the answer."""
        asked = Text()
        asked.append(request.tool, style=_NAME)
        if request.target is None:
            asked.append(" asks for a yes.")
            about: list[Text] = []
        else:
            asked.append(" asks for a yes:")
            # Only a line feed breaks a line here. Whatever else would is named instead.
            lines = request.target.split("\n")
            about = [Text(f"    {spelled_out(line)}") for line in lines]
        longer = None if self._granting is None else request.standing
        keys = [Text("  y  yes, this once")]
        if longer is not None:
            keys.append(Text(f"  a  yes, and do not ask again in this session about {longer}"))
        keys.append(Text("  n  no"))
        question = [asked, *about, Text(request.reason, style=_NOTE), *keys]
        offered = "yn" if longer is None else "yan"
        try:
            key = await self.prompt.ask(offered, f"[{'/'.join(offered)}]", question)
        except asyncio.CancelledError:
            self._say("(the question was withdrawn)")
            raise
        # Only a key that was offered and says yes is a yes. Whatever else comes back is a
        # no, also what should never come back at all.
        if key not in tuple(offered[:-1]):
            self._say("(not allowed)")
            return "rejected"
        if key == "a" and self._written_down(request):
            self._say("(allowed, and not asked about again in this session)")
        else:
            self._say("(allowed, this once)")
        return "allowed-once"

    def _written_down(self, request: ApprovalRequest) -> bool:
        """Write the longer yes down, and tell whether that was done."""
        assert self._granting is not None
        try:
            self._granting.grant(request.session, request.tool, request.target)
        except ValueError:
            return False
        return True

    def _say(self, text: str) -> None:
        self.prompt.show([Text(text, style=_NOTE)])
