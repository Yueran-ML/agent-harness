"""The shell tool: a command, run by the shell of the place where commands run.

This module is a plugin. Loading it registers one tool. Where commands run
on this machine that is ``powershell`` on Windows and ``bash`` everywhere
else. Where the sandbox runs them in another place, it is the shell of that
place, and the only one that this tool writes for is ``bash``. The name of a
tool has to stay the same while a program runs, because rules are written
with it. So the sandbox is asked once, when the tool is made, and a call
that finds commands running somewhere else than they did then is refused.

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
which runs a command so that it is held, and the permissions, which decide
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
moment. It also has the cmdlets that read files, such as ``Get-Content``,
read UTF-8, which they would otherwise not do for a file without a mark.
After the command it leaves with the exit code of the program that
ran last, if that program failed, and with 1 if a cmdlet failed last. That
has to be asked inside the text of the command, because from outside a
block always looks like a success. So one line is put after the command.

A program that writes in another encoding than that of the console comes
out garbled, and Python is one when its output is not a terminal. For
Python that is put right by a variable of the environment, unless the user
has set it, and another has it open files in UTF-8.

PowerShell is started by its full path. Windows would otherwise look for a
program of that name in the directory this program was started in, before
it looks where the system is, and that directory is often a repository.

**bash.** Everywhere else the command is handed to ``bash -c``, with no
input. A command can be as long as the system lets an argument be, which is
about 128 KiB on Linux, so longer ones are refused here with a plain word.

**bash in another place.** A command for another place is first an argument
of the program on this machine that takes it there, and on Windows all the
arguments of a program together have room for 32767 characters. So the
command goes on the input, as with PowerShell, and a fixed line reads it
there and runs it with no input of its own. In the other place it is one
argument of bash in the end, so it may be as long as an argument may be
there and no longer.

The fixed line also makes the command write its errors where it writes its
output. From another place the two come back on two ways, and nothing says
which of them arrives first: an error would show up lines after the output
that it belongs to. Written to one place, they keep their order.
"""

import logging
import sys
from collections.abc import Callable
from dataclasses import dataclass, replace

from pydantic import BaseModel, Field

from columbina import shells
from columbina.agent import RUNTIME_FACTS, Facts
from columbina.kernel import Context
from columbina.permissions import PERMISSIONS, Splitter, split_bash, split_powershell
from columbina.sandbox import SANDBOX, Sandbox, SandboxUnavailableError
from columbina.spill import SPILL, Saved, SpillFile, SpillStore
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "BASH",
    "BASH_ELSEWHERE",
    "DEFAULT_SECONDS",
    "HERE",
    "MAX_SAVED_BYTES",
    "MAX_SECONDS",
    "NAME",
    "Flavour",
    "ShellArgs",
    "ShellResult",
    "apply",
    "flavour",
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


@dataclass(frozen=True, slots=True)
class Flavour:
    """A shell, and the way a command is handed to it.

    Attributes:
        name: The name of the tool.
        splitter: Takes a command of this shell apart.
        about: What the description of the tool begins with.
        fact: What a model is told about who runs commands.
        max_command_bytes: How long a command may be.
        invocation: Returns what to run for a command and what to give it as
            input. It is given the program of the shell and the command.
    """

    name: str
    splitter: Splitter
    about: str
    fact: str
    max_command_bytes: int
    invocation: Callable[[str, str], tuple[list[str], bytes | None]]


def _as_an_argument(shell: str, command: str) -> tuple[list[str], bytes | None]:
    return [shell, "--noprofile", "--norc", "-c", command], None


_READ_AND_RUN_BASH = (
    'command=$(cat && printf x) && exec bash --noprofile --norc -c "${command%x}" </dev/null 2>&1'
)
"""Reads a command from its input and becomes a bash that runs it.

What is read ends in a mark that is taken off again. A shell drops the line ends at the
end of what it reads this way, and a command may end in one that matters: a line that
ends in a backslash goes on in the next.
"""


def _on_the_input(shell: str, command: str) -> tuple[list[str], bytes | None]:
    return [shell, "--noprofile", "--norc", "-c", _READ_AND_RUN_BASH], command.encode("utf-8")


BASH = Flavour(
    name="bash",
    splitter=split_bash,
    about="Run a command in bash.",
    fact="the tool bash",
    max_command_bytes=120_000,
    invocation=_as_an_argument,
)
"""bash on this machine: the command is an argument."""

BASH_ELSEWHERE = replace(BASH, invocation=_on_the_input)
"""bash in another place: the command comes on the input of a fixed line."""

