"""The provider that runs commands in a container of Docker.

Only the ``docker`` program is used, the way a person would use it. There is
nothing here that the engine has to be told about beforehand, and nothing is
left with it afterwards.

**The program.** The system would find ``docker`` by its name, and on
Windows it looks in the directory that this program was started in before it
looks anywhere else. That directory is often a repository. So the looking is
done here, :func:`found`: only the entries of the search path that are whole
paths count, and none that lies in a working directory. The program is run
by its name many times, and a name may be a link, so the whole way from the
name to the file is walked, :func:`trusted`: a link or a directory on that
way that lies in a working directory could be changed by a command in a
container, which would then choose what this machine runs next.

**The engine.** Not every engine that answers to ``docker`` holds a command
the way this provider needs. What an engine says of itself is read by
:func:`supported`, and an engine that maps the users of a container to other
users of the machine, or that is too old to leave out what is mounted below
a directory, is refused in words that say which of the two it is.

**What a command sees.** Of this machine a container gets the working
directory and nothing else, :func:`mounts`. In ``read-only`` mode the
directory cannot be written. In ``workspace-write`` mode the protected names
at its top are mounted over it once more, read-only. A source is never taken
from a name alone: the engine follows a link that it is given as a source,
so a ``.git`` that is a link to somewhere else would bring that place into
the container. Every source is resolved here first. What leads out of the
working directory gets no mount, and inside the container such a link then
leads nowhere.

**Which directory.** The engine is given the working directory by its name,
and a name can be made to lead elsewhere by whoever can write the directory
above it: a command of another session whose working directory that is. So
a session keeps the place that its name first led to. And between the look
at a directory and its mount nothing may be put in its place. On Windows
this program holds the directory open for that long, which keeps everyone
from moving it. Elsewhere the container is asked by which numbers it knows
its working directory, :func:`given`, and is not used if they are those of
another one.

**Who a command is.** What keeps a container alive runs in it as root, and
commands run as another user, :func:`user_of_commands`. With every
capability dropped, that user cannot send the keeper a signal and cannot
change the programs of the image. What a command left running is stopped by
killing every process of that user, which the kernel does in one go and
which needs no record of what was started.
"""

import asyncio
import contextlib
import errno
import logging
import os
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import weakref
from collections.abc import Callable, Coroutine, Sequence
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Literal, TypeVar

from pydantic import BaseModel, ConfigDict, Field, ValidationError, field_validator

from columbina import process
from columbina.process import Completed, Writable
from columbina.sandbox.base import (
    PROTECTED,
    Mode,
    SandboxUnavailableError,
    Strength,
    held_less_well,
)
from columbina.session import Session

__all__ = [
    "DEFAULT_IMAGE",
    "HOME",
    "PROBE",
    "Docker",
    "DockerConfig",
    "Engine",
    "absent",
    "asked",
    "called",
    "ended",
    "found",
    "given",
    "kept",
    "mounts",
    "quoted",
    "removed",
    "shape",
    "sockets",
    "supported",
    "there",
    "told",
    "trusted",
    "unavailable",
    "usable",
    "user_of_commands",
]

DEFAULT_IMAGE = "python:3.12"
"""The image of a container, unless the config names another."""

PROBE = "/run/columbina-probe"
"""Where a container finds the token that shows it sees the files of this machine."""

HOME = "/tmp/columbina-home"
"""The home directory of a command, in its container. It is made for each command."""

logger = logging.getLogger(__name__)

_PROGRAM = "docker.exe" if sys.platform == "win32" else "docker"

_WORKSPACE = "/workspace"
"""Where a container has the working directory, on a machine whose paths it cannot have."""

_ASK_SECONDS = 30.0
"""How long the engine has for an answer."""

_STOP_SECONDS = 10.0
"""How long stopping what a command left may take, all rounds of it together."""

_ROUND_SECONDS = 0.1
"""How long the end of a program that asked for a command is waited for between two rounds."""

_REMOVE_SECONDS = 30.0
"""How long the engine has to remove a container."""

_KEEPER_SECONDS = 10.0
"""How long what kept a container is waited for once the container is let go."""

_LOOK_SECONDS = 5.0
"""How often the containers are looked through for those that nobody needs."""

_SOCKET_ENTRIES = 200_000
"""How many entries of a working directory are looked at for sockets, at most."""

_SOCKET_SECONDS = 3.0
"""How long the look for sockets in a working directory may take."""

_TOO_LARGE = "the working directory is too large to be looked through for sockets"

_ATTEMPTS = 3
"""How many containers a command looks at before it gives up: the one that its session
has, one that was being made for it, and a new one."""

_NEW_ONE = (
    "The next command gets a new container, without what was installed outside the working "
    "directory."
)
_ENDED = (
    f"The container ended while this command ran, so the command may have been cut short. "
    f"{_NEW_ONE}"
)
_LEFT = (
    "What this command left running in the container could not be stopped, so the container "
    f"was removed. {_NEW_ONE}"
)
_STILL = (
    "What this command left running in the container could not be stopped, and the container "
    "could not be seen to be gone: it may go on running. Tell the user to remove it with: "
    f"docker rm -f {{name}}. {_NEW_ONE}"
)
_RESHAPED = (
    "This command changed what .git or .columbina is at the top of the working directory. "
    f"{_NEW_ONE} There a command cannot write what is new of the two."
)
"""What a result says when a command cost its session the container."""

_USERS = (
    "Either the engine maps users in a way that this sandbox does not work with, or this "
    "program has the directory through one of its further groups, which commands in a "
    "container do not have"
)
"""What may be behind a user of commands who cannot do what this program can."""

_UNASKED = (
    "this engine does not show a container the files of this machine by the numbers that the "
    "machine knows them by, so a container cannot be asked which directory it was given"
)
"""Why commands are held in part only where a container cannot say what it was given."""

_ASKS = sys.platform != "win32"
"""Whether a container is asked which directory it was given.

Where this program can hold the working directory while a container is made, nothing is
put in its place meanwhile, and what a share makes of the numbers does not matter.
"""

if sys.platform == "win32":  # pragma: windows only
    import _winapi

    _APART: dict[str, Any] = {"creationflags": subprocess.CREATE_NO_WINDOW}
    """What keeps a container has no window, and hears nothing of the keys of the user."""

    def _me() -> tuple[int, int]:
        """Return the numbers of the user and the group of this program, which Windows has not."""
        return 0, 0

    def _anchored(cwd: Path) -> int | None:
        """Hold a directory open so that nothing can be put in its place, and return the hold.

        Windows lets nobody move aside a directory that a program has open without leave to
        delete it, nor a directory above that one. That goes for the share through which a
        container has the files as well.

        Raises:
            OSError: The directory cannot be opened.
        """
        read_and_write, a_directory = 1 | 2, 0x02000000
        return _winapi.CreateFile(
            str(cwd),
            _winapi.GENERIC_READ,
            read_and_write,
            0,
            _winapi.OPEN_EXISTING,
            a_directory,
            0,
        )

    def _released(hold: int | None) -> None:
        """Let go of what :func:`_anchored` holds."""
        if hold is not None:
            _winapi.CloseHandle(hold)

    def _reached(cwd: Path) -> tuple[int, int] | None:
        """Return the numbers of a directory, if no link lies on the way to it.

        It is asked by its name. Whoever holds the directory meanwhile knows that the answer
        stays true.

        Raises:
            FileNotFoundError: Nothing is there.
            OSError: What is there is no directory, or a link lies on the way.
        """
        if Path(os.path.realpath(cwd)) != cwd:
            raise NotADirectoryError(errno.ENOTDIR, "a link lies on the way to it", str(cwd))
        told = cwd.stat()
        if not stat.S_ISDIR(told.st_mode):
            raise NotADirectoryError(errno.ENOTDIR, "it is no directory", str(cwd))
        return _numbers(told)

