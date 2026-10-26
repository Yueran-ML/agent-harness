"""Where stage 4 ends: a conversation at a terminal, from the command line to the last key.

The program is the one that ``columbina`` starts without a command, built
from the bundle as it is shipped. Two things are other than in real use. The
model is scripted. And the person at the terminal is a script as well, which
waits for something to be on the screen and then presses keys.

The repository is the one of stage 3: a calculator with a mistake, and a
check that fails because of it.
"""

from pathlib import Path
from typing import Any

import pytest

from columbina.apps.launcher import main
from columbina.builtin_tools import shell
from columbina.llm import render_requests
from columbina.llm.adapters import scripted
from columbina.session import TurnEndEvent, UserMessageEvent
from columbina.session.store import SESSION_FILE
from support import user as at_the_terminal
from support.launch import profile, stored
from support.neutral import neutral_requests, neutral_screen
from support.scenario import CHECK, TASK, check, fresh
from support.snapshot import stored as snapshot

SNAPSHOT = Path(__file__).parent / "snapshots" / "a_session_at_the_terminal.txt"
SCREEN = Path(__file__).parent / "snapshots" / "the_screen_of_a_session.txt"
ENTER = "\r"
ESCAPE = "\x1b"
WAIT = "python -c 'import time; time.sleep(60)'"
ANSWER = "mean() divided by one too few. I fixed calc.py, and the check prints ok now."
ALSO = "Remember to say which file it was."


def call(name: str, **arguments: object) -> dict[str, Any]:
    return {"calls": [{"name": name, "arguments": arguments}]}


WHAT_THE_MODEL_DOES: tuple[dict[str, Any], ...] = (
    {"text": "I will run the check first.", **call(shell.NAME, command=CHECK)},
    call("read", path="calc.py"),
    call("edit", path="calc.py", old="(len(values) - 1)", new="len(values)"),
    call(shell.NAME, command=CHECK),
    {"text": ANSWER},
    call(shell.NAME, command=WAIT),
    {"text": "Here I am."},
)

WHAT_THE_USER_DOES: tuple[dict[str, Any], ...] = (
    # A task.
    {"sees": "/help lists the commands.", "keys": TASK + ENTER},
    # The check needs a yes. It is given for the rest of the session.
    {"sees": f"    {CHECK}\n", "asked": True, "keys": "a"},
    # Something is said while the turn runs.
    {"sees": "(allowed, and not asked about again in this session)\n", "keys": ALSO + ENTER},
    # A second task, with a command that runs for a long time, once the first turn is over: the
    # line under the answer says so.
    {"sees": f"{ANSWER}\n\n", "keys": "Now wait a minute." + ENTER},
    {"sees": f"    {WAIT}\n", "asked": True, "keys": "y"},
    # It is stopped.
    {"sees": "(allowed, this once)\n", "keys": ESCAPE},
    # A third task, and the end.
    {"sees": "(stopped)\n", "keys": "Are you there?" + ENTER},
    {"sees": "Here I am.\n", "keys": "/exit" + ENTER},
)


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    return fresh(tmp_path / "repository", monkeypatch)


def somebody(*steps: dict[str, Any], **config: Any) -> dict[str, Any]:
    """The entry that puts a scripted user at the terminal."""
    user = {"id": "user", "plugin": "support.user", "config": {"script": list(steps), **config}}
    return {"insert": [user]}


def printed() -> str:
    """What the conversation showed above its prompt."""
    return "".join(at_the_terminal.SCREENS[-1].printed).replace("\r\n", "\n")


