"""Small faults to put into the code, to see whether the tests notice.

A mutant is a replacement of text in one source file. ``mutate.py`` puts each
one in, runs the tests that ought to notice, and takes it out again. A mutant
that the tests do not notice points at something they do not check.

The text to replace must occur exactly once in its file. ``test_mutants.py``
checks that for every mutant, so a list cannot go stale without a test failing.

Left out on purpose are mutants that change nothing that can be observed. One
example: counting the lines after the wanted ones on the slow path of the
reader instead of the fast one. Another: not waiting for the rest of the
output of a program that has ended. The event loop hands over what was on its
way before the run goes on, though nothing says that it must. A third: asking
again where an open file is after the system has said that it is elsewhere.
The second look agrees with the first unless a link is put back between them.
A fourth: remembering the last message of every source of a user message, and not
only of the sources of context. Nothing asks for the others. A fifth: letting the
library that renders the lines of a conversation read markup. It reads markup in
plain strings only, and the lines that it is handed are no plain strings.

A mutant of code that only one platform runs says which. On another platform
no test can notice it, and ``mutate.py`` passes it over.
"""

from dataclasses import dataclass, replace

LOOP = "src/columbina/agent_loop/loop.py"
INBOX = "src/columbina/agent/inbox.py"
AGENT_CONTEXT = "src/columbina/agent/context.py"
AGENT_PACKAGE = "src/columbina/agent/__init__.py"
CANCELLATION = "src/columbina/cancellation.py"
READ = "src/columbina/builtin_tools/read.py"
PROCESS = "src/columbina/process.py"
PIPELINE = "src/columbina/tools/pipeline.py"
SEAM = "src/columbina/fs/__init__.py"
PATHS = "src/columbina/fs/paths.py"
BOUND = "src/columbina/fs/bound.py"
SANDBOX = "src/columbina/sandbox/__init__.py"
WRITE = "src/columbina/builtin_tools/write.py"
EDIT = "src/columbina/builtin_tools/edit.py"
WALK = "src/columbina/fs/walk.py"
GLOB = "src/columbina/builtin_tools/glob.py"
APPROVAL = "src/columbina/approval/__init__.py"
GREP = "src/columbina/builtin_tools/grep.py"
SHELL = "src/columbina/builtin_tools/shell.py"
PATTERNS = "src/columbina/fs/patterns.py"
RUNTIME = "src/columbina/context/runtime.py"
LAUNCHER = "src/columbina/apps/launcher.py"
BASE_BUNDLE = "src/columbina/bundles/base.yml"
INSTRUCTIONS = "src/columbina/context/instructions.py"
PROMPT = "src/columbina/system_prompt/__init__.py"
RULES = "src/columbina/permissions/rules.py"
COMMANDS = "src/columbina/permissions/commands.py"
PERMISSIONS = "src/columbina/permissions/__init__.py"
GREP_WORKER = "src/columbina/builtin_tools/grep_worker.py"
CLI_LINES = "src/columbina/apps/cli/lines.py"
CLI_TRANSCRIPT = "src/columbina/apps/cli/transcript.py"
CLI_PROMPT = "src/columbina/apps/cli/prompt.py"
CLI_APPROVAL = "src/columbina/apps/cli/approval.py"
CLI_STATUS = "src/columbina/apps/cli/status.py"
CLI_FRONTEND = "src/columbina/apps/cli/__init__.py"
CLI_BUNDLE = "src/columbina/bundles/cli.yml"
SLASH = "src/columbina/commands/__init__.py"
COMMAND_TOOLS = "src/columbina/commands/tools.py"
COMMAND_SESSIONS = "src/columbina/commands/sessions.py"
COMMAND_PERMISSIONS = "src/columbina/commands/permissions.py"
TERMINAL_TEXT = "src/columbina/apps/terminal_text.py"
HEADLESS = "src/columbina/apps/headless.py"
SESSION_STORE = "src/columbina/session/store.py"
SESSION_SERVICE = "src/columbina/session/service.py"
UNSEEN = "src/columbina/unseen.py"


@dataclass(frozen=True, slots=True)
class Mutant:
    """One fault.

    Attributes:
        name: What goes wrong, in a few words.
        path: The file, from the root of the repository.
        changes: Pairs of the text to replace and the text to put there.
        only_on: The ``sys.platform`` that runs the code, or ``None`` if every one does.
    """

    name: str
    path: str
    changes: tuple[tuple[str, str], ...]
    only_on: str | None = None


@dataclass(frozen=True, slots=True)
class MutantSet:
    """Mutants and the tests that ought to find them.

    Attributes:
        tests: What pytest is run on.
        mutants: The faults.
    """

    tests: tuple[str, ...]
    mutants: tuple[Mutant, ...]


def mutant(name: str, path: str, old: str, new: str, *more: tuple[str, str]) -> Mutant:
    return Mutant(name, path, ((old, new), *more))


def on_windows(fault: Mutant) -> Mutant:
    return replace(fault, only_on="win32")


_LOOP = (
    mutant(
        "tools cancelled: started calls get no result",
        LOOP,
        "                        outcomes[index] = pending.outcome or pending.fail(\n"
        '                            "TOOL_CANCELLED", _CUT_SHORT\n'
        "                        )\n",
        "                        pass\n",
    ),
    mutant(
        "tools cancelled: calls that were not taken up get no result",
        LOOP,
        "                        write()\n                        outcomes[index] = self._not_run(call)\n",
        "                        continue\n",
    ),
    mutant(
        "interrupted reply: its tool calls get no result",
        LOOP,
        "                for asked in message.tool_calls:\n"
        "                    session.append(self._not_run(asked).to_event())\n"
        "                raise\n",
        "                raise\n",
    ),
    mutant(
        "step end is not written when a step fails",
        LOOP,
        "            try:\n"
        "                ended = await self._step(turn, step)\n"
        "            finally:\n"
        "                session.append(StepEndEvent(turn=turn, step=step))\n",
        "            ended = await self._step(turn, step)\n"
        "            session.append(StepEndEvent(turn=turn, step=step))\n",
    ),
    mutant(
        "turn numbers do not advance",
        LOOP,
        "        turn = TURN.of(session).turn + 1\n",
        "        turn = max(TURN.of(session).turn, 1)\n",
    ),
    mutant(
        "results are written as they come, not in the order of the model",
        LOOP,
        "            while written in outcomes:\n"
        "                session.append(outcomes[written].to_event())\n"
        "                written += 1\n",
        "            for position in sorted(outcomes):\n"
        "                if position not in done:\n"
        "                    done.add(position)\n"
        "                    session.append(outcomes[position].to_event())\n"
        "            written += 0\n",
        ("        written = 0\n", "        written = 0\n        done: set[int] = set()\n"),
    ),
    mutant(
        "the turn end is not written after a cancellation",
        LOOP,
        "        session.append(TurnEndEvent(turn=turn, reason=reason, detail=detail))\n"
        "        await session.flush()\n",
        '        if reason != "aborted":\n'
        "            session.append(TurnEndEvent(turn=turn, reason=reason, detail=detail))\n"
        "        await session.flush()\n",
    ),
    mutant(
        "an exclusive call does not wait for the pool",
        LOOP,
        "                    room = self._max_parallel - 1 if pending.concurrency_safe else 0\n",
        "                    room = self._max_parallel - 1\n",
    ),
    mutant(
        "a call is checked before it can run",
        LOOP,
        "                    await until_running(room)\n"
        "                    await session.flush()\n"
        "                    await pending.authorize()\n",
        "                    await pending.authorize()\n"
        "                    await until_running(room)\n"
        "                    await session.flush()\n",
    ),
    mutant(
        "cancel clears nothing from the inbox",
        LOOP,
        "        if not keep_inbox and not self.session.closed:\n            clear_inbox(self.session)\n",
        "        if False:\n            clear_inbox(self.session)\n",
    ),
    mutant(
        "listeners of the loop are awaited without a guard",
        LOOP,
        '        return await cancellation.guard(answer, what=f"a listener of {event.name}")\n',
        "        return await answer\n",
    ),
    mutant(
        "a step asks the model again although the turn was told to end",
        LOOP,
        '            if ended == "concluded":\n',
        '            if ended == "never":\n',
    ),
    mutant(
        "a turn takes its task without removing it from the inbox",
        INBOX,
        "    if task:\n"
        '        session.append(InboxSplicedEvent(queue="next-turn", removed=1))\n'
        "    return task\n",
        "    return task\n",
    ),
    mutant(
        "a step takes nothing from the inbox",
        INBOX,
        "    if inbox.next_step:\n"
        '        session.append(InboxSplicedEvent(queue="next-step", removed=len(inbox.next_step)))\n'
        "    return inbox.next_step\n",
        "    return ()\n",
    ),
    mutant(
        "a turn that is being cancelled still takes a task",
        LOOP,
        "        if first_step and not cancellation.requests():\n",
        "        if first_step:\n",
    ),
    mutant(
        "a turn that is being cancelled still takes the input for its next step",
        LOOP,
        "        if cancellation.requests():\n            return ()\n        claimed = claim_for_step",
        "        claimed = claim_for_step",
    ),
    mutant(
        "emptying the inbox drops what was given meanwhile",
        INBOX,
        "    tasks = _still_first(INBOX.of(session).next_turn, waiting.next_turn)\n",
        "    tasks = len(INBOX.of(session).next_turn)\n",
    ),
    mutant(
        "emptying the inbox counts the tasks before a listener took some away",
        INBOX,
        "    tasks = _still_first(INBOX.of(session).next_turn, waiting.next_turn)\n",
        "    tasks = len(waiting.next_turn)\n",
    ),
    mutant(
        "a result that comes back in spite of a cancellation is accepted",
        CANCELLATION,
        "    await kept_back(before)\n    return result\n",
        "    return result\n",
    ),
    mutant(
        "a pending cancellation is not let through by the guard",
        CANCELLATION,
        "    await asyncio.sleep(0)\n    return current_task().cancelling()\n",
        "    return current_task().cancelling()\n",
    ),
)

_READ = (
    mutant(
        "fast path one line too early",
        READ,
        "if self.lines + breaks < self._first or self.lines >= self._end:",
        "if self.lines + breaks <= self._first or self.lines >= self._end:",
    ),
    mutant(
        "a return is always taken from the head",
        READ,
        "                if self._passed:\n"
        "                    self._passed -= 1\n"
        "                else:\n"
        "                    self._head = self._head[:-1]\n",
        "                self._head = self._head[:-1]\n",
    ),
    mutant(
        "a return is kept in a line that was cut short",
        READ,
        "            if self._returned:\n",
        "            if self._returned and not cut_short:\n",
    ),
    mutant(
        "the note of a long line counts one character too many",
        READ,
        "        self._passed += max(len(text) - room, 0)\n",
        "        self._passed += max(len(text) - room + 1, 0)\n",
    ),
    mutant(
        "a line that was cut short is said to be whole",
        READ,
        'counted = f"at least {self._passed}" if cut_short else str(self._passed)',
        "counted = str(self._passed)",
    ),
    mutant(
        "an empty piece forgets the return before it",
        READ,
        "        if not text:\n            return\n",
        "",
    ),
    mutant(
        "a return in the middle of a line is remembered",
        READ,
        '        self._returned = text.endswith("\\r")\n',
        '        self._returned = self._returned or text.endswith("\\r")\n',
    ),
    mutant(
        "a character that the line ends inside of is not shown",
        READ,
        '                self._take(b"", final=True)\n',
        "                pass\n",
    ),
    mutant(
        "a character that a cut line ends inside of is shown as broken",
        READ,
        "            if not cut_short:\n"
        "                # Bytes at the end that begin a character and do not finish it are shown too.\n",
        "            if True:\n",
    ),
    mutant(
        "the size limit is off by one",
        READ,
        "if self.shown and self._size > MAX_BYTES:",
        "if self.shown and self._size >= MAX_BYTES:",
    ),
    mutant(
        "the first line may be dropped for its size",
        READ,
        "if self.shown and self._size > MAX_BYTES:",
        "if self._size > MAX_BYTES:",
    ),
    mutant(
        "the line that was cut short is not counted",
        READ,
        "        reader.end_line(cut_short=not complete)\n        seen += 1\n",
        "        reader.end_line(cut_short=not complete)\n        seen += 1 if complete else 0\n",
    ),
    mutant(
        "whether a line is open is only known after a wanted piece",
        READ,
        '            self._take(chunk[start:])\n        self.open = not chunk.endswith(b"\\n")\n',
        '            self._take(chunk[start:])\n            self.open = not chunk.endswith(b"\\n")\n',
    ),
    mutant(
        "the head may grow beyond what is shown",
        READ,
        "        self._head += text[:room]\n",
        "        self._head += text\n",
    ),
    mutant(
        "a line that ends is not counted after the output is full",
        READ,
        "        if not cut_short:\n            self.lines += 1\n",
        "        if not cut_short and self._wanted():\n            self.lines += 1\n",
    ),
    mutant(
        "the file is never found complete at the limit",
        READ,
        "            complete = not stopped.is_set() and not file.read(1)\n",
        "            complete = False\n",
    ),
    mutant(
        "the count of a line leaks into the next",
        READ,
        '            self._head = ""\n            self._passed = 0\n            self._returned = False\n',
        '            self._head = ""\n            self._returned = False\n',
    ),
    mutant(
        "a path that leads elsewhere than when it was checked is read",
        READ,
        "        if where.relative != run.target:",
        "        if False:",
    ),
    mutant(
        "a file that was read is not remembered as shown",
        READ,
        "    ctx.on(RESULT, remember)\n",
        "",
    ),
    mutant(
        "what a call has read counts before its result is before the model",
        READ,
        "            fs.showing(result.permit, result.fingerprint)",
        "            fs.shown(result.permit, result.fingerprint)",
    ),
    mutant(
        "a read that showed no line of a file counts as having shown it",
        READ,
        "            and result.shows\n",
        "",
    ),
    mutant(
        "a file counts as shown though other words took the place of its content",
        READ,
        "            and result.text in outcome.text\n",
        "",
    ),
    mutant(
        "the thread is not told to stop",
        READ,
        "            # A thread cannot be cancelled. It is told to stop, and does so at its next piece.\n"
        "            stopped.set()\n",
        "            pass\n",
    ),
    mutant(
        "the thread does not listen",
        READ,
        "while scanned < _SCAN_BYTES and not stopped.is_set():",
        "while scanned < _SCAN_BYTES:",
    ),
    mutant(
        "more than is looked at is read in the last piece",
        READ,
        "chunk = file.read(min(_CHUNK_BYTES, _SCAN_BYTES - scanned))",
        "chunk = file.read(_CHUNK_BYTES)",
    ),
    mutant(
        "the note forgets to say how to read on",
        READ,
        '    read_on = f" Pass offset={last} to read on." if last < part.lines else ""\n',
        '    read_on = ""\n',
    ),
    mutant(
        "a file is taken for text whatever it holds",
        READ,
        '            if scanned == 0 and b"\\x00" in chunk[:_SNIFF_BYTES]:\n'
        "                return None\n",
        "",
    ),
)

_PROCESS = (
    mutant(
        "what a program started is not stopped",
        PROCESS,
        "        # However the wait ended, nothing that the program started goes on.\n"
        "        tree.stop()\n",
        "        # However the wait ended, nothing that the program started goes on.\n",
    ),
    mutant(
        "the rest of the output is waited for without end",
        PROCESS,
        "            await _within(watch.silent, _DRAIN_SECONDS)\n",
        "            await watch.silent\n",
    ),
    mutant(
        "a program that was stopped is waited for without end",
        PROCESS,
        "            await _within(watch.exited, _REAP_SECONDS)\n",
        "            await watch.exited\n",
    ),
    mutant(
        "a program that comes into being after its caller left is not ended",
        PROCESS,
        "        creating.add_done_callback(_abandoned)\n",
        "",
    ),
    mutant(
        "a program that ran out of time keeps its exit code",
        PROCESS,
        "        exit_code=None if timed_out else exit_code,",
        "        exit_code=exit_code,",
    ),
    mutant(
        "running out of time goes unnoticed",
        PROCESS,
        "        timed_out = not await _within(watch.exited, timeout_seconds)",
        "        await _within(watch.exited, timeout_seconds)",
    ),
    mutant(
        "the beginning takes all the room",
        PROCESS,
        "        self._head_room = limit // 2",
        "        self._head_room = limit",
    ),
    mutant(
        "the end is kept without a limit",
        PROCESS,
        "        del self._tail[: max(0, len(self._tail) - self._tail_room)]\n",
        "",
    ),
    mutant(
        "output of exactly the limit counts as too much",
        PROCESS,
        "        if self.total <= self._head_room + self._tail_room:",
        "        if self.total < self._head_room + self._tail_room:",
    ),
    mutant(
        "half a character is left where the beginning is cut",
        PROCESS,
        "        head = bytes(self._head[: len(self._head) - unfinished])",
        "        head = bytes(self._head)",
    ),
    mutant(
        "half a character is left where the end begins",
        PROCESS,
        "        return head, bytes(self._tail[rest:])",
        "        return head, bytes(self._tail)",
    ),
    mutant(
        "one byte is counted like several",
        PROCESS,
        '    if count == 1:\n        return "[1 byte of output is left out here]"\n',
        "",
    ),
    mutant(
        "the line about what is left out follows the beginning without a break",
        PROCESS,
        '        gap = "" if not head or head.endswith("\\n") else "\\n"',
        '        gap = ""',
    ),
    mutant(
        "secrets are passed on when no environment is given",
        PROCESS,
        "            env=cleaned(os.environ) if env is None else dict(env),",
        "            env=dict(os.environ) if env is None else dict(env),",
    ),
    mutant(
        "a secret is known only in capitals",
        PROCESS,
        "        if not any(word in name.upper() for word in _SECRET_WORDS)",
        "        if not any(word in name for word in _SECRET_WORDS)",
    ),
    mutant(
        "a password is no secret",
        PROCESS,
        '_SECRET_WORDS = ("KEY", "PASSWORD", "SECRET", "TOKEN")',
        '_SECRET_WORDS = ("KEY", "SECRET", "TOKEN")',
    ),
    mutant(
        "the error stream is dropped",
        PROCESS,
        "            stderr=subprocess.STDOUT,",
        "            stderr=subprocess.DEVNULL,",
    ),
    mutant(
        "a program given no input waits for some",
        PROCESS,
        "            stdin=subprocess.DEVNULL if stdin is None else subprocess.PIPE,",
        "            stdin=subprocess.PIPE,",
    ),
    mutant(
        "the input of a program has no end",
        PROCESS,
        "            feed.write_eof()\n",
        "",
    ),
    on_windows(
        mutant(
            "the program is not put into its job",
            PROCESS,
            "                        _kernel32.AssignProcessToJobObject(job, process),",
            "                        True,",
        )
    ),
    on_windows(
        mutant(
            "the job does not end with its handle",
            PROCESS,
            "                limits.BasicLimitInformation.LimitFlags = _KILL_ON_JOB_CLOSE",
            "                limits.BasicLimitInformation.LimitFlags = 0",
        )
    ),
    on_windows(
        mutant(
            "the program runs before it is in its job",
            PROCESS,
            '{"creationflags": _CREATE_SUSPENDED | subprocess.CREATE_NO_WINDOW}',
            '{"creationflags": subprocess.CREATE_NO_WINDOW}',
        )
    ),
    on_windows(
        mutant(
            "a program that could not be put into a job is left as it is",
            PROCESS,
            "        # The program has not run a single instruction, and it does not get to.\n"
            "        transport.close()\n",
            "        # The program has not run a single instruction, and it does not get to.\n",
        )
    ),
)