else:  # pragma: posix only
    _APART: dict[str, Any] = {"start_new_session": True}
    """What keeps a container is in a session of its own, away from the keys of the user."""

    def _me() -> tuple[int, int]:
        """Return the numbers of the user and the group of this program."""
        return os.getuid(), os.getgid()

    def _anchored(cwd: Path) -> int | None:
        """Hold a directory so that nothing can be put in its place: here nothing does that."""
        return None

    def _released(hold: int | None) -> None:
        """Let go of what :func:`_anchored` holds."""

    def _reached(cwd: Path) -> tuple[int, int] | None:
        """Return the numbers of a directory that is reached without following a link.

        Each part of the path is opened from the one before it, and none of them may be a
        link. What is opened so is the directory itself, whatever becomes of its name.

        Raises:
            FileNotFoundError: Nothing is there.
            OSError: What is there is no directory, or a link lies on the way.
        """
        how = getattr(os, "O_PATH", os.O_RDONLY) | os.O_DIRECTORY
        at = os.open(cwd.anchor, how)
        try:
            for part in cwd.parts[1:]:
                below = os.open(part, how | os.O_NOFOLLOW, dir_fd=at)
                os.close(at)
                at = below
            return _numbers(os.fstat(at))
        finally:
            os.close(at)


_HOPS = 40
"""How many links are followed on the way from the name of a program to its file."""

_STEPS_BACK_FIRST = sys.platform == "win32"
"""Whether the system takes the steps back out of a path before it looks at anything.

Windows does: a path that goes into a link and one step back names the directory that the
link lies in, whatever the link leads to. The other systems go where the link leads and take
the directory above that.
"""

_OLDEST = 25
"""The first version of the engine that can leave out what is mounted below a directory."""

_INFO = (
    '{"version":{{json .ServerVersion}},"system":{{json .OperatingSystem}},'
    '"security":{{json .SecurityOptions}},"os":{{json .OSType}}}'
)
"""What the engine is asked to say of itself, as one object of JSON."""

_ENTRY = (
    "if command -v bash >/dev/null 2>&1; "
    f'then echo "ready $(cat {PROBE}/token 2>/dev/null)"; '
    "else echo missing; fi; exec cat"
)
"""What a container runs: it says whether commands can run in it, and then waits.

It waits by reading its input, which this program holds. When the input
ends, for whatever reason, the container ends.
"""

_CALL = 'mkdir -p "$HOME" && exec "$@"'
"""What a command is handed to: it makes the home directory and becomes the command."""

_STOP = """uid=$1
round=0
while :; do
  kill -KILL -1 2>/dev/null
  left=
  for entry in /proc/[0-9]*; do
    [ "$entry" = "/proc/$$" ] && continue
    while read -r key value rest; do
      if [ "$key" = "Uid:" ]; then
        [ "$value" = "$uid" ] && left=yes
        break
      fi
    done 2>/dev/null < "$entry/status"
  done
  [ -z "$left" ] && { echo none; exit 0; }
  round=$((round + 1))
  [ "$round" -ge 50 ] && { echo left; exit 0; }
done
"""
"""Stops every process of the user that runs it, and says whether none is left.

It uses nothing but what a shell has built in. A command may have used up
the processes that a container may have, and then no program can be started
to look for them.
"""

_KNOWN = "known"
"""A directory beside the token. With it a container shows two directories that this
machine knows, and that tells how the numbers of the container go with those of the machine."""

_USABLE = (
    "test -r . && test -x . && { if test -w .; then echo writable; else echo readable; fi; "
    f"stat -c %d:%i . {PROBE} {PROBE}/{_KNOWN} 2>/dev/null; true; }}"
)
"""Says what the user that runs it can do with the directory it is run in.

After that it says by which numbers it knows that directory and the two directories that
came with the token, if the image has the program for it.
"""

_UNREACHABLE = (
    "failed to connect to the docker api",
    "cannot connect to the docker daemon",
    "error during connect",
    "docker daemon is not running",
)
"""What the ``docker`` program says, in lower case, when no engine answers it."""


class DockerConfig(BaseModel):
    """The config of the provider.

    Attributes:
        program: The whole path of the ``docker`` program. Without it the
            program is looked for on the search path.
        image: The image of a container. It has to have bash, and it is not
            fetched: a command fails with a word about it if it is not on
            this machine.
        network: ``none`` for a container without a network, or ``bridge``
            for the usual network of Docker. With a network a command is
            held in part only, since it can send away what it reads.
        env: Variables for the commands in a container.
        pids_limit: How many processes a container may have.
        memory: How much memory a container may have, as Docker writes it:
            a number, and ``b``, ``k``, ``m`` or ``g`` after it.
        cpus: How many processors a container may use.
        start_timeout_seconds: How long a container is waited for.
        idle_seconds: After how long without a command a container is let go.
    """

    model_config = ConfigDict(extra="forbid")

    program: str | None = None
    image: str = Field(default=DEFAULT_IMAGE, pattern=r"^[A-Za-z0-9][A-Za-z0-9._/:@-]*$")
    network: Literal["none", "bridge"] = "none"
    env: dict[str, str] = Field(default_factory=dict)
    pids_limit: int = Field(default=2048, ge=16)
    memory: str | None = Field(default=None, pattern=r"^[1-9][0-9]*[bkmg]?$")
    cpus: float | None = Field(default=None, gt=0)
    start_timeout_seconds: float = Field(default=60.0, gt=0)
    idle_seconds: float = Field(default=1800.0, gt=0)

    @field_validator("program")
    @classmethod
    def _whole(cls, program: str | None) -> str | None:
        if program is not None and not Path(program).is_absolute():
            raise ValueError(
                f"the docker program is named by its whole path, and {program!r} is none"
            )
        return program

    @field_validator("env")
    @classmethod
    def _named(cls, env: dict[str, str]) -> dict[str, str]:
        for key, value in env.items():
            if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key) is None:
                raise ValueError(f"{key!r} is no name of a variable of the environment")
            if "\0" in value:
                raise ValueError(
                    f"the value of the variable {key} has a character that none can have"
                )
        return env


class _Told(BaseModel):
    """What an engine answers when it is asked what it is."""

    version: str
    system: str
    security: list[str] | None
    os: str


@dataclass(frozen=True, slots=True)
class Engine:
    """What an engine says of itself.

    Attributes:
        version: Its version.
        system: What it runs on, in its own words. They are for showing and
            for nothing else.
    """

    version: str
    system: str


def _plain(target: str) -> str:
    """Return what a link leads to as a plain path.

    Windows tells of some links in the long form of a path, which begins
    with a mark of its own. It names the same place.
    """
    if target.startswith("\\\\?\\UNC\\"):
        return "\\\\" + target[8:]
    if target.startswith("\\\\?\\"):
        return target[4:]
    return target


def _leads(here: Path) -> str | None:
    """Return what a link says it leads to, or ``None`` for a name that is no link."""
    try:
        return _plain(str(here.readlink()))
    except (OSError, ValueError):
        # A file or a directory, or nothing. Windows also has names that are neither a
        # plain file nor a link, such as a file that is fetched only when it is opened, and
        # of those Python says so in a way of its own.
        return None


def _numbers(told: os.stat_result) -> tuple[int, int] | None:
    """Return the numbers by which the system tells a file or a directory from every other.

    They are its device and its inode, and they are the same under every
    name that it goes by. ``None`` stands for a file system that numbers
    nothing.
    """
    return (told.st_dev, told.st_ino) if told.st_ino else None


def _itself(place: Path) -> tuple[int, int] | None:
    """Return the numbers of what a path leads to, or ``None`` if nothing is there."""
    try:
        return _numbers(place.stat())
    except OSError:
        return None