@pytest.mark.timeout(120)
def test_a_repository_is_mended_in_a_conversation(
    unregistered: None,
    home: Path,
    repository: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    models: list[scripted.Scripted] = []

    class Watched(scripted.Scripted):
        def __init__(self, *args: Any, **options: Any) -> None:
            super().__init__(*args, **options)
            models.append(self)

    monkeypatch.setattr(scripted, "Scripted", Watched)
    # The window is wide, so that no line is broken at a place that depends on how long the
    # name of the working directory is on this machine.
    user = somebody(*WHAT_THE_USER_DOES, columns=400)
    chosen = profile(tmp_path, *WHAT_THE_MODEL_DOES, extends="cli", extra=[user])

    code = main(["--profile", chosen, "--cwd", str(repository), "--session-id", "s-1"])

    assert code == 0
    assert check(repository) == (0, "ok\n")
    shown = printed()
    # The check was asked about once, although it ran twice.
    assert shown.count(f"{shell.NAME} asks for a yes:\n    {CHECK}\n") == 1
    assert shown.count("    ok\n    [exit code 0]\n") == 1
    assert "    mean([1.0, 2.0, 3.0]) is 3.0, expected 2.0\n    [exit code 1]\n" in shown
    assert "    - (len(values) - 1)\n    + len(values)\n" in shown
    # What was said on the way reached the model when its next step began.
    assert f"{chr(0xBB)} {ALSO}\n" in shown
    left = stored(home, repository)
    ends = [event.data for event in left.events if isinstance(event.data, TurnEndEvent)]
    assert ends == [
        TurnEndEvent(turn=1, reason="completed"),
        TurnEndEvent(turn=2, reason="aborted", detail="user"),
        TurnEndEvent(turn=3, reason="completed"),
    ]
    told = [event.data.source for event in left.events if isinstance(event.data, UserMessageEvent)]
    assert told == [
        "context/instructions",
        "context/runtime",
        "user",
        "steer",
        "user",
        "context/interruption",
        "user",
    ]

    sent = neutral_requests(models[0].requests, repository, shell.NAME)
    seen = render_requests(sent) + "\n"
    assert seen == snapshot(SNAPSHOT, seen)
    # What stood on the screen in the end, with the lines that the user typed among it.
    standing = at_the_terminal.SCREENS[-1].display.lines()
    looked = neutral_screen(standing, repository, shell.NAME)
    assert looked == snapshot(SCREEN, looked)


@pytest.mark.timeout(120)
def test_the_program_that_is_shipped_has_its_commands(
    home: Path, repository: Path, tmp_path: Path
) -> None:
    steps = (
        {"sees": "/help lists the commands.", "keys": "/help" + ENTER},
        {"sees": "goes to the agent.\n", "keys": "/exit" + ENTER},
    )
    chosen = profile(tmp_path, extends="cli", extra=[somebody(*steps)])

    code = main(["--profile", chosen, "--cwd", str(repository)])

    assert code == 0
    assert (
        "/compact      Sum up the older part of the conversation to make room: /compact [WHAT "
        "TO KEEP].\n"
        "/context      Say what the conversation is made of, and how much room is left.\n"
        "/effort       Show how hard the model thinks, or choose a level.\n"
        "/exit         Leave.\n"
        "/export       Write the conversation into a page of HTML, or of Markdown: /export "
        "[md].\n"
        "/fork         Go on in a copy of this session, and leave the session as it is.\n"
        "/help         List the commands.\n"
        "/mcp          List the MCP servers: how each is connected, and how many tools it has.\n"
        "/model        Show the model, or name another.\n"
        "/new          End this session and begin another one.\n"
        "/permissions  Show what the session may do, or change it to a preset.\n"
        "/plan         Go into plan mode, where nothing is changed, or out of it.\n"
        "/rewind       Go back to before a turn: /rewind [N [files|conversation]], /rewind back.\n"
        "/sessions     List the sessions of this directory.\n"
        "/skill        Hand a skill to the agent: /skill NAME [TASK].\n"
        "/skills       List the skills, and what cannot be used as one.\n"
        "/todos        Show the list of things to do that the agent keeps.\n"
        "/tools        List the tools of the agent, and how their calls went.\n"
        "/verdict      Say whether the last turn did what was asked: /verdict right|wrong "
        "[NOTE].\n"
    ) in printed()


@pytest.mark.timeout(120)
def test_what_is_logged_in_a_conversation_stands_above_the_prompt_and_nowhere_else(
    home: Path,
    repository: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    # A session whose file ends in a line that was never finished: taking it up cuts
    # the line off, and the log says so.
    first = (
        {"sees": "/help lists the commands.", "keys": "Hello?" + ENTER},
        {"sees": "Hi.\n", "keys": "/exit" + ENTER},
    )
    arguments = ["--cwd", str(repository), "--session-id", "s-1"]
    held = profile(tmp_path, {"text": "Hi."}, extends="cli", extra=[somebody(*first)])
    main([*arguments, "--profile", held])
    (file,) = home.rglob(SESSION_FILE)
    with file.open("ab") as log:
        log.write(b'{"seq": 99')
    capsys.readouterr()

    again = ({"sees": "columbina: cut an unfinished line", "keys": "/exit" + ENTER},)
    code = main(
        [*arguments, "--profile", profile(tmp_path, extends="cli", extra=[somebody(*again)])]
    )

    assert code == 0
    assert "columbina: cut an unfinished line of 10 bytes off the end of " in printed()
    assert capsys.readouterr().err == ""


@pytest.mark.timeout(120)
def test_a_no_keeps_the_command_from_running(home: Path, repository: Path, tmp_path: Path) -> None:
    replies = (call(shell.NAME, command=CHECK), {"text": "I was not allowed to run it."})
    steps = (
        {"sees": "/help lists the commands.", "keys": "Run the check." + ENTER},
        {"sees": f"    {CHECK}\n", "asked": True, "keys": "n"},
        {"sees": "I was not allowed to run it.\n", "keys": "/exit" + ENTER},
    )
    chosen = profile(tmp_path, *replies, extends="cli", extra=[somebody(*steps)])

    code = main(["--profile", chosen, "--cwd", str(repository)])

    assert code == 0
    shown = printed()
    assert "(not allowed)\n" in shown
    assert "APPROVAL_REJECTED" in shown
    assert "[exit code" not in shown
    assert check(repository)[0] == 1