_TOOLS = (
    mutant(
        "the guards are not asked",
        PIPELINE,
        "        elif await self._passes_the_guards(invocation):\n            self._authorized = True\n",
        "        else:\n            self._authorized = True\n",
    ),
    mutant(
        "a listener can put another call in the place of the checked one",
        PIPELINE,
        "            if passed is not invocation:",
        "            if False:",
    ),
    mutant(
        "arguments that were changed before the guards are judged all the same",
        PIPELINE,
        "        elif invocation.args.model_dump() != self._written:",
        "        elif False:",
    ),
    mutant(
        "arguments that were changed after the checks are run all the same",
        PIPELINE,
        "            if invocation.args.model_dump() != self._written:",
        "            if False:",
    ),
    mutant(
        "a tool that left while a listener held the call runs",
        PIPELINE,
        "            if self._tools.registry.get(tool.name) is not tool:",
        "            if False:",
    ),
    mutant(
        "a tool is not told what the checks were told",
        PIPELINE,
        "        aimed = ToolRun(run.call_id, run.cwd, run.session, target)",
        "        aimed = ToolRun(run.call_id, run.cwd, run.session)",
    ),
    mutant(
        "a call is checked although its tool has left",
        PIPELINE,
        "        if self._gone(invocation.tool) is not None:\n            return\n",
        "",
    ),
    mutant(
        "a tool that took the name of another counts as that one",
        PIPELINE,
        "        if self._tools.registry.get(tool.name) is tool:",
        "        if tool.name in self._tools.registry:",
    ),
)

_FS = (
    mutant(
        "a session follows its working directory wherever it leads",
        SEAM,
        "        if first == where.root:",
        "        if True:",
    ),
    mutant(
        "a place that was found without the seam is not held to the working directory",
        SEAM,
        "        moved = self._moved(access.session, access.where)\n"
        "        if moved is not None:\n"
        "            return moved\n",
        "",
    ),
    mutant(
        "locating through the seam says nothing about a working directory that has moved",
        SEAM,
        "        if moved is not None:\n            raise ToolFailure(moved.reason, code=moved.code)\n",
        "",
    ),
    mutant(
        "the refusal of a restriction is passed over",
        SEAM,
        "            if isinstance(refusal, Refuse):\n                return refusal\n",
        "",
    ),
    mutant(
        "anything in the working directory may be written",
        SEAM,
        '        if access.kind == "read" and where.inside:',
        "        if where.inside:",
    ),
    mutant(
        "anything may be read, wherever it is",
        SEAM,
        '        if access.kind == "read" and where.inside:',
        '        if access.kind == "read":',
    ),
    mutant(
        "a grant allows by any answer that is not nothing",
        SEAM,
        '                if cast("object", grant(access)) is True:',
        "                if grant(access):",
    ),
    mutant(
        "a restriction that fails lets the access through",
        SEAM,
        '                logger.error("the restriction %r of file access failed", name, exc_info=error)\n'
        "                return Refuse(\n"
        '                    f"The access could not be checked and did not happen: {_describe(error)}",\n'
        '                    "CHECK_FAILED",\n'
        "                )\n",
        '                logger.error("the restriction %r of file access failed", name, exc_info=error)\n'
        "                continue\n",
    ),
    mutant(
        "an answer of a restriction that is no refusal lets the access through",
        SEAM,
        "            if refusal is not None:\n"
        '                logger.error("the restriction %r of file access gave no refusal", name)\n',
        "            if False:\n"
        '                logger.error("the restriction %r of file access gave no refusal", name)\n',
    ),
    mutant(
        "an access that was refused gets a permit all the same",
        SEAM,
        "        if refusal is not None:\n"
        "            raise ToolFailure(refusal.reason, code=refusal.code)\n",
        "",
    ),
    mutant(
        "a permit to write opens for reading",
        SEAM,
        '        if self.access.kind != "read":\n'
        '            raise ValueError("this permit is not for reading")\n',
        "",
    ),
    mutant(
        "no path means the working directory",
        PATHS,
        "    if not written.strip():\n"
        '        raise ToolFailure("No path was given.", code="INVALID_PATH")\n',
        "",
    ),
    mutant(
        "a path that climbs out counts as inside",
        PATHS,
        "    inside = path.parts[: len(top)] == top",
        "    inside = True",
    ),
    on_windows(
        mutant(
            "names are compared after their case was lowered",
            PATHS,
            "    inside = path.parts[: len(top)] == top",
            "    inside = path.is_relative_to(root)",
        )
    ),
    mutant(
        "what a search comes upon counts as lying where the search began",
        PATHS,
        "    return _placed(directory.root, directory.path / name, None)",
        "    return _placed(directory.path, directory.path / name, None)",
    ),
    on_windows(
        mutant(
            "a path that names another machine is handed to the system",
            PATHS,
            "        if _begins_with_two_slashes(written):\n"
            '            raise _refused(written, "a path that begins with two slashes names another machine.")\n',
            "",
        )
    ),
    on_windows(
        mutant(
            "a link that leads to another machine is followed",
            PATHS,
            "                if _leads_away(target):\n"
            '                    raise _refused(written, "a link on the way leads to another machine.")\n',
            "",
        )
    ),
    on_windows(
        mutant(
            "behind the prefix that switches the usual reading off, everything is a disk",
            PATHS,
            '            return not (_on_a_drive(rest) or rest.upper().startswith("VOLUME{"))',
            "            return False",
        )
    ),
    on_windows(
        mutant(
            "a volume without a letter counts as another machine",
            PATHS,
            '            return not (_on_a_drive(rest) or rest.upper().startswith("VOLUME{"))',
            "            return not _on_a_drive(rest)",
        )
    ),
    on_windows(
        mutant(
            "a step back is left in the path that is walked",
            PATHS,
            "        pending = os.path.normpath(joined)",
            "        pending = joined",
        )
    ),
    on_windows(
        mutant(
            "links are followed in a circle for ever",
            PATHS,
            "        for _ in range(_MAX_LINKS + 1):",
            "        while True:",
        )
    ),
    on_windows(
        mutant(
            "a device with an ending to its name is a file",
            PATHS,
            '        if name.split(".")[0].rstrip(" ").upper() in _DEVICES:',
            "        if name.upper() in _DEVICES:",
        )
    ),
    on_windows(
        mutant(
            "a name may end with a dot",
            PATHS,
            '        if name[-1] in ". ":',
            '        if name[-1] == " ":',
        )
    ),
    on_windows(
        mutant(
            "a name may hold a colon",
            PATHS,
            '        if ":" in name:\n            return "a colon inside a name is not allowed."\n',
            "",
        )
    ),
    mutant(
        "what was opened is not looked at again",
        BOUND,
        "        if not _is_at(descriptor, path):\n"
        '            raise MovedError(f"{path} was somewhere else when it was opened")\n',
        "",
    ),
    mutant(
        "a device is read like a file",
        BOUND,
        "        if not stat.S_ISREG(status.st_mode):\n"
        '            raise NotAFileError(f"{path} is not a regular file")\n',
        "",
    ),
    mutant(
        "a file that is refused stays open",
        BOUND,
        "    except BaseException:\n        os.close(descriptor)\n        raise\n",
        "    except BaseException:\n        raise\n",
    ),
    on_windows(
        mutant(
            "another case of a name is another file",
            BOUND,
            "        return _plain(one) == _plain(other) or _same_entries(_plain(one), _plain(other))",
            "        return _plain(one) == _plain(other)",
        )
    ),
    mutant(
        "a link counts as the place that it leads to",
        BOUND,
        "        return os.path.samestat(left.lstat(), right.lstat())",
        "        return os.path.samestat(left.stat(), right.stat())",
    ),
    mutant(
        "without an answer, any file at the place will do",
        BOUND,
        "    return os.path.samestat(os.fstat(descriptor), named) and _spelled_alike(still, str(expected))",
        "    return _spelled_alike(still, str(expected))",
    ),
    mutant(
        "without an answer, the same file will do wherever the path leads now",
        BOUND,
        "    return os.path.samestat(os.fstat(descriptor), named) and _spelled_alike(still, str(expected))",
        "    return os.path.samestat(os.fstat(descriptor), named)",
    ),
    mutant(
        "a file that the model never saw is taken for one that changed",
        BOUND,
        '    if seen is None:\n        raise NotShownError(f"{path} was never seen")\n',
        "",
    ),
    mutant(
        "a file that changed since it was seen is replaced",
        BOUND,
        "    if (existing.st_size, existing.st_mtime_ns) != seen:\n"
        '        raise ChangedError(f"{path} is not as it was seen")\n',
        "",
    ),
    mutant(
        "a change that keeps the size goes unnoticed",
        BOUND,
        "    if (existing.st_size, existing.st_mtime_ns) != seen:",
        "    if existing.st_size != seen[0]:",
    ),
    mutant(
        "a directory is taken for a file that was never seen",
        BOUND,
        "    if not stat.S_ISREG(existing.st_mode):\n"
        '        raise NotAFileError(f"{path} is not a regular file")\n',
        "",
    ),
    mutant(
        "nothing is asked before the new content is made ready",
        BOUND,
        "        existing = _there(directory, path, seen)\n        beside =",
        "        existing = None\n        beside =",
    ),
    mutant(
        "the file is not looked at again when the new content is ready",
        BOUND,
        "            # changed, gone or come into being meanwhile.\n"
        "            existing = _there(directory, path, seen)\n",
        "            # changed, gone or come into being meanwhile.\n",
    ),
    mutant(
        "a new file takes its name from whatever has it",
        BOUND,
        "                    directory.place(beside, name)",
        "                    directory.rename(beside, name)",
    ),
    mutant(
        "a replacement that was given up goes ahead",
        BOUND,
        "            if given_up is not None and given_up.is_set():",
        "            if False:",
    ),
    mutant(
        "what is said about the file is looked up after it got its name",
        BOUND,
        "        directory.sync()\n        return written\n",
        "        directory.sync()\n        return directory.status(name)\n",
    ),
    mutant(
        "what was written beside the file stays there after a failure",
        BOUND,
        "        except BaseException:\n            directory.remove(beside)\n            raise\n",
        "        except BaseException:\n            raise\n",
    ),
    mutant(
        "directories that are missing are not made",
        BOUND,
        "        held = held.enter(name)\n",
        "        pass\n",
    ),
    mutant(
        "a file in the way is treated as a directory to be made",
        BOUND,
        "        if present.exists():\n"
        '            raise NotADirectoryError(f"{present} is in the way, and it is no directory")\n',
        "",
    ),
    on_windows(
        mutant(
            "a directory is held without asking where it is",
            BOUND,
            "            if actual is None or not _spelled_alike(actual, str(path)):\n"
            "                self.close()\n"
            '                raise MovedError(f"{path} was somewhere else when it was opened")\n',
            "",
        )
    ),
    on_windows(
        mutant(
            "a file that is busy for a moment is given up at once",
            BOUND,
            "                    if attempt == _RENAME_ATTEMPTS:\n                        raise\n",
            "                    raise\n",
        )
    ),
    mutant(
        "what the agent wrote does not count as shown",
        SEAM,
        "        self._fs.shown(self, fingerprint)\n        return fingerprint\n",
        "        return fingerprint\n",
    ),
    mutant(
        "content that was made from another state than the model was shown is written",
        SEAM,
        "        if made_from is not None and shown != made_from:",
        "        if False:",
    ),
    mutant(
        "an older state on its way to the model undoes what the agent wrote",
        SEAM,
        "            self._coming.get(access.session, {}).pop(key, None)\n",
        "",
    ),
    mutant(
        "a step that begins delivers nothing",
        SEAM,
        "        if isinstance(committed.event.data, StepStartEvent):",
        "        if False:",
    ),
    mutant(
        "every event of the log delivers what was read",
        SEAM,
        "        if isinstance(committed.event.data, StepStartEvent):",
        "        if True:",
    ),
    mutant(
        "a call that is cancelled leaves its thread behind",
        SEAM,
        "        while not working.done() and loop.time() < until:",
        "        while False:",
    ),
    mutant(
        "work is not told that it is no longer wanted",
        SEAM,
        "        given_up.set()\n        loop = asyncio.get_running_loop()\n",
        "        loop = asyncio.get_running_loop()\n",
    ),
    mutant(
        "a second cancellation ends the stay with the thread",
        SEAM,
        "                # Cancelled once more. The thread is no further for that.\n"
        "                continue\n",
        "                break\n",
    ),
    mutant(
        "a permit to read replaces",
        SEAM,
        '        if self.access.kind != "write":\n'
        '            raise ValueError("this permit is not for writing")\n',
        "",
    ),
)

_SANDBOX = (
    mutant(
        "a session that is read-only is written to",
        SANDBOX,
        '        if mode == "read-only":',
        "        if False:",
    ),
    mutant(
        "what belongs to the repository is written like anything else",
        SANDBOX,
        '        protected = _protected(access) if mode == "workspace-write" else None',
        "        protected = None",
    ),
    mutant(
        "what belongs to the repository is kept from a session with full access",
        SANDBOX,
        '        protected = _protected(access) if mode == "workspace-write" else None',
        "        protected = _protected(access)",
    ),
    mutant(
        "a protected name counts only below the working directory",
        SANDBOX,
        "    for part in where.path.parts:",
        '    for part in where.relative.split("/"):',
    ),
    mutant(
        "what a protected name leads to is written under its own name",
        SANDBOX,
        "        if led == where.path or led in where.path.parents:",
        "        if False:",
    ),
    mutant(
        "the directory that a protected name leads to is itself written",
        SANDBOX,
        "        if led == where.path or led in where.path.parents:",
        "        if led in where.path.parents:",
    ),
    mutant(
        "another case of its name makes a directory unprotected",
        SANDBOX,
        "        if part.casefold() in PROTECTED:",
        "        if part in PROTECTED:",
    ),
    mutant(
        "a session that may write its working directory may write anywhere",
        SANDBOX,
        '        return mode == "workspace-write" and access.kind == "write" and access.where.inside',
        '        return mode == "workspace-write" and access.kind == "write"',
    ),
    mutant(
        "full access is no wider than the working directory",
        SANDBOX,
        '        if mode == "danger-full-access":\n            return True\n',
        "",
    ),
    mutant(
        "the mode that the log names is passed over",
        SANDBOX,
        "        return _MODE.of(session) or self._default",
        "        return self._default",
    ),
    mutant(
        "a mode is written although the log names it already",
        SANDBOX,
        "        if _MODE.of(session) != mode:",
        "        if True:",
    ),
    mutant(
        "the strength of the provider is not told",
        SANDBOX,
        '        return "none" if provider is None else provider.strength',
        '        return "none"',
    ),
    mutant(
        "a command is confined in the mode of the config",
        SANDBOX,
        "        held = provider.confine(argv, mode=self.mode(session), cwd=cwd)",
        "        held = provider.confine(argv, mode=self._default, cwd=cwd)",
    ),
    mutant(
        "a provider that is not there is no reason to stop",
        SANDBOX,
        "        if provider is None:\n            raise SandboxUnavailableError(",
        "        if False:\n            raise SandboxUnavailableError(",
    ),
)

_FILES = (
    mutant(
        "a write goes ahead where the path leads elsewhere than it did",
        WRITE,
        "        if where.relative != run.target:",
        "        if False:",
    ),
    mutant(
        "a file that was replaced is said to have been made",
        WRITE,
        '        deed = "Replaced" if was_there else "Created"',
        '        deed = "Created"',
    ),
    mutant(
        "a last line without a line break is not counted",
        WRITE,
        '    count = content.count("\\n") + (1 if content and not content.endswith("\\n") else 0)',
        '    count = content.count("\\n")',
    ),
    mutant(
        "a write is taken for a read by the checks",
        WRITE,
        '        effect="write",',
        '        effect="read",',
    ),
    mutant(
        "an edit goes ahead where the path leads elsewhere than it did",
        EDIT,
        "        if where.relative != run.target:",
        "        if False:",
    ),
    mutant(
        "an edit that changes nothing is carried out",
        EDIT,
        "        if args.old == args.new:",
        "        if False:",
    ),
    mutant(
        "an edit speaks about a file before it is known that the model saw it",
        EDIT,
        "    fs.must_have_shown(reading, fingerprint)\n",
        "",
    ),
    mutant(
        "several places are replaced without being asked for",
        EDIT,
        "    if count > 1 and not args.replace_all:",
        "    if False:",
    ),
    mutant(
        "a file of Windows gets plain line feeds",
        EDIT,
        "    if _ends_lines_like_windows(text):",
        "    if False:",
    ),
    mutant(
        "a file with both kinds of line breaks is taken for one of Windows",
        EDIT,
        '    return "\\r\\n" in text and "\\n" not in text.replace("\\r\\n", "")',
        '    return "\\r\\n" in text',
    ),
    mutant(
        "a file that is too large is edited",
        EDIT,
        "    if len(raw) > _MAX_BYTES:",
        "    if False:",
    ),
    mutant(
        "a file with bytes that no text holds is edited",
        EDIT,
        "        if bytes(1) in raw[:_SNIFF_BYTES]:",
        "        if False:",
    ),
    mutant(
        "an edit is taken for a read by the checks",
        EDIT,
        '        effect="write",',
        '        effect="read",',
    ),
)

