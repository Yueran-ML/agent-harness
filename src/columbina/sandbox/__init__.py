"""The sandbox: how much a session may change, and who runs its commands so that they are held.

This module is a plugin. Loading it provides :data:`SANDBOX`.

A session has a mode, which is written into its log and so comes back when
the session is taken up again.

``read-only``
    Nothing is written.
``workspace-write``
    Files in the working directory are written, except what belongs to the
    repository and to this program: ``.git`` and ``.columbina``. That is
    what lies in a directory of such a name, wherever on the path the name
    comes, and what such a name at the top of the working directory leads
    to if it is a link. A repository that keeps its history in a directory
    of another name, and points to it from a file, is not found this way.
``danger-full-access``
    Everything is read and written, wherever it is.

For the tools that work on files, the mode is enforced here, through the seam
of the file system: the plugin registers one grant and one restriction, which
come from the same table. Without this plugin nothing registers a grant, and
then no tool writes a file.

A command is another matter. What a program does once it runs cannot be
checked from in here, so it is run inside something that holds it. That is
the job of a provider, and the provider runs the command itself: what holds
a command may have to be made before the command runs, and what the command
left has to be stopped after it. The provider ``none`` runs a command on
this machine as it is. It holds nothing and says so by its strength, and
whoever decides about commands goes by that.

A provider may run commands in another place than this machine, and that
place has a shell of its own. The provider names it, and the tool that takes
commands from a model writes for that shell.
"""

import os
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import ClassVar, Literal, Protocol

from pydantic import BaseModel

from columbina import process, shells
from columbina.agent import RUNTIME_FACTS, Facts
from columbina.fs import FS, Access, Refuse
from columbina.kernel import Context, Registry, ServiceKey
from columbina.process import Completed, Writable
from columbina.session import SESSIONS, EventData, Projection, Session, SessionEvent

__all__ = [
    "PROTECTED",
    "SANDBOX",
    "Config",
    "Mode",
    "Sandbox",
    "SandboxModeEvent",
    "SandboxProvider",
    "SandboxUnavailableError",
    "Strength",
    "apply",
    "inject",
    "name",
]

name = "sandbox"
inject = (FS, SESSIONS)

Mode = Literal["read-only", "workspace-write", "danger-full-access"]
"""How much a session may change."""

Strength = Literal["none", "partial", "full"]
"""How well a provider holds a command: not at all, in part, or fully."""

PROTECTED = (".git", ".columbina")
"""The names of the directories that are not written in ``workspace-write`` mode."""


class SandboxModeEvent(EventData):
    """The mode of a session was set."""

    event_type: ClassVar[str] = "sandbox/mode"
    ignorable: ClassVar[bool] = True

    mode: Mode


class SandboxUnavailableError(Exception):
    """What is to hold commands is not there, or could not hold this one.

    The text says what is missing, in words for whoever can put it right.
    """


class SandboxProvider(Protocol):
    """Something that runs a command so that it is held.

    Attributes:
        strength: How well it holds a command. A provider that cannot do what
            it says here raises from :meth:`run`. It does not run a command
            that is held less well.
        shell: The shell of the place where it runs commands, such as
            ``bash``, if that place is not this machine. ``None`` if commands
            run on this machine, with the shell that it has. A provider says
            the same for as long as it is there.
    """

    @property
    def strength(self) -> Strength:
        """How well it holds a command."""
        ...

    @property
    def shell(self) -> str | None:
        """The shell of the place where it runs commands, if that is not this machine."""
        ...

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
    ) -> Completed:
        """Run a command to its end, held in the given mode, and say how it ended.

        When this returns or raises, everything that the command started has
        been stopped, as with :func:`columbina.process.run`, whose arguments
        the last four are.

        Raises:
            SandboxUnavailableError: The command could not be held, so it was
                not run.
            OSError: The command could not be started.
        """
        ...


class _Unconfined:
    """The provider that holds nothing. A command runs on this machine, as it is."""

    strength: Strength = "none"
    shell: str | None = None

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
    ) -> Completed:
        return await process.run(
            argv,
            cwd=cwd,
            env=shells.environment(),
            stdin=stdin,
            timeout_seconds=timeout_seconds,
            overflow=overflow,
            max_overflow_bytes=max_overflow_bytes,
        )


_MODES: dict[Mode, str] = {
    "read-only": "read-only. No file can be written.",
    "workspace-write": (
        "workspace-write. Files in the working directory can be written, except in .git and "
        ".columbina. Outside it no file can be read or written."
    ),
    "danger-full-access": "danger-full-access. Files anywhere can be read and written.",
}
"""What a model is told about each mode."""

_STRENGTHS: dict[Strength, str] = {
    "none": "not at all. A command can do whatever the user of this machine can do.",
    "partial": "in part. A command may still reach more than the mode of the sandbox allows.",
    "full": "fully. A command can do no more than the mode of the sandbox allows.",
}
"""What a model is told about how well commands are held."""


def _last_mode(mode: Mode | None, event: SessionEvent[EventData]) -> Mode | None:
    data = event.data
    return data.mode if isinstance(data, SandboxModeEvent) else mode


