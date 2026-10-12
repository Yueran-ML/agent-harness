"""A server of the Model Context Protocol that a test starts as a program of its own.

It is talked to over its standard input and output, as a server is that a user lists.
Whenever it is started, it writes a line into the file that the variable ``STARTS`` names:
the number of its process, and the names of the variables that it was given. A test reads
there how often the server was started and what reached it.
"""

import json
import os
import sys
from pathlib import Path

from mcp.server.mcpserver import MCPServer

INSTRUCTIONS = "Sums are done with add, and never by heart."

server = MCPServer("sums", instructions=INSTRUCTIONS)


@server.tool()
def add(a: int, b: int) -> int:
    """Add two numbers."""
    return a + b


@server.tool()
def crash() -> str:
    """End the process at once, as a server does that dies in the middle of a call."""
    os._exit(1)


def main() -> None:
    """Write down that the server was started, and serve until the input ends."""
    started = {"pid": os.getpid(), "given": sorted(os.environ)}
    with Path(os.environ["STARTS"]).open("a", encoding="utf-8") as starts:
        starts.write(json.dumps(started) + "\n")
    print(f"sums: process {os.getpid()} listens", file=sys.stderr, flush=True)
    server.run("stdio")


if __name__ == "__main__":
    main()
