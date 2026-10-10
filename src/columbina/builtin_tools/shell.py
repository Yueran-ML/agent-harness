"""The shell tool: a command, run by the shell of this platform.

This module is a plugin. Loading it registers one tool: ``powershell`` on
Windows and ``bash`` everywhere else. The name of a tool has to stay the same
on one platform, because rules are written with it.

Every call is a process of its own. Nothing is left of it when it ends: the
processes that the command started are stopped with it, whether it came to
its end, ran out of time or was cancelled. A directory that a command
changed to, and a variable that it set, are gone with the call.

A command that ends with another exit code than 0 has not failed as a call:
the model reads the output and the exit code and decides what that means.
The call fails only if the command could not be started at all.

Of a long output the model gets the beginning and the end. Where the program
has a place for what is too large for a conversation, the whole output is
written there as it comes, and the result says where it is. A command may
write more than memory should hold, so this cannot wait until it has ended.
Not all of it may get there: the place takes so much and no more, and a disk
can fill up. The result then says how much of it did.

What a command does once it runs cannot be checked from here. That is why
the tool needs two services and is not loaded without them: the sandbox,
which says how well a command is held, and the permissions, which decide
whether a command runs and whom to ask. The tool tells the permissions how
its commands are taken apart.

**PowerShell.** On Windows the shell is Windows PowerShell 5.1, which every
Windows has. Four ways of handing it a command were tried on this machine,
against what a tool needs: text of any script comes out right, the exit code
of a program is passed on, a cmdlet that fails gives another code than 0,
errors are plain text, a command may be long, a working directory may have
spaces in its path, and nothing is left behind.

- ``-EncodedCommand`` writes errors as XML, whatever it is told, and the
  command line of Windows has no room for a command of 12000 characters.
- ``-Command -`` reads the command from the input the way a person types
  it: a block of several lines does not run.
- A script in a file works, but there has to be a file, the policy of the
  machine has to let scripts run, and the path of the file shows up in every
  error.
- A fixed line on the command line that reads the command from the input
  and runs it meets all seven, with a command of a million characters as
  well as with one word. This is what the tool does.

The fixed line first sets the encoding of the console to UTF-8, in both
directions. PowerShell writes in whatever encoding the console has at that
moment. After the command it leaves with the exit code of the program that
ran last, if that program failed, and with 1 if a cmdlet failed last. That
has to be asked inside the text of the command, because from outside a
block always looks like a success. So one line is put after the command.

A program that writes in another encoding than that of the console comes
out garbled, and Python is one when its output is not a terminal. For
Python that is put right by a variable of the environment, unless the user
has set it.

PowerShell is started by its full path. Windows would otherwise look for a
program of that name in the directory this program was started in, before
it looks where the system is, and that directory is often a repository.

**bash.** Everywhere else the command is handed to ``bash -c``, with no
input. A command can be as long as the system lets an argument be, which is
about 128 KiB on Linux, so longer ones are refused here with a plain word.
"""

import logging
import os
import shutil
import sys
from collections.abc import Callable
from dataclasses import dataclass, replace
from pathlib import Path

from pydantic import BaseModel, Field

from columbina import process
from columbina.agent import RUNTIME_FACTS, Facts
from columbina.kernel import Context
from columbina.permissions import PERMISSIONS, Splitter, split_bash, split_powershell
from columbina.sandbox import SANDBOX, Sandbox, SandboxUnavailableError
from columbina.spill import SPILL, Saved, SpillFile, SpillStore
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "DEFAULT_SECONDS",
    "MAX_COMMAND_BYTES",
    "MAX_SAVED_BYTES",
    "MAX_SECONDS",
    "NAME",
    "ShellArgs",
    "ShellResult",
    "apply",
    "inject",
    "name",
    "shell_tool",
]

name = "tool-shell"
inject = (TOOLS, SANDBOX, PERMISSIONS)

logger = logging.getLogger(__name__)

DEFAULT_SECONDS = 120.0
"""How long a command may run if the call does not say."""

MAX_SECONDS = 600.0
"""How long a command may run at most."""

MAX_SAVED_BYTES = 64 * 1024 * 1024 - 64 * 1024
"""How much of the output of one command is saved at most.

The read tool looks no further into a file than 64 MiB. This is a little
less: the place may break long lines as it saves, and every break is a byte
more.
"""

