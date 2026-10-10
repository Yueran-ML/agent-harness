"""Running a hook: what it is told, how its command is started, and how its answer is read.

A hook is told what happens as one line of JSON on its standard input. It
answers in one of two ways. The plain one is its exit code: 0 says go on, 2
says stop this, and what it wrote to its error stream is the reason. The
finer one is JSON on its standard output with exit code 0, in which it can
ask for somebody to be asked, give a reason, or add something for the model
to read. Both are what other tools do with their hooks, so that a hook
written for one of them works here.

What a hook answers is read with some care for mistakes on either side.

- A hook that says stop and gives no reason stops all the same: it said so.
- A field that is not known is passed over. One field too many does not turn
  a hook that wanted to stop something into one that lets it through.
- A field that is known and cannot be honoured here is told in a warning.
- A hook that fails, takes too long or writes more than can be read stops
  nothing. It is told as a failure, and whatever it was about goes on.

The command runs in the shell that the tool for commands uses. On Windows
that shell takes its script from its input, which a hook needs for what it
is told, so the command goes in as an argument instead. What the shell
itself has to say about an error comes as XML there, and is turned back
into text.

Nothing here knows of agents. :mod:`columbina.hooks` decides when a hook
runs and what is done with its answer.
"""

import base64
import html
import json
import re
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, cast

from columbina import process, shells
from columbina.hooks.config import Hook

__all__ = [
    "CONTEXT_EVENTS",
    "MAX_COMMAND",
    "MAX_OUTPUT_BYTES",
    "Decision",
    "Result",
    "Status",
    "cut",
    "from_clixml",
    "invocation",
    "merged",
    "payload",
    "read",
    "run",
]

MAX_COMMAND = 8000
"""How many characters the command of a hook may have."""

MAX_OUTPUT_BYTES = 64_000
"""How much of what a hook writes to either of its streams is read."""

CONTEXT_EVENTS = ("SessionStart", "UserPromptSubmit", "PostToolUse")
"""The moments at which a hook can add something for the model to read."""

_PLAIN_CONTEXT = ("SessionStart", "UserPromptSubmit")
_NO_REASON = "A hook stopped this and gave no reason."
_UNSUPPORTED = ("continue", "stopReason", "updatedInput", "suppressOutput")
_PERMISSION = {"allow": "allow", "deny": "block", "ask": "ask"}

Status = Literal["completed", "blocked", "failed", "timed-out"]
"""How the run of a hook ended."""

Decision = Literal["allow", "ask", "block"]
"""What a hook wants done with what it was asked about."""


@dataclass(frozen=True, slots=True)
class Result:
    """What came of a hook, or of all hooks of one moment.

    Attributes:
        status: How the run ended. ``blocked`` if the hook said stop.
        decision: What the hook wants: nothing in particular, the thing let
            through, somebody asked, or the thing stopped.
        reason: Why, for the model to read. Empty unless the hook asks for
            somebody to be asked or says stop.
        context: What the hook adds for the model to read.
        message: What the hook has to say to the person.
        warnings: What was wrong with the answer of the hook, for the log.
        exit_code: The exit code of the command, if it came to its end.
    """

    status: Status = "completed"
    decision: Decision | None = None
    reason: str = ""
    context: str = ""
    message: str = ""
    warnings: tuple[str, ...] = ()
    exit_code: int | None = None


def cut(text: str, limit: int) -> str:
    """Return a text in no more characters than ``limit``, with its middle left out if need be."""
    if len(text) <= limit:
        return text
    note = f"\n[{len(text) - limit} characters are left out here]\n"
    head = limit // 2
    return text[:head] + note + text[len(text) - (limit - head) :]


def payload(
    event: str, session_id: str, transcript: Path, cwd: Path, more: Mapping[str, object]
) -> bytes:
    """Return what a hook is told: one line of JSON, with a line end after it.

    Args:
        event: The moment.
        session_id: The id of the session.
        transcript: Where the log of the session lies.
        cwd: The working directory of the agent.
        more: What this moment adds to what every hook is told.
    """
    told = {
        "session_id": session_id,
        "transcript_path": str(transcript),
        "cwd": str(cwd),
        "hook_event_name": event,
        **more,
    }
    return json.dumps(told, ensure_ascii=False).encode("utf-8") + b"\n"


if sys.platform == "win32":  # pragma: windows only

    def invocation(shell: str, command: str) -> list[str]:
        """Return what to run for the command of a hook.

        The script is handed over as an argument, in the one encoding that the
        shell takes there, so that its input stays free for what the hook is
        told.
        """
        script = f"{shells.PREAMBLE}\n{command}{shells.AFTER}"
        encoded = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
        return [
            shell, "-NoLogo", "-NoProfile", "-NonInteractive",
            "-InputFormat", "None", "-OutputFormat", "Text", "-EncodedCommand", encoded,
        ]  # fmt: skip

else:  # pragma: posix only

    def invocation(shell: str, command: str) -> list[str]:
        """Return what to run for the command of a hook."""
        return [shell, "--noprofile", "--norc", "-c", command]


_CLIXML = "#< CLIXML"
_ERROR = re.compile(r'<S S="Error">(.*?)</S>', re.DOTALL)
_ESCAPE = re.compile(r"_x([0-9A-Fa-f]{4})_")


def from_clixml(text: str) -> str:
    """Return what the shell of Windows wrote about an error, as plain text.

    That shell writes its own errors to the error stream as XML. Text that
    is none of that comes back as it is.
    """
    if not text.lstrip().startswith(_CLIXML):
        return text
    parts = [html.unescape(part) for part in _ERROR.findall(text)]
    joined = _ESCAPE.sub(lambda found: chr(int(found.group(1), 16)), "".join(parts))
    return joined.replace("\r\n", "\n")