def trusted(program: Path, outside: Sequence[Path]) -> bool:
    """Tell whether a program can be run by its name without going through a working directory.

    The name is walked the way the system walks it, one part after the
    other, and a link is followed to where it leads. At no step may the way
    be in one of the directories ``outside``: what lies there, a link or a
    directory or the file itself, can be changed by whoever can write there,
    also after this look. Whether a program lies at the end of the way is
    another question.

    A directory is known by its name and by what it is. A system that takes
    no notice of case may leave a name the way it was written, and then one
    directory has more names than can be compared.

    Args:
        program: The whole path of the program, as it will be run.
        outside: Directories that the way must not touch, each with every
            link on its own path resolved.

    Returns:
        Whether the way stays outside all of them, to its end.
    """
    if _STEPS_BACK_FIRST:
        program = Path(os.path.normpath(program))
    apart = {_itself(place) for place in outside} - {None}
    done = Path(program.anchor)
    left = list(program.parts[1:])
    hops = 0
    while left:
        part = left.pop(0)
        if part == "..":
            done = done.parent
            continue
        here = done / part
        leads = _leads(here)
        if leads is None:
            # Its name is taken as the system spells it, which may be another way than it
            # was written.
            here = Path(os.path.realpath(here))
            if any(here == place or place in here.parents for place in outside):
                return False
            if _itself(here) in apart:
                return False
            done = here
            continue
        hops += 1
        if hops > _HOPS:
            return False
        # The link lies in a directory that is outside all of them. Where it leads is
        # walked from the start, and counted from the directory of the link.
        way = done / leads
        done, left = Path(way.anchor), [*way.parts[1:], *left]
    return True


def found(search: str, outside: Sequence[Path] = ()) -> str | None:
    """Return the path of the ``docker`` program that a search path leads to, or ``None``.

    Args:
        search: The search path, as the environment has it.
        outside: Directories that the program must not come from, and that
            no link on the way to it may lie in: the directory that this
            program was started in, and the working directory of a session.
    """
    apart = [Path(os.path.realpath(directory)) for directory in outside]
    for written in search.split(os.pathsep):
        entry = Path(written.strip('"'))
        program = entry / _PROGRAM
        # An entry that is no whole path means a place below the current directory.
        if not entry.is_absolute():
            continue
        if program.is_file() and os.access(program, os.X_OK) and trusted(program, apart):
            return str(program)
    return None


def absent(config: DockerConfig, search: str, outside: Sequence[Path]) -> str | None:
    """Say what is wrong if there is no ``docker`` program to run, or return ``None``.

    Nothing is run for this, and no engine is asked: an engine may come up
    later, and a program that is not there will not.

    Args:
        config: The config of the provider.
        search: The search path, as the environment has it.
        outside: Directories that the program must not come from.
    """
    named = config.program
    if named is None:
        if found(search, outside) is not None:
            return None
        return (
            "commands are to run in containers, and no docker program was found on the search "
            "path. No command will run until there is one: install Docker, or name the "
            "program in the config of the sandbox"
        )
    if Path(named).is_file():
        return None
    return (
        f"commands are to run in containers with the docker program {named}, which is not "
        "there. No command will run until it is"
    )


def told(config: DockerConfig, platform: str) -> dict[str, str]:
    """Return what a model is to know of the container that its commands run in.

    Args:
        config: The config of the provider.
        platform: The platform of this machine, as ``sys.platform`` has it.

    Returns:
        The facts, each under the name that it is told by.
    """
    where = "the same path as here"
    if platform == "win32":
        where = (
            f"{_WORKSPACE}. The file tools take a path that begins with {_WORKSPACE} for the "
            "file of this machine"
        )
    network = "no network" if config.network == "none" else "the network of Docker"
    return {
        "commands run in": (
            f"a Linux container of the image {config.image}, as a user that is not root. Of "
            "this machine the container has the working directory and nothing else, at "
            f"{where}. It has {network}. What a command installs outside the working "
            "directory is gone when the container is, and nothing can be installed into the "
            "directories of the system."
        )
    }


def asked(program: str) -> list[str]:
    """Return the command that asks the engine what it is."""
    return [program, "info", "--format", _INFO]


def supported(said: str) -> Engine:
    """Read what an engine said of itself, and return it if commands can be held with it.

    Raises:
        SandboxUnavailableError: It did not say, or it is an engine that this
            provider does not work with. The text says which.
    """
    try:
        told = _Told.model_validate_json(said)
    except ValidationError:
        raise SandboxUnavailableError(
            "the Docker engine did not say what it is, so it is not known whether it holds a "
            "command"
        ) from None
    if told.os != "linux":
        raise SandboxUnavailableError(
            f"the Docker engine runs containers of the system {told.os!r}, and commands need a "
            "Linux container"
        )
    number = re.match(r"(\d+)\.", told.version)
    if number is None or int(number.group(1)) < _OLDEST:
        raise SandboxUnavailableError(
            f"the Docker engine is version {told.version}, and version {_OLDEST} or newer is needed"
        )
    for mapping in ("rootless", "userns"):
        if any(option.startswith(f"name={mapping}") for option in told.security or ()):
            raise SandboxUnavailableError(
                "the Docker engine maps the users of a container to other users of this "
                f"machine ({mapping}), and this sandbox does not work with such an engine"
            )
    return Engine(told.version, told.system)


def user_of_commands(platform: str, uid: int, gid: int) -> str:
    """Return the user that commands run as in a container, as ``docker`` takes it.

    On Linux a container sees the files of the working directory as the
    kernel has them, so commands run as the user of this program: they can
    write what that user can write, and what they make belongs to that user.
    Elsewhere the files reach a container through a share that shows them as
    writable by anyone, and any user that is not root will do.

    Args:
        platform: The platform of this machine, as ``sys.platform`` has it.
        uid: The number of the user of this program.
        gid: The number of its group.

    Raises:
        SandboxUnavailableError: This program runs as root on Linux. A
            command would then be the same user as what keeps its container
            alive, and could stop it.
    """
    if platform != "linux":
        return "1000:1000"
    if uid == 0:
        raise SandboxUnavailableError(
            "this program runs as root, and commands in a container would be root as well. "
            "Run it as an ordinary user"
        )
    return f"{uid}:{gid}"


@dataclass(frozen=True, slots=True)
class _Protected:
    """A protected name at the top of a directory, from one look at the disk.

    Attributes:
        name: The name, as it is written there.
        kind: What it is, in words that are compared and not shown.
        cover: What is mounted read-only for it, if anything: the name
            itself, or the first directory below the working directory on
            the way to what a link names.
        whole: Whether the directory has to be read-only as a whole for it.
    """

    name: str
    kind: str
    cover: Path | None
    whole: bool


def _what(path: Path) -> str:
    """Say in a word what lies at a path.

    ``unknown`` is what the system will not look at. Windows answers so for
    a link that a command made in a container, with an error that is none
    of those for a thing that is not there.
    """
    try:
        mode = path.stat().st_mode
    except (FileNotFoundError, NotADirectoryError):
        return "nothing"
    except OSError:
        return "unknown"
    if stat.S_ISDIR(mode):
        return "directory"
    return "file" if stat.S_ISREG(mode) else "other"


def _led(cwd: Path) -> list[_Protected]:
    """Return the protected names at the top of a directory, and what each of them asks for.

    A name that is a link protects what it leads to, as far as that lies in
    the directory. What is mounted read-only for it is the first directory
    on the way there. With only the end of the way read-only, a command
    could move a directory above it aside and make another one, and the link
    would name whatever the command put there. What is mounted cannot be
    moved aside.
    """
    names: list[_Protected] = []
    with os.scandir(cwd) as entries:
        for entry in entries:
            # The disk may not tell upper case from lower, so neither does this.
            if entry.name.casefold() not in PROTECTED:
                continue
            real = Path(os.path.realpath(entry.path))
            what = _what(real)
            kind = what if real == cwd / entry.name else f"link to {what} {real}"
            cover, whole = None, real == cwd
            if cwd in real.parents:
                first = cwd / real.relative_to(cwd).parts[0]
                there = what if first == real else _what(first)
                if there in ("directory", "file"):
                    cover = first
                # What cannot be looked at cannot be mounted over, and may lead anywhere.
                whole = there == "unknown"
            names.append(_Protected(entry.name, kind, cover, whole))
    return sorted(names, key=lambda one: one.name)