_GRACE_SECONDS = 30.0
"""How much longer than its command a call may take, to stop what the command started."""

if sys.platform == "win32":  # pragma: windows only
    NAME = "powershell"
    MAX_COMMAND_BYTES = 1_000_000

    _SPLITTER: Splitter = split_powershell
    _AFTER = "\nif (-not $?) { if ($LASTEXITCODE) { exit $LASTEXITCODE } else { exit 1 } }\n"
    _READ_AND_RUN = (
        "$ProgressPreference = 'SilentlyContinue'; "
        "[Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false; "
        "[Console]::OutputEncoding = [Console]::InputEncoding; "
        "$OutputEncoding = [Console]::InputEncoding; "
        ". ([ScriptBlock]::Create([Console]::In.ReadToEnd()))"
    )
    _FACT = "the tool powershell, which is Windows PowerShell 5.1 and not bash"
    _ABOUT = (
        "Run a command in Windows PowerShell 5.1. This is not bash: join commands with ; "
        "(there is no && and no ||), write $env:NAME for a variable of the environment, and "
        "use the cmdlets or programs of Windows (Get-ChildItem, Get-Content, Select-String)."
    )

    def _shell() -> str | None:
        """Return the path of Windows PowerShell, or ``None`` if it is not where it belongs."""
        root = os.environ.get("SYSTEMROOT", "C:\\Windows")
        found = Path(root, "System32", "WindowsPowerShell", "v1.0", "powershell.exe")
        return str(found) if found.is_file() else None

    def _invocation(shell: str, command: str) -> tuple[list[str], bytes | None]:
        """Return what to run for a command, and what to give it as input."""
        argv = [shell, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", _READ_AND_RUN]
        return argv, (command + _AFTER).encode("utf-8")

    def _environment() -> dict[str, str]:
        """Return the environment of a command: this one, without what sounds like a secret."""
        cleaned = process.cleaned(os.environ)
        # Python writes to a pipe in the code page of the system unless it is told otherwise.
        cleaned.setdefault("PYTHONIOENCODING", "utf-8")
        return cleaned

else:  # pragma: posix only
    NAME = "bash"
    MAX_COMMAND_BYTES = 120_000

    _SPLITTER: Splitter = split_bash
    _FACT = "the tool bash"
    _ABOUT = "Run a command in bash."

    def _shell() -> str | None:
        """Return the path of bash, or ``None`` if there is none."""
        return shutil.which("bash")

    def _invocation(shell: str, command: str) -> tuple[list[str], bytes | None]:
        """Return what to run for a command, and what to give it as input."""
        return [shell, "--noprofile", "--norc", "-c", command], None

    def _environment() -> dict[str, str]:
        """Return the environment of a command: this one, without what sounds like a secret."""
        return process.cleaned(os.environ)


class ShellArgs(BaseModel):
    """The arguments of the shell tool."""

    command: str = Field(min_length=1, description="The command to run.")
    timeout_seconds: float | None = Field(
        default=None,
        gt=0,
        description=(
            f"How many seconds the command may run. Without it, {DEFAULT_SECONDS:g}. "
            f"More than {MAX_SECONDS:g} counts as {MAX_SECONDS:g}."
        ),
    )


@dataclass(frozen=True, slots=True)
class ShellResult:
    """What a command gave.

    Attributes:
        text: What the model reads.
        complete: Where the whole output is, if the text does not hold all
            of it and all of it was saved. Whatever keeps a result within
            bounds goes by this, and does not save it a second time.
    """

    text: str
    complete: Saved | None = None


class _Place:
    """Where the whole output of one command goes, once it is more than memory keeps.

    A command that writes little has nothing made for it. The file is made
    when the run asks for it.
    """

    __slots__ = ("_file", "_run", "_store")

    def __init__(self, store: SpillStore, run: ToolRun) -> None:
        self._store = store
        self._run = run
        self._file: SpillFile | None = None

    def open(self) -> SpillFile:
        """Make the file that takes the output."""
        self._file = self._store.open(self._run.session, self._run.cwd, f"{NAME}-output.txt")
        return self._file

    def close(self) -> Saved | None:
        """End the saving. Return what the place says of it, or ``None`` if there is nothing."""
        if self._file is None:
            return None
        try:
            return self._file.close()
        except Exception as error:
            # The call may be on its way out, cancelled. No fault of a place keeps it in.
            fault = None if isinstance(error, OSError) else error
            logger.error(
                "the whole output of a command could not be saved: %s", error, exc_info=fault
            )
            return None


def _target(args: ShellArgs, run: ToolRun) -> str:
    return args.command


def _text(args: ShellArgs, result: ShellResult) -> str:
    return result.text


def shell_tool(
    sandbox: Sandbox, shell: str, spill: Callable[[], SpillStore | None]
) -> Tool[ShellArgs, ShellResult]:
    """Make the shell tool, which runs its commands with the program at ``shell``.

    Args:
        sandbox: Says how a command is held.
        shell: The path of the program that runs the commands.
        spill: Returns the place for what is too large for a conversation, or
            ``None`` if the program has none. It is asked at every call.
    """

    async def run_command(args: ShellArgs, run: ToolRun) -> ShellResult:
        if len(args.command.encode("utf-8")) > MAX_COMMAND_BYTES:
            raise ToolFailure(
                f"The command is longer than {MAX_COMMAND_BYTES} bytes, which is more than "
                "can be handed to the shell. Put what it does into a file and run that.",
                code="COMMAND_TOO_LONG",
            )
        seconds = min(args.timeout_seconds or DEFAULT_SECONDS, MAX_SECONDS)
        argv, given = _invocation(shell, args.command)
        try:
            confined = sandbox.confine(argv, session=run.session, cwd=run.cwd)
        except SandboxUnavailableError as error:
            raise ToolFailure(
                f"The command was not run: {error}.", code="SANDBOX_UNAVAILABLE"
            ) from None
        store = spill()
        place = None if store is None else _Place(store, run)
        try:
            done = await process.run(
                confined.argv,
                cwd=run.cwd,
                env=_environment(),
                stdin=given,
                timeout_seconds=seconds,
                overflow=None if place is None else place.open,
                max_overflow_bytes=MAX_SAVED_BYTES,
            )
        except OSError as error:
            raise ToolFailure(f"The command could not be run: {error}") from error
        finally:
            # However the run ended, also by the call being cancelled: it writes no more.
            saved = None if place is None else place.close()
        output = done.output.replace("\r\n", "\n")
        if output and not output.endswith("\n"):
            output += "\n"
        lines = [f"[exit code {done.exit_code}]"]
        if done.timed_out:
            lines = [f"[the command was stopped after {seconds:g} seconds]"]
        complete = None
        if place is not None and done.omitted_bytes:
            if saved is None or not done.saved_bytes:
                lines.append("(The whole output could not be saved.)")
            elif done.saved_bytes < done.output_bytes:
                lines.append(
                    f"(Only the first {done.saved_bytes} bytes of the output are saved, in "
                    f"{saved.locator}. {saved.hint})"
                )
            else:
                complete = replace(saved, what="output")
                lines.append(complete.told())
        return ShellResult(output + "\n".join(lines), complete)

    return Tool(
        name=NAME,
        description=(
            f"{_ABOUT} The command runs in the working directory, in a process of its own: "
            "what it changes in the shell, such as the directory or a variable, is gone "
            "with the call, and so is every process that it started. Its output comes "
            "back with its exit code. It gets no input, so it must not wait for any."
        ),
        parameters=ShellArgs,
        execute=run_command,
        render=_text,
        effect="run",
        target=_target,
        timeout_seconds=MAX_SECONDS + _GRACE_SECONDS,
    )


def apply(ctx: Context, config: object) -> None:
    """Register the shell tool, and tell the permissions how its commands are taken apart.

    Raises:
        RuntimeError: The shell of this platform is not installed.
    """
    shell = _shell()
    if shell is None:
        raise RuntimeError(f"{NAME} is not installed, so there is no tool that runs commands")
    ctx.use(TOOLS).add(ctx, shell_tool(ctx.use(SANDBOX), shell, lambda: ctx.get(SPILL)))
    ctx.use(PERMISSIONS).splitters.add(ctx, NAME, _SPLITTER)

    def facts(gathered: Facts) -> None:
        gathered.lines["commands are run by"] = _FACT

    ctx.on(RUNTIME_FACTS, facts)
