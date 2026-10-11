"""The names under which the tools of a server are offered to a model.

A server names its tools as it likes, and two servers may use one name. A
model needs names that say where a tool comes from, and a service takes only
names of 64 letters, digits, underscores and dashes. So a tool is offered as
``mcp__<server>__<name>``, and a name that does not fit is given a form that
does.

The form depends on the name of the server and the name of the tool alone,
whatever else the server offers. So a tool keeps its name from one start of
the program to the next, and what is written down about a call names the
same tool later.
"""

import hashlib
import re

__all__ = ["MAX_NAME", "PREFIX", "SERVER", "tool_name"]

PREFIX = "mcp__"
"""What the name of every tool of a server begins with."""

MAX_NAME = 64
"""How many characters the name of a tool may have."""

SERVER = re.compile(r"(?=.{1,32}\Z)[A-Za-z0-9-]+(?:_[A-Za-z0-9-]+)*")
"""What a server may be called here: up to 32 letters, digits and dashes, and underscores inside.

Two underscores part the name of the server from the name of the tool. A
server whose name held two of them in a row, or began or ended with one,
could be given a tool that another server has under the same whole name.
"""

_OTHER = re.compile(r"[^A-Za-z0-9_-]")
_KEPT = 51
_DIGITS = 12


def tool_name(server: str, raw: str) -> str:
    """Return the name under which a tool of a server is offered.

    A character of the tool's own name that is no letter, digit, underscore
    or dash becomes an underscore. A name that was changed in this way, or
    that is too long, is cut to 51 characters and gets an underscore and 12
    hexadecimal digits after it. The digits are the beginning of the SHA-256
    of the server's name, a zero byte and the tool's own name, so two tools
    whose names were made alike are still told apart.

    Args:
        server: The name of the server, which matches :data:`SERVER`.
        raw: The name that the server gives the tool.
    """
    plain = _OTHER.sub("_", raw)
    name = f"{PREFIX}{server}__{plain}"
    if plain == raw and len(name) <= MAX_NAME:
        return name
    # Half of a pair that a name may hold is written as it is, and breaks nothing.
    named = f"{server}\0{raw}".encode("utf-8", "surrogatepass")
    digest = hashlib.sha256(named).hexdigest()
    return f"{name[:_KEPT]}_{digest[:_DIGITS]}"