def shape(cwd: Path) -> tuple[tuple[str, str], ...]:
    """Return what the protected names at the top of a directory are.

    A container is made for the shape that it finds. A command may give the
    directory another one: it can make a ``.git`` where there was none, it
    can put a directory in the place of a link, and it can make what a link
    names and nothing was. Whoever keeps a container compares the shape
    after a command with the one before it.

    Args:
        cwd: The directory, with every link on its path resolved.

    Returns:
        Each such name as it is written there, and what it is: a directory,
        a file, a link with what it leads to and where, something else, or
        something that the system will not look at.
    """
    return tuple((one.name, one.kind) for one in _led(cwd))


def quoted(field: str) -> str:
    """Return one field of a mount the way ``docker`` reads it.

    The fields of a mount are separated by commas, and are read the way a
    line of CSV is. A field that has a comma, a quote or the end of a line in
    it is put in quotes, with every quote in it written twice.
    """
    if any(character in field for character in ',"\r\n'):
        return '"' + field.replace('"', '""') + '"'
    return field


def _mount(source: Path, target: str, *, readonly: bool) -> str:
    """Return one mount, the way ``docker`` takes it.

    Raises:
        SandboxUnavailableError: A path has a line break in it. Docker reads
            the fields the way a line of CSV is read, and that takes the two
            characters at the end of a line of Windows for one, also between
            quotes: the mount would be of another path than the one written.
    """
    for written in (str(source), target):
        if "\r" in written or "\n" in written:
            raise SandboxUnavailableError(
                f"the path {written!r} has a line break in it, and Docker does not read such "
                "a path the way it is written. No container is made"
            )
    fields = ["type=bind", quoted(f"source={source}"), quoted(f"target={target}")]
    if readonly:
        fields.append("readonly")
    # What is mounted below the source stays out: it is another file system, with rules of
    # its own, and a mount that is read-only does not make it so on every kernel.
    fields.append("bind-recursive=disabled")
    return ",".join(fields)


def mounts(mode: Mode, cwd: Path, target: str) -> list[str]:
    """Return the mounts of a container for a working directory in a mode.

    Args:
        mode: The mode of the session.
        cwd: The working directory, with every link on its path resolved.
        target: Where the container has it.

    Returns:
        The working directory first. In ``workspace-write`` mode, what the
        protected names at its top are, read-only, and for a name that is a
        link the first directory on the way to what it names, as far as
        that lies in the working directory.

    Raises:
        SandboxUnavailableError: A path that would be mounted has a line
            break in it.
    """
    return _laid(mode, cwd, target, _led(cwd))[1]


def _laid(mode: Mode, cwd: Path, target: str, led: Sequence[_Protected]) -> tuple[bool, list[str]]:
    """Return whether a container gets a directory read-only as a whole, and its mounts.

    Args:
        mode: The mode of the session.
        cwd: The working directory, with every link on its path resolved.
        target: Where the container has it.
        led: The protected names at its top, from one look at it.
    """
    if mode == "danger-full-access":
        return False, [_mount(cwd, target, readonly=False)]
    # A session that was started inside a protected directory writes nothing there either.
    whole = mode == "read-only" or any(part.casefold() in PROTECTED for part in cwd.parts)
    # A protected name that leads to the directory itself protects all of it, and so does
    # one of which it cannot be told what it protects.
    if whole or any(one.whole for one in led):
        return True, [_mount(cwd, target, readonly=True)]
    over = {f"{target}/{one.cover.name}": one.cover for one in led if one.cover is not None}
    return False, [
        _mount(cwd, target, readonly=False),
        *(_mount(source, inside, readonly=True) for inside, source in sorted(over.items())),
    ]


def _seen(cwd: Path) -> tuple[tuple[str, str], ...] | None:
    """Return the shape of a directory, or ``None`` if the directory cannot be looked at."""
    try:
        return shape(cwd)
    except OSError:
        # It is gone, or it may not be read. Either way it is not what a container was made for.
        return None


def _looked(
    cwd: Path,
) -> tuple[tuple[int, int] | None, tuple[tuple[str, str], ...] | None] | None:
    """Return the numbers of a directory and its shape, or ``None`` if it is not reached.

    Not reached is a directory that is gone, and one whose name leads
    through a link now.
    """
    try:
        itself = _reached(cwd)
    except OSError:
        return None
    return itself, _seen(cwd)


def given(
    said: Sequence[str],
    itself: tuple[int, int] | None,
    token_at: tuple[int, int] | None,
    known: tuple[int, int] | None,
) -> bool | None:
    """Tell whether a container was given the directory that was looked at.

    A container says by which numbers it knows three directories: its
    working directory, the directory of its token, and a directory in that
    one. This machine has numbers of its own for all three. Where the files
    reach a container as the kernel of this machine has them, the numbers
    are the same. A share may shift the inodes, by the same amount for every
    directory, and has devices of its own. The two directories that came
    with the token tell which of these it is, and then the working directory
    has to fit.

    Args:
        said: What the container says of the three, ``device:inode`` each.
        itself: The numbers of the working directory on this machine.
        token_at: Those of the directory of the token.
        known: Those of the directory in it.

    Returns:
        Whether the working directory of the container is the one that this
        machine knows by ``itself``. ``None`` if the numbers of the
        container have nothing to do with those of this machine, or one of
        them is missing.
    """
    read = [re.fullmatch(r"(\d+):(\d+)", one) for one in said]
    if len(read) != 3 or itself is None or token_at is None or known is None:
        return None
    if read[0] is None or read[1] is None or read[2] is None:
        return None
    device, inode = int(read[0][1]), int(read[0][2])
    shift = int(read[1][2]) - token_at[1]
    if int(read[2][2]) - known[1] != shift:
        return None
    if inode - itself[1] != shift:
        return False
    # A device is compared where the container has the devices of this machine.
    return int(read[1][1]) != token_at[0] or device == itself[0]


def sockets(cwd: Path, entries: int, seconds: float) -> str | None:
    """Look through a directory for sockets, and say what stands against a full hold.

    A socket in the working directory is reached through a mount, read-only
    or not, and with no network. It is a door to whatever listens there on
    this machine, and that may be an agent that holds keys, or the engine of
    Docker itself. Whether one is there is known only by looking. Links are
    not followed: what they lead to is not in the container.

    Args:
        cwd: The directory.
        entries: How many entries are looked at, at most.
        seconds: How long the look may take.

    Returns:
        ``None`` if the whole directory was looked through and no socket is
        in it. Otherwise the reason why a command there is held in part
        only: a socket and where it lies, a part that could not be looked
        into, or that there was more than could be looked through.
    """
    until = time.monotonic() + seconds
    waiting = [cwd]
    seen = 0
    while waiting:
        directory = waiting.pop()
        try:
            with os.scandir(directory) as listed:
                for entry in listed:
                    seen += 1
                    if seen > entries or time.monotonic() >= until:
                        return _TOO_LARGE
                    if entry.is_dir(follow_symlinks=False):
                        waiting.append(Path(entry.path))
                    elif _is_socket(entry):
                        where = Path(entry.path).relative_to(cwd).as_posix()
                        return (
                            f"a socket lies in the working directory, at {where}, and a "
                            "command can talk to whatever listens there"
                        )
        except OSError:
            return "a part of the working directory could not be looked through for sockets"
    return None


def _is_socket(entry: os.DirEntry[str]) -> bool:
    """Tell whether an entry of a directory is a socket. The system is asked of few entries."""
    if entry.is_file(follow_symlinks=False) or entry.is_symlink():
        return False
    return stat.S_ISSOCK(entry.stat(follow_symlinks=False).st_mode)


