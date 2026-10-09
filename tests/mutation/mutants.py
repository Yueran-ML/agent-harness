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

A mutant of code that only one platform runs says which. On another platform
no test can notice it, and ``mutate.py`` passes it over.
"""

from dataclasses import dataclass, replace

LOOP = "src/columbina/agent_loop/loop.py"
INBOX = "src/columbina/agent/inbox.py"
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
RULES = "src/columbina/permissions/rules.py"
COMMANDS = "src/columbina/permissions/commands.py"
PERMISSIONS = "src/columbina/permissions/__init__.py"
GREP_WORKER = "src/columbina/builtin_tools/grep_worker.py"


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
        '        if letter not in " \\n\\t" and unicodedata.category(letter) in _HIDDEN:',
        "        if False:",
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

SETS: dict[str, MutantSet] = {
    "approval": MutantSet(("tests/approval",), _APPROVAL),
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
}
"""The sets of mutants, by the name that ``mutate.py`` takes."""