_SEARCH = (
    mutant(
        "the seam is not asked about what a walk comes upon",
        WALK,
        '                permit = fs.permit(Access(session, cwd, where, "read", tool, named=False))',
        '                permit = Permit(Access(session, cwd, where, "read", tool, named=False), fs)',
    ),
    mutant(
        "what a walk comes upon counts as named by the call",
        WALK,
        '                permit = fs.permit(Access(session, cwd, where, "read", tool, named=False))',
        '                permit = fs.permit(Access(session, cwd, where, "read", tool))',
    ),
    mutant(
        "what the seam refuses ends the walk",
        WALK,
        "            except ToolFailure:\n                tally.refused += 1\n                continue\n",
        "            except ToolFailure:\n                tally.refused += 1\n                return\n",
    ),
    on_windows(
        mutant(
            "a name that the system reads as another is handed on",
            WALK,
            "            if not usable(entry.name):",
            "            if False:",
        )
    ),
    mutant(
        "the history of a repository is walked",
        WALK,
        '            if entry.kind == "directory" and entry.name in SKIPPED:',
        "            if False:",
    ),
    mutant(
        "a link is handed on like a file",
        WALK,
        '            if entry.kind not in ("file", "directory"):',
        "            if False:",
    ),
    mutant(
        "a walk looks at one entry more than it may",
        WALK,
        "            if tally.entries >= limit:",
        "            if tally.entries > limit:",
    ),
    mutant(
        "a walk that stopped early does not say so",
        WALK,
        "                tally.cut_short = True\n",
        "",
    ),
    mutant(
        "a directory that cannot be listed ends the walk",
        WALK,
        "            tally.unreadable += 1\n            continue\n",
        "            tally.unreadable += 1\n            return\n",
    ),
    mutant(
        "a walk goes down before it goes on",
        WALK,
        "        directory, names = waiting.popleft()",
        "        directory, names = waiting.pop()",
    ),
    mutant(
        "a pattern of one name is looked for at the top only",
        GLOB,
        '    if len(names) == 1:\n        names = ["**", *names]\n',
        "",
    ),
    mutant(
        "two stars stand for one directory at least",
        PATTERNS,
        "    reached = _with_none_skipped(parts, {0})",
        "    reached = {0}",
    ),
    mutant(
        "two stars in the middle stand for one directory at least",
        PATTERNS,
        "        reached = _with_none_skipped(parts, onward)",
        "        reached = onward",
    ),
    mutant(
        "two stars stand for one directory at most",
        PATTERNS,
        "                # The name is one more of those that this part stands for.\n"
        "                onward.add(at)\n",
        "                onward.add(at + 1)\n",
    ),
    mutant(
        "a path fits a pattern that it only fits the beginning of",
        PATTERNS,
        "    return len(parts) in reached",
        "    return bool(reached)",
    ),
    mutant(
        "a directory is listed whole, however much of it is of use",
        BOUND,
        "    listed = [_listed(entry) for entry in itertools.islice(found, at_most)]",
        "    listed = [_listed(entry) for entry in found]",
    ),
    mutant(
        "a directory that holds more than was taken is not said to",
        WALK,
        "        if more:\n",
        "        if False:\n",
    ),
    mutant(
        "a walk asks for all of a directory",
        WALK,
        "                bound.listing, directory.path, limit - tally.entries + 1\n",
        "                bound.listing, directory.path\n",
    ),
    mutant(
        "a pattern may step out of the directory that is searched",
        GLOB,
        '    if ".." in names:',
        "    if False:",
    ),
    mutant(
        "the file that was changed first comes first",
        GLOB,
        "    newest = sorted(matched, key=lambda found: (-found.modified_ns, found.where.relative))",
        "    newest = sorted(matched, key=lambda found: (found.modified_ns, found.where.relative))",
    ),
    mutant(
        "more than a hundred files are listed",
        GLOB,
        "    lines = [found.where.relative for found in newest[:MAX_RESULTS]]",
        "    lines = [found.where.relative for found in newest]",
    ),
    mutant(
        "what was kept from the search is not mentioned",
        WALK,
        "    if tally.refused:",
        "    if False:",
    ),
    mutant(
        "a search does not say that it stopped early",
        WALK,
        "    if tally.cut_short:",
        "    if False:",
    ),
    mutant(
        "directories that could not be listed are not mentioned",
        WALK,
        "    if tally.unreadable:",
        "    if False:",
    ),
    mutant(
        "one thing that was passed is counted like many",
        WALK,
        '    return f"1 {one}" if number == 1 else f"{number} {many}"',
        '    return f"{number} {many}"',
    ),
    mutant(
        "a file is searched as if it were a directory",
        GLOB,
        "        if not root.path.is_dir():",
        "        if False:",
    ),
    mutant(
        "a search goes ahead where the directory leads elsewhere than it did",
        GLOB,
        "        if root.relative != run.target:",
        "        if False:",
    ),
    on_windows(
        mutant(
            "upper and lower case count on a disk that takes them for one",
            GLOB,
            "    flags = re.IGNORECASE if IGNORES_CASE else 0",
            "    flags = 0",
        )
    ),
)

_GREP = (
    mutant(
        "a search goes ahead where the path leads elsewhere than it did",
        GREP,
        "        if root.relative != run.target:",
        "        if False:",
    ),
    mutant(
        "the file that a call names counts as one that the search came upon",
        GREP,
        '        permit = fs.permit(Access(run.session, run.cwd, root, "read", "grep"))',
        '        permit = fs.permit(Access(run.session, run.cwd, root, "read", "grep", named=False))',
    ),
    mutant(
        "a file that the call names is searched only if its path fits",
        GREP,
        "            return _only(Found(root, (), status.st_size, status.st_mtime_ns, permit)), None",
        "            return _only(Found(root, (), status.st_size, status.st_mtime_ns, permit)), fits",
    ),
    mutant(
        "files whose paths do not fit are searched",
        GREP,
        "                    if fits is not None and not fits(found.below):",
        "                    if False:",
    ),
    mutant(
        "a search that has as many lines as it shows goes on",
        GREP,
        "                    if len(search.lines) >= MAX_MATCHES:",
        "                    if False:",
    ),
    mutant(
        "a search that left files does not say that there may be more",
        GREP,
        "                            search.cut = True\n                            break\n",
        "                            break\n",
    ),
    mutant(
        "a search reads more than it may",
        GREP,
        "                    if search.searched_bytes >= MAX_BYTES:",
        "                    if False:",
    ),
    mutant(
        "a search that stopped reading does not say so",
        GREP,
        "                        search.out_of_room = True\n",
        "",
    ),
    mutant(
        "what a search has read is not counted",
        GREP,
        "                search.searched_bytes += sum(len(content) for _, content in texts)\n",
        "",
    ),
    mutant(
        "a file that is listed as too large is opened",
        GREP,
        "                    if found.size > MAX_FILE_BYTES:",
        "                    if False:",
    ),
    mutant(
        "a file of just the size that may be searched is passed over",
        GREP,
        "                    if found.size > MAX_FILE_BYTES:",
        "                    if found.size >= MAX_FILE_BYTES:",
    ),
    mutant(
        "a file that turns out too large is searched",
        GREP,
        "        elif len(content) > MAX_FILE_BYTES:",
        "        elif False:",
    ),
    mutant(
        "a file that does not look like text is searched",
        GREP,
        '        if b"\\x00" in content[:_SNIFF_BYTES]:',
        "        if False:",
    ),
    mutant(
        "a zero byte anywhere makes a file something else than text",
        GREP,
        '        if b"\\x00" in content[:_SNIFF_BYTES]:',
        '        if b"\\x00" in content:',
    ),
    mutant(
        "a file that does not look like text is read to its end",
        GREP,
        '                if b"\\x00" not in content:',
        "                if True:",
    ),
    mutant(
        "a file that is somewhere else by now fails the search",
        GREP,
        "        except (OSError, ToolFailure):",
        "        except OSError:",
    ),
    mutant(
        "reading goes on after the search was given up",
        GREP,
        "        if stopped.is_set() or size >= BATCH_BYTES:",
        "        if size >= BATCH_BYTES:",
    ),
    mutant(
        "a batch takes whatever the files hold",
        GREP,
        "        if stopped.is_set() or size >= BATCH_BYTES:",
        "        if stopped.is_set():",
    ),
    mutant(
        "what a batch holds is not counted",
        GREP,
        "            size += len(content)\n    return texts, unread, taken",
        "    return texts, unread, taken",
    ),
    mutant(
        "files that a batch left are forgotten",
        GREP,
        "            while waiting and not (search.cut or search.out_of_room):",
        "            if waiting:",
    ),
    mutant(
        "a search with enough lines goes on among the files that a batch left",
        GREP,
        "                    search.cut = search.cut or len(search.lines) >= MAX_MATCHES\n",
        "",
    ),
    mutant(
        "a search that read enough goes on among the files that a batch left",
        GREP,
        "                    search.out_of_room = search.searched_bytes >= MAX_BYTES\n",
        "",
    ),
    mutant(
        "a pattern is not looked at when there is nothing to search",
        GREP,
        "                if not search.asked:",
        "                if False:",
    ),
    mutant(
        "a search does not note that it has asked",
        GREP,
        "    search.asked = True\n",
        "",
    ),
    mutant(
        "the time of a search does not count while it finds and reads",
        GREP,
        "            async with asyncio.timeout_at(deadline):",
        "            async with asyncio.timeout_at(None):",
    ),
    mutant(
        "a search that ends does not tell its reading to stop",
        GREP,
        "            stopped.set()\n",
        "            pass\n",
    ),
    mutant(
        "the matching process is started with what lies in the working directory",
        GREP,
        '            [sys.executable, "-I", "-S", "-c", _PROGRAM],',
        '            [sys.executable, "-S", "-c", _PROGRAM],',
    ),
    mutant(
        "the matching process is started with what is installed",
        GREP,
        '            [sys.executable, "-I", "-S", "-c", _PROGRAM],',
        '            [sys.executable, "-I", "-c", _PROGRAM],',
    ),
    mutant(
        "every batch may show as many lines as the whole search",
        GREP,
        "            stdin=_request(args, MAX_MATCHES - len(search.lines), texts),",
        "            stdin=_request(args, MAX_MATCHES, texts),",
    ),
    mutant(
        "a pattern that cannot be used counts as a search that failed",
        GREP,
        "    if answers and answers[0].error is not None:",
        "    if False:",
    ),
    mutant(
        "a process that failed after it answered is believed",
        GREP,
        "        done.exit_code == 0\n",
        "        True\n",
    ),
    mutant(
        "an answer without its last line is believed",
        GREP,
        "        and answers[-1].done\n",
        "",
    ),
    mutant(
        "an answer about fewer files than were sent is believed",
        GREP,
        "        and (answers[-1].cut or answers[-1].files == len(texts))\n",
        "",
    ),
    mutant(
        "an answer about a file before the first is believed",
        GREP,
        "        and all(0 <= hit.id < len(texts) for hit in hits)",
        "        and all(hit.id < len(texts) for hit in hits)",
    ),
    mutant(
        "what the matching process left out is forgotten",
        GREP,
        "    search.cut = answers[-1].cut",
        "    search.cut = False",
    ),
    mutant(
        "the rest of a long line is not mentioned",
        GREP,
        '        more = f" [{hit.more} more characters]" if hit.more else ""',
        '        more = ""',
    ),
    mutant(
        "a search that shows nothing says nothing",
        GREP,
        "    if not lines:",
        "    if False:",
    ),
    mutant(
        "files that are too large are not mentioned",
        GREP,
        '    if large := search.unread["large"]:',
        "    if large := 0:",
    ),
    mutant(
        "files that do not look like text are not mentioned",
        GREP,
        '    if strange := search.unread["not text"]:',
        "    if strange := 0:",
    ),
    mutant(
        "files that could not be read are not mentioned",
        GREP,
        '    if unreadable := search.unread["unreadable"]:',
        "    if unreadable := 0:",
    ),
    mutant(
        "upper and lower case count whatever the call says",
        GREP_WORKER,
        '            pattern = re.compile(request["pattern"], re.IGNORECASE if request["ignore_case"] else 0)',
        '            pattern = re.compile(request["pattern"], 0)',
    ),
    mutant(
        "a warning about the pattern is let out",
        GREP_WORKER,
        '            warnings.simplefilter("ignore")\n',
        "",
    ),
    mutant(
        "input that ends in the middle of a file counts as whole",
        GREP_WORKER,
        "            return 1\n",
        "            break\n",
    ),
    mutant(
        "the matching process reads on after it has enough",
        GREP_WORKER,
        "    while not cut:",
        "    while True:",
    ),
    mutant(
        "what follows the last line break is a line",
        GREP_WORKER,
        "        while start < len(content) and not cut:",
        "        while start <= len(content) and not cut:",
    ),
    mutant(
        "the carriage return of a line break is shown",
        GREP_WORKER,
        '.decode("utf-8", errors="replace").removesuffix("\\r")',
        '.decode("utf-8", errors="replace")',
    ),
    mutant(
        "lines are counted from two",
        GREP_WORKER,
        "        number = 0\n",
        "        number = 1\n",
    ),
    mutant(
        "one line more than asked for is answered",
        GREP_WORKER,
        "            if left == 0:",
        "            if left < 0:",
    ),
    mutant(
        "more lines than were asked for go unsaid",
        GREP_WORKER,
        "                cut = True\n",
        "                pass\n",
    ),
    mutant(
        "a file that was not searched to its end counts as searched",
        GREP_WORKER,
        "        if not cut:\n            files += 1\n",
        "        files += 1\n",
    ),
    mutant(
        "a long line is answered whole",
        GREP_WORKER,
        '                        "text": line[:width],',
        '                        "text": line,',
    ),
    mutant(
        "what was cut off a long line is not counted",
        GREP_WORKER,
        '                        "more": max(0, len(line) - width),',
        '                        "more": 0,',
    ),
)

_APPROVAL = (
    mutant(
        "with the policy never somebody is asked all the same",
        APPROVAL,
        '        if self.policy(request.session) == "never":',
        "        if False:",
    ),
    mutant(
        "a listener that fails takes the caller down with it",
        APPROVAL,
        '            logger.error("a listener of %r failed", APPROVAL_ASK.name, exc_info=error)\n'
        '            return "unavailable"\n',
        "            raise\n",
    ),
    mutant(
        "nobody to ask counts as a yes",
        APPROVAL,
        '            # Nobody is there to answer.\n            return "unavailable"\n',
        '            return "allowed-once"\n',
    ),
    mutant(
        "an answer that is none of the four is passed on",
        APPROVAL,
        "        if not isinstance(answer, str) or answer not in _OUTCOMES:",
        "        if False:",
    ),
    mutant(
        "an answer that only says it is one of the four is passed on",
        APPROVAL,
        "        if not isinstance(answer, str) or answer not in _OUTCOMES:",
        "        if answer not in _OUTCOMES:",
    ),
    mutant(
        "a question is not written into the log",
        APPROVAL,
        "        session.append(\n"
        "            ApprovalAskedEvent(call_id=request.call_id, tool=request.tool, "
        "summary=request.summary)\n"
        "        )\n",
        "",
    ),
    mutant(
        "an answer is not written into the log",
        APPROVAL,
        "            session.append(ApprovalDecidedEvent(call_id=request.call_id, outcome=outcome))",
        "            pass",
    ),
    mutant(
        "a question that was cut short is written down as one that nobody could answer",
        APPROVAL,
        '        outcome: Outcome = "cancelled"',
        '        outcome: Outcome = "unavailable"',
    ),
    mutant(
        "a policy is written again though the log names it",
        APPROVAL,
        "        if _POLICY.of(session) != policy:",
        "        if True:",
    ),
    mutant(
        "the policy of the config counts though the log names another",
        APPROVAL,
        "        return _POLICY.of(session) or self._default",
        "        return self._default",
    ),
    mutant(
        "the policy in the log is not read back",
        APPROVAL,
        "    return data.policy if isinstance(data, ApprovalPolicyEvent) else policy",
        "    return policy",
    ),
    mutant(
        "a program without approval cannot read a log that names a policy",
        APPROVAL,
        '    event_type: ClassVar[str] = "approval/policy"\n    ignorable: ClassVar[bool] = True',
        '    event_type: ClassVar[str] = "approval/policy"\n    ignorable: ClassVar[bool] = False',
    ),
    mutant(
        "a program without approval cannot read a log that holds a question",
        APPROVAL,
        '    event_type: ClassVar[str] = "approval/asked"\n    ignorable: ClassVar[bool] = True',
        '    event_type: ClassVar[str] = "approval/asked"\n    ignorable: ClassVar[bool] = False',
    ),
    mutant(
        "a program without approval cannot read a log that holds an answer",
        APPROVAL,
        '    event_type: ClassVar[str] = "approval/decided"\n    ignorable: ClassVar[bool] = True',
        '    event_type: ClassVar[str] = "approval/decided"\n    ignorable: ClassVar[bool] = False',
    ),
)

