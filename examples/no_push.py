"""A plugin that keeps the agent from pushing with git: a guard that refuses such a command.

A guard is asked about every call that the permissions let through, and it can only refuse:
whatever it answers, a call that something else refused stays refused. The model reads the
reason of a refusal as the result of its call.
"""

import re

from columbina.kernel import Context
from columbina.tools import TOOLS, Deny, Invocation

name = "no-push"
inject = (TOOLS,)

PUSH = re.compile(r"\bgit\b.*\bpush\b")


async def refused(ctx: Context, invocation: Invocation) -> Deny | None:
    """Refuse a command that pushes, and say nothing of any other call."""
    command = invocation.target or ""
    if invocation.tool.effect == "run" and PUSH.search(command):
        return Deny("Pushing with git is left to the user. Say what is to be pushed.", "NO_PUSH")
    return None


def apply(ctx: Context, config: object) -> None:
    """Register the guard."""
    ctx.use(TOOLS).guards.add(ctx, name, refused)
