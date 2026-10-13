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
    One file for each container that is there, and one for each command that
    runs in one.
"""

import contextlib
import csv
import json
import os
import signal
import subprocess
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

# This is run as a program, from wherever. What it shares with the tests is found from here.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from support.processes import alive

HOME = Path(os.environ["COLUMBINA_FAKE_DOCKER"])
CONTAINERS = HOME / "containers"
LOOKED = HOME / "looked"
"""There when a stop has looked and found nothing, where a test has a command wait for that."""

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
    for _ in range(200):
        try:
            return json.loads((HOME / "told.json").read_text(encoding="utf-8"))
        except FileNotFoundError:
            return {}
        except PermissionError:
            # The test puts another file in its place at this moment, and Windows lets
            # nobody open it meanwhile.
            time.sleep(0.01)
    raise RuntimeError("what the test told could not be read")


def shown(path: Path, what: object) -> None:
    """Write a file so that whoever looks for it finds all of it or nothing.

    A test looks while this writes. Under its own name the file appears only when it is
    written to its end.
    """
    unfinished = path.with_name(f"{path.name}.{os.getpid()}.part")
    unfinished.write_text(json.dumps(what), encoding="utf-8")
    unfinished.replace(path)


def cleared(path: Path) -> None:
    """Take a file away, if it is there.

    Several of these programs take the same file away at the same moment, and Windows
    refuses one of them for as long as the other is at it.
    """
    for _ in range(200):
        try:
            path.unlink(missing_ok=True)
        except PermissionError:
            time.sleep(0.01)
        else:
            return
    raise RuntimeError(f"{path.name} could not be taken away")


def until(so: Callable[[], bool], seconds: float = 30.0) -> None:
    """Wait until something is so. A wait that nothing ends is given up after a long time."""
    deadline = time.monotonic() + seconds
    while not so() and time.monotonic() < deadline:
        time.sleep(0.01)


def note(**what: object) -> None:
    """Write down a call, in a file of its own: several of these run at the same time."""
    (HOME / "calls").mkdir(exist_ok=True)
    # The clock of Windows stands still for milliseconds, and a process may note twice in one.
    NOTED.append(None)
    name = f"{time.time_ns():020d}-{os.getpid()}-{len(NOTED)}.json"
    shown(HOME / "calls" / name, what)


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
    except OSError:
        # It is not there, or it is being taken away, which Windows tells in another way.
        return None
    return known if alive(known["pid"]) else None


def command(name: str, pid: int) -> Path:
    """Return where it is written down that a process is a command of a container.

    Each command has a file of its own, which is named after its process. What is in it
    says nothing. Several of these programs look and write at the same time: one starts a
    command while another stops what runs. With one list for all of them, a stop that
    clears the list would clear away a command that was written down after it had looked.
    """
    return CONTAINERS / f"{name}.{pid}.pid"


def commands(name: str) -> list[int]:
    """Return the processes that are written down as the commands of a container."""
    return sorted(int(known.name.split(".")[-2]) for known in CONTAINERS.glob(f"{name}.*.pid"))


def forgotten(name: str) -> None:
    """Take away all that is written down of a container that is gone.

    A file that another of these programs is still writing is left to that program: it
    has another name until it is written to its end.
    """
    for left in CONTAINERS.glob(f"{name}.*"):
        if left.suffix != ".part":
            cleared(left)


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
    if how.get("run") == "lost":
        # The engine made a container, and the program lost the engine before a word came.
        return fail(125, 'level=error msg="error waiting for container: unexpected EOF"')
    CONTAINERS.mkdir(exist_ok=True)
    known = {"pid": os.getpid(), "mounts": mounted, "image": image}
    shown(CONTAINERS / f"{name}.json", known)
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
    forgotten(name)
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
        numbers(known, workdir, str(how.get("numbers", "same")), words)
        return int(how.get("usable_exit", 0))
    if how.get("exec") == "refused":
        return fail(1, "Error response from daemon: the container is paused")
    time.sleep(float(how.get("late", 0)))
    if how.get("stop") == "waits-for-a-start":
        # A test has the command start when a stop has looked and found nothing, not before.
        until(LOOKED.exists)
    # The command runs where its directory really is.
    source = next(mount["source"] for mount in known["mounts"] if mount["target"] == workdir)
    # Ending the program that asked for a command does not end the command in a container.
    # Where the system lets a process leave the one that started it, it is the same here.
    apart: dict[str, Any] = {} if sys.platform == "win32" else {"start_new_session": True}
    child = subprocess.Popen(words, cwd=source, env={**os.environ, **given}, **apart)
    # Under its name the file is there when it is closed: a stop that finds it takes it away.
    shown(command(name, child.pid), None)
    note(started=child.pid)
    code = child.wait()
    # It has ended, and its number is free for another process from here on. Nobody is to
    # end that one for a command. Where this program is ended first, the command is still
    # written down, and a stop finds it.
    cleared(command(name, child.pid))
    return STOPPED if code in (STOPPED, -signal.SIGTERM, -9) else code


def numbers(known: dict[str, Any], workdir: str, way: str, over: list[str]) -> None:
    """Say by which numbers a container knows what it was given, as its ``stat`` would.

    That is the working directory, the directory of the token, a directory in that one,
    and then each thing that is mounted over a name in the working directory and was asked
    about. A test says in which way the numbers are to differ from those of this machine:
    ``none`` for an image without the program, ``shifted`` for a share that adds to every
    inode and has devices of its own, ``unlike`` for numbers that have nothing to do with
    those of this machine, ``another`` and ``another-device`` for a working directory that
    is not the one that was mounted, ``another-cover`` for something else over a name.
    """
    if way == "none":
        return
    sources = {mount["target"]: mount["source"] for mount in known["mounts"]}
    probe = Path(sources[PROBE])
    covers = [Path(sources[f"{workdir}/{name.removeprefix('./')}"]) for name in over]
    for at, source in enumerate((Path(sources[workdir]), probe, probe / "known", *covers)):
        told = source.stat()
        device, inode = told.st_dev, told.st_ino
        if way == "shifted":
            device, inode = 73, inode + 2
        if at == 0 and way == "another":
            inode += 1
        if at == 0 and way == "another-device":
            device += 1
        if at == 2 and way == "unlike":
            inode += 5
        if at > 2 and way == "another-cover":
            inode += 1
        print(f"{device}:{inode}", flush=True)


def stop(name: str, how: dict[str, Any]) -> int:
    """Do what the stop of the provider does in a real container: end every command in it.

    It ends what it saw when it looked. A command that starts after the look is left for
    the next stop, as it is in a real container.
    """
    seen = commands(name)
    running = [pid for pid in seen if alive(pid)]
    note(stop=name, running=len(running))
    if how.get("stop") == "fails":
        return fail(1, "Error response from daemon: the engine is out of sorts")
    hung = HOME / "hung"
    if how.get("stop") == "hangs" or (how.get("stop") == "hangs-once" and not hung.exists()):
        hung.write_text("")
        time.sleep(60)
    if how.get("stop") == "left":
        print("left", flush=True)
        return 0
    if how.get("stop") == "waits-for-a-start" and not seen and not LOOKED.exists():
        # A test has a command start between the look of a stop and what the stop does
        # then. The first stop that finds nothing says that it has looked, and goes on when
        # a command is written down.
        LOOKED.write_text("")
        until(lambda: bool(commands(name)))
    for pid in running:
        end(pid)
    for pid in seen:
        cleared(command(name, pid))
    print("none", flush=True)
    return int(how.get("stop_exit", 0))


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
    if how.get("rm") == "slow":
        time.sleep(1.5)
    known = container(name)
    if known is not None:
        for pid in (*commands(name), known["pid"]):
            end(pid)
        forgotten(name)
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