_PERMISSIONS = (
    mutant(
        "a rule named after what tools do speaks about the tool of that name only",
        RULES,
        "    return wild(rule.tool, name) or (rule.tool == effect and effect in _CLASSES)",
        "    return wild(rule.tool, name)",
    ),
    mutant(
        "a rule named none speaks about every tool that does nothing",
        RULES,
        "    return wild(rule.tool, name) or (rule.tool == effect and effect in _CLASSES)",
        "    return wild(rule.tool, name) or rule.tool == effect",
    ),
    mutant(
        "a pattern for files does not speak about what lies below what it names",
        RULES,
        "    # What speaks about a directory speaks about all that is in it.\n"
        '    names = (*names, "**")\n',
        "",
    ),
    mutant(
        "a pattern of one name is looked for at the top only",
        RULES,
        '        start = "every depth"\n        names = ("**", *names)\n',
        '        start = "every depth"\n',
    ),
    mutant(
        "a name that is pinned to the top is looked for at every depth",
        RULES,
        '    elif len(names) == 1 and names != ("..",) and not written.startswith("./"):',
        '    elif len(names) == 1 and names != ("..",):',
    ),
    mutant(
        "a pattern for every depth does not speak about files outside",
        RULES,
        '    if start == "full path" or (start == "every depth" and not inside):',
        '    if start == "full path":',
    ),
    mutant(
        "a star takes a step out of the working directory",
        RULES,
        "    return _steps_back(parts) == back and sequence_fits("
        "parts[back:], relative[back:], wild)",
        "    return sequence_fits(parts, relative, wild)",
    ),
    mutant(
        "a star stands for nothing or for what follows at once",
        RULES,
        "        elif star >= 0:",
        "        elif False:",
    ),
    mutant(
        "a pattern that ends with a star needs a character for it",
        RULES,
        '    return all(letter == "*" for letter in pattern[at:])',
        "    return at == len(pattern)",
    ),
    mutant(
        "space around a command keeps a rule from speaking about it",
        RULES,
        "    return rule.pattern is None or wild(rule.pattern, command.strip())",
        "    return rule.pattern is None or wild(rule.pattern, command)",
    ),
    mutant(
        "space around a pattern is a part of the pattern",
        RULES,
        "        pattern = rest[:-1].strip()",
        "        pattern = rest[:-1]",
    ),
    mutant(
        "a file outside the working directory has no path from there",
        RULES,
        "        relative = Path(os.path.relpath(where.path, where.root)).as_posix()",
        "        relative = None",
    ),
    on_windows(
        mutant(
            "upper and lower case count in a pattern for files",
            RULES,
            "_folded = str.lower if IGNORES_CASE else str",
            "_folded = str",
        )
    ),
    mutant(
        "a character that cannot be seen is a letter like any other",
        COMMANDS,
        '        if letter not in " \\n\\t" and unseen(letter):\n',
        "        if False:\n",
    ),
    mutant(
        "a curved quotation mark is a letter for every shell",
        COMMANDS,
        "        if letter in quotes_of_other_kinds:",
        "        if False:",
    ),
    mutant(
        "between double quotes nothing is expanded",
        COMMANDS,
        "            if letter == '\"' and any("
        "inner in in_double_quotes for inner in text[at + 1 : end]):",
        "            if False:",
    ),
    mutant(
        "a joint with nothing before it is passed over",
        COMMANDS,
        '                elif joint != "\\n":',
        "                elif False:",
    ),
    mutant(
        "a joint with nothing after it is passed over",
        COMMANDS,
        '                owed = owed or joint in ("&&", "||", "|")',
        "                owed = False",
    ),
    mutant(
        "a command that is empty is a plain command",
        COMMANDS,
        "    if owed or not commands:",
        "    if owed:",
    ),
    mutant(
        "what a shell expands or redirects is a letter",
        COMMANDS,
        "        if letter in plain_breaks:",
        "        if False:",
    ),
    mutant(
        "the word that stops the reading of a line is a word",
        COMMANDS,
        '    if apart is None or any("--%" in simple.split(" ") for simple in apart):',
        "    if apart is None:",
    ),
    mutant(
        "a rule that refuses a part of a line lets the line through",
        PERMISSIONS,
        "    spoken_of = (command, *(apart or ()))",
        "    spoken_of = (command,)",
    ),
    mutant(
        "one allowed command allows the line that it stands in",
        PERMISSIONS,
        "        if all(rule is not None for rule in allowing):",
        "        if any(rule is not None for rule in allowing):",
    ),
    mutant(
        "a rule that allows a tool does not allow what cannot be taken apart",
        PERMISSIONS,
        "        if rule.pattern is None and covers_command(rule, tool, command):",
        "        if False:",
    ),
    mutant(
        "a command needs no rule in any session",
        PERMISSIONS,
        '    if mode == "danger-full-access":',
        "    if True:",
    ),
    mutant(
        "a command needs a rule in a session with full access too",
        PERMISSIONS,
        '    if mode == "danger-full-access":',
        "    if False:",
    ),
    mutant(
        "a project may allow",
        PERMISSIONS,
        "    return RuleSet(held.deny, held.ask)",
        "    return held",
    ),
    mutant(
        "the rules of a project are read anew for every call",
        PERMISSIONS,
        "        if session not in self._ready:",
        "        if True:",
    ),
    mutant(
        "rules of a project that lead out of it are read",
        PERMISSIONS,
        "    if not where.inside:\n        return _Unreadable(",
        "    if False:\n        return _Unreadable(",
    ),
    mutant(
        "rules of a project are read whatever their size",
        PERMISSIONS,
        "        if status.st_size > MAX_FILE_BYTES:",
        "        if False:",
    ),
    mutant(
        "the rules of the user are not read",
        PERMISSIONS,
        "        written + _user_rules(config.user_file),",
        "        written,",
    ),
    mutant(
        "a search opens a file that somebody is to be asked about",
        PERMISSIONS,
        "        if not access.named:",
        "        if False:",
    ),
    mutant(
        "a file that somebody was asked about is refused where it is opened",
        PERMISSIONS,
        "        if not access.named:",
        "        if True:",
    ),
    mutant(
        "a call that somebody is to be asked about runs unasked",
        PERMISSIONS,
        '        if decided.verdict == "allow":',
        '        if decided.verdict != "deny":',
    ),
    mutant(
        "a no counts as a yes",
        PERMISSIONS,
        '        if outcome == "allowed-once":',
        '        if outcome != "unavailable":',
    ),
    mutant(
        "the preset is written over the mode that the log names",
        PERMISSIONS,
        "        if self._sandbox.recorded(session) is None:",
        "        if True:",
    ),
    mutant(
        "the preset is written over the policy that the log names",
        PERMISSIONS,
        "        if self._approval.recorded(session) is None:",
        "        if True:",
    ),
    mutant(
        "every event of the log writes the preset",
        PERMISSIONS,
        "        if isinstance(committed.event.data, TurnStartEvent):",
        "        if True:",
    ),
    mutant(
        "a question shows a command with its line breaks",
        PERMISSIONS,
        '    line = " ".join(target.split())',
        "    line = target",
    ),
    mutant(
        "a question shows a long command whole in its one line",
        PERMISSIONS,
        "    if len(line) > _SUMMARY_CHARACTERS:",
        "    if False:",
    ),
    mutant(
        "a question does not hand over what the call acts on",
        PERMISSIONS,
        "                target=invocation.target,\n",
        "",
    ),
    mutant(
        "a session where nobody is asked is told that somebody said no",
        PERMISSIONS,
        '                if self._approval.policy(session) == "never"',
        "                if False",
    ),
)

_SHELL = (
    mutant(
        "a command may run as long as the call says",
        SHELL,
        "        seconds = min(args.timeout_seconds or DEFAULT_SECONDS, MAX_SECONDS)",
        "        seconds = args.timeout_seconds or DEFAULT_SECONDS",
    ),
    mutant(
        "the length of a command is counted in letters",
        SHELL,
        '        if len(args.command.encode("utf-8")) > MAX_COMMAND_BYTES:',
        "        if len(args.command) > MAX_COMMAND_BYTES:",
    ),
    mutant(
        "the tool runs its command and not what the sandbox handed back",
        SHELL,
        "                confined.argv,\n",
        "                argv,\n",
    ),
    mutant(
        "a command is given the environment with its secrets",
        SHELL,
        "                env=_environment(),",
        "                env=dict(os.environ),",
    ),
    mutant(
        "the exit code is put behind the last character of the output",
        SHELL,
        '        if output and not output.endswith("\\n"):',
        "        if False:",
    ),
    mutant(
        "a command that was stopped is said to have ended",
        SHELL,
        "        if done.timed_out:",
        "        if False:",
    ),
    mutant(
        "the call is stopped at the moment its command would be",
        SHELL,
        "        timeout_seconds=MAX_SECONDS + _GRACE_SECONDS,",
        "        timeout_seconds=MAX_SECONDS,",
    ),
    mutant(
        "a command counts as something that does nothing to the world",
        SHELL,
        '        effect="run",',
        '        effect="none",',
    ),
    mutant(
        "the permissions are not told how the commands of the tool are taken apart",
        SHELL,
        "    ctx.use(PERMISSIONS).splitters.add(ctx, NAME, _SPLITTER)\n",
        "",
    ),
    on_windows(
        mutant(
            "the output keeps the line ends of Windows",
            SHELL,
            '        output = done.output.replace("\\r\\n", "\\n")',
            "        output = done.output",
        )
    ),
    on_windows(
        mutant(
            "python is left to write in the code page of the system",
            SHELL,
            '        cleaned.setdefault("PYTHONIOENCODING", "utf-8")\n',
            "",
        )
    ),
    on_windows(
        mutant(
            "what ran last does not decide the exit code",
            SHELL,
            '        return argv, (command + _AFTER).encode("utf-8")',
            '        return argv, command.encode("utf-8")',
        )
    ),
    on_windows(
        mutant(
            "powershell is found wherever Windows finds a program of that name",
            SHELL,
            "        return str(found) if found.is_file() else None",
            '        return "powershell.exe"',
        )
    ),
)

_PROMPT = (
    mutant(
        "sections come by their names, whatever their numbers",
        PROMPT,
        "key=lambda item: (item[1].order, item[0])",
        "key=lambda item: item[0]",
    ),
    mutant(
        "sections come in the order in which they were added",
        PROMPT,
        "        ordered = tuple(sorted(self.sections.items(), "
        "key=lambda item: (item[1].order, item[0])))",
        "        ordered = tuple(self.sections.items())",
    ),
    mutant(
        "a section without text leaves a gap",
        PROMPT,
        '        text = "\\n\\n".join(part for part in parts if part)',
        '        text = "\\n\\n".join(parts)',
    ),
    mutant(
        "the text of the spec is left out",
        PROMPT,
        "        parts = [base.strip(), *(section.text.strip() for _, section in ordered)]",
        "        parts = [section.text.strip() for _, section in ordered]",
    ),
    mutant(
        "an answer of a listener that is no text is used",
        PROMPT,
        "        if not isinstance(answer, str):",
        "        if False:",
    ),
    mutant(
        "the instructions are asked for at every step",
        LOOP,
        "            if step == 0:\n                try:\n"
        "                    await self._reconcile_system_message()",
        "            if True:\n                try:\n"
        "                    await self._reconcile_system_message()",
    ),
    mutant(
        "the loop does not ask the system prompt service",
        LOOP,
        "        if assembling is not None:",
        "        if False:",
    ),
    mutant(
        "a turn whose instructions failed leaves its input in the inbox",
        LOOP,
        "                    opening = error\n",
        "                    raise\n",
    ),
)

_CONTEXT = (
    mutant(
        "sources of context are asked in the order in which they were added",
        LOOP,
        "        ordered = sorted(self._context.items(), key=lambda item: (item[1].order, item[0]))",
        "        ordered = list(self._context.items())",
    ),
    mutant(
        "what a source gives is taken for text whatever it is",
        LOOP,
        "            if not isinstance(text, str):\n"
        '                raise TypeError(f"{what} gave {text!r}, which is no text")\n',
        "",
    ),
    mutant(
        "a note is written only when its text is new",
        LOOP,
        '            if source.kind == "note":',
        "            if False:",
    ),
    mutant(
        "text that stays the same is written again",
        LOOP,
        "            elif last is None or last.data != written:",
        "            elif True:",
    ),
    mutant(
        "text of a source that replaces is put after the conversation",
        LOOP,
        '                if source.kind == "replace" and last is not None '
        "and last.seq in session.surface:",
        "                if False:",
    ),
    mutant(
        "text of a source that appends takes the place of its old text",
        LOOP,
        '                if source.kind == "replace" and last is not None '
        "and last.seq in session.surface:",
        "                if last is not None and last.seq in session.surface:",
    ),
    mutant(
        "text that the model no longer sees is replaced all the same",
        LOOP,
        '                if source.kind == "replace" and last is not None '
        "and last.seq in session.surface:",
        '                if source.kind == "replace" and last is not None:',
    ),
    mutant(
        "a closing tag in a text ends the frame around it",
        LOOP,
        '    body = text.strip().replace("</context", "<\\\\/context")',
        "    body = text.strip()",
    ),
    mutant(
        "every end of a turn is told of",
        AGENT_CONTEXT,
        '    if ended is None or ended.reason not in ("aborted", "interrupted"):',
        "    if ended is None:",
    ),
    mutant(
        "a turn that was cancelled is not told of",
        AGENT_CONTEXT,
        '    if ended is None or ended.reason not in ("aborted", "interrupted"):',
        '    if ended is None or ended.reason != "interrupted":',
    ),
    mutant(
        "a second interruption in a row is not told of",
        AGENT_CONTEXT,
        'INTERRUPTION = ContextSource(_interruption, order=200, kind="note")',
        'INTERRUPTION = ContextSource(_interruption, order=200, kind="append")',
    ),
    mutant(
        "the agent service does not register the note after an interruption",
        AGENT_PACKAGE,
        '    agents.context.add(ctx, "interruption", INTERRUPTION)\n',
        "",
    ),
)

_FACTS = (
    mutant(
        "facts come in the order in which they were added",
        RUNTIME,
        "for name, value in sorted(facts.lines.items()))",
        "for name, value in facts.lines.items())",
    ),
    mutant(
        "the working directory is not among the facts",
        RUNTIME,
        '    facts = Facts(agent, {"working directory": str(agent.cwd)})',
        "    facts = Facts(agent)",
    ),
    mutant(
        "no plugin is asked for its facts",
        RUNTIME,
        "    agent.ctx.emit(RUNTIME_FACTS, facts)\n",
        "",
    ),
    mutant(
        "facts that have not changed are told again",
        RUNTIME,
        'ContextSource(_runtime, order=100, kind="append")',
        'ContextSource(_runtime, order=100, kind="note")',
    ),
    mutant(
        "the mode of the sandbox is not told",
        SANDBOX,
        '        gathered.lines["sandbox mode"] = _MODES[sandbox.mode(gathered.agent.session)]\n',
        "",
    ),
    mutant(
        "the mode of the config is told whatever the mode of the session",
        SANDBOX,
        '        gathered.lines["sandbox mode"] = _MODES[sandbox.mode(gathered.agent.session)]',
        '        gathered.lines["sandbox mode"] = _MODES[config.mode]',
    ),
    mutant(
        "how well commands are held is not told",
        SANDBOX,
        '        gathered.lines["commands are held"] = _STRENGTHS[sandbox.strength()]\n',
        "",
    ),
    mutant(
        "the policy of approval is not told",
        APPROVAL,
        '        gathered.lines["approval"] = _POLICIES[approval.policy(gathered.agent.session)]\n',
        "        pass\n",
    ),
    mutant(
        "what runs commands is not told",
        SHELL,
        '        gathered.lines["commands are run by"] = _FACT\n',
        "        pass\n",
    ),
)

_INSTRUCTIONS = (
    mutant(
        "the most specific file comes first",
        INSTRUCTIONS,
        "    parts = [_OPENING, *reversed(kept)] if kept else []",
        "    parts = [_OPENING, *kept] if kept else []",
    ),
    mutant(
        "the opening stands there when no file is given",
        INSTRUCTIONS,
        "    parts = [_OPENING, *reversed(kept)] if kept else []",
        "    parts = [_OPENING, *reversed(kept)]",
    ),
    mutant(
        "the directories above the project are looked at too",
        INSTRUCTIONS,
        "            return list(reversed(up[: steps + 1]))",
        "            return list(reversed(up))",
    ),
    mutant(
        "without a repository every directory above is looked at",
        INSTRUCTIONS,
        "    return [here]",
        "    return list(reversed(up))",
    ),
    mutant(
        "the repository that is furthest away is the project",
        INSTRUCTIONS,
        "    for steps, above in enumerate(up):",
        "    for steps, above in reversed(list(enumerate(up))):",
    ),
    mutant(
        "what marks a repository is followed to see whether it is there",
        INSTRUCTIONS,
        'if os.path.lexists(above / ".git"):',
        'if os.path.exists(above / ".git"):',
    ),
    mutant(
        "a file is called by its name alone wherever it lies",
        INSTRUCTIONS,
        '            label = "/".join([*[".."] * steps_up, file_name])',
        "            label = file_name",
    ),
    mutant(
        "a file of the same content is given twice",
        INSTRUCTIONS,
        "if content.removeprefix(codecs.BOM_UTF8).strip() and content not in known:",
        "if content.removeprefix(codecs.BOM_UTF8).strip():",
    ),
    mutant(
        "a file that holds a mark and nothing else is given",
        INSTRUCTIONS,
        "if content.removeprefix(codecs.BOM_UTF8).strip() and content not in known:",
        "if content.strip() and content not in known:",
    ),
    mutant(
        "a link is opened like a file",
        INSTRUCTIONS,
        "                if link is not None:",
        "                if False:",
    ),
    mutant(
        "nothing is said about a link that was not read",
        INSTRUCTIONS,
        "                    if _named_by(directory, link) not in looked_for:",
        "                    if False:",
    ),
    mutant(
        "a link to another instruction file is told like any other",
        INSTRUCTIONS,
        "                    if _named_by(directory, link) not in looked_for:",
        "                    if True:",
    ),
    mutant(
        "a link is told from a file by what it leads to",
        BOUND,
        "    return path.readlink() if _is_link(path.lstat()) else None",
        "    return path.readlink() if _is_link(path.stat()) else None",
    ),
    mutant(
        "a file that cannot be read is passed over",
        INSTRUCTIONS,
        "            except (FileNotFoundError, bound.NotAFileError):",
        "            except OSError:",
    ),
    mutant(
        "a file of the user that cannot be read is passed over",
        INSTRUCTIONS,
        "        except FileNotFoundError:",
        "        except OSError:",
    ),
    mutant(
        "a file is read to its end however long it is",
        INSTRUCTIONS,
        "                    take(label, file.read(config.max_bytes + 1))",
        "                    take(label, file.read())",
    ),
    mutant(
        "a file that just fits is cut",
        INSTRUCTIONS,
        "        fits = len(file.content) <= room",
        "        fits = len(file.content) < room",
    ),
    mutant(
        "a wider file that does not fit is cut and not left out",
        INSTRUCTIONS,
        "        if left_out or (kept and not fits):",
        "        if left_out:",
    ),
    mutant(
        "a small file is given although a more specific one was left out",
        INSTRUCTIONS,
        "        if left_out or (kept and not fits):",
        "        if kept and not fits:",
    ),
    mutant(
        "nothing is said about the files that were left out",
        INSTRUCTIONS,
        "    if left_out:\n        notes.append(",
        "    if False:\n        notes.append(",
    ),
    mutant(
        "the files that were left out are named the most specific first",
        INSTRUCTIONS,
        "bytes: {', '.join(reversed(left_out))}]",
        "bytes: {', '.join(left_out)}]",
    ),
    mutant(
        "nothing is said about a file that was cut",
        INSTRUCTIONS,
        "        if not fits:\n            text +=",
        "        if False:\n            text +=",
    ),
    mutant(
        "a character that the cut goes through shows as a mark",
        INSTRUCTIONS,
        'errors="replace" if fits else "ignore")',
        'errors="replace")',
    ),
    mutant(
        "lines keep the ending of the system that the file was written on",
        INSTRUCTIONS,
        '        text = text.replace("\\r\\n", "\\n").strip()',
        "        text = text.strip()",
    ),
    mutant(
        "a name with a slash in it is taken for the name of a file",
        INSTRUCTIONS,
        '            plain = file_name.strip() and not set(file_name) & {"/", "\\\\", chr(0)}',
        "            plain = file_name.strip()",
    ),
    mutant(
        "an empty name is taken for the name of a file",
        INSTRUCTIONS,
        '            if not plain or file_name in (".", "..") or not usable(file_name):',
        '            if file_name in (".", "..") or not usable(file_name or "x"):',
    ),
    mutant(
        "the model is not told that the files are gone",
        INSTRUCTIONS,
        "        if said is None and _GIVEN.of(agent.session):",
        "        if False:",
    ),
    mutant(
        "a session that never had instructions is told that they are gone",
        INSTRUCTIONS,
        "        if said is None and _GIVEN.of(agent.session):",
        "        if said is None:",
    ),
    mutant(
        "instructions that have not changed are given again in every turn",
        INSTRUCTIONS,
        'ContextSource(render, order=0, kind="replace")',
        'ContextSource(render, order=0, kind="note")',
    ),
    mutant(
        "new instructions follow the conversation and the old ones stay",
        INSTRUCTIONS,
        'ContextSource(render, order=0, kind="replace")',
        'ContextSource(render, order=0, kind="append")',
    ),
    mutant(
        "the instructions come after the facts about the run",
        INSTRUCTIONS,
        'ContextSource(render, order=0, kind="replace")',
        'ContextSource(render, order=500, kind="replace")',
    ),
)