if sys.platform == "win32":  # pragma: windows only
    _READ_AND_RUN = f"{shells.PREAMBLE}; . ([ScriptBlock]::Create([Console]::In.ReadToEnd()))"

    def _with_what_follows(shell: str, command: str) -> tuple[list[str], bytes | None]:
        argv = [shell, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", _READ_AND_RUN]
        return argv, (command + shells.AFTER).encode("utf-8")

    POWERSHELL = Flavour(
        name="powershell",
        splitter=split_powershell,
        about=(
            "Run a command in Windows PowerShell 5.1. This is not bash: join commands with ; "
            "(there is no && and no ||), write $env:NAME for a variable of the environment, "
            "and use the cmdlets or programs of Windows (Get-ChildItem, Get-Content, "
            "Select-String)."
        ),
        fact="the tool powershell, which is Windows PowerShell 5.1 and not bash",
        max_command_bytes=1_000_000,
        invocation=_with_what_follows,
    )
    """Windows PowerShell on this machine: the command comes on the input of a fixed line."""

    HERE = POWERSHELL

else:  # pragma: posix only
    HERE = BASH

NAME = HERE.name
"""The name of the tool where commands run on this machine."""


def flavour(elsewhere: str | None) -> Flavour:
    """Return the shell that commands are written for, and the way they are handed to it.

    Args:
        elsewhere: The shell of the place where the sandbox runs commands, or
            ``None`` if they run on this machine.

    Raises:
        RuntimeError: The place has a shell that this tool cannot write for.
    """
    if elsewhere is None:
        return HERE
    if elsewhere == "bash":
        return BASH_ELSEWHERE
    raise RuntimeError(
        f"the sandbox runs commands with {elsewhere!r}, and the tool that runs commands "
        "writes for bash only where they run in another place"
    )


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

    __slots__ = ("_file", "_name", "_run", "_store")

    def __init__(self, store: SpillStore, run: ToolRun, tool: str) -> None:
        self._store = store
        self._run = run
        self._name = f"{tool}-output.txt"
        self._file: SpillFile | None = None

    def open(self) -> SpillFile:
        """Make the file that takes the output."""
        self._file = self._store.open(self._run.session, self._run.cwd, self._name)
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
    sandbox: Sandbox,
    shell: str,
    spill: Callable[[], SpillStore | None],
    elsewhere: str | None = None,
) -> Tool[ShellArgs, ShellResult]:
    """Make the shell tool, which runs its commands with the program ``shell``.

    Args:
        sandbox: Runs a command so that it is held.
        shell: The program that runs the commands: its path on this machine,
            or its name in the place where the sandbox runs commands.
        spill: Returns the place for what is too large for a conversation, or
            ``None`` if the program has none. It is asked at every call.
        elsewhere: The shell of the place where the sandbox runs commands, as
            the sandbox names it, or ``None`` if they run on this machine.

    Raises:
        RuntimeError: The place has a shell that this tool cannot write for.
    """
    way = flavour(elsewhere)

    async def run_command(args: ShellArgs, run: ToolRun) -> ShellResult:
        if "\0" in args.command:
            # Each shell would fail in a way of its own: one program says that an argument
            # is no text, another leaves the character out without a word.
            raise ToolFailure(
                "The command has a NUL character in it, and no shell takes a command with "
                "one. Write that byte another way, with printf for one.",
                code="NUL_IN_COMMAND",
            )
        if len(args.command.encode("utf-8")) > way.max_command_bytes:
            raise ToolFailure(
                f"The command is longer than {way.max_command_bytes} bytes, which is more "
                "than can be handed to the shell. Put what it does into a file and run that.",
                code="COMMAND_TOO_LONG",
            )
        if sandbox.shell() != elsewhere:
            # A command written for one shell is not handed to another one.
            raise ToolFailure(
                "The command was not run: the place where the sandbox runs commands is "
                "another one than when this tool was made. Start the program again.",
                code="SANDBOX_UNAVAILABLE",
            )
        seconds = min(args.timeout_seconds or DEFAULT_SECONDS, MAX_SECONDS)
        argv, given = way.invocation(shell, args.command)
        store = spill()
        place = None if store is None else _Place(store, run, way.name)
        try:
            done = await sandbox.run(
                argv,
                session=run.session,
                cwd=run.cwd,
                stdin=given,
                timeout_seconds=seconds,
                overflow=None if place is None else place.open,
                max_overflow_bytes=MAX_SAVED_BYTES,
                call_id=run.call_id,
            )
        except SandboxUnavailableError as error:
            raise ToolFailure(
                f"The command was not run: {error}.", code="SANDBOX_UNAVAILABLE"
            ) from None
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
        # What ran the command may have something to say of the run.
        lines.extend(f"({note})" for note in done.notes)
        return ShellResult(output + "\n".join(lines), complete)

    return Tool(
        name=way.name,
        description=(
            f"{way.about} The command runs in the working directory, in a process of its own: "
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
        RuntimeError: Commands run on this machine and its shell is not
            installed, or they run in a place with a shell that this tool
            cannot write for.
    """
    sandbox = ctx.use(SANDBOX)
    elsewhere = sandbox.shell()
    way = flavour(elsewhere)
    # In another place the shell is found by its name, by whatever runs commands there.
    shell = shells.find() if elsewhere is None else elsewhere
    if shell is None:
        raise RuntimeError(f"{NAME} is not installed, so there is no tool that runs commands")
    ctx.use(TOOLS).add(ctx, shell_tool(sandbox, shell, lambda: ctx.get(SPILL), elsewhere))
    permissions = ctx.use(PERMISSIONS)
    permissions.splitters.add(ctx, way.name, way.splitter)
    # What this tool runs is held as well as the sandbox holds anything.
    permissions.sandboxed.add(ctx, way.name, True)

    def facts(gathered: Facts) -> None:
        gathered.lines["commands are run by"] = way.fact

    ctx.on(RUNTIME_FACTS, facts)