if sys.platform == "win32":  # pragma: windows only

    def _doubt(cwd: Path) -> str | None:
        """Say what stands against a full hold of the commands in a directory: nothing.

        On Windows the files reach a container through a share into another
        kernel, and no socket goes that way.
        """
        return None

else:  # pragma: posix only

    def _doubt(cwd: Path) -> str | None:
        """Say what stands against a full hold of the commands in a directory, if anything."""
        return sockets(cwd, _SOCKET_ENTRIES, _SOCKET_SECONDS)


@dataclass(frozen=True, slots=True)
class _Layout:
    """What a container for a directory is given, worked out on the disk in one go.

    Attributes:
        mounted: Its mounts.
        read_only: Whether it gets the directory read-only as a whole.
        shape: What the protected names at the top of the directory are, if
            the mounts depend on that.
        doubt: Why commands in it are held in part only, if they are.
        probe: A directory of this machine that holds a token.
        token: The token.
        itself: The numbers of the directory, reached without a link.
        token_at: The numbers of the directory of the token.
        known: The numbers of a directory in that one.
        hold: What keeps the directory from being moved aside, where a
            program can do that. Whoever gets this lets go of it.
    """

    mounted: list[str]
    read_only: bool
    shape: tuple[tuple[str, str], ...] | None
    doubt: str | None
    probe: Path
    token: str
    itself: tuple[int, int] | None
    token_at: tuple[int, int] | None
    known: tuple[int, int] | None
    hold: int | None


def _layout(mode: Mode, cwd: Path, target: str) -> _Layout:
    """Work out what a container for a directory is given, and make the directory of its token.

    The directory is looked at once. The mounts and the shape that is kept
    of them come from that one look, so that the two cannot disagree. Where
    a program can hold a directory against being moved aside, it is held
    from before the look, and what is returned says so.

    Raises:
        SandboxUnavailableError: The directory is not there, or its name
            leads through a link now, or it cannot be looked at.
    """
    hold: int | None = None
    try:
        try:
            hold = _anchored(cwd)
            itself = _reached(cwd)
            led = _led(cwd)
        except FileNotFoundError:
            # Docker Desktop would make a source that is not there, on this machine.
            raise SandboxUnavailableError(f"the working directory {cwd} is not there") from None
        except OSError as error:
            if isinstance(error, NotADirectoryError) or error.errno in (errno.ELOOP, errno.EMLINK):
                raise SandboxUnavailableError(
                    f"the working directory {cwd} is no directory that can be reached without "
                    "following a link. Something may have been put in its place. No container "
                    "is made"
                ) from error
            raise SandboxUnavailableError(
                f"the working directory {cwd} could not be looked at: {error.strerror or error}"
            ) from error
        read_only, mounted = _laid(mode, cwd, target, led)
        told = tuple((one.name, one.kind) for one in led) if mode == "workspace-write" else None
        doubt = _doubt(cwd)
        probe = Path(tempfile.mkdtemp(prefix="columbina-probe-"))
        token = secrets.token_hex(16)
        (probe / "token").write_text(token, encoding="ascii")
        (probe / _KNOWN).mkdir()
        # What keeps a container is a root that may read no more than anyone may.
        probe.chmod(0o755)
        (probe / "token").chmod(0o644)
        (probe / _KNOWN).chmod(0o755)
        at = _itself(probe), _itself(probe / _KNOWN)
        return _Layout(mounted, read_only, told, doubt, probe, token, itself, *at, hold)
    except BaseException:
        _released(hold)
        raise


def kept(
    program: str, config: DockerConfig, name: str, mounted: Sequence[str], probe: Path
) -> list[str]:
    """Return the command that starts a container and keeps it for as long as its input is open.

    Args:
        program: The ``docker`` program.
        config: The config of the provider.
        name: The name of the container.
        mounted: The mounts of the working directory, from :func:`mounts`.
        probe: A directory of this machine with a file ``token`` in it. The
            container says what it reads there, which shows whether the
            engine sees the files of this machine.
    """
    argv = [program, "run", "--rm", "--interactive", "--pull", "never", "--init"]
    # What keeps the container is root whatever user the image names, and commands are not.
    argv += ["--user", "0:0", "--name", name, "--label", "columbina=1"]
    argv += ["--network", config.network, "--cap-drop", "ALL"]
    argv += ["--security-opt", "no-new-privileges", "--pids-limit", str(config.pids_limit)]
    if config.memory is not None:
        argv += ["--memory", config.memory]
    if config.cpus is not None:
        argv += ["--cpus", f"{config.cpus:g}"]
    for one in (*mounted, _mount(probe, PROBE, readonly=True)):
        argv += ["--mount", one]
    return [*argv, "--workdir", "/", "--entrypoint", "sh", config.image, "-c", _ENTRY]


def called(
    program: str, config: DockerConfig, name: str, user: str, target: str, argv: Sequence[str]
) -> list[str]:
    """Return the command that runs a command in a container, in the working directory.

    The command gets the variables of the image, those of the config, and a
    home directory of its own. Nothing of the environment of this program
    goes with it.

    Args:
        program: The ``docker`` program.
        config: The config of the provider.
        name: The name of the container.
        user: The user that the command runs as, from :func:`user_of_commands`.
        target: Where the container has the working directory.
        argv: The command.
    """
    env = {"LANG": "C.UTF-8", "TERM": "dumb", **config.env, "HOME": HOME}
    given = [word for key, value in env.items() for word in ("--env", f"{key}={value}")]
    start = [program, "exec", "--interactive", "--user", user, "--workdir", target, *given, name]
    return [*start, "sh", "-c", _CALL, "sh", *argv]


def ended(program: str, name: str, user: str) -> list[str]:
    """Return the command that stops all that the commands of a container left running.

    It prints ``none`` when nothing of the user of the commands is left, and
    ``left`` when something would not go.
    """
    number = user.partition(":")[0]
    return [program, "exec", "--user", user, name, "sh", "-c", _STOP, "sh", number]


def usable(program: str, name: str, user: str, target: str) -> list[str]:
    """Return the command that asks what a command can do with the working directory.

    It prints ``writable`` or ``readable``, and nothing if the user of the
    commands cannot so much as read the directory.
    """
    return [program, "exec", "--user", user, "--workdir", target, name, "sh", "-c", _USABLE]


def there(program: str, name: str) -> list[str]:
    """Return the command that prints the name of a container if the engine still has it."""
    return [program, "ps", "--all", "--filter", f"name=^{name}$", "--format", "{{.Names}}"]


def removed(program: str, name: str) -> list[str]:
    """Return the command that removes a container, whatever runs in it."""
    return [program, "rm", "--force", name]


def _unreachable(said: str) -> bool:
    """Tell whether what the ``docker`` program wrote says that no engine answered it."""
    low = said.casefold()
    return any(words in low for words in _UNREACHABLE)


def unavailable(image: str, said: str) -> str:
    """Say why no container could be started, in words for whoever can put it right.

    Args:
        image: The image that the container was to have.
        said: What the ``docker`` program wrote.
    """
    # The engine says "no such image" of an image that it does not have. The docker program
    # writes other words when it goes to fetch one, and those are not taken for these.
    low = said.casefold()
    if "no such image" in low:
        return (
            f"the image {image} is not on this machine, and no image is fetched for a command. "
            f"Fetch it with: docker pull {image}"
        )
    if _unreachable(said):
        return (
            "the Docker engine does not answer. Start Docker Desktop, or the service of Docker, "
            "and run the command again"
        )
    lines = [line.strip() for line in said.splitlines() if line.strip()]
    told = lines[0][:300] if lines else "it did not say why"
    return f"Docker could not start a container: {told}"


_T = TypeVar("_T")


