"""The provider that runs commands in a container of Docker.

Only the ``docker`` program is used, the way a person would use it. There is
nothing here that the engine has to be told about beforehand, and nothing is
left with it afterwards.

**The program.** The system would find ``docker`` by its name, and on
Windows it looks in the directory that this program was started in before it
looks anywhere else. That directory is often a repository. So the looking is
done here, :func:`found`: only the entries of the search path that are whole
paths count, and none that lies in a working directory.

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

**Who a command is.** What keeps a container alive runs in it as root, and
commands run as another user, :func:`user_of_commands`. With every
capability dropped, that user cannot send the keeper a signal and cannot
change the programs of the image. What a command left running is stopped by
killing every process of that user, which the kernel does in one go and
which needs no record of what was started.
"""

import os
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError, field_validator

from columbina.sandbox.base import PROTECTED, Mode, SandboxUnavailableError

__all__ = [
    "DEFAULT_IMAGE",
    "HOME",
    "PROBE",
    "DockerConfig",
    "Engine",
    "asked",
    "called",
    "ended",
    "found",
    "kept",
    "mounts",
    "quoted",
    "removed",
    "shape",
    "supported",
    "there",
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

_PROGRAM = "docker.exe" if sys.platform == "win32" else "docker"

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

_USABLE = "test -r . && test -x . && if test -w .; then echo writable; else echo readable; fi"
"""Says what the user that runs it can do with the directory it is run in."""

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


def found(search: str, outside: Sequence[Path] = ()) -> str | None:
    """Return the path of the ``docker`` program that a search path leads to, or ``None``.

    Args:
        search: The search path, as the environment has it.
        outside: Directories that the program must not come from: the one
            that this program was started in, and the working directory of
            a session. What a link there leads to counts as being there.
    """
    apart = [Path(os.path.realpath(directory)) for directory in outside]
    for written in search.split(os.pathsep):
        entry = Path(written.strip('"'))
        # An entry that is no whole path means a place below the current directory.
        if not entry.is_absolute():
            continue
        real = Path(os.path.realpath(entry / _PROGRAM))
        if any(real == directory or directory in real.parents for directory in apart):
            continue
        if real.is_file() and os.access(real, os.X_OK):
            return str(entry / _PROGRAM)
    return None


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


def _led(cwd: Path) -> list[tuple[str, str, Path]]:
    """Return each protected name at the top of a directory, what it is, and where it leads."""
    names: list[tuple[str, str, Path]] = []
    with os.scandir(cwd) as entries:
        for entry in entries:
            # The disk may not tell upper case from lower, so neither does this.
            if entry.name.casefold() not in PROTECTED:
                continue
            real = Path(os.path.realpath(entry.path))
            if real != cwd / entry.name:
                kind = f"link to {real}"
            else:
                kind = "directory" if real.is_dir() else "file" if real.is_file() else "other"
            names.append((entry.name, kind, real))
    return sorted(names)


def shape(cwd: Path) -> tuple[tuple[str, str], ...]:
    """Return what the protected names at the top of a directory are.

    A container is made for the shape that it finds. A command may give the
    directory another one: it can make a ``.git`` where there was none, and
    it can put a directory in the place of a link. Whoever keeps a container
    compares the shape after a command with the one before it.

    Args:
        cwd: The directory, with every link on its path resolved.

    Returns:
        Each such name as it is written there, and what it is: a directory,
        a file, a link and where it leads, or none of these.
    """
    return tuple((name, kind) for name, kind, _ in _led(cwd))


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
        protected names at its top are or lead to, read-only, as far as it
        lies in the working directory.
    """
    if mode == "danger-full-access":
        return [_mount(cwd, target, readonly=False)]
    # A session that was started inside a protected directory writes nothing there either.
    whole = mode == "read-only" or any(part.casefold() in PROTECTED for part in cwd.parts)
    over: dict[str, Path] = {}
    if not whole:
        for _, _, real in _led(cwd):
            if real == cwd:
                # A protected name that leads to the directory itself protects all of it.
                whole = True
            elif cwd in real.parents and (real.is_dir() or real.is_file()):
                over[f"{target}/{real.relative_to(cwd).as_posix()}"] = real
    if whole:
        return [_mount(cwd, target, readonly=True)]
    return [
        _mount(cwd, target, readonly=False),
        *(_mount(real, inside, readonly=True) for inside, real in sorted(over.items())),
    ]


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


def unavailable(image: str, said: str) -> str:
    """Say why no container could be started, in words for whoever can put it right.

    Args:
        image: The image that the container was to have.
        said: What the ``docker`` program wrote.
    """
    low = said.casefold()
    if "no such image" in low or "unable to find image" in low:
        return (
            f"the image {image} is not on this machine, and no image is fetched for a command. "
            f"Fetch it with: docker pull {image}"
        )
    if any(words in low for words in _UNREACHABLE):
        return (
            "the Docker engine does not answer. Start Docker Desktop, or the service of Docker, "
            "and run the command again"
        )
    lines = [line.strip() for line in said.splitlines() if line.strip()]
    told = lines[0][:300] if lines else "it did not say why"
    return f"Docker could not start a container: {told}"