_BASE = (
    mutant(
        "the preset of the command line is not handed on",
        LAUNCHER,
        '                config["preset"] = args.permission',
        "                pass",
    ),
    mutant(
        "permissions that were given no preset lose the one they have",
        LAUNCHER,
        "            if args.permission is not None:",
        "            if True:",
    ),
    mutant(
        "the rules of the command line take the place of those of the profile",
        LAUNCHER,
        '                config["allow"] = [*cast("list[object]", rules), *args.allow]',
        '                config["allow"] = [*args.allow]',
    ),
    mutant(
        "the rules of the command line come before those of the profile",
        LAUNCHER,
        '                config["allow"] = [*cast("list[object]", rules), *args.allow]',
        '                config["allow"] = [*args.allow, *cast("list[object]", rules)]',
    ),
    mutant(
        "permissions get an empty list of rules when the command line gives none",
        LAUNCHER,
        "            if args.allow:",
        "            if True:",
    ),
    mutant(
        "a preset is handed to permissions that are switched off",
        LAUNCHER,
        "        if entry.id == _PERMISSIONS and not entry.disabled:",
        "        if entry.id == _PERMISSIONS:",
    ),
    mutant(
        "a preset with nowhere to go is dropped without a word",
        LAUNCHER,
        '    raise _UsageError(\n        f"the profile has no entry {_PERMISSIONS!r}, or it is',
        '    return\n    raise _UsageError(\n        f"the profile has no entry {_PERMISSIONS!r}, or it is',
    ),
    mutant(
        "rules under allow that are no list are taken apart letter by letter",
        LAUNCHER,
        "                if not isinstance(rules, list):",
        "                if False:",
    ),
    mutant(
        "a config that is no mapping is taken for an empty one",
        LAUNCHER,
        "    if not isinstance(config, dict):",
        "    if False:",
    ),
    mutant(
        "a session of the base program may write anywhere",
        BASE_BUNDLE,
        "          preset: workspace-write",
        "          preset: full-access",
    ),
    mutant(
        "the rules of the user are not read",
        BASE_BUNDLE,
        "          user_file: ${COLUMBINA_HOME}/permissions.yml",
        "          deny: []",
    ),
    mutant(
        "the instructions of the user are not read",
        BASE_BUNDLE,
        "          user_file: ${COLUMBINA_HOME}/AGENTS.md",
        "          max_bytes: 65536",
    ),
    mutant(
        "the base program has no tool that runs commands",
        BASE_BUNDLE,
        "        plugin: columbina.builtin_tools.shell",
        "        plugin: columbina.builtin_tools.shell\n        disabled: true",
    ),
    mutant(
        "the base program has no tool that searches in files",
        BASE_BUNDLE,
        "        plugin: columbina.builtin_tools.grep",
        "        plugin: columbina.builtin_tools.grep\n        disabled: true",
    ),
    mutant(
        "the model is not told where it runs",
        BASE_BUNDLE,
        "        plugin: columbina.context.runtime",
        "        plugin: columbina.context.runtime\n        disabled: true",
    ),
    mutant(
        "a request that fails for a passing reason is not made again",
        BASE_BUNDLE,
        "      - id: retry\n        plugin: columbina.retry",
        "      - id: retry\n        plugin: columbina.retry\n        disabled: true",
    ),
    mutant(
        "plugins have nowhere to add to the system prompt",
        BASE_BUNDLE,
        "        plugin: columbina.system_prompt",
        "        plugin: columbina.system_prompt\n        disabled: true",
    ),
)

_TRANSCRIPT = (
    mutant(
        "a carriage return before a line feed stays in the line",
        CLI_LINES,
        '        return [line.removesuffix("\\r") for line in complete]\n',
        "        return list(complete)\n",
    ),
    mutant(
        "the line that is not complete is dropped when the text ends",
        CLI_LINES,
        "        return line or None\n",
        "        return None\n",
    ),
    mutant(
        "the line that is not complete is handed over and kept",
        CLI_LINES,
        '        line, self.partial = self.partial.removesuffix("\\r"), ""\n',
        '        line = self.partial.removesuffix("\\r")\n',
    ),
    mutant(
        "a carriage return that waited for its line feed is handed over at the end",
        CLI_LINES,
        '        line, self.partial = self.partial.removesuffix("\\r"), ""\n',
        '        line, self.partial = self.partial, ""\n',
    ),
    mutant(
        "a reply that was shown as it arrived is shown again",
        CLI_TRANSCRIPT,
        "            if not self._shown:\n                shown.extend(",
        "            if True:\n                shown.extend(",
    ),
    mutant(
        "a reply that did not arrive in pieces is not shown",
        CLI_TRANSCRIPT,
        "            if not self._shown:\n                shown.extend(",
        "            if False:\n                shown.extend(",
    ),
    mutant(
        "after a reply that arrived in pieces the next one is taken for shown too",
        CLI_TRANSCRIPT,
        "            self._shown = False\n            self._word = None\n            return shown\n",
        "            self._word = None\n            return shown\n",
    ),
    mutant(
        "the line of a reply that is not complete is not closed when the reply is written down",
        CLI_TRANSCRIPT,
        "            shown = self._close()\n            if not self._shown:",
        "            shown: list[Text] = []\n            if not self._shown:",
    ),
    mutant(
        "the line that is not complete is not closed when its block ends",
        CLI_TRANSCRIPT,
        "            self._word = None\n            return self._close()\n",
        "            self._word = None\n            return []\n",
    ),
    mutant(
        "what the user gave as a task is shown again",
        CLI_TRANSCRIPT,
        '            if data.source != "steer":\n                return []\n',
        '            if data.source not in ("steer", "user"):\n                return []\n',
    ),
    mutant(
        "what the user said on the way is not shown when the model reads it",
        CLI_TRANSCRIPT,
        '            return [fit(f"{self._symbols.steer} {line}") for line in _said(data).splitlines()]\n',
        "            return []\n",
    ),
    mutant(
        "the end of a turn leaves no empty line",
        CLI_TRANSCRIPT,
        "            return [*_end(data), Text()]\n",
        "            return _end(data)\n",
    ),
    mutant(
        "the time that somebody took to answer counts as time of the call",
        CLI_TRANSCRIPT,
        "            self._began[data.call_id] = event.time\n        return []\n",
        "            pass\n        return []\n",
    ),
    mutant(
        "a request that is made again is not told",
        CLI_TRANSCRIPT,
        "        if isinstance(data, RetryEvent):\n",
        "        if isinstance(data, RetryEvent) and not data.code:\n",
    ),
    mutant(
        "a turn that somebody else stopped does not say who",
        CLI_TRANSCRIPT,
        '        told = "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"\n',
        '        told = "(stopped)"\n',
    ),
    mutant(
        "a turn that the user stopped names the user",
        CLI_TRANSCRIPT,
        '        told = "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"\n',
        '        told = "(stopped)" if detail is None else f"(stopped: {detail})"\n',
    ),
    mutant(
        "a turn that ended with an error says nothing",
        CLI_TRANSCRIPT,
        '    if data.reason == "error":\n',
        '    if data.reason == "no error":\n',
    ),
    mutant(
        "a reply that was cut off says nothing",
        CLI_TRANSCRIPT,
        '    if data.reason == "max-tokens":\n',
        '    if data.reason == "no limit":\n',
    ),
    mutant(
        "a turn that was not allowed to go on says nothing",
        CLI_TRANSCRIPT,
        '    if data.reason == "blocked":\n',
        '    if data.reason == "not blocked":\n',
    ),
    mutant(
        "a turn that the end of the program cut short says nothing",
        CLI_TRANSCRIPT,
        '    if data.reason == "interrupted":\n',
        '    if data.reason == "not interrupted":\n',
    ),
    mutant(
        "a call is told by all of its arguments",
        CLI_TRANSCRIPT,
        "    for named in _NAMED_FIRST:\n",
        "    for named in ():\n",
    ),
    mutant(
        "a call with one text among its arguments is told by all of them",
        CLI_TRANSCRIPT,
        "    if len(texts) == 1:\n        return texts[0]\n",
        "    if len(texts) == 0:\n        return texts[0]\n",
    ),
    mutant(
        "what a call acts on is shown in full on its line",
        CLI_TRANSCRIPT,
        "    if len(joined) > _SUMMARY_CHARACTERS:\n",
        "    if len(joined) > 100 * _SUMMARY_CHARACTERS:\n",
    ),
    mutant(
        "what a call acts on keeps its line breaks on the line of the call",
        CLI_TRANSCRIPT,
        '    joined = " ".join(text.split())\n',
        "    joined = text\n",
    ),
    mutant(
        "arguments that cannot be read are not shown",
        CLI_TRANSCRIPT,
        "        told = _one_line(data.arguments if arguments is None else _acted_on(arguments))\n",
        '        told = _one_line("" if arguments is None else _acted_on(arguments))\n',
    ),
    mutant(
        "a call that changes text does not show what goes and what comes",
        CLI_TRANSCRIPT,
        "        if isinstance(old, str) and isinstance(new, str):\n",
        "        if isinstance(old, int) and isinstance(new, str):\n",
    ),
    mutant(
        "a long change is shown whole",
        CLI_TRANSCRIPT,
        "_CHANGED_LINES = 6\n",
        "_CHANGED_LINES = 600\n",
    ),
    mutant(
        "a call that writes a file does not say how much",
        CLI_TRANSCRIPT,
        "        if isinstance(content, str):\n",
        "        if isinstance(content, int):\n",
    ),
    mutant(
        "a result does not say how long the call took",
        CLI_TRANSCRIPT,
        "        took = [] if began is None else [_took(time - began)]\n",
        "        took: list[str] = []\n",
    ),
    mutant(
        "the time of a call is counted from the first call there was",
        CLI_TRANSCRIPT,
        "        began = self._began.pop(call_id, None)\n",
        "        began = min(self._began.values(), default=None)\n",
    ),
    mutant(
        "the time of a call is always told in milliseconds",
        CLI_TRANSCRIPT,
        "    if milliseconds < 1000:\n",
        "    if milliseconds < 1000 or True:\n",
    ),
    mutant(
        "a minute is told in seconds",
        CLI_TRANSCRIPT,
        "    if milliseconds < 60_000:\n",
        "    if milliseconds < 60_000_000:\n",
    ),
    mutant(
        "the output of a command is shown whole",
        CLI_TRANSCRIPT,
        "_OUTPUT_LINES = 5\n",
        "_OUTPUT_LINES = 500\n",
    ),
    mutant(
        "what any tool gave back is shown, not only the output of a command",
        CLI_TRANSCRIPT,
        "        if not command:\n            return [head]\n",
        "        if not lines:\n            return [head]\n",
    ),
    mutant(
        "the output of a command is counted in lines like what a file holds",
        CLI_TRANSCRIPT,
        "        if lines and not command:\n",
        "        if lines:\n",
    ),
    mutant(
        "a call that failed does not show its code",
        CLI_TRANSCRIPT,
        '            failed = ", ".join([f"{self._symbols.failed} {data.error.code}", *took])\n',
        '            failed = ", ".join([f"{self._symbols.failed}", *took])\n',
    ),
    mutant(
        "a call that failed shows everything that was said about it",
        CLI_TRANSCRIPT,
        "_ERROR_LINES = 3\n",
        "_ERROR_LINES = 300\n",
    ),
    mutant(
        "a result is told as one of a command whatever the call was",
        CLI_TRANSCRIPT,
        "        command = call_id in self._commands\n",
        "        command = bool(self._commands)\n",
    ),
    mutant(
        "the rest of what is shown by its beginning is not counted",
        CLI_TRANSCRIPT,
        "    if len(lines) > at_most:\n        rest = ",
        "    if len(lines) > 100 * at_most:\n        rest = ",
    ),
    mutant(
        "what came before the end that is shown is not counted",
        CLI_TRANSCRIPT,
        "    if len(lines) > at_most:\n        earlier = ",
        "    if len(lines) > 100 * at_most:\n        earlier = ",
    ),
    mutant(
        "the output of a command is shown by its beginning and not by its end",
        CLI_TRANSCRIPT,
        '        return [head, *_ending(lines, _OUTPUT_LINES, "    ")]\n',
        '        return [head, *_beginning(lines, _OUTPUT_LINES, "    ")]\n',
    ),
    mutant(
        "what is shown by its end is shown from its beginning on",
        CLI_TRANSCRIPT,
        '    shown.extend(fit(f"{before}{line}") for line in lines[-at_most:])\n',
        '    shown.extend(fit(f"{before}{line}") for line in lines)\n',
    ),
    mutant(
        "one line more is told as lines",
        CLI_TRANSCRIPT,
        '    return f"{number} {what}" if number == 1 else f"{number} {what}s"\n',
        '    return f"{number} {what}s"\n',
    ),
    mutant(
        "no word says that the model thinks",
        CLI_TRANSCRIPT,
        '                self._word = "thinking..."\n',
        "                self._word = None\n",
    ),
    mutant(
        "no word says that the model prepares a call",
        CLI_TRANSCRIPT,
        '                self._word = f"preparing a call of {chunk.name}..."\n',
        "                self._word = None\n",
    ),
    mutant(
        "the word about what the model does stays when text arrives",
        CLI_TRANSCRIPT,
        "            self._word = None\n            self._shown = True\n",
        "            self._shown = True\n",
    ),
    mutant(
        "the word about what the model does stands before the line that is not complete",
        CLI_TRANSCRIPT,
        "        if self._lines.partial:\n            return fit(self._lines.partial)\n",
        "        if self._lines.partial and self._word is None:\n            return fit(self._lines.partial)\n",
    ),
    mutant(
        "a character that the library drops goes missing from a line",
        CLI_TRANSCRIPT,
        "    return Text(harmless(written), style=style)\n",
        "    return Text(written, style=style)\n",
    ),
    mutant(
        "a character that the library drops goes missing from the name of a call",
        CLI_TRANSCRIPT,
        "        line.append(harmless(data.name), style=_NAME)\n",
        "        line.append(data.name, style=_NAME)\n",
    ),
    mutant(
        "a character that the library drops goes missing from what a call acts on",
        CLI_TRANSCRIPT,
        '            line.append(harmless(f" {told}"))\n',
        '            line.append(f" {told}")\n',
    ),
    mutant(
        "a character that the library drops goes missing from the code of a failure",
        CLI_TRANSCRIPT,
        "            head.append(harmless(failed), style=_BAD)\n",
        "            head.append(failed, style=_BAD)\n",
    ),
    mutant(
        "a long line is broken by the program and not by the terminal",
        CLI_TRANSCRIPT,
        "        console.print(_shown_safely(line), soft_wrap=True)\n",
        "        console.print(_shown_safely(line), soft_wrap=False)\n",
    ),
    mutant(
        "what a model or a tool wrote goes to the terminal as it came",
        CLI_TRANSCRIPT,
        "        console.print(_shown_safely(line), soft_wrap=True)\n",
        "        console.print(line, soft_wrap=True)\n",
    ),
    mutant(
        "a line loses its colours where something in it is replaced",
        CLI_TRANSCRIPT,
        "    shown = line.copy()\n",
        "    shown = Text()\n",
    ),
    mutant(
        "colours are sent although none are wanted",
        CLI_TRANSCRIPT,
        "        no_color=not colours,\n",
        "        no_color=False,\n",
        ("        force_terminal=colours,\n", "        force_terminal=True,\n"),
        (
            '        color_system="standard" if colours else None,\n',
            '        color_system="standard",\n',
        ),
    ),
    mutant(
        "the marks are not chosen by what the screen can show",
        CLI_TRANSCRIPT,
        '            call=terminal.symbol(wanted.call, "*"),\n',
        "            call=wanted.call,\n",
    ),
    mutant(
        "a session that is taken up again shows all of its turns",
        CLI_TRANSCRIPT,
        "    for event in events[starts[-1] :]:\n",
        "    for event in events[starts[0] :]:\n",
    ),
    mutant(
        "a session that is taken up again shows the whole of its last answer",
        CLI_TRANSCRIPT,
        "_RECAP_LINES = 12\n",
        "_RECAP_LINES = 1200\n",
    ),
    mutant(
        "a session that is taken up again shows what plugins told the model",
        CLI_TRANSCRIPT,
        '        if isinstance(data, UserMessageEvent) and data.source in ("user", "steer"):\n',
        "        if isinstance(data, UserMessageEvent):\n",
    ),
    mutant(
        "a session that is taken up again marks what was said on the way like a task",
        CLI_TRANSCRIPT,
        '            mark = ">" if data.source == "user" else symbols.steer\n',
        '            mark = ">"\n',
    ),
    mutant(
        "a session that is taken up again does not say how its last turn ended",
        CLI_TRANSCRIPT,
        "            ended = _end(data)\n",
        "            ended = []\n",
    ),
    mutant(
        "a session that is taken up again shows an answer of nothing but spaces",
        CLI_TRANSCRIPT,
        "        elif isinstance(data, AssistantMessageEvent) and data.message.text.strip():\n",
        "        elif isinstance(data, AssistantMessageEvent):\n",
    ),
    mutant(
        "a session that is taken up again leaves no empty line",
        CLI_TRANSCRIPT,
        "    return [*shown, *_ending(answer, _RECAP_LINES), *ended, Text()]\n",
        "    return [*shown, *_ending(answer, _RECAP_LINES), *ended]\n",
    ),
    mutant(
        "a session without a turn says that it is taken up again",
        CLI_TRANSCRIPT,
        "    if not starts:\n        return []\n",
        "    if not starts:\n        starts = [0]\n",
    ),
)