async def _to_the_end(doing: Coroutine[Any, Any, _T]) -> _T:
    """Wait for something to its end, also when whoever waits is cancelled meanwhile.

    What is waited for here leaves something behind if it is cut short: a
    container that nobody keeps, or a command that nobody stops.

    Raises:
        asyncio.CancelledError: Whoever waited was cancelled. This is raised
            when the thing is done, and what it gave or failed with is lost.
    """
    task = asyncio.ensure_future(doing)
    cancelled = False
    while not task.done():
        try:
            await asyncio.wait({task})
        except asyncio.CancelledError:
            cancelled = True
    if cancelled:
        if not task.cancelled() and task.exception() is not None:
            logger.error(
                "what was done for a container failed after its caller had left",
                exc_info=task.exception(),
            )
        raise asyncio.CancelledError
    return task.result()


async def _ended(keeper: asyncio.subprocess.Process) -> None:
    """Wait for what kept a container to end, and end it if it does not."""
    try:
        await asyncio.wait_for(keeper.wait(), _KEEPER_SECONDS)
    except TimeoutError:
        with contextlib.suppress(ProcessLookupError):
            keeper.kill()
        await keeper.wait()


@dataclass(slots=True, eq=False)
class _Kept:
    """A container that is kept for a session.

    Attributes:
        session: The session.
        mode: The mode that the container was made for.
        cwd: The working directory that it was made for, on this machine.
        target: Where the container has that directory.
        program: The ``docker`` program that made it.
        user: The user that commands run as in it.
        name: Its name.
        probe: The directory of this machine that holds its token.
        shape: What the protected names at the top of the directory were
            when it was made, if its mounts depend on that.
        itself: The numbers of the directory when it was made. A directory
            that lies under the same name later may be another one.
        doubt: Why the commands in it are held in part only, if they are.
        used: When a command last ran in it, on the clock that only goes on.
        keeper: The program whose input keeps it alive.
        gone: Whether it has been let go.
        never: Whether it never came to be: what was to keep it ended
            without a word.
        leaving: The letting go of it, once that has begun. It gives whether
            the container was seen to be gone.
    """

    session: Session
    mode: Mode
    cwd: Path
    target: str
    program: str
    user: str
    name: str
    probe: Path
    shape: tuple[tuple[str, str], ...] | None
    itself: tuple[int, int] | None
    doubt: str | None
    used: float
    keeper: asyncio.subprocess.Process | None = None
    gone: bool = False
    never: bool = False
    leaving: asyncio.Future[bool] | None = None

    def alive(self) -> bool:
        """Tell whether what keeps the container is still there."""
        return self.keeper is not None and self.keeper.returncode is None


