"""A stand-in for the ``docker`` program, for machines that have no engine.

It is run as a program, with the arguments that ``docker`` would get, and
answers the few commands that the provider of the sandbox uses: ``info``,
``run``, ``exec``, ``ps`` and ``rm``. A container is a process of this
program that reads its input to the end, as the real one does. A command in a
container is run on this machine, in the directory that was mounted where
the command is to run.

What it does not do is hold anything: no user, no mount and no network is
real here. Whether the commands that the provider writes do what they are
meant to do is tried with a real engine. This is for everything around
them: when a container is started, kept and let go, and what happens when
the engine fails in one way or another.

It keeps what it knows in the directory that the variable
``COLUMBINA_FAKE_DOCKER`` names:

``told.json``
    How it is to behave, written by the test. Without the file, or without
    a word in it, it behaves like an engine with which all is well.
``calls``
    Every call of it, a file each, named so that they sort by time: the
    arguments, and for some what it saw at that moment.
``containers``
    One file for each container that is there.
"""

import contextlib
import csv
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

# This is run as a program, from wherever. What it shares with the tests is found from here.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from support.processes import alive

HOME = Path(os.environ["COLUMBINA_FAKE_DOCKER"])
CONTAINERS = HOME / "containers"

DESKTOP = (
    '{"version":"29.8.2","system":"Docker Desktop",'
    '"security":["name=seccomp,profile=builtin","name=cgroupns"],"os":"linux"}'
)
DOWN = (
    "failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine; "
    "check if the path is correct and if the daemon is running"
)
PROBE = "/run/columbina-probe"
STOPPED = 137
"""The exit code of a command that was killed, as the real program reports it."""

NOTED: list[None] = []
"""As many entries as this process has written down calls."""


def told() -> dict[str, Any]:
    """Return how the test wants this to behave."""
    try:
        return json.loads((HOME / "told.json").read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}


def note(**what: object) -> None:
    """Write down a call, in a file of its own: several of these run at the same time."""
    (HOME / "calls").mkdir(exist_ok=True)
    # The clock of Windows stands still for milliseconds, and a process may note twice in one.
    NOTED.append(None)
    name = f"{time.time_ns():020d}-{os.getpid()}-{len(NOTED)}.json"
    (HOME / "calls" / name).write_text(json.dumps(what), encoding="utf-8")


def fail(code: int, words: str) -> int:
    print(words, file=sys.stderr, flush=True)
    return code


def end(pid: int) -> None:
    """End a process, whatever it is doing, so that it can be told afterwards that it was ended."""
    with contextlib.suppress(OSError):
        # On Windows the number becomes the exit code of the process.
        os.kill(pid, STOPPED if sys.platform == "win32" else signal.SIGKILL)


def fields(mount: str) -> dict[str, str]:
    """Read one mount the way the engine does: as a line of CSV."""
    read: dict[str, str] = {}
    for field in next(csv.reader([mount])):
        key, _, value = field.partition("=")
        read[key] = value
    return read