_TERMINAL = (
    mutant(
        "escape stops nothing",
        CLI_PROMPT,
        "        def stop(event: KeyPressEvent) -> None:\n            self._cancel()\n",
        "        def stop(event: KeyPressEvent) -> None:\n            pass\n",
    ),
    mutant(
        "escape stops what is not running",
        CLI_PROMPT,
        "        running = Condition(lambda: self.busy and self._answer is None)\n",
        "        running = Condition(lambda: self._answer is None)\n",
    ),
    mutant(
        "escape waits five seconds before it counts",
        CLI_PROMPT,
        "        self._session.app.ttimeoutlen = _ESCAPE_SECONDS\n",
        "        self._session.app.ttimeoutlen = 50 * _ESCAPE_SECONDS\n",
        (
            "        self._session.app.timeoutlen = _ESCAPE_SECONDS\n",
            "        self._session.app.timeoutlen = 50 * _ESCAPE_SECONDS\n",
        ),
    ),
    mutant(
        "ctrl+c does not stop what is running",
        CLI_PROMPT,
        "            elif self.busy:\n                self._cancel()\n",
        "            elif self.busy:\n                pass\n",
    ),
    mutant(
        "ctrl+c does not clear the line",
        CLI_PROMPT,
        "            elif event.current_buffer.text:\n                event.current_buffer.reset()\n",
        "            elif event.current_buffer.text:\n                pass\n",
    ),
    mutant(
        "the first ctrl+c on an empty line leaves",
        CLI_PROMPT,
        "            elif self._leaving:\n",
        "            elif not self._leaving:\n",
    ),
    mutant(
        "the second ctrl+c does not leave",
        CLI_PROMPT,
        "            elif self._leaving:\n                event.app.exit(result=None)\n",
        "            elif self._leaving:\n                pass\n",
    ),
    mutant(
        "ctrl+c on an empty line does not say how to leave",
        CLI_PROMPT,
        '                self.show([Text("Press Ctrl+C again to leave.", style="dim")])\n',
        "                pass\n",
    ),
    mutant(
        "typing does not take back the wish to leave",
        CLI_PROMPT,
        "        # Somebody who types has not made up their mind to leave.\n        self._leaving = False\n",
        "        # Somebody who types has not made up their mind to leave.\n        pass\n",
    ),
    mutant(
        "a new line does not take back the wish to leave",
        CLI_PROMPT,
        "        self._leaving = False\n        try:\n            return await self._session.prompt_async()\n",
        "        try:\n            return await self._session.prompt_async()\n",
    ),
    mutant(
        "ctrl+d leaves from a line with text in it",
        CLI_PROMPT,
        "        at_rest_and_empty = Condition(lambda: self._answer is None and not self.typed)\n",
        "        at_rest_and_empty = Condition(lambda: self._answer is None)\n",
    ),
    mutant(
        "ctrl+d does not leave",
        CLI_PROMPT,
        "        def leave(event: KeyPressEvent) -> None:\n            event.app.exit(result=None)\n",
        "        def leave(event: KeyPressEvent) -> None:\n            pass\n",
    ),
    mutant(
        "a terminal that goes away is an error",
        CLI_PROMPT,
        "        except EOFError:\n            return None\n",
        "        except EOFError:\n            raise\n",
    ),
    mutant(
        "a key that is no answer is taken for one",
        CLI_PROMPT,
        "            if len(key) == 1 and key in self._keys:\n",
        "            if key:\n",
    ),
    mutant(
        "an answer in capitals is no answer",
        CLI_PROMPT,
        "            key = event.data.lower()\n",
        "            key = event.data\n",
    ),
    mutant(
        "a line that is submitted while a question is asked is handed over",
        CLI_PROMPT,
        '        @bindings.add("enter", filter=asking)\n',
        '        @bindings.add("f24", filter=asking)\n',
    ),
    mutant(
        "escape does not answer a question",
        CLI_PROMPT,
        '        @bindings.add("escape", filter=asking)\n',
        '        @bindings.add("f23", filter=asking)\n',
    ),
    mutant(
        "ctrl+d does not answer a question",
        CLI_PROMPT,
        '        @bindings.add("c-d", filter=asking)\n',
        '        @bindings.add("f22", filter=asking)\n',
    ),
    mutant(
        "escape answers a question with the first of its keys",
        CLI_PROMPT,
        "        def decline(event: KeyPressEvent) -> None:\n            self._answered(self._keys[-1])\n",
        "        def decline(event: KeyPressEvent) -> None:\n            self._answered(self._keys[0])\n",
    ),
    mutant(
        "ctrl+c answers a question with the first of its keys",
        CLI_PROMPT,
        "            if self._answer is not None:\n                self._answered(self._keys[-1])\n",
        "            if self._answer is not None:\n                self._answered(self._keys[0])\n",
    ),
    mutant(
        "ctrl+c stops what is running although a question is asked",
        CLI_PROMPT,
        "            if self._answer is not None:\n                self._answered(self._keys[-1])\n",
        "            if self._answer is not None:\n                self._answered(self._keys[-1])\n                self._cancel()\n",
    ),
    mutant(
        "a second key changes the answer that was given",
        CLI_PROMPT,
        "        if self._answer is not None and not self._answer.done():\n",
        "        if self._answer is not None:\n            self._answer = asyncio.get_running_loop().create_future()\n",
    ),
    mutant(
        "what was typed before a question is gone after it",
        CLI_PROMPT,
        "                buffer.set_document(aside, bypass_readonly=True)\n",
        "                pass\n",
    ),
    mutant(
        "what was typed before a question stays in the line while it is asked",
        CLI_PROMPT,
        "            aside = buffer.document if app.is_running and not app.is_done else None\n            buffer.reset()\n",
        "            aside = buffer.document if app.is_running and not app.is_done else None\n",
    ),
    mutant(
        "two questions are asked at the same time",
        CLI_PROMPT,
        "        async with self._asking:\n            app = self._session.app\n",
        "        if True:\n            app = self._session.app\n",
    ),
    mutant(
        "the words of a question are shown before its turn has come",
        CLI_PROMPT,
        "        async with self._asking:\n            app = self._session.app\n",
        "        self.show(about)\n        async with self._asking:\n            app = self._session.app\n",
        (
            "            self.show(about)\n            self._redraw()\n",
            "            self._redraw()\n",
        ),
    ),
    mutant(
        "the words of a question are not shown",
        CLI_PROMPT,
        "            self.show(about)\n            self._redraw()\n",
        "            self._redraw()\n",
    ),
    mutant(
        "a question does not take the place of the mark",
        CLI_PROMPT,
        '            parts.append(("", f"{self._choices} " if self._counting else _WAIT))\n',
        '            parts.append(("", _MARK))\n',
    ),
    mutant(
        "the mark stays when a question has been answered",
        CLI_PROMPT,
        "            finally:\n                self._answer = None\n",
        "            finally:\n",
    ),
    mutant(
        "the half of a reply stays above a line that is submitted",
        CLI_PROMPT,
        "        if self._partial.plain and not self._session.app.is_done:\n",
        "        if self._partial.plain:\n",
    ),
    mutant(
        "the line of a reply that is not complete is not drawn",
        CLI_PROMPT,
        "        if self._partial.plain and not self._session.app.is_done:\n",
        "        if self._partial.plain and self._session.app.is_done:\n",
    ),
    mutant(
        "the line of a reply that is not complete goes to the terminal as it came",
        CLI_PROMPT,
        "            unfinished = harmless(self._partial.plain)\n",
        "            unfinished = self._partial.plain\n",
    ),
    mutant(
        "a long line that is not complete is drawn whole",
        CLI_PROMPT,
        '            parts.append((quiet, _end_of(unfinished, columns - 1) + "\\n"))\n',
        '            parts.append((quiet, unfinished + "\\n"))\n',
    ),
    mutant(
        "a long line that is not complete fills the row to its last cell",
        CLI_PROMPT,
        '            parts.append((quiet, _end_of(unfinished, columns - 1) + "\\n"))\n',
        '            parts.append((quiet, _end_of(unfinished, columns) + "\\n"))\n',
    ),
    mutant(
        "the end of a long line is measured in characters and not in cells",
        CLI_PROMPT,
        "    if get_cwidth(text) <= cells:\n",
        "    if len(text) <= cells:\n",
    ),
    mutant(
        "the end of a long line is cut without the points that say so",
        CLI_PROMPT,
        "    return _CUT + text[kept:]\n",
        "    return text[kept:]\n",
    ),
    mutant(
        "a line that grows is drawn at every step",
        CLI_PROMPT,
        "        self._session.app.min_redraw_interval = _REDRAW_SECONDS\n",
        "        self._session.app.min_redraw_interval = 0\n",
    ),
    mutant(
        "a line that is not complete is not drawn anew when it grows",
        CLI_PROMPT,
        "        self._partial = text\n        self._redraw()\n",
        "        self._partial = text\n",
    ),
    mutant(
        "the line below the prompt is not drawn anew when something begins to run",
        CLI_PROMPT,
        "        # The line below the prompt says so.\n        self._redraw()\n",
        "        # The line below the prompt says so.\n",
    ),
    mutant(
        "the line below the prompt is not asked for",
        CLI_PROMPT,
        "            bottom_toolbar=lambda: harmless(self.status()),\n",
        '            bottom_toolbar=lambda: "",\n',
    ),
    mutant(
        "what is shown while nobody is asked for a line is lost",
        CLI_PROMPT,
        '            print_formatted_text(ANSI(sent), output=self._terminal.output, end="")\n',
        "            if self._session.app.is_running:\n"
        '                print_formatted_text(ANSI(sent), output=self._terminal.output, end="")\n',
    ),
    mutant(
        "colours are sent to a terminal that wants none",
        CLI_PROMPT,
        "        sent = on_screen(lines, colours=self._terminal.colours)\n",
        "        sent = on_screen(lines, colours=True)\n",
    ),
    mutant(
        "the question shows only the first line of what is asked about",
        CLI_APPROVAL,
        '            lines = request.target.split("\\n")\n',
        '            lines = request.target.split("\\n")[:1]\n',
    ),
    mutant(
        "a line break that is no line feed breaks a line of the question",
        CLI_APPROVAL,
        '            lines = request.target.split("\\n")\n',
        "            lines = request.target.splitlines()\n",
    ),
    mutant(
        "what cannot be seen is not named in the question",
        CLI_APPROVAL,
        '            about = [Text(f"    {spelled_out(line)}") for line in lines]\n',
        '            about = [Text(f"    {line}") for line in lines]\n',
    ),
    mutant(
        "the question does not say why it is asked",
        CLI_APPROVAL,
        "        question = [asked, *about, fit(request.reason, _NOTE), *keys]\n",
        "        question = [asked, *about, *keys]\n",
    ),
    mutant(
        "a call that acts on nothing in particular is asked about like one that does",
        CLI_APPROVAL,
        '            asked.append(" asks for a yes.")\n',
        '            asked.append(" asks for a yes:")\n',
    ),
    mutant(
        "a longer yes is offered where nobody can write it down",
        CLI_APPROVAL,
        "        longer = None if self._granting is None else request.standing\n",
        "        longer = request.standing\n",
    ),
    mutant(
        "a longer yes is never offered",
        CLI_APPROVAL,
        "        longer = None if self._granting is None else request.standing\n",
        "        longer = None\n",
    ),
    mutant(
        "the question does not say what a longer yes would cover",
        CLI_APPROVAL,
        '            keys.append(fit(f"  a  yes, and do not ask again in this session about {longer}"))\n',
        '            keys.append(fit("  a  yes, and do not ask again in this session"))\n',
    ),
    mutant(
        "a key that brings no letter is an answer",
        CLI_PROMPT,
        "            if len(key) == 1 and key in self._keys:\n",
        "            if key in self._keys:\n",
    ),
    mutant(
        "a key that was under way when a question came answers it",
        CLI_PROMPT,
        "            if not self._counting:\n",
        "            if False:\n",
    ),
    mutant(
        "typing on does not make the wait for the keys begin anew",
        CLI_PROMPT,
        "                self._last_key = asyncio.get_running_loop().time()\n",
        "                pass\n",
    ),
    mutant(
        "the keys of a question count as soon as it is asked",
        CLI_PROMPT,
        "            left = self._last_key + _SETTLE_SECONDS - loop.time()\n",
        "            left = 0.0\n",
    ),
    mutant(
        "the keys of a question count before its words are on the screen",
        CLI_PROMPT,
        "        await self._printed()\n",
        "",
    ),
    mutant(
        "the keys of a question are shown before they count",
        CLI_PROMPT,
        '            parts.append(("", f"{self._choices} " if self._counting else _WAIT))\n',
        '            parts.append(("", f"{self._choices} "))\n',
    ),
    mutant(
        "a line that was submitted is put aside like one that is being typed",
        CLI_PROMPT,
        "            aside = buffer.document if app.is_running and not app.is_done else None\n",
        "            aside = buffer.document\n",
    ),
    mutant(
        "the line below the prompt goes to the terminal as it came",
        CLI_PROMPT,
        "            bottom_toolbar=lambda: harmless(self.status()),\n",
        "            bottom_toolbar=lambda: self.status(),\n",
    ),
    mutant(
        "whatever is not a no is a yes",
        CLI_APPROVAL,
        "        if key not in tuple(offered[:-1]):\n",
        '        if key == "n":\n',
    ),
    mutant(
        "a longer yes that was not offered is a yes",
        CLI_APPROVAL,
        "        if key not in tuple(offered[:-1]):\n",
        '        if key not in ("y", "a"):\n',
    ),
    mutant(
        "a no is taken for a yes",
        CLI_APPROVAL,
        '            self._say("(not allowed)")\n            return "rejected"\n',
        '            self._say("(not allowed)")\n            return "allowed-once"\n',
    ),
    mutant(
        "a yes for this once is written down for the session",
        CLI_APPROVAL,
        '        if key == "a" and self._written_down(request):\n',
        '        if key in "ay" and self._written_down(request):\n',
    ),
    mutant(
        "a yes for the session is not written down",
        CLI_APPROVAL,
        '        if key == "a" and self._written_down(request):\n',
        '        if key == "a":\n',
    ),
    mutant(
        "a yes for the session that could not be written down is told as written",
        CLI_APPROVAL,
        "        except ValueError:\n            return False\n",
        "        except ValueError:\n            return True\n",
    ),
    mutant(
        "a question that is withdrawn does not say so",
        CLI_APPROVAL,
        '            self._say("(the question was withdrawn)")\n            raise\n',
        "            raise\n",
    ),
    mutant(
        "a question that is withdrawn counts as a no",
        CLI_APPROVAL,
        '            self._say("(the question was withdrawn)")\n            raise\n',
        '            self._say("(the question was withdrawn)")\n            return "rejected"\n',
    ),
)