class Docker:
    """The provider: a command runs in a container that is kept for its session.

    A container is made when the first command of a session comes, and the
    commands of the session run in it one after another. After each of them
    everything that it left running is stopped. A container is let go when
    its session has another mode, when another directory lies where its
    working directory was, when the session is over, when no command came
    for a while, and whenever it is in doubt what runs in it. Its life hangs
    on a pipe that this program holds, so no container outlives this
    program, however it ends.

    Args:
        config: The config of the provider.
    """

    shell: str | None = "bash"

    def __init__(self, config: DockerConfig) -> None:
        self._config = config
        self._kept: dict[str, _Kept] = {}
        self._starting: dict[str, asyncio.Future[_Kept]] = {}
        self._turns: dict[str, asyncio.Lock] = {}
        self._engine: Engine | None = None
        self._tool: str | None = None
        self._choosing = asyncio.Lock()
        self._first: weakref.WeakKeyDictionary[Session, Path] = weakref.WeakKeyDictionary()
        self._leaving: set[asyncio.Future[bool]] = set()
        self._looking: asyncio.Task[None] | None = None
        self._over = asyncio.Event()

    def strength(self, session: Session) -> Strength:
        """Say how well the commands of a session are held.

        Fully, unless a container has a network, which lets a command send
        away what it reads, or the container of the session was found to
        leave a door open. Before a session has a container nothing has been
        found: what is said then is what the config promises.
        """
        if self._config.network != "none":
            return "partial"
        container = self._kept.get(session.id)
        return "full" if container is None or container.doubt is None else "partial"

    def facts(self, session: Session) -> dict[str, str]:
        """Return what a model is to know of the container that its commands run in."""
        return told(self._config, sys.platform)

    def workdir(self, session: Session) -> str | None:
        """Return where a container has the working directory, if not under its own path.

        On a machine whose paths a container cannot have, the directory is
        mounted at a path of the container's own.
        """
        return _WORKSPACE if sys.platform == "win32" else None

    async def run(
        self,
        argv: Sequence[str],
        *,
        session: Session,
        mode: Mode,
        cwd: Path,
        stdin: bytes | None,
        timeout_seconds: float,
        overflow: Callable[[], Writable] | None,
        max_overflow_bytes: int,
        expected: Strength,
    ) -> Completed:
        """Run a command in the container of its session, and say how it ended.

        The command is one of the place where the container runs: its
        program is looked for there. The arguments from ``stdin`` to
        ``max_overflow_bytes`` are those of :func:`columbina.process.run`.

        Raises:
            SandboxUnavailableError: No container could be made, or the
                command could not be handed to it, or the container does not
                hold it as well as was expected, or the working directory of
                the session leads elsewhere than it did. The command was not
                run.
        """
        async with self._turns.setdefault(session.id, asyncio.Lock()):
            real = Path(await asyncio.to_thread(os.path.realpath, cwd))
            first = self._first.setdefault(session, real)
            if first != real:
                # Whoever can write the directory above can make the name lead anywhere.
                # The file tools hold a session to its first place as well.
                raise SandboxUnavailableError(
                    f"the working directory of this session was {first} when its first "
                    f"command ran, and it leads to {real} now. No command is run there. Tell "
                    "the user that the session has to be started again"
                )
            container = await self._held(session, mode, real)
            if expected == "full" and self.strength(session) != "full":
                # Let through for a full hold, which the container that is there now does
                # not give. What stands against it was found when the container was made.
                raise held_less_well(container.doubt)
            return await self._ran(
                container, argv, stdin, timeout_seconds, overflow, max_overflow_bytes
            )

    async def close(self) -> None:
        """Let every container go. No command is run afterwards."""
        self._over.set()
        if self._looking is not None:
            # Whatever became of the look, the containers are let go below.
            await asyncio.gather(self._looking, return_exceptions=True)
        # A container that is being made is let go when it is there.
        await asyncio.gather(*self._starting.values(), return_exceptions=True)
        await asyncio.gather(
            *(
                self._let_go(container, "the program ends")
                for container in list(self._kept.values())
            )
        )
        # And those that a command or the look was letting go already. Whoever closes may
        # end the program next, and nothing is to be half removed then.
        await asyncio.gather(*self._leaving, return_exceptions=True)

    async def _held(self, session: Session, mode: Mode, cwd: Path) -> _Kept:
        """Return the container of a session for a mode and a directory, and make it if need be.

        The container that the session has may not be the one for this
        command, and then it is let go and another is made. That is done a
        few times and no more: something may end the containers of a session
        as they are made, or go on changing its directory.

        Raises:
            SandboxUnavailableError: No container could be made, or those
                that were there were of no use.
        """
        why = None
        for _ in range(_ATTEMPTS):
            container = await self._at_hand(session, mode, cwd)
            why = await self._outdated(container, mode)
            if why is None:
                return container
            await _to_the_end(self._let_go(container, why))
        raise SandboxUnavailableError(
            f"{_ATTEMPTS} containers in a row were of no use for this command. Of the last "
            f"one: {why}"
        )

    async def _at_hand(self, session: Session, mode: Mode, cwd: Path) -> _Kept:
        """Return the container that a session has, or the one being made for it, or a new one."""
        if self._over.is_set():
            raise SandboxUnavailableError("the sandbox is closed, so no command is run any more")
        starting = self._starting.get(session.id)
        if starting is None:
            container = self._kept.get(session.id)
            if container is not None:
                return container
            starting = asyncio.ensure_future(self._started(session, mode, cwd))
            self._starting[session.id] = starting
        # Whoever waits may be cancelled. The container is made all the same, and is there
        # for the next command: a start that is cut short may leave a container that nobody
        # knows of.
        container = await asyncio.shield(starting)
        if self._over.is_set():
            # Closed while it was made, and let go by whoever closed.
            raise SandboxUnavailableError("the sandbox is closed, so no command is run any more")
        return container

    async def _outdated(self, container: _Kept, mode: Mode) -> str | None:
        """Say why a container is not the one for a command, or return ``None`` if it is."""
        if container.gone or not container.alive():
            return "it has ended"
        if container.mode != mode:
            return "its session has another mode now"
        looked = await asyncio.to_thread(_looked, container.cwd)
        if looked is None or looked[0] != container.itself:
            # Moved aside or removed, by whoever else works there. A container goes on
            # showing the directory that it was given, whatever lies under its name.
            return "its working directory is not the directory that it was made for"
        if container.shape is not None and container.shape != looked[1]:
            # Something made or changed a protected name since: a command that was cut
            # short, or whoever else works in the directory.
            return "what is protected in its directory is not what it was"
        return None

    async def _started(self, session: Session, mode: Mode, cwd: Path) -> _Kept:
        """Make a container for a session, and keep it.

        Raises:
            SandboxUnavailableError: No container that holds commands could
                be made. Nothing is left of the attempt.
        """
        try:
            program = await self._chosen(cwd)
            user = user_of_commands(sys.platform, *_me())
            await self._known(program)
            target = self.workdir(session) or cwd.as_posix()
            layout = await asyncio.to_thread(_layout, mode, cwd, target)
            try:
                container = _Kept(
                    session=session,
                    mode=mode,
                    cwd=cwd,
                    target=target,
                    program=program,
                    user=user,
                    name=f"columbina-{secrets.token_hex(6)}",
                    probe=layout.probe,
                    shape=layout.shape,
                    itself=layout.itself,
                    doubt=layout.doubt,
                    used=time.monotonic(),
                )
                try:
                    await self._made(container, layout)
                except BaseException:
                    await _to_the_end(self._let_go(container, "it could not be made ready"))
                    raise
            finally:
                # Held from before the look. From here on the container has the directory
                # itself, whatever becomes of its name.
                _released(layout.hold)
            self._kept[session.id] = container
            if self._looking is None or self._looking.done():
                self._looking = asyncio.ensure_future(self._look())
            return container
        finally:
            del self._starting[session.id]

    async def _made(self, container: _Kept, layout: _Layout) -> None:
        """Start a container and see that commands can run in it as they are meant to."""
        start = kept(container.program, self._config, container.name, layout.mounted, layout.probe)
        try:
            container.keeper = await asyncio.create_subprocess_exec(
                *start,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=tempfile.gettempdir(),
                env=process.cleaned(os.environ),
                **_APART,
            )
        except OSError as error:
            raise SandboxUnavailableError(
                f"the docker program {container.program} could not be started: {error}"
            ) from error
        await self._ready(container, layout.token)
        await self._usable(container, layout)

    async def _ready(self, container: _Kept, token: str) -> None:
        """Wait for a container to say that commands can run in it.

        Raises:
            SandboxUnavailableError: It did not say so. The text says what it
                said, or what Docker said.
        """
        keeper = container.keeper
        assert keeper is not None
        assert keeper.stdout is not None
        assert keeper.stderr is not None
        image, seconds = self._config.image, self._config.start_timeout_seconds
        try:
            line = await asyncio.wait_for(keeper.stdout.readline(), seconds)
        except TimeoutError:
            raise SandboxUnavailableError(
                f"the container did not come up within {seconds:g} seconds"
            ) from None
        except ValueError:
            # More than a line can be, and no end of it: whatever runs there, it is not
            # what this program started.
            line = b"it wrote a line without end where it was to say that it is ready"
        said = line.decode("utf-8", errors="replace").strip()
        if said == f"ready {token}":
            return
        if said == "missing":
            raise SandboxUnavailableError(
                f"the image {image} has no bash, and commands are run with bash. Name an image "
                "that has it in the config of the sandbox"
            )
        if said.startswith("ready"):
            raise SandboxUnavailableError(
                "the Docker engine does not see the files of this machine, so it cannot give a "
                "container the working directory. It may run on another machine, or the "
                "directory of temporary files is not shared with it"
            )
        if not said:
            # It ended without a word, so Docker will have said why. A container that
            # runs says a word first, so none came to be.
            container.never = True
            with contextlib.suppress(TimeoutError):
                wrote = await asyncio.wait_for(keeper.stderr.read(), _KEEPER_SECONDS)
                said = wrote.decode("utf-8", errors="replace")
        raise SandboxUnavailableError(unavailable(image, said))

    async def _usable(self, container: _Kept, layout: _Layout) -> None:
        """See that a command can do with the working directory what its mode says, and no more.

        This is asked of the container, as the user of the commands. How an
        engine maps that user to one of this machine is not for this program
        to guess. The container also says which directory it was given,
        where that has to be asked.

        Raises:
            SandboxUnavailableError: A command could do less than it should,
                or more, or the container has another directory than the one
                that was looked at.
        """
        read_only = layout.read_only
        asking = usable(container.program, container.name, container.user, container.target)
        answer = await self._docker(asking, _ASK_SECONDS)
        said = answer.output.split()
        can = said[0] if said else ""
        if answer.exit_code != 0 or can not in ("writable", "readable"):
            raise SandboxUnavailableError(
                f"commands run in the container as the user {container.user}, who cannot read "
                f"the working directory there. {_USERS}"
            )
        if read_only and can == "writable":
            raise SandboxUnavailableError(
                "the working directory was to be read-only in the container, and a command "
                "could write it there"
            )
        if not read_only and can == "readable" and os.access(container.cwd, os.W_OK):
            raise SandboxUnavailableError(
                f"commands run in the container as the user {container.user}, who cannot "
                f"write the working directory there, although this program can. {_USERS}"
            )
        if not _ASKS:
            return
        same = given(said[1:], layout.itself, layout.token_at, layout.known)
        if same is False:
            raise SandboxUnavailableError(
                "the container was given another directory than the working directory of this "
                "session. Something put another directory or a link in its place while the "
                "container was made. No command is run in it"
            )
        if same is None:
            container.doubt = container.doubt or _UNASKED

    async def _chosen(self, cwd: Path) -> str:
        """Return the ``docker`` program for a container of a working directory.

        The program is looked for once, and then it serves for as long as
        this provider lives. A later look could find what a command has put
        into the working directory of its session since, if the search path
        leads there. So a directory that holds the program, or a link on the
        way to it, gets no container: its commands could write what every
        other container is asked with. One session is seen to at a time,
        for two that start together would each take the program of the
        other.

        Raises:
            SandboxUnavailableError: There is no program that can be trusted
                with this directory.
        """
        async with self._choosing:
            self._tool = await asyncio.to_thread(self._program, cwd)
            return self._tool

    def _program(self, cwd: Path) -> str:
        """Return the ``docker`` program that can be trusted with a working directory.

        Raises:
            SandboxUnavailableError: There is none.
        """
        outside = [Path(os.path.realpath(place)) for place in (Path.cwd(), cwd)]
        named = self._config.program
        program = self._tool if named is None else named
        if program is None:
            program = found(os.environ.get("PATH", ""), outside)
            if program is None:
                raise SandboxUnavailableError(
                    "no docker program was found on the search path. Install Docker, or name "
                    "the program in the config of the sandbox. A program that lies in the "
                    "working directory is not taken"
                )
            return program
        whose = "was found on the search path" if named is None else "the config names"
        if not Path(program).is_file():
            raise SandboxUnavailableError(
                f"the docker program that {whose}, {program}, is not there"
            )
        if not trusted(Path(program), outside):
            raise SandboxUnavailableError(
                f"the docker program that {whose}, {program}, lies in the working directory, or "
                "a link on the way to it does. A command could put another program there, so "
                "it is not run"
            )
        return program

    async def _known(self, program: str) -> None:
        """Ask the engine what it is, once, and see that commands can be held with it.

        Raises:
            SandboxUnavailableError: It does not answer, or it is an engine
                that this provider does not work with.
        """
        if self._engine is not None:
            return
        answer = await self._docker(asked(program), _ASK_SECONDS)
        if answer.exit_code != 0:
            wrote = f"{answer.output}\n{answer.error_output}"
            if _unreachable(wrote):
                raise SandboxUnavailableError(unavailable(self._config.image, wrote))
        self._engine = supported(answer.output if answer.exit_code == 0 else "")

    async def _docker(self, argv: Sequence[str], seconds: float) -> Completed:
        """Run the ``docker`` program for an answer, with what it says and its errors apart.

        Raises:
            SandboxUnavailableError: The program could not be started.
        """
        try:
            return await process.run(
                argv, cwd=Path(tempfile.gettempdir()), timeout_seconds=seconds, streams="apart"
            )
        except OSError as error:
            raise SandboxUnavailableError(str(error)) from error

    async def _ran(
        self,
        container: _Kept,
        argv: Sequence[str],
        stdin: bytes | None,
        seconds: float,
        overflow: Callable[[], Writable] | None,
        max_overflow_bytes: int,
    ) -> Completed:
        """Run a command in a container, and stop what it left there."""
        settled: list[bool] = []

        async def stop_inside(exited: asyncio.Future[None]) -> None:
            settled.append(await self._stopped(container, exited))

        command = called(
            container.program, self._config, container.name, container.user, container.target, argv
        )
        try:
            done = await process.run(
                command,
                cwd=Path(tempfile.gettempdir()),
                stdin=stdin,
                timeout_seconds=seconds,
                overflow=overflow,
                max_overflow_bytes=max_overflow_bytes,
                before_stop=stop_inside,
            )
        except OSError as error:
            raise SandboxUnavailableError(str(error)) from error
        except asyncio.CancelledError:
            if settled != [True]:
                # Whether the command ran in the container, runs there, or is yet to start
                # there, is not known. Nothing is to go on running, so the container goes.
                why = "a command was cancelled and could not be seen to end"
                await _to_the_end(self._let_go(container, why))
            raise
        finally:
            container.used = time.monotonic()
        notes = await _to_the_end(self._after(container, settled))
        return replace(done, notes=notes) if notes else done

    async def _after(self, container: _Kept, settled: Sequence[bool]) -> tuple[str, ...]:
        """Stop what a command left in a container, and return what is to be said of it.

        Args:
            container: The container.
            settled: Whether the command was stopped from inside and seen to
                end, if its time was up. Empty if it ended by itself.
        """
        # Let go by whoever closed the provider, or ended by itself. The loop may hear of
        # the end of what kept it a moment after the command has come back.
        if container.gone or not container.alive():
            await self._let_go(container, "it ended while a command ran")
            return (_ENDED,)
        if not (settled[0] if settled else await self._stopped(container)):
            why = "what a command left running in it could not be stopped"
            if await self._let_go(container, why):
                return (_LEFT,)
            return (_STILL.format(name=container.name),)
        if container.shape is not None:
            now = await asyncio.to_thread(_seen, container.cwd)
            if now != container.shape:
                await self._let_go(
                    container, "a command changed what is protected in its directory"
                )
                return (_RESHAPED,)
        return ()

    async def _stopped(self, container: _Kept, until: asyncio.Future[None] | None = None) -> bool:
        """Stop all that the commands of a container left running, and tell whether none is left.

        Args:
            container: The container.
            until: Done when the program that asked for a command has ended,
                if that program is still there. For as long as it is, the
                command may run, and it may be yet to start: the engine may
                have taken the command and not started it. So the stop is
                repeated until the program has ended, and done once more
                then. If the program does not end in time, nothing is known,
                and the answer is no. It is no as well as soon as a round
                does not come back: the engine may run that round later,
                and it would stop whatever runs in the container then.
        """
        loop = asyncio.get_running_loop()
        deadline = loop.time() + _STOP_SECONDS
        while until is not None and not until.done():
            if loop.time() >= deadline:
                return False
            if await self._round(container, deadline) is None:
                return False
            await asyncio.wait({until}, timeout=_ROUND_SECONDS)
        return await self._round(container, deadline) is True

    async def _round(self, container: _Kept, deadline: float) -> bool | None:
        """Stop once what runs for commands in a container, and tell whether none of it is left.

        Returns:
            Whether none is left, as the round itself said. ``None`` if the
            round gave no answer: it took too long and was given up, or it
            failed, or it could not be asked for.
        """
        seconds = max(deadline - asyncio.get_running_loop().time(), 1.0)
        asking = ended(container.program, container.name, container.user)
        try:
            answer = await self._docker(asking, seconds)
        except SandboxUnavailableError:
            return None
        said = answer.output.strip()
        # A round that was given up has no exit code either.
        if answer.exit_code != 0 or said not in ("none", "left"):
            return None
        return said == "none"

    async def _let_go(self, container: _Kept, why: str) -> bool:
        """Let a container go, and tell whether the engine was seen to have it no more.

        A container is let go once. Whoever asks for it while that is under
        way waits for the same end, and gets the same answer. And whoever
        waits may be cancelled: the container is let go all the same.
        """
        if container.leaving is None:
            container.gone = True
            if self._kept.get(container.session.id) is container:
                del self._kept[container.session.id]
            logger.debug("the container %s is let go: %s", container.name, why)
            container.leaving = asyncio.ensure_future(self._removed(container))
            self._leaving.add(container.leaving)
            container.leaving.add_done_callback(self._leaving.discard)
        return await asyncio.shield(container.leaving)

    async def _removed(self, container: _Kept) -> bool:
        """Have the engine remove a container, and tell whether it was seen to be gone.

        Closing the input of what keeps a container ends it. A command may
        have found a way to keep it from that, so the engine is told to
        remove it as well, and is asked afterwards whether it is gone. Only
        a look that worked says so. An engine that does not answer may run
        the container yet, and then the user is told how to remove it.
        """
        keeper = container.keeper
        # With no look to say otherwise, what never came to be is not there.
        gone = keeper is None or container.never
        try:
            if keeper is not None:
                assert keeper.stdin is not None
                keeper.stdin.close()
                await self._docker(removed(container.program, container.name), _REMOVE_SECONDS)
                await _ended(keeper)
                asking = there(container.program, container.name)
                answer = await self._docker(asking, _ASK_SECONDS)
                if answer.exit_code == 0:
                    gone = not answer.output.strip()
        except SandboxUnavailableError:
            pass
        finally:
            await asyncio.to_thread(shutil.rmtree, container.probe, ignore_errors=True)
        if not gone:
            logger.warning(
                "the container %s may still be there. Remove it with: docker rm -f %s",
                container.name,
                container.name,
            )
        return gone

    async def _look(self) -> None:
        """Look through the containers every few seconds, and let go those that nobody needs."""
        while self._kept:
            with contextlib.suppress(TimeoutError):
                await asyncio.wait_for(self._over.wait(), _LOOK_SECONDS)
                return
            now = time.monotonic()
            for container in list(self._kept.values()):
                turn = self._turns.get(container.session.id)
                if turn is not None and turn.locked():
                    continue
                if container.session.closed:
                    why = "its session is over"
                    self._turns.pop(container.session.id, None)
                elif not container.alive():
                    why = "it has ended"
                elif now - container.used >= self._config.idle_seconds:
                    why = "no command came for a while"
                else:
                    continue
                await self._let_go(container, why)