def container(name: str) -> dict[str, Any] | None:
    """Return what is known of a container, or ``None`` if there is none of that name."""
    try:
        known = json.loads((CONTAINERS / f"{name}.json").read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    return known if alive(known["pid"]) else None


def commands(name: str) -> list[int]:
    """Return the processes of the commands that run in a container."""
    try:
        return [int(line) for line in (CONTAINERS / f"{name}.pids").read_text().split()]
    except FileNotFoundError:
        return []


def info(how: dict[str, Any]) -> int:
    if how.get("engine") == "down":
        return fail(1, DOWN)
    print(how.get("info", DESKTOP), flush=True)
    return int(how.get("info_exit", 0))


def run(argv: list[str], how: dict[str, Any]) -> int:
    name = argv[argv.index("--name") + 1]
    image = argv[argv.index("--entrypoint") + 2]
    mounted = [fields(argv[at + 1]) for at, word in enumerate(argv) if word == "--mount"]
    if how.get("engine") == "down":
        return fail(125, f"docker: {DOWN}")
    if how.get("run") == "no-image":
        return fail(125, f"docker: Error response from daemon: No such image: {image}")
    if how.get("run") == "breaks":
        return fail(
            125, "docker: Error response from daemon: the engine is out of sorts\n\nSee help."
        )
    CONTAINERS.mkdir(exist_ok=True)
    known = {"pid": os.getpid(), "mounts": mounted, "image": image}
    (CONTAINERS / f"{name}.json").write_text(json.dumps(known), encoding="utf-8")
    token = ""
    for mount in mounted:
        if mount["target"] == PROBE:
            token = Path(mount["source"], "token").read_text(encoding="ascii")
    first = {"missing": "missing", "another-token": "ready of-another-machine", "odd": "hello"}
    if how.get("run") == "endless":
        print("x" * 200_000, end="", flush=True)
    elif how.get("run") != "silent":
        print(first.get(str(how.get("run")), f"ready {token}"), flush=True)
    # A container lives for as long as its input is open.
    sys.stdin.buffer.read()
    while how.get("run") == "stays":
        time.sleep(0.05)
    for pid in commands(name):
        end(pid)
    for left in CONTAINERS.glob(f"{name}.*"):
        left.unlink(missing_ok=True)
    return 0


def execute(argv: list[str], how: dict[str, Any]) -> int:
    given: dict[str, str] = {}
    workdir = "/"
    at = 0
    while argv[at].startswith("--"):
        if argv[at] == "--env":
            key, _, value = argv[at + 1].partition("=")
            given[key] = value
        elif argv[at] == "--workdir":
            workdir = argv[at + 1]
        at += 1 if argv[at] == "--interactive" else 2
    name, script, words = argv[at], argv[at + 3], argv[at + 5 :]
    if how.get("engine") == "down":
        return fail(1, DOWN)
    known = container(name)
    if known is None:
        return fail(1, f"Error response from daemon: No such container: {name}")
    if "kill -KILL -1" in script:
        return stop(name, how)
    if "test -w ." in script:
        readonly = any(
            "readonly" in mount for mount in known["mounts"] if mount["target"] == workdir
        )
        answer = how.get("usable", "readable" if readonly else "writable")
        print(answer, flush=True)
        return int(how.get("usable_exit", 0))
    if how.get("exec") == "refused":
        return fail(1, "Error response from daemon: the container is paused")
    time.sleep(float(how.get("late", 0)))
    # The command runs where its directory really is.
    source = next(mount["source"] for mount in known["mounts"] if mount["target"] == workdir)
    # Ending the program that asked for a command does not end the command in a container.
    # Where the system lets a process leave the one that started it, it is the same here.
    apart: dict[str, Any] = {} if sys.platform == "win32" else {"start_new_session": True}
    child = subprocess.Popen(words, cwd=source, env={**os.environ, **given}, **apart)
    with (CONTAINERS / f"{name}.pids").open("a") as pids:
        pids.write(f"{child.pid}\n")
    note(started=child.pid)
    code = child.wait()
    return STOPPED if code in (STOPPED, -signal.SIGTERM, -9) else code


def stop(name: str, how: dict[str, Any]) -> int:
    """Do what the stop of the provider does in a real container: end every command in it."""
    running = [pid for pid in commands(name) if alive(pid)]
    note(stop=name, running=len(running))
    if how.get("stop") == "fails":
        return fail(1, "Error response from daemon: the engine is out of sorts")
    if how.get("stop") == "hangs":
        time.sleep(60)
    if how.get("stop") == "left":
        print("left", flush=True)
        return 0
    for pid in running:
        end(pid)
    (CONTAINERS / f"{name}.pids").unlink(missing_ok=True)
    print("none", flush=True)
    return 0


def listed(argv: list[str], how: dict[str, Any]) -> int:
    name = argv[argv.index("--filter") + 1].removeprefix("name=^").removesuffix("$")
    if how.get("engine") == "down":
        return fail(1, DOWN)
    if how.get("ps") == "fails":
        return fail(1, "Error response from daemon: the engine is out of sorts")
    if container(name) is not None or told().get("ps") == "stuck":
        print(name, flush=True)
    return 0


def remove(argv: list[str], how: dict[str, Any]) -> int:
    name = argv[-1]
    if how.get("engine") == "down":
        return fail(1, DOWN)
    if how.get("rm") == "fails":
        return fail(1, "Error response from daemon: the engine is out of sorts")
    known = container(name)
    if known is not None:
        for pid in (*commands(name), known["pid"]):
            end(pid)
        for left in CONTAINERS.glob(f"{name}.*"):
            left.unlink(missing_ok=True)
    return 0


def main(argv: list[str]) -> int:
    how = told()
    # What of the variables of a test reached this program.
    note(argv=argv, given=sorted(name for name in os.environ if name.startswith("COLUMBINA_TEST_")))
    verb, rest = argv[0], argv[1:]
    if verb == "info":
        return info(how)
    if verb == "run":
        return run(rest, how)
    if verb == "exec":
        return execute(rest, how)
    if verb == "ps":
        return listed(rest, how)
    if verb == "rm":
        return remove(rest, how)
    return fail(2, f"the stand-in knows no command {verb!r}")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