_CLI = (
    mutant(
        "the line below the prompt shows the tokens of the first request and not of the last",
        CLI_STATUS,
        "    for event in reversed(session.events):\n",
        "    for event in session.events:\n",
    ),
    mutant(
        "what was read from the cache is not counted as sent",
        CLI_STATUS,
        "            return usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens\n",
        "            return usage.input_tokens + usage.cache_write_tokens\n",
    ),
    mutant(
        "what was written to the cache is not counted as sent",
        CLI_STATUS,
        "            return usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens\n",
        "            return usage.input_tokens + usage.cache_read_tokens\n",
    ),
    mutant(
        "a reply that failed does not count as a request",
        CLI_STATUS,
        "        if isinstance(data, AssistantMessageEvent | AssistantAttemptEvent):\n",
        "        if isinstance(data, AssistantMessageEvent):\n",
    ),
    mutant(
        "the share of the window is not told",
        CLI_STATUS,
        "        if model.context_window:\n",
        "        if not model.context_window:\n",
    ),
    mutant(
        "the share of the cache is that of what was written to it",
        CLI_STATUS,
        '        parts.append(f"cache {round(100 * total.cache_read_tokens / everything)}%")\n',
        '        parts.append(f"cache {round(100 * total.cache_write_tokens / everything)}%")\n',
    ),
    mutant(
        "the share of the cache leaves out what was written to it",
        CLI_STATUS,
        "    everything = total.input_tokens + total.cache_read_tokens + total.cache_write_tokens\n",
        "    everything = total.input_tokens + total.cache_read_tokens\n",
    ),
    mutant(
        "the line below the prompt does not say that escape stops a turn",
        CLI_STATUS,
        '    if busy:\n        parts.append("esc stops")\n',
        '    if not busy:\n        parts.append("esc stops")\n',
    ),
    mutant(
        "the line below the prompt does not say how well commands are held",
        CLI_STATUS,
        '        parts.append(f"{mode or sandbox.mode(session)}, {_HELD[sandbox.strength()]}")\n',
        '        parts.append(f"{mode or sandbox.mode(session)}")\n',
    ),
    mutant(
        "tokens are counted in thousands from the first one on",
        CLI_STATUS,
        "    if tokens < 1000:\n        return str(tokens)\n",
        "    if tokens < 0:\n        return str(tokens)\n",
    ),
    mutant(
        "many thousands of tokens are told with a part of a thousand",
        CLI_STATUS,
        "    if tokens < 100_000:\n",
        "    if tokens < 1_000_000:\n",
    ),
    mutant(
        "a million tokens is told in thousands",
        CLI_STATUS,
        '    if tokens < 1_000_000:\n        return f"{tokens // 1000}k"\n',
        '    if tokens < 1_000_000_000:\n        return f"{tokens // 1000}k"\n',
    ),
    mutant(
        "before the first request the line shows a count of nothing",
        CLI_STATUS,
        "    if sent:\n",
        "    if sent >= 0:\n",
    ),
    mutant(
        "a line that is typed while a turn runs is a task for the next turn",
        CLI_FRONTEND,
        "        elif prompt.busy:\n            agent.steer(line)\n",
        "        elif prompt.busy:\n            agent.followup(line)\n",
    ),
    mutant(
        "a line that is typed while the agent is at rest is said on the way",
        CLI_FRONTEND,
        "            agent.followup(line)\n            prompt.busy = True\n",
        "            agent.steer(line)\n            prompt.busy = True\n",
    ),
    mutant(
        "a command is carried out while a turn runs",
        CLI_FRONTEND,
        "            if prompt.busy:\n                waits =",
        "            if prompt.busy and not line:\n                waits =",
    ),
    mutant(
        "a line of spaces is a task",
        CLI_FRONTEND,
        "        line = line.strip()\n        if not line:\n            return\n",
        "        if not line:\n            return\n",
    ),
    mutant(
        "a command with spaces before it goes to the model",
        CLI_FRONTEND,
        "        if is_command(line):\n            if prompt.busy:",
        "        if is_command(typed):\n            if prompt.busy:",
        (
            "        line = line.strip()\n        if not line:\n            return\n",
            "        typed, line = line, line.strip()\n        if not line:\n            return\n",
        ),
    ),
    mutant(
        "the answer of a command is not shown",
        CLI_FRONTEND,
        "            if said:\n                # An empty line sets the answer apart from what comes after it.\n",
        "            if not said:\n                # An empty line sets the answer apart from what comes after it.\n",
    ),
    mutant(
        "the answer of a command has no empty line after it",
        CLI_FRONTEND,
        "                prompt.show([*(fit(part) for part in said.splitlines()), Text()])\n",
        "                prompt.show([fit(part) for part in said.splitlines()])\n",
    ),
    mutant(
        "a command that has nothing to say leaves an empty line",
        CLI_FRONTEND,
        "            if said:\n                # An empty line sets the answer apart from what comes after it.\n",
        "            if True:\n                # An empty line sets the answer apart from what comes after it.\n",
    ),
    mutant(
        "escape does not stop the turn of the agent",
        CLI_FRONTEND,
        '        prompt.on_cancel = lambda: agent.cancel("user")\n',
        "        prompt.on_cancel = None\n",
    ),
    mutant(
        "a turn that is stopped from the keys throws away what waits for the agent",
        CLI_FRONTEND,
        '        prompt.on_cancel = lambda: agent.cancel("user")\n',
        '        prompt.on_cancel = lambda: agent.cancel("budget")\n',
    ),
    mutant(
        "a question of approval is not put to the person at the terminal",
        CLI_FRONTEND,
        "        agent.ctx.on(APPROVAL_ASK, Responder(prompt, self._ctx.get(PERMISSIONS)))\n",
        "        Responder(prompt, self._ctx.get(PERMISSIONS))\n",
    ),
    mutant(
        "the person at the terminal cannot say yes for the rest of the session",
        CLI_FRONTEND,
        "        agent.ctx.on(APPROVAL_ASK, Responder(prompt, self._ctx.get(PERMISSIONS)))\n",
        "        agent.ctx.on(APPROVAL_ASK, Responder(prompt, None))\n",
    ),
    mutant(
        "the model that was named last is not the one that is asked",
        CLI_FRONTEND,
        '        return config.model_copy(update={"model": self._model.model})\n',
        "        return config\n",
    ),
    mutant(
        "naming another model changes nothing",
        CLI_FRONTEND,
        '            self._model = self._model.model_copy(update={"model": rest})\n',
        "            pass\n",
    ),
    mutant(
        "the command that ends the conversation does not end it",
        CLI_FRONTEND,
        '        self._leaving = True\n        return ""\n',
        '        return ""\n',
    ),
    mutant(
        "a new session is begun on top of the old one",
        CLI_FRONTEND,
        "        await self._close()\n        opened = await self._open(None, prompt)\n",
        "        opened = await self._open(None, prompt)\n",
    ),
    mutant(
        "a new session takes up the one that the conversation began on",
        CLI_FRONTEND,
        "        opened = await self._open(None, prompt)\n",
        "        opened = await self._open(self._config.session_id, prompt)\n",
    ),
    mutant(
        "the session that is taken up again is the oldest and not the one used last",
        CLI_FRONTEND,
        "            return stored[0].id\n",
        "            return stored[-1].id\n",
    ),
    mutant(
        "a session that is named is passed over for the one that was used last",
        CLI_FRONTEND,
        "        if config.session_id is not None or not config.resume:\n",
        "        if not config.resume:\n",
    ),
    mutant(
        "the session that was used last is taken up although nobody asked for it",
        CLI_FRONTEND,
        "        if config.session_id is not None or not config.resume:\n",
        "        if config.session_id is not None:\n",
    ),
    mutant(
        "nothing says that no session is stored yet",
        CLI_FRONTEND,
        "        prompt.show([Text(note, style=_NOTE)])\n        return None\n",
        "        return None\n",
    ),
    mutant(
        "a session that is taken up again does not show its last turn",
        CLI_FRONTEND,
        "        prompt.show(recap(agent.session.events, self._symbols))\n",
        "        recap(agent.session.events, self._symbols)\n",
    ),
    mutant(
        "the conversation does not begin with a word about itself",
        CLI_FRONTEND,
        "        prompt.show([head, hint, Text()])\n",
        "        prompt.show([Text()])\n",
    ),
    mutant(
        "the word that a conversation begins with has no empty line after it",
        CLI_FRONTEND,
        "        prompt.show([head, hint, Text()])\n",
        "        prompt.show([head, hint])\n",
    ),
    mutant(
        "what the program logs is not shown above the prompt",
        CLI_FRONTEND,
        "        log.addHandler(shown)\n",
        "        pass\n",
    ),
    mutant(
        "what the program logs is still shown when the conversation is over",
        CLI_FRONTEND,
        "            log.removeHandler(shown)\n",
        "            pass\n",
    ),
    mutant(
        "an error in the log is shown like a note",
        CLI_FRONTEND,
        "        style = _BAD if record.levelno >= logging.ERROR else _NOTE\n",
        "        style = _NOTE\n",
    ),
    mutant(
        "the agent is left open when the user leaves",
        CLI_FRONTEND,
        "        await asyncio.shield(agent.close())\n",
        "        pass\n",
    ),
    mutant(
        "the conversation goes on although the agent has stopped for good",
        CLI_FRONTEND,
        "            if self._gone.is_set():\n                return 1\n",
        "            if self._gone.is_set():\n                self._gone.clear()\n",
    ),
    mutant(
        "an agent that has stopped for good is taken for a user who left",
        CLI_FRONTEND,
        "            if self._gone.is_set():\n                return 1\n",
        "            if self._gone.is_set():\n                return 0\n",
    ),
    mutant(
        "nothing says that the agent has stopped",
        CLI_FRONTEND,
        '            prompt.show([fit(f"(the agent has stopped: {error})", _BAD)])\n',
        "            pass\n",
    ),
    mutant(
        "a program without a terminal says that all went well",
        CLI_FRONTEND,
        "            print(_NO_TERMINAL, file=sys.stderr)\n            return 2\n",
        "            print(_NO_TERMINAL, file=sys.stderr)\n            return 0\n",
    ),
    mutant(
        "a program without a terminal does not say what is missing",
        CLI_FRONTEND,
        "            print(_NO_TERMINAL, file=sys.stderr)\n            return 2\n",
        "            return 2\n",
    ),
    mutant(
        "a session that cannot be opened is told as a conversation that went well",
        CLI_FRONTEND,
        '            prompt.show([fit(f"columbina: {error}", _BAD)])\n            return 1\n',
        '            prompt.show([fit(f"columbina: {error}", _BAD)])\n            return 0\n',
    ),
    mutant(
        "the terminal that a test provides is passed over for the real one",
        CLI_FRONTEND,
        "        terminal = self._ctx.get(TERMINAL) or real()\n",
        "        terminal = real()\n",
    ),
    mutant(
        "the marks are not chosen for the terminal",
        CLI_FRONTEND,
        "        self._symbols = Symbols.of(terminal)\n",
        "        pass\n",
    ),
    mutant(
        "the line of a reply that is not complete is not drawn while it is written",
        CLI_FRONTEND,
        "        prompt.show(self._transcript.chunk(frame.chunk))\n        prompt.partial = self._transcript.partial\n",
        "        prompt.show(self._transcript.chunk(frame.chunk))\n",
    ),
    mutant(
        "the word about what the model does stays when the reply is written down",
        CLI_FRONTEND,
        "        prompt.show(self._transcript.event(committed.event))\n        prompt.partial = self._transcript.partial\n",
        "        prompt.show(self._transcript.event(committed.event))\n",
    ),
    mutant(
        "the line below the prompt is empty",
        CLI_FRONTEND,
        "        return status(session, self._model, sandbox, busy=self._prompt.busy, mode=mode)\n",
        '        return ""\n',
    ),
    mutant(
        "the line below the prompt names the model that the conversation began with",
        CLI_FRONTEND,
        "        return status(session, self._model, sandbox, busy=self._prompt.busy, mode=mode)\n",
        "        return status(session, self._config.model, sandbox, busy=self._prompt.busy, mode=mode)\n",
    ),
    mutant(
        "the line below the prompt shows the mode of the sandbox though somebody knows better",
        CLI_STATUS,
        '        parts.append(f"{mode or sandbox.mode(session)}, {_HELD[sandbox.strength()]}")\n',
        '        parts.append(f"{sandbox.mode(session)}, {_HELD[sandbox.strength()]}")\n',
    ),
    mutant(
        "the line below the prompt does not ask the permissions what the session goes by",
        CLI_FRONTEND,
        "        mode = None if permissions is None else permissions.in_force(session)[0]\n",
        "        mode = None\n",
    ),
    mutant(
        "a turn that did not begin with a line from the keys has nobody waiting for its end",
        CLI_FRONTEND,
        "            # in such a turn has to be noticed as well.\n            self._watch(prompt, agent)\n",
        "            # in such a turn has to be noticed as well.\n",
    ),
    mutant(
        "leaving is a success although the agent stopped for good while it was closed",
        CLI_FRONTEND,
        '            prompt.show([fit(f"(the agent has stopped: {failure})", _BAD)])\n            return 1\n',
        '            prompt.show([fit(f"(the agent has stopped: {failure})", _BAD)])\n            return 0\n',
    ),
    mutant(
        "what stopped the agent while it was closed is not looked for",
        CLI_FRONTEND,
        "        except Exception as error:\n            return error\n        return None\n",
        "        except Exception:\n            return None\n        return None\n",
    ),
    mutant(
        "an agent that stopped for good is told of a second time when it is closed",
        CLI_FRONTEND,
        "        if failure is not None and not self._gone.is_set():\n",
        "        if failure is not None:\n",
    ),
    mutant(
        "while the prompt is there the log also goes where it went before",
        CLI_FRONTEND,
        "        for other in others:\n            log.removeHandler(other)\n        log.addHandler(shown)\n",
        "        log.addHandler(shown)\n",
    ),
    mutant(
        "the log is not handed back when the conversation is over",
        CLI_FRONTEND,
        "            for other in others:\n                log.addHandler(other)\n",
        "",
    ),
    mutant(
        "what is shown is not begun anew for the terminal and the session that are there",
        CLI_FRONTEND,
        "        self._agent = agent\n        self._transcript = Transcript(self._symbols)\n",
        "        self._agent = agent\n",
    ),
    mutant(
        "a turn that begins by itself is not known to be running",
        CLI_FRONTEND,
        "        if isinstance(data, TurnStartEvent):\n            prompt.busy = True\n",
        "        if isinstance(data, TurnStartEvent):\n            pass\n",
    ),
)

_SLASH = (
    mutant(
        "the name of a command is taken with the capitals it was typed in",
        SLASH,
        "        command = self.registry.get(named.lower())\n",
        "        command = self.registry.get(named)\n",
    ),
    mutant(
        "what follows the name of a command keeps the spaces around it",
        SLASH,
        "            return await command.run(agent, rest.strip())\n",
        "            return await command.run(agent, rest)\n",
    ),
    mutant(
        "the name of a command ends at its last space and not at its first",
        SLASH,
        '        named, _, rest = line[len(_MARK) :].partition(" ")\n',
        '        named, _, rest = line[len(_MARK) :].rpartition(" ")\n',
    ),
    mutant(
        "a command that fails stops the program",
        SLASH,
        "        except Exception as error:\n",
        "        except KeyboardInterrupt as error:\n",
    ),
    mutant(
        "a command that fails is not logged",
        SLASH,
        '            logger.error("the command %s%s failed", _MARK, named.lower(), exc_info=error)\n',
        "            pass\n",
    ),
    mutant(
        "a command that fails does not say what went wrong",
        SLASH,
        '            return f"The command {_MARK}{named.lower()} failed: {type(error).__name__}: {error}"\n',
        '            return f"The command {_MARK}{named.lower()} failed."\n',
    ),
    mutant(
        "a name that no command has is an error",
        SLASH,
        "        if command is None:\n",
        "        if command is None and not named:\n",
    ),
    mutant(
        "a line that does not begin with a slash is taken for a command",
        SLASH,
        "        if not is_command(line):\n",
        "        if not line:\n",
    ),
    mutant(
        "the commands are listed in the order in which they were registered",
        SLASH,
        "            for named in sorted(self.registry)\n",
        "            for named in self.registry\n",
    ),
    mutant(
        "the list of the commands is not set in columns",
        SLASH,
        '            f"{_MARK}{named.ljust(width)}  {self.registry[named].summary}"\n',
        '            f"{_MARK}{named}  {self.registry[named].summary}"\n',
    ),
    mutant(
        "the list of the commands does not say where other lines go",
        SLASH,
        '        lines.append("A line that does not begin with a slash goes to the agent.")\n',
        "        pass\n",
    ),
    mutant(
        "there is no command that lists the commands",
        SLASH,
        '    commands.registry.add(ctx, "help", Command(commands.help, "List the commands."))\n',
        "    pass\n",
    ),
    mutant(
        "a tool is listed with the whole of its description",
        COMMAND_TOOLS,
        '        told = tool.description.strip().partition("\\n")[0]\n',
        "        told = tool.description.strip()\n",
    ),
    mutant(
        "the list of the tools does not say what a call of each does",
        COMMAND_TOOLS,
        '        lines.append(f"{tool.name.ljust(width)}  ({_DOES[tool.effect]})  {told}")\n',
        '        lines.append(f"{tool.name.ljust(width)}  {told}")\n',
    ),
    mutant(
        "the list of the tools is not set in columns",
        COMMAND_TOOLS,
        '        lines.append(f"{tool.name.ljust(width)}  ({_DOES[tool.effect]})  {told}")\n',
        '        lines.append(f"{tool.name}  ({_DOES[tool.effect]})  {told}")\n',
    ),
    mutant(
        "a program without tools cannot list them",
        COMMAND_TOOLS,
        "    if not tools.registry:\n",
        "    if tools.registry is None:\n",
    ),
    mutant(
        "the session that is open is not marked in the list",
        COMMAND_SESSIONS,
        '        mark = "*" if session.id == current else " "\n',
        '        mark = " "\n',
    ),
    mutant(
        "every session is listed, however many there are",
        COMMAND_SESSIONS,
        "_MOST = 20\n",
        "_MOST = 2000\n",
    ),
    mutant(
        "the list does not say that there are older sessions",
        COMMAND_SESSIONS,
        "    if len(stored) > _MOST:\n",
        "    if len(stored) > 100 * _MOST:\n",
    ),
    mutant(
        "a session in which nothing was said shows nothing in its place",
        COMMAND_SESSIONS,
        '        said = session.first_input or "(nothing said yet)"\n',
        '        said = session.first_input or ""\n',
    ),
    mutant(
        "a directory without sessions gives an empty list and no word about it",
        COMMAND_SESSIONS,
        "    if not stored:\n",
        "    if stored is None:\n",
    ),
    mutant(
        "the list of the sessions does not say when each was used last",
        COMMAND_SESSIONS,
        '        lines.append(f"{mark} {session.id.ljust(width)}  {_when(session.modified_ms)}  {said}")\n',
        '        lines.append(f"{mark} {session.id.ljust(width)}  {said}")\n',
    ),
    mutant(
        "the list of the sessions does not say how another one is taken up",
        COMMAND_SESSIONS,
        '    lines.append("The one with the star is open. Another one is taken up with --session-id.")\n',
        "    pass\n",
    ),
    mutant(
        "a preset that does not exist is given to the session",
        COMMAND_PERMISSIONS,
        "        if rest not in PRESETS:\n",
        "        if not rest:\n",
    ),
    mutant(
        "naming a preset changes nothing",
        COMMAND_PERMISSIONS,
        "        permissions.set_preset(agent.session, rest)\n",
        "        pass\n",
    ),
    mutant(
        "what a session may do is shown as the sandbox has it before the first turn",
        COMMAND_PERMISSIONS,
        "        mode, policy = permissions.in_force(session)\n",
        "        mode, policy = sandbox.mode(session), approval.policy(session)\n",
    ),
    mutant(
        "what was allowed for the rest of the session is not shown",
        COMMAND_PERMISSIONS,
        "        for tool, target in permissions.granted(session):\n",
        "        for tool, target in permissions.granted(session)[:0]:\n",
    ),
    mutant(
        "a tool that was allowed as a whole is shown with nothing after it",
        COMMAND_PERMISSIONS,
        '            what = tool if target is None else f"{tool}: {target}"\n',
        '            what = f"{tool}: {target}"\n',
    ),
    mutant(
        "the command does not say that the rules of the project cannot be read",
        COMMAND_PERMISSIONS,
        '            lines.append("The rules of this project cannot be read, so nothing is allowed.")\n',
        '            lines.append("deny: (none)")\n',
    ),
    mutant(
        "the rules that ask are not shown",
        COMMAND_PERMISSIONS,
        '            lines.append(_rules("ask", rules.ask))\n',
        "            pass\n",
    ),
    mutant(
        "a kind without rules shows nothing in their place",
        COMMAND_PERMISSIONS,
        "    return f\"{kind}: {', '.join(str(rule) for rule in rules) or '(none)'}\"\n",
        "    return f\"{kind}: {', '.join(str(rule) for rule in rules)}\"\n",
    ),
    mutant(
        "the command does not say how the mode and the policy are changed",
        COMMAND_PERMISSIONS,
        "        lines.append(f\"To change the mode and the policy: /permissions {' | '.join(PRESETS)}\")\n",
        "        pass\n",
    ),
    mutant(
        "the command does not say that the agent is told of the change",
        COMMAND_PERMISSIONS,
        '            "The agent is told when its next turn begins."\n',
        '            ""\n',
    ),
)

_TEXT = (
    mutant(
        "a control character goes to the terminal",
        TERMINAL_TEXT,
        '_ACTED_ON = frozenset({"Cc", "Cs", "Zl", "Zp"})\n',
        '_ACTED_ON = frozenset({"Cs", "Zl", "Zp"})\n',
    ),
    mutant(
        "half of a pair goes to the terminal",
        TERMINAL_TEXT,
        '_ACTED_ON = frozenset({"Cc", "Cs", "Zl", "Zp"})\n',
        '_ACTED_ON = frozenset({"Cc", "Zl", "Zp"})\n',
    ),
    mutant(
        "a character that breaks the line without being a line feed goes to the terminal",
        TERMINAL_TEXT,
        '_ACTED_ON = frozenset({"Cc", "Cs", "Zl", "Zp"})\n',
        '_ACTED_ON = frozenset({"Cc", "Cs"})\n',
    ),
    mutant(
        "a character that turns the direction of text goes to the terminal",
        TERMINAL_TEXT,
        "    return character in _TURNS or unicodedata.category(character) in _ACTED_ON\n",
        "    return unicodedata.category(character) in _ACTED_ON\n",
    ),
    mutant(
        "the marks that say a direction without turning it go to the terminal",
        TERMINAL_TEXT,
        "    chr(code) for code in (0x061C, 0x200E, 0x200F, *range(0x202A, 0x202F), *range(0x2066, 0x206A))\n",
        "    chr(code) for code in (*range(0x202A, 0x202F), *range(0x2066, 0x206A))\n",
    ),
    mutant(
        "the characters that set a piece of text apart in its direction go to the terminal",
        TERMINAL_TEXT,
        "    chr(code) for code in (0x061C, 0x200E, 0x200F, *range(0x202A, 0x202F), *range(0x2066, 0x206A))\n",
        "    chr(code) for code in (0x061C, 0x200E, 0x200F, *range(0x202A, 0x202F))\n",
    ),
    mutant(
        "a tab is replaced like a control character",
        TERMINAL_TEXT,
        '        STAND_IN if character not in "\\t\\n" and _acted_on(character) else character\n',
        '        STAND_IN if character not in "\\n" and _acted_on(character) else character\n',
    ),
    mutant(
        "a line feed is replaced like a control character",
        TERMINAL_TEXT,
        '        STAND_IN if character not in "\\t\\n" and _acted_on(character) else character\n',
        '        STAND_IN if character not in "\\t" and _acted_on(character) else character\n',
    ),
    mutant(
        "what stands in for a character is a question mark",
        TERMINAL_TEXT,
        "STAND_IN = chr(0xFFFD)\n",
        'STAND_IN = "?"\n',
    ),
    mutant(
        "a plain space is named in what a person is asked",
        TERMINAL_TEXT,
        '        f"<U+{ord(character):04X}>" if character not in " \\t\\n" and unseen(character) else character\n',
        '        f"<U+{ord(character):04X}>" if character not in "\\t\\n" and unseen(character) else character\n',
    ),
    mutant(
        "a character is named in lower case and without its leading zeros",
        TERMINAL_TEXT,
        '        f"<U+{ord(character):04X}>" if character not in " \\t\\n" and unseen(character) else character\n',
        '        f"<U+{ord(character):x}>" if character not in " \\t\\n" and unseen(character) else character\n',
    ),
    mutant(
        "a text that Python calls printable is not looked at where a person is asked",
        TERMINAL_TEXT,
        "    if text.isascii() and text.isprintable():\n",
        "    if text.isprintable():\n",
    ),
    mutant(
        "nothing is named in what a person is asked",
        TERMINAL_TEXT,
        '        f"<U+{ord(character):04X}>" if character not in " \\t\\n" and unseen(character) else character\n',
        '        f"<U+{ord(character):04X}>" if character not in " \\t\\n" and False else character\n',
    ),
    mutant(
        "an argument that is not known is repeated as it was given",
        LAUNCHER,
        "        super().error(harmless(message))\n",
        "        super().error(message)\n",
    ),
    mutant(
        "printed json leaves the characters that a terminal acts on as they are",
        TERMINAL_TEXT,
        '    return f"\\\\u{ord(character):04x}" if _acted_on(character) else character\n',
        "    return character\n",
    ),
    mutant(
        "printed json writes other languages by the numbers of their characters",
        TERMINAL_TEXT,
        '    return f"\\\\u{ord(character):04x}" if _acted_on(character) else character\n',
        '    return f"\\\\u{ord(character):04x}"\n',
    ),
    mutant(
        "printed json leaves the character that deletes as it is",
        TERMINAL_TEXT,
        '_IN_JSON = re.compile("[^\\t\\n -~]")\n',
        '_IN_JSON = re.compile("[^\\t\\n -\\x7f]")\n',
    ),
    mutant(
        "an answer goes to a terminal as the model wrote it",
        HEADLESS,
        "                print(harmless(answer) if sys.stdout.isatty() else answer)\n",
        "                print(answer)\n",
    ),
    mutant(
        "an answer is changed also where a file or a program gets it",
        HEADLESS,
        "                print(harmless(answer) if sys.stdout.isatty() else answer)\n",
        "                print(harmless(answer))\n",
    ),
    mutant(
        "what is said about a run goes out as it came",
        HEADLESS,
        '    print(harmless(f"columbina: {message}"), file=sys.stderr)\n',
        '    print(f"columbina: {message}", file=sys.stderr)\n',
    ),
    mutant(
        "the lines of json of a run leave such characters as they are",
        HEADLESS,
        '    sys.stdout.write(escaped(json.dumps(line, ensure_ascii=False)) + "\\n")\n',
        '    sys.stdout.write(json.dumps(line, ensure_ascii=False) + "\\n")\n',
    ),
    mutant(
        "what the launcher says goes out as it came",
        LAUNCHER,
        '    print(harmless(f"columbina: {message}"), file=sys.stderr)\n',
        '    print(f"columbina: {message}", file=sys.stderr)\n',
    ),
    mutant(
        "what is logged during a run goes out as it came",
        LAUNCHER,
        "        return harmless(super().format(record))\n",
        "        return super().format(record)\n",
    ),
    mutant(
        "the entries are printed to a terminal as they are",
        LAUNCHER,
        "            sys.stdout.write(harmless(dumped) if sys.stdout.isatty() else dumped)\n",
        "            sys.stdout.write(dumped)\n",
    ),
    mutant(
        "the entries are changed also where a file or a program gets them",
        LAUNCHER,
        "            sys.stdout.write(harmless(dumped) if sys.stdout.isatty() else dumped)\n",
        "            sys.stdout.write(harmless(dumped))\n",
    ),
)

