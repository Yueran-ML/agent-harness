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
        "            if passed is not invocation:\n                raise _NotTheCheckedCallError\n",
        "",
    ),
    mutant(
        "a call runs although its tool has left",
        PIPELINE,
        "        gone = self._gone(tool)\n        if gone is not None:\n            return gone\n",
        "",
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
        "    if path.parts[: len(top)] == top:",
        "    if True:",
    ),
    on_windows(
        mutant(
            "names are compared after their case was lowered",
            PATHS,
            "    if path.parts[: len(top)] == top:",
            "    if path.is_relative_to(Path(root)):",
        )
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
            '            if part.split(".")[0].rstrip(" ").upper() in _DEVICES:',
            "            if part.upper() in _DEVICES:",
        )
    ),
    on_windows(
        mutant(
            "a name may end with a dot",
            PATHS,
            '            if part[-1] in ". ":',
            '            if part[-1] == " ":',
        )
    ),
    on_windows(
        mutant(
            "a name may hold a colon",
            PATHS,
            '            if ":" in part:\n'
            '                raise _refused(written, "a colon inside a name is not allowed.")\n',
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
        "            if not os.path.samestat(left.lstat(), right.lstat()):",
        "            if not os.path.samestat(left.stat(), right.stat()):",
    ),
    mutant(
        "two paths of different depth may be one place",
        BOUND,
        "    if len(first.parts) != len(second.parts):\n        return False\n",
        "",
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
        "            if seen is None:\n"
        '                raise NotShownError(f"{path} was never seen")\n',
        "",
    ),
    mutant(
        "a file that changed since it was seen is replaced",
        BOUND,
        "            if (existing.st_size, existing.st_mtime_ns) != seen:\n"
        '                raise ChangedError(f"{path} is not as it was seen")\n',
        "",
    ),
    mutant(
        "a change that keeps the size goes unnoticed",
        BOUND,
        "            if (existing.st_size, existing.st_mtime_ns) != seen:",
        "            if existing.st_size != seen[0]:",
    ),
    mutant(
        "a directory is taken for a file that was never seen",
        BOUND,
        "            if not stat.S_ISREG(existing.st_mode):\n"
        '                raise NotAFileError(f"{path} is not a regular file")\n',
        "",
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
        "a permit to read replaces",
        SEAM,
        '        if self.access.kind != "write":\n'
        '            raise ValueError("this permit is not for writing")\n',
        "",
    ),
)

SETS: dict[str, MutantSet] = {
    "loop": MutantSet(("tests/agent_loop", "tests/agent", "tests/test_cancellation.py"), _LOOP),
    "tools": MutantSet(("tests/tools",), _TOOLS),
    "read": MutantSet(("tests/builtin_tools",), _READ),
    "process": MutantSet(("tests/test_process.py",), _PROCESS),
    "fs": MutantSet(("tests/fs", "tests/builtin_tools"), _FS),
}
"""The sets of mutants, by the name that ``mutate.py`` takes."""