def _field(fields: Mapping[str, object], key: str) -> str:
    value = fields.get(key)
    return value.strip() if isinstance(value, str) else ""


def _from_json(event: str, text: str, limit: int) -> Result:
    """Read the JSON that a hook wrote to its output with exit code 0."""
    try:
        loaded: object = json.loads(text)
    except (ValueError, RecursionError):
        return Result("failed", warnings=("its output begins like JSON and is none",), exit_code=0)
    # Text that begins with a brace and is JSON is an object.
    fields = cast("dict[str, object]", loaded)
    warnings = [
        f"its answer has the field {name!r}, which is not honoured here"
        for name in _UNSUPPORTED
        if name in fields
    ]
    decision: Decision | None = None
    reason = ""
    context = ""
    top = fields.get("decision")
    if top == "block":
        decision, reason = "block", _field(fields, "reason")
    elif top == "approve":
        decision = "allow"
    elif top is not None:
        warnings.append("its answer has a decision that is neither block nor approve")
    specific = fields.get("hookSpecificOutput")
    if isinstance(specific, dict):
        about = cast("dict[str, object]", specific)
        if about.get("hookEventName") == event:
            wanted = about.get("permissionDecision")
            if event == "PreToolUse" and isinstance(wanted, str) and wanted in _PERMISSION:
                # What is said about this one call counts before what is said in general.
                decision = cast("Decision", _PERMISSION[wanted])
                reason = _field(about, "permissionDecisionReason")
            elif wanted is not None:
                warnings.append("its answer has a permission decision that cannot be used here")
            if event in CONTEXT_EVENTS:
                context = _field(about, "additionalContext")
        else:
            warnings.append(
                "its answer speaks of another moment than the one that it was asked about, "
                "and that part is passed over"
            )
    if decision == "block" and not reason:
        reason = _NO_REASON
    if decision == "allow":
        reason = ""
    return Result(
        "blocked" if decision == "block" else "completed",
        decision,
        cut(reason, limit),
        cut(context, limit),
        cut(_field(fields, "systemMessage"), limit),
        tuple(warnings),
        0,
    )


def read(event: str, done: process.Completed, limit: int) -> Result:
    """Read what a hook answered.

    Args:
        event: The moment that the hook was run for.
        done: How its command ended, with its two streams apart.
        limit: How many characters a reason and what is added for the model
            may each have.
    """
    if done.timed_out:
        return Result("timed-out")
    errors = from_clixml(done.error_output).strip()
    code = done.exit_code
    if code == 2:
        return Result("blocked", "block", cut(errors or _NO_REASON, limit), exit_code=2)
    if code != 0:
        said = errors.partition("\n")[0]
        why = f"it ended with the exit code {code}" + (f": {cut(said, 200)}" if said else "")
        return Result("failed", warnings=(why,), exit_code=code)
    if done.omitted_bytes:
        why = f"it wrote more than the {MAX_OUTPUT_BYTES} bytes that are read"
        return Result("failed", warnings=(why,), exit_code=0)
    text = done.output.strip()
    if text.startswith("{"):
        return _from_json(event, text, limit)
    context = text if event in _PLAIN_CONTEXT else ""
    return Result(context=cut(context, limit), exit_code=0)


_RANK: dict[Decision | None, int] = {None: 0, "allow": 1, "ask": 2, "block": 3}


def merged(results: Sequence[Result]) -> Result:
    """Put together what the hooks of one moment answered.

    The strictest of them counts: stop before ask, ask before let through.
    The reason is that of the hooks that wanted just that, and of no other.
    What they add for the model is put together in their order.
    """
    decision: Decision | None = None
    for result in results:
        if _RANK[result.decision] > _RANK[decision]:
            decision = result.decision
    reasons = [result.reason for result in results if result.decision == decision and result.reason]
    return Result(
        "blocked" if decision == "block" else "completed",
        decision,
        "\n\n".join(reasons) if decision in ("ask", "block") else "",
        "\n\n".join(result.context for result in results if result.context),
        "\n".join(result.message for result in results if result.message),
        tuple(warning for result in results for warning in result.warnings),
    )


async def run(
    hook: Hook, event: str, told: bytes, *, shell: str, cwd: Path, project: Path, limit: int
) -> Result:
    """Run one hook and return what it answered. This fails for nobody.

    Args:
        hook: The hook.
        event: The moment that it is run for.
        told: What it is told, see :func:`payload`.
        shell: The path of the shell that runs its command.
        cwd: The directory to run it in.
        project: The root of the project, which the hook finds in its
            environment.
        limit: How many characters of a reason, and of what is added for the
            model, are taken.
    """
    if len(hook.command) > MAX_COMMAND:
        why = f"its command has more than {MAX_COMMAND} characters, and was not run"
        return Result("failed", warnings=(why,))
    env = shells.environment()
    env["CLAUDE_PROJECT_DIR"] = env["COLUMBINA_PROJECT_DIR"] = str(project)
    try:
        done = await process.run(
            invocation(shell, hook.command),
            cwd=cwd,
            env=env,
            stdin=told,
            timeout_seconds=hook.timeout,
            max_output_bytes=MAX_OUTPUT_BYTES,
            streams="apart",
        )
    except OSError as error:
        return Result("failed", warnings=(f"its command could not be started: {error}",))
    return read(event, done, limit)