_MODE: Projection[Mode | None] = Projection("sandbox-mode", None, _last_mode)


def _protected(access: Access) -> str | None:
    """Return the protected directory that a file lies in or is, if there is one.

    The whole path counts, and not only the part below the working
    directory: a session that was started inside ``.git`` writes nothing
    there either. A protected name at the top of the working directory may
    be a link, and then the directory that it leads to is protected under
    its own name too.
    """
    where = access.where
    for part in where.path.parts:
        # The disk may not tell upper case from lower, so neither does this.
        if part.casefold() in PROTECTED:
            return part
    for name in PROTECTED:
        led = Path(os.path.realpath(where.root / name))
        if led == where.path or led in where.path.parents:
            return name
    return None


class Config(BaseModel):
    """The config of the sandbox.

    Attributes:
        mode: The mode of a session whose log names none.
        provider: The name of the provider that runs commands.
    """

    mode: Mode = "workspace-write"
    provider: str = "none"


class Sandbox:
    """The mode of each session, and the provider that runs commands.

    Attributes:
        providers: The providers, by name.
    """

    __slots__ = ("_default", "_provider", "providers")

    def __init__(self, default: Mode, provider: str) -> None:
        self.providers: Registry[SandboxProvider] = Registry("sandbox provider")
        self._default: Mode = default
        self._provider = provider

    def recorded(self, session: Session) -> Mode | None:
        """Return the mode that the log of a session names, or ``None`` if it names none."""
        return _MODE.of(session)

    def mode(self, session: Session) -> Mode:
        """Return the mode of a session: the one in its log, or else the one of the config."""
        return _MODE.of(session) or self._default

    def set_mode(self, session: Session, mode: Mode) -> None:
        """Write the mode of a session into its log, unless the log names that mode already."""
        if _MODE.of(session) != mode:
            session.append(SandboxModeEvent(mode=mode))

    def strength(self) -> Strength:
        """Return how well commands are held: the strength of the provider, if it is there."""
        provider = self.providers.get(self._provider)
        return "none" if provider is None else provider.strength

    def shell(self) -> str | None:
        """Return the shell of the place where commands run, if that is not this machine.

        ``None`` means the shell of this machine. That is also the answer
        while the provider is not there: nothing runs then.
        """
        provider = self.providers.get(self._provider)
        return None if provider is None else provider.shell

    async def run(
        self,
        argv: Sequence[str],
        *,
        session: Session,
        cwd: Path,
        stdin: bytes | None = None,
        timeout_seconds: float,
        overflow: Callable[[], Writable] | None = None,
        max_overflow_bytes: int = 64 * 1024 * 1024,
    ) -> Completed:
        """Run a command to its end, held in the mode of the session.

        The arguments after the directory are those of
        :func:`columbina.process.run`.

        Raises:
            SandboxUnavailableError: The provider that the config names is not
                there, or it could not hold the command. A command is not run
                unconfined in its place.
            OSError: The command could not be started.
        """
        provider = self.providers.get(self._provider)
        if provider is None:
            raise SandboxUnavailableError(
                f"the sandbox provider {self._provider!r} is not there, so no command can be run"
            )
        return await provider.run(
            argv,
            session=session,
            mode=self.mode(session),
            cwd=cwd,
            stdin=stdin,
            timeout_seconds=timeout_seconds,
            overflow=overflow,
            max_overflow_bytes=max_overflow_bytes,
        )

    def grants(self, access: Access) -> bool:
        """Tell whether the mode of the session allows what the seam does not allow by itself."""
        mode = self.mode(access.session)
        if mode == "danger-full-access":
            return True
        return mode == "workspace-write" and access.kind == "write" and access.where.inside

    def restricts(self, access: Access) -> Refuse | None:
        """Refuse a write that the mode of the session does not allow, and say why."""
        if access.kind != "write":
            return None
        mode = self.mode(access.session)
        written = access.where.written
        if mode == "read-only":
            return Refuse(f"The session is read-only, so {written} cannot be written.", "READ_ONLY")
        protected = _protected(access) if mode == "workspace-write" else None
        if protected is not None:
            return Refuse(
                f"{written} is in {protected}, which is not written in this mode.", "PROTECTED"
            )
        return None


SANDBOX = ServiceKey[Sandbox]("sandbox")
"""The sandbox."""


def apply(ctx: Context, config: Config) -> None:
    """Provide the sandbox, and let the mode of a session decide about its files."""
    sandbox = Sandbox(config.mode, config.provider)
    ctx.use(SESSIONS).declare(ctx, SandboxModeEvent)
    sandbox.providers.add(ctx, "none", _Unconfined())
    seam = ctx.use(FS)
    seam.grants.add(ctx, "sandbox", sandbox.grants)
    seam.restrictions.add(ctx, "sandbox", sandbox.restricts)

    def facts(gathered: Facts) -> None:
        gathered.lines["sandbox mode"] = _MODES[sandbox.mode(gathered.agent.session)]
        gathered.lines["commands are held"] = _STRENGTHS[sandbox.strength()]

    ctx.on(RUNTIME_FACTS, facts)
    ctx.provide(SANDBOX, sandbox)