_STORED = (
    mutant(
        "the sessions are listed with the oldest first",
        SESSION_STORE,
        "    return sorted(listed, key=lambda one: (one.modified_ms, one.id), reverse=True)\n",
        "    return sorted(listed, key=lambda one: (one.modified_ms, one.id))\n",
    ),
    mutant(
        "what plugins told the model counts as the first thing the user said",
        SESSION_STORE,
        '            if said.source != "user":\n                continue\n',
        '            if said.source == "nobody":\n                continue\n',
    ),
    mutant(
        "a session that cannot be read hides the others",
        SESSION_STORE,
        '            logger.warning("the session in %s is left out of the list: %s", entry, error)\n'
        "            continue\n",
        "            raise\n",
    ),
    mutant(
        "a session that cannot be read is left out without a word",
        SESSION_STORE,
        '            logger.warning("the session in %s is left out of the list: %s", entry, error)\n',
        "            pass\n",
    ),
    mutant(
        "something else that lies among the sessions is warned about",
        SESSION_STORE,
        "        except (FileNotFoundError, NotADirectoryError):\n",
        "        except ChildProcessError:\n",
    ),
    mutant(
        "a long first input is shown whole",
        SESSION_STORE,
        "            if len(first) > _SHOWN_OF_AN_INPUT + 1:\n",
        "            if len(first) > 100 * _SHOWN_OF_AN_INPUT:\n",
    ),
    mutant(
        "a first input of several lines is shown with all of them",
        SESSION_STORE,
        '            first = " ".join(texts).strip().partition("\\n")[0].rstrip()\n',
        '            first = " ".join(texts).strip()\n',
    ),
    mutant(
        "a last line that was never finished makes a session unreadable",
        SESSION_STORE,
        '            if not line.endswith(b"\\n"):\n                # A last line that was never finished.\n                break\n',
        '            if not line.endswith(b"\\n"):\n                # A last line that was never finished.\n                pass\n',
    ),
    mutant(
        "the first user message is looked for only in the first event",
        SESSION_STORE,
        '            if event.get("type") != UserMessageEvent.event_type:\n                continue\n',
        '            if event.get("type") != UserMessageEvent.event_type:\n                break\n',
    ),
    mutant(
        "a file that does not begin like a session is listed",
        SESSION_STORE,
        '    with path.open("rb") as file:\n        decode_header(file.readline())\n',
        '    with path.open("rb") as file:\n        file.readline()\n',
    ),
    mutant(
        "a directory that is not there yet is an error and not an empty list",
        SESSION_STORE,
        "        entries = list(directory.iterdir())\n    except FileNotFoundError:\n        return []\n",
        "        entries = list(directory.iterdir())\n    except FileNotFoundError:\n        raise\n",
    ),
    mutant(
        "the time of a session is told in seconds and not in milliseconds",
        SESSION_STORE,
        "            modified = file.stat().st_mtime_ns // 1_000_000\n",
        "            modified = file.stat().st_mtime_ns // 1_000_000_000\n",
    ),
    mutant(
        "sessions that are kept in memory are looked for in a store that is not there",
        SESSION_SERVICE,
        "        if self.root is None:\n            return []\n        return stored_sessions(",
        "        return stored_sessions(",
    ),
    mutant(
        "the sessions of every working directory are looked for in one place",
        SESSION_SERVICE,
        "        return stored_sessions(self.root / workspace_key(cwd))\n",
        '        return stored_sessions(self.root / workspace_key("."))\n',
    ),
)

_GRANTS = (
    mutant(
        "what was allowed for the rest of a session does not count for a file",
        PERMISSIONS,
        '        if granted:\n            return Decided("allow", None, _GRANTED)\n',
        '        if granted and not tool:\n            return Decided("allow", None, _GRANTED)\n',
    ),
    mutant(
        "what was allowed for the rest of a session does not count for a command",
        PERMISSIONS,
        '    if granted and apart is not None:\n        return Decided("allow", None, _GRANTED)\n',
        '    if granted and apart is None:\n        return Decided("allow", None, _GRANTED)\n',
    ),
    mutant(
        "a command that cannot be taken apart runs because it was allowed for the session",
        PERMISSIONS,
        "    if granted and apart is not None:\n",
        "    if granted:\n",
    ),
    mutant(
        "what was allowed for the session counts for the tool, whatever it acts on",
        PERMISSIONS,
        "            granted=(tool.name, invocation.target) in self._grants.get(session, ()),\n",
        "            granted=any(tool.name == held[0] for held in self._grants.get(session, ())),\n",
    ),
    mutant(
        "what was allowed for the session counts for whatever tool acts on the same thing",
        PERMISSIONS,
        "            granted=(tool.name, invocation.target) in self._grants.get(session, ()),\n",
        "            granted=any(\n"
        "                invocation.target == held[1] for held in self._grants.get(session, ())\n"
        "            ),\n",
    ),
    mutant(
        "a question does not say what a yes for the session would cover",
        PERMISSIONS,
        "                standing=self.standing(session, tool.name, invocation.target),\n",
        "                standing=None,\n",
    ),
    mutant(
        "the mode of the preset does not count before it is written down",
        PERMISSIONS,
        "            self._sandbox.recorded(session) or mode,\n",
        "            self._sandbox.mode(session),\n",
    ),
    mutant(
        "the mode that the log names does not count against the preset",
        PERMISSIONS,
        "            self._sandbox.recorded(session) or mode,\n",
        "            mode,\n",
    ),
    mutant(
        "the policy of the preset does not count before it is written down",
        PERMISSIONS,
        "            self._approval.recorded(session) or policy,\n",
        "            self._approval.policy(session),\n",
    ),
    mutant(
        "the policy that the log names does not count against the preset",
        PERMISSIONS,
        "            self._approval.recorded(session) or policy,\n",
        "            policy,\n",
    ),
    mutant(
        "a call that has to be asked about every time is allowed for a session",
        PERMISSIONS,
        "        if self.standing(session, tool, target) is None:\n",
        "        if self.standing(session, tool, target) is None and not tool:\n",
    ),
    mutant(
        "a command that cannot be taken apart can be allowed for a session",
        PERMISSIONS,
        "            if target is None or splitter is None or splitter(target) is None:\n",
        "            if target is None or splitter is None:\n",
    ),
    mutant(
        "a command can be allowed for a session where no splitter knows its shell",
        PERMISSIONS,
        "            if target is None or splitter is None or splitter(target) is None:\n",
        "            if target is None or (splitter is not None and splitter(target) is None):\n",
    ),
    mutant(
        "a tool that acts on nothing in particular is allowed on nothing",
        PERMISSIONS,
        '        return f"any call of {tool}" if target is None else f"{tool} on {target}"\n',
        '        return f"{tool} on {target}"\n',
    ),
    mutant(
        "a preset gives a session its mode and not its policy",
        PERMISSIONS,
        "        self._sandbox.set_mode(session, mode)\n        self._approval.set_policy(session, policy)\n",
        "        self._sandbox.set_mode(session, mode)\n",
    ),
    mutant(
        "a preset gives a session its policy and not its mode",
        PERMISSIONS,
        "        self._sandbox.set_mode(session, mode)\n        self._approval.set_policy(session, policy)\n",
        "        self._approval.set_policy(session, policy)\n",
    ),
    mutant(
        "a preset that does not exist is taken for the first one",
        PERMISSIONS,
        '        if chosen is None:\n            raise ValueError(f"there is no preset {preset!r}")\n',
        "        if chosen is None:\n            chosen = next(iter(_PRESETS.values()))\n",
    ),
)

_CHAT = (
    mutant(
        "without a command a task is meant and not a conversation",
        LAUNCHER,
        "        given.insert(0, _CHAT)\n",
        "        given.insert(0, _RUN)\n",
    ),
    mutant(
        "an option that stands first is taken for a command",
        LAUNCHER,
        '    if not given or (given[0].startswith("-") and given[0] not in _NOT_A_COMMAND):\n',
        "    if not given:\n",
    ),
    mutant(
        "asking for help begins a conversation",
        LAUNCHER,
        '_NOT_A_COMMAND = ("-h", "--help", "--version")\n',
        '_NOT_A_COMMAND = ("--version",)\n',
    ),
    mutant(
        "asking for the version begins a conversation",
        LAUNCHER,
        '_NOT_A_COMMAND = ("-h", "--help", "--version")\n',
        '_NOT_A_COMMAND = ("-h", "--help")\n',
    ),
    mutant(
        "a session can be named and looked for at once",
        LAUNCHER,
        "    if args.command == _CHAT and args.resume and args.session_id is not None:\n",
        "    if args.command == _RUN and args.resume and args.session_id is not None:\n",
    ),
    mutant(
        "the wish to take up the last session is not handed to the conversation",
        LAUNCHER,
        '            elif args.resume:\n                config["resume"] = True\n',
        '            elif args.resume:\n                config["resume"] = False\n',
    ),
    mutant(
        "a conversation loads the program that runs one task",
        LAUNCHER,
        '    _shared(chat, "cli")\n',
        '    _shared(chat, "headless")\n',
    ),
    mutant(
        "a task loads the program of a conversation",
        LAUNCHER,
        '    _shared(run, "headless")\n',
        '    _shared(run, "cli")\n',
    ),
    mutant(
        "a task does not write its log to the standard error",
        LAUNCHER,
        "    log.addHandler(handler)\n    try:\n",
        "    if args.command == _CHAT:\n        log.addHandler(handler)\n    try:\n",
    ),
    mutant(
        "what a conversation logs before its prompt is there has nowhere to go",
        LAUNCHER,
        "    log.addHandler(handler)\n    try:\n",
        "    if args.command == _RUN:\n        log.addHandler(handler)\n    try:\n",
    ),
    mutant(
        "a session that the profile names is taken up though the last one was asked for",
        LAUNCHER,
        '                config.pop("session_id", None)\n',
        "                pass\n",
    ),
    mutant(
        "a working directory that is not there is taken",
        LAUNCHER,
        "                if not Path(args.cwd).is_dir():\n",
        "                if not Path(args.cwd).parent.is_dir():\n",
    ),
    mutant(
        "a file is taken for a working directory",
        LAUNCHER,
        "                if not Path(args.cwd).is_dir():\n",
        "                if not Path(args.cwd).exists():\n",
    ),
    mutant(
        "the conversation has no command that lists the tools",
        CLI_BUNDLE,
        "        plugin: columbina.commands.tools\n",
        "        plugin: columbina.commands.tools\n        disabled: true\n",
    ),
    mutant(
        "the conversation has no command that shows what the session may do",
        CLI_BUNDLE,
        "        plugin: columbina.commands.permissions\n",
        "        plugin: columbina.commands.permissions\n        disabled: true\n",
    ),
    mutant(
        "the conversation has no command that lists the sessions",
        CLI_BUNDLE,
        "        plugin: columbina.commands.sessions\n",
        "        plugin: columbina.commands.sessions\n        disabled: true\n",
    ),
    mutant(
        "the conversation asks another model than the one that is the default",
        CLI_BUNDLE,
        "            model: ${COLUMBINA_MODEL:-deepseek-flash}\n",
        "            model: ${COLUMBINA_MODEL:-deepseek-pro}\n",
    ),
    mutant(
        "the conversation does not build on the base program",
        CLI_BUNDLE,
        "extends: base\n",
        "extends: minimal\n",
    ),
)

_UNSEEN = (
    mutant(
        "a control character is seen",
        UNSEEN,
        '    return unicodedata.category(character)[0] in "CZ" or character in _DRAWN_AS_NOTHING\n',
        '    return unicodedata.category(character)[0] in "Z" or character in _DRAWN_AS_NOTHING\n',
    ),
    mutant(
        "a space of another width is seen",
        UNSEEN,
        '    return unicodedata.category(character)[0] in "CZ" or character in _DRAWN_AS_NOTHING\n',
        '    return unicodedata.category(character)[0] in "C" or character in _DRAWN_AS_NOTHING\n',
    ),
    mutant(
        "a letter or a mark that is drawn as nothing is seen",
        UNSEEN,
        '    return unicodedata.category(character)[0] in "CZ" or character in _DRAWN_AS_NOTHING\n',
        '    return unicodedata.category(character)[0] in "CZ"\n',
    ),
    mutant(
        "the mark that joins what stands beside it is seen",
        UNSEEN,
        "        0x034F,  # joins the characters beside it\n",
        "",
    ),
    mutant(
        "the pattern of Braille without dots is seen",
        UNSEEN,
        "        0x2800,  # the pattern of Braille without dots\n",
        "",
    ),
    mutant(
        "the first of the characters that choose a form is seen",
        UNSEEN,
        "        *range(0xFE00, 0xFE10),  # choose the form of the character before them\n",
        "        *range(0xFE01, 0xFE10),  # choose the form of the character before them\n",
    ),
    mutant(
        "the last of the characters that choose a form is seen",
        UNSEEN,
        "        *range(0xFE00, 0xFE10),  # choose the form of the character before them\n",
        "        *range(0xFE00, 0xFE0F),  # choose the form of the character before them\n",
    ),
    mutant(
        "the character after those that choose a form is not seen",
        UNSEEN,
        "        *range(0xFE00, 0xFE10),  # choose the form of the character before them\n",
        "        *range(0xFE00, 0xFE11),  # choose the form of the character before them\n",
    ),
    mutant(
        "the last of the characters that choose a form of a letter of Mongolian is seen",
        UNSEEN,
        "        *range(0x180B, 0x1810),  # choose the form of a letter of Mongolian\n",
        "        *range(0x180B, 0x180F),  # choose the form of a letter of Mongolian\n",
    ),
    mutant(
        "the last of the further characters that choose a form is seen",
        UNSEEN,
        "        *range(0xE0100, 0xE01F0),\n",
        "        *range(0xE0100, 0xE01EF),\n",
    ),
    mutant(
        "a filler of Hangul is seen",
        UNSEEN,
        "        0x1160,\n",
        "",
    ),
    mutant(
        "a vowel of Khmer that is not written is seen",
        UNSEEN,
        "        0x17B5,\n",
        "",
    ),
)

SETS: dict[str, MutantSet] = {
    "approval": MutantSet(("tests/approval",), _APPROVAL),
    "facts": MutantSet(("tests/context",), _FACTS),
    "base": MutantSet(("tests/apps", "tests/test_package.py"), _BASE),
    "instructions": MutantSet(("tests/context",), _INSTRUCTIONS),
    "context": MutantSet(("tests/agent_loop",), _CONTEXT),
    "prompt": MutantSet(("tests/system_prompt", "tests/agent_loop/test_turns.py"), _PROMPT),
    "shell": MutantSet(("tests/builtin_tools/test_shell.py",), _SHELL),
    "permissions": MutantSet(("tests/permissions",), _PERMISSIONS),
    "search": MutantSet(
        (
            "tests/fs/test_walk.py",
            "tests/fs/test_patterns.py",
            "tests/fs/test_bound.py",
            "tests/builtin_tools/test_glob.py",
        ),
        _SEARCH,
    ),
    "grep": MutantSet(
        ("tests/builtin_tools/test_grep.py", "tests/builtin_tools/test_grep_worker.py"), _GREP
    ),
    "files": MutantSet(
        ("tests/builtin_tools/test_write.py", "tests/builtin_tools/test_edit.py"), _FILES
    ),
    "loop": MutantSet(("tests/agent_loop", "tests/agent", "tests/test_cancellation.py"), _LOOP),
    "tools": MutantSet(("tests/tools",), _TOOLS),
    "read": MutantSet(
        (
            "tests/builtin_tools/test_read.py",
            "tests/builtin_tools/test_in_the_loop.py",
            "tests/builtin_tools/test_write.py",
            "tests/builtin_tools/test_edit.py",
        ),
        _READ,
    ),
    "process": MutantSet(("tests/test_process.py",), _PROCESS),
    "fs": MutantSet(("tests/fs", "tests/builtin_tools"), _FS),
    "sandbox": MutantSet(("tests/sandbox",), _SANDBOX),
    "transcript": MutantSet(
        ("tests/apps/cli/test_lines.py", "tests/apps/cli/test_transcript.py"), _TRANSCRIPT
    ),
    "terminal": MutantSet(
        (
            "tests/apps/cli/test_prompt.py",
            "tests/apps/cli/test_approval.py",
            "tests/apps/cli/test_what_the_screen_shows.py",
        ),
        _TERMINAL,
    ),
    "cli": MutantSet(
        (
            "tests/apps/cli/test_status.py",
            "tests/apps/cli/test_cli.py",
            "tests/apps/cli/test_a_session_at_the_terminal.py",
        ),
        _CLI,
    ),
    "commands": MutantSet(("tests/commands",), _SLASH),
    "text": MutantSet(
        (
            "tests/apps/test_terminal_text.py",
            "tests/apps/test_headless.py",
            "tests/apps/test_launcher.py",
        ),
        _TEXT,
    ),
    "stored": MutantSet(("tests/session/test_stored.py",), _STORED),
    "grants": MutantSet(("tests/permissions",), _GRANTS),
    "chat": MutantSet(("tests/apps", "tests/test_package.py"), _CHAT),
    "unseen": MutantSet(("tests/test_unseen.py", "tests/apps/test_terminal_text.py"), _UNSEEN),
}
"""The sets of mutants, by the name that ``mutate.py`` takes."""
