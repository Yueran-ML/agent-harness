"""The shell that commands are run with on this system, and the environment that they get.

Two things run commands for a session: the tool that a model calls, and the
hooks of the user. Both use the same shell and hand it the same environment,
and this is where they find the two. Each of them starts the shell in its
own way, because one of them needs the input of the shell for itself.

On Windows the shell is Windows PowerShell 5.1, which every installation has.
Elsewhere it is bash.
"""

import os
import shutil
import sys
from pathlib import Path

from columbina import process

if sys.platform == "win32":  # pragma: windows only
    __all__ = ["AFTER", "PREAMBLE", "environment", "find"]

    PREAMBLE = (
        "$ProgressPreference = 'SilentlyContinue'; "
        "[Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false; "
        "[Console]::OutputEncoding = [Console]::InputEncoding; "
        "$OutputEncoding = [Console]::InputEncoding; "
        "$PSDefaultParameterValues['Get-Content:Encoding'] = 'UTF8'; "
        "$PSDefaultParameterValues['Import-Csv:Encoding'] = 'UTF8'"
    )
    """What a script begins with, so that text goes in and comes out as UTF-8.

    Windows PowerShell reads a file without a mark of its encoding in the code
    page of the system, and a file of UTF-8 in Chinese comes out garbled. So
    the cmdlets that read text read UTF-8 unless they are told otherwise
    (Select-String does so already). What
    writes is left as it is: in this PowerShell UTF-8 is written with a mark.
    """

    AFTER = "\nif (-not $?) { if ($LASTEXITCODE) { exit $LASTEXITCODE } else { exit 1 } }\n"
    """What a script ends with, so that the shell ends with the exit code of its last command."""

    def find() -> str | None:
        """Return the path of Windows PowerShell, or ``None`` if it is not where it belongs."""
        root = os.environ.get("SYSTEMROOT", "C:\\Windows")
        found = Path(root, "System32", "WindowsPowerShell", "v1.0", "powershell.exe")
        return str(found) if found.is_file() else None

    def environment() -> dict[str, str]:
        """Return the environment of a command: this one, without what sounds like a secret."""
        cleaned = process.cleaned(os.environ)
        # Python writes to a pipe in the code page of the system unless it is told otherwise,
        # and opens a file in it too.
        cleaned.setdefault("PYTHONIOENCODING", "utf-8")
        cleaned.setdefault("PYTHONUTF8", "1")
        return cleaned

else:  # pragma: posix only
    __all__ = ["environment", "find"]

    def find() -> str | None:
        """Return the path of bash, or ``None`` if there is none."""
        return shutil.which("bash")

    def environment() -> dict[str, str]:
        """Return the environment of a command: this one, without what sounds like a secret."""
        return process.cleaned(os.environ)
