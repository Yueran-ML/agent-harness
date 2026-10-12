"""A tool of a server in a conversation at a terminal: asked about once, for the session.

The program is the one that ``columbina`` starts without a command. The model
is scripted, the person at the terminal is a script, and the server is a
program of its own, as in ``tests/apps/test_extensions.py``.
"""

import sys
from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.session import ToolResultEvent
from support import user as at_the_terminal
from support.launch import profile, stored

SUMS = Path(__file__).parents[2] / "fixtures" / "mcp_server.py"
ENTER = "\r"
TASK = "Add 2 and 3, and 20 and 22."
ASKED = "mcp__sums__add asks for a yes.\n"


def call(name: str, **arguments: object) -> dict[str, Any]:
    return {"calls": [{"name": name, "arguments": arguments}]}


@pytest.mark.timeout(120)
def test_a_yes_for_the_session_is_asked_for_once_however_often_the_tool_is_called(
    home: Path, work: Path, tmp_path: Path
) -> None:
    sums = {
        "id": "sums",
        "plugin": "columbina.mcp.server",
        "config": {
            "server": "sums",
            "command": sys.executable,
            "args": [str(SUMS)],
            "env": {"STARTS": str(tmp_path / "starts.jsonl")},
        },
    }
    steps = [
        {"sees": "/help lists the commands.", "keys": TASK + ENTER},
        # The tool runs what a server likes, so it needs a yes. One is given for the session.
        {"sees": ASKED, "asked": True, "keys": "a"},
        {"sees": "5 and 42.\n", "keys": "/exit" + ENTER},
    ]
    # The window is wide, so that no line is broken at a place that depends on the machine.
    user = {"id": "user", "plugin": "support.user", "config": {"script": steps, "columns": 400}}
    chosen = profile(
        tmp_path,
        call("mcp__sums__add", a=2, b=3),
        call("mcp__sums__add", a=20, b=22),
        {"text": "5 and 42."},
        extends="cli",
        extra=[{"insert": [user, sums]}],
    )

    code = main(["--profile", chosen, "--cwd", str(work), "--session-id", "s-1"])

    assert code == 0
    shown = "".join(at_the_terminal.SCREENS[-1].printed).replace("\r\n", "\n")
    # The question says why it is put, and what the longer yes would be for: the tool, since
    # the tool names nothing that one call of it could be told from another by.
    assert (
        f"{ASKED}"
        "No rule allows calls of this tool.\n"
        "  y  yes, this once\n"
        "  a  yes, and do not ask again in this session about any call of mcp__sums__add\n"
        "  n  no\n"
        "(allowed, and not asked about again in this session)\n"
    ) in shown
    # The tool was called twice, and both calls were answered by the server. It was asked
    # about once.
    assert shown.count(ASKED) == 1
    came = [
        (event.data.message.content[0].text, event.data.error)
        for event in stored(home, work).events
        if isinstance(event.data, ToolResultEvent)
    ]
    assert came == [("5", None), ("42", None)]
