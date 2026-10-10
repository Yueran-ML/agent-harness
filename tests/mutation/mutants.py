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
HEADLESS_BUNDLE = "src/columbina/bundles/headless.yml"
INSTRUCTIONS = "src/columbina/context/instructions.py"
PROMPT = "src/columbina/system_prompt/__init__.py"
RULES = "src/columbina/permissions/rules.py"
COMMANDS = "src/columbina/permissions/commands.py"
PERMISSIONS = "src/columbina/permissions/__init__.py"
GREP_WORKER = "src/columbina/builtin_tools/grep_worker.py"
CLI_LINES = "src/columbina/apps/cli/lines.py"
CLI_TRANSCRIPT = "src/columbina/apps/cli/transcript.py"
APPS_COMMON = "src/columbina/apps/common.py"
CLI_PROMPT = "src/columbina/apps/cli/prompt.py"
CLI_APPROVAL = "src/columbina/apps/cli/approval.py"
CLI_STATUS = "src/columbina/apps/cli/status.py"
CLI_FRONTEND = "src/columbina/apps/cli/__init__.py"
CLI_BUNDLE = "src/columbina/bundles/cli.yml"
SLASH = "src/columbina/commands/__init__.py"
COMMAND_TOOLS = "src/columbina/commands/tools.py"
COMMAND_SESSIONS = "src/columbina/commands/sessions.py"
COMMAND_PERMISSIONS = "src/columbina/commands/permissions.py"
COMMAND_COMPACT = "src/columbina/commands/compact.py"
COMMAND_TODOS = "src/columbina/commands/todos.py"
COMMAND_FORK = "src/columbina/commands/fork.py"
AGENTS_SERVICE = "src/columbina/agent/agent.py"
APPS_SESSIONS = "src/columbina/apps/sessions.py"
TODO = "src/columbina/todo/__init__.py"
TERMINAL_TEXT = "src/columbina/apps/terminal_text.py"
HEADLESS = "src/columbina/apps/headless.py"
SESSION_STORE = "src/columbina/session/store.py"
SURFACE = "src/columbina/session/surface.py"
SESSION_SERVICE = "src/columbina/session/service.py"
UNSEEN = "src/columbina/unseen.py"
TOKEN_METER = "src/columbina/token_meter/__init__.py"
SPILL_LOCAL = "src/columbina/spill/local.py"
SPILL_POLICY = "src/columbina/spill/policy.py"
SPILL_SEAM = "src/columbina/spill/__init__.py"
COMPACTION_SEAM = "src/columbina/compaction/__init__.py"
TRIM = "src/columbina/compaction/basic/trim.py"
REGION = "src/columbina/compaction/basic/region.py"
SUMMARY = "src/columbina/compaction/basic/summary.py"
BASIC = "src/columbina/compaction/basic/__init__.py"
PROJECTION = "src/columbina/session/projection.py"
BUDGET = "src/columbina/guard/budget.py"
TOOL = "src/columbina/tools/tool.py"
PROJECT = "src/columbina/project.py"
SKILL_FILES = "src/columbina/skills/files.py"


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
    mutant(
        "an agent that has ended takes work to do between turns",
        LOOP,
        '        if self._ending() or self._failure is not None or self._driver.done():\n            raise AgentBusyError(f"the agent {self.id!r} has ended")\n',
        "",
    ),
    mutant(
        "an agent that is not at rest takes work to do between turns",
        LOOP,
        '        if not self._idle.is_set():\n            raise AgentBusyError(f"the agent {self.id!r} is not at rest")\n',
        "",
    ),
    mutant(
        "an agent counts as at rest while it does work between turns",
        LOOP,
        "        self._idle.clear()\n        self._wake.set()\n        return cast(",
        "        self._wake.set()\n        return cast(",
    ),
    mutant(
        "work that the agent is told to stop before it began is done all the same",
        LOOP,
        "        self._drop_work()\n        task = self._turn_task\n",
        "        task = self._turn_task\n",
    ),
    mutant(
        "work that waits when the driver of the agent ends is left waiting",
        LOOP,
        "            self._idle.set()\n            self._drop_work()\n",
        "            self._idle.set()\n",
    ),
    mutant(
        "work between turns cannot be stopped with the agent",
        LOOP,
        "        task = self._turn_task = asyncio.ensure_future(_did(work.job))\n",
        "        task = asyncio.ensure_future(_did(work.job))\n",
    ),
    mutant(
        "what work between turns gives is not handed to whoever asked",
        LOOP,
        "                work.done.set_result(result)\n",
        "                pass\n",
    ),
    mutant(
        "what work between turns raises is not handed to whoever asked",
        LOOP,
        "                work.done.set_exception(error)\n",
        "                pass\n",
    ),
    mutant(
        "whoever stops waiting for work between turns ends the agent",
        LOOP,
        "            if not work.done.done():\n                work.done.set_result(result)\n",
        "            if True:\n                work.done.set_result(result)\n",
    ),
    mutant(
        "nobody is told that work between turns was stopped",
        LOOP,
        "            self._turn_task = None\n            # However it ended, nobody is left waiting for it.\n            work.done.cancel()\n",
        "            self._turn_task = None\n",
    ),
    mutant(
        "work that fails ends the agent if whoever asked has stopped waiting",
        LOOP,
        "            if not work.done.done():\n                work.done.set_exception(error)\n",
        "            if True:\n                work.done.set_exception(error)\n",
    ),
    mutant(
        "a driver that is told to stop goes on after the work that it was doing",
        LOOP,
        "            # Either the driver is being cancelled, or only the work was stopped.\n            if cancellation.requested_since(before):\n",
        "            # Either the driver is being cancelled, or only the work was stopped.\n            if False:\n",
    ),
    mutant(
        "an agent goes on from all of a session, wherever it was to be cut",
        AGENTS_SERVICE,
        "            session = self._sessions.fork(scope, spec.cwd, source, upto=upto, session_id=session_id)\n",
        "            session = self._sessions.fork(scope, spec.cwd, source, session_id=session_id)\n",
    ),
    mutant(
        "an agent that goes on from a copy shares the session that was copied",
        AGENTS_SERVICE,
        "            session = self._sessions.fork(scope, spec.cwd, source, upto=upto, session_id=session_id)\n",
        "            session = self._sessions.resume(scope, spec.cwd, session_id)\n",
    ),
    mutant(
        "a copy that cannot be made keeps its id",
        AGENTS_SERVICE,
        "            session = self._sessions.fork(scope, spec.cwd, source, upto=upto, session_id=session_id)\n            return self._start(scope, factory, spec, session)\n        except BaseException:\n            await scope.fiber.dispose()\n            raise\n",
        "            session = self._sessions.fork(scope, spec.cwd, source, upto=upto, session_id=session_id)\n            return self._start(scope, factory, spec, session)\n        except BaseException:\n            raise\n",
    ),
    mutant(
        "an agent that is ending takes work to do between turns",
        LOOP,
        "        if self._ending() or self._failure is not None or self._driver.done():\n            raise AgentBusyError(",
        "        if self._closing or self._failure is not None or self._driver.done():\n            raise AgentBusyError(",
    ),
    mutant(
        "an agent that is ending begins another turn",
        LOOP,
        "        while not self._ending() and not self._halted:\n",
        "        while not self._closing and not self._halted:\n",
    ),
    mutant(
        "an agent is told about the tools that its spec leaves out",
        LOOP,
        "        schemas = self._tools.schemas(self.spec.tools)\n",
        "        schemas = self._tools.schemas()\n",
    ),
    mutant(
        "an agent can call a tool that its spec leaves out",
        LOOP,
        "        return self._tools.begin(self.ctx, call, run, self.spec.tools)\n",
        "        return self._tools.begin(self.ctx, call, run)\n",
    ),
    mutant(
        "a turn of no steps is a limit that a spec can set",
        AGENTS_SERVICE,
        "    max_steps: int | None = Field(default=None, ge=1)\n",
        "    max_steps: int | None = Field(default=None, ge=0)\n",
    ),
    mutant(
        "an agent that is created for a delegation does not say how deep",
        AGENTS_SERVICE,
        "                delegation_depth=delegation_depth,\n",
        "",
    ),
    mutant(
        "an agent that is created for a delegation does not say by whom",
        AGENTS_SERVICE,
        "                parent=parent,\n",
        "",
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
        "            stderr=subprocess.STDOUT if errors is None",
        "            stderr=subprocess.DEVNULL if errors is None",
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
    mutant(
        "the place for the whole output is opened with the first piece",
        PROCESS,
        "        if self._open is not None and self.kept.total + len(data) > self.kept.limit:\n",
        "        if self._open is not None:\n",
    ),
    mutant(
        "the place is opened for an output that memory keeps whole",
        PROCESS,
        "        if self._open is not None and self.kept.total + len(data) > self.kept.limit:\n",
        "        if self._open is not None and self.kept.total + len(data) >= self.kept.limit:\n",
    ),
    mutant(
        "what memory had kept until then does not go to the place",
        PROCESS,
        "                self._save(self.kept.held())\n",
        "                pass\n",
    ),
    mutant(
        "more goes to the place than there is room for",
        PROCESS,
        "        taken = data[: self._room - self.saved]\n",
        "        taken = data\n",
    ),
    mutant(
        "a piece that was cut short is counted whole",
        PROCESS,
        "        self.saved += len(taken)\n",
        "        self.saved += len(data)\n",
    ),
    mutant(
        "a place that failed is asked again",
        PROCESS,
        '        """Leave the place alone from now on, and write down why."""\n        self._elsewhere = None\n',
        '        """Leave the place alone from now on, and write down why."""\n',
    ),
    mutant(
        "the failure of a place is not written down",
        PROCESS,
        '        logger.error("the whole output of a program %s: %s", what, error, exc_info=fault)\n',
        "        pass\n",
    ),
    mutant(
        "a fault of a place is written down without its trace",
        PROCESS,
        "        fault = None if isinstance(error, OSError) else error\n",
        "        fault = None\n",
    ),
    mutant(
        "what the system refuses a place is written down with a trace",
        PROCESS,
        "        fault = None if isinstance(error, OSError) else error\n",
        "        fault = error\n",
    ),
    mutant(
        "a fault of a place that cannot be opened loses a piece of the output",
        PROCESS,
        '            except Exception as error:\n                self._give_up("cannot be kept", error)\n',
        '            except OSError as error:\n                self._give_up("cannot be kept", error)\n',
    ),
    mutant(
        "a fault of a place that cannot be written to loses a piece of the output",
        PROCESS,
        '        except Exception as error:\n            self._give_up("could not be written", error)\n',
        '        except OSError as error:\n            self._give_up("could not be written", error)\n',
    ),
    mutant(
        "a place can be opened after the run is over",
        PROCESS,
        '        """Write nothing elsewhere from now on. Whoever opened the place is about to close it."""\n        self._open = None\n',
        '        """Write nothing elsewhere from now on. Whoever opened the place is about to close it."""\n',
    ),
    mutant(
        "a place is written to after the run is over",
        PROCESS,
        "        self._open = None\n        self._elsewhere = None\n\n    def _save",
        "        self._open = None\n\n    def _save",
    ),
    mutant(
        "a run does not say that it is over",
        PROCESS,
        "        written.end()\n",
        "        pass\n",
    ),
    mutant(
        "a limit of less than nothing is taken for what goes elsewhere",
        PROCESS,
        "        if max_overflow_bytes < 0:\n",
        "        if False:\n",
    ),
    mutant(
        "what was left out does not count as written",
        PROCESS,
        "        output_bytes=kept.total,\n",
        "        output_bytes=kept.total - kept.omitted,\n",
    ),
    mutant(
        "what went elsewhere is not counted",
        PROCESS,
        "        saved_bytes=written.saved,\n",
        "        saved_bytes=0,\n",
    ),
    mutant(
        "what goes to the error stream is put with the output although the two were to be apart",
        PROCESS,
        "        if fd == 2 and self._errors is not None:\n            self._errors.add(data)\n        else:\n            self._kept.add(data)\n",
        "        self._kept.add(data)\n",
    ),
    mutant(
        "a program whose streams are to be apart writes to one pipe all the same",
        PROCESS,
        "            stderr=subprocess.STDOUT if errors is None else subprocess.PIPE,\n",
        "            stderr=subprocess.STDOUT,\n",
    ),
    mutant(
        "the error stream may be ten times as long as the output",
        PROCESS,
        '    errors = HeadTail(max_output_bytes) if streams == "apart" else None\n',
        '    errors = HeadTail(10 * max_output_bytes) if streams == "apart" else None\n',
    ),
    mutant(
        "what was written to the error stream is not counted",
        PROCESS,
        "        error_bytes=0 if errors is None else errors.total,\n",
        "        error_bytes=0,\n",
    ),
    mutant(
        "what is left out of the error stream is not counted",
        PROCESS,
        "        error_omitted_bytes=0 if errors is None else errors.omitted,\n",
        "        error_omitted_bytes=0,\n",
    ),
    mutant(
        "the whole output is kept elsewhere although the streams are apart",
        PROCESS,
        '    if streams == "apart" and overflow is not None:\n',
        "    if False:\n",
    ),
    mutant(
        "a run does not wait for the error stream to be read to its end",
        PROCESS,
        "        self._writing = {1} if errors is None else {1, 2}\n",
        "        self._writing = {1}\n",
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
    mutant(
        "a caller is told about the tools that it may not use",
        PIPELINE,
        "        return [name for name in self.registry if only is None or name in only]\n",
        "        return list(self.registry)\n",
    ),
    mutant(
        "a tool that a caller may not use is carried out for it",
        PIPELINE,
        "        tool = self.registry.get(call.name) if only is None or call.name in only else None\n",
        "        tool = self.registry.get(call.name)\n",
    ),
    mutant(
        "a model is told nothing of the schema that a tool brings",
        TOOL,
        "        if self.input_schema is not None:\n            brought = dict(self.input_schema)\n",
        "        if False:\n            brought = dict(self.input_schema)\n",
    ),
    mutant(
        "the schema that a tool brings changes when whoever gave it changes it",
        TOOL,
        '        object.__setattr__(self, "input_schema", copy.deepcopy(dict(brought)))\n',
        "",
    ),
    mutant(
        "a schema of its own goes with a model of the parameters",
        TOOL,
        "        if parameters is not JsonArguments:\n            raise TypeError(\n",
        "        if False:\n            raise TypeError(\n",
    ),
    mutant(
        "arguments of which nothing is known need no schema for the model",
        TOOL,
        "            if parameters is JsonArguments:\n                raise TypeError(\n",
        "            if False:\n                raise TypeError(\n",
    ),
    mutant(
        "a schema that describes no object is taken for the arguments of a tool",
        TOOL,
        '        if brought.get("type") != "object":\n',
        "        if False:\n",
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
    mutant(
        "the whole of a long output goes nowhere",
        SHELL,
        "                overflow=None if place is None else place.open,\n",
        "                overflow=None,\n",
    ),
    mutant(
        "any amount of the output of a command is saved",
        SHELL,
        "                max_overflow_bytes=MAX_SAVED_BYTES,\n",
        "",
    ),
    mutant(
        "what was being saved stays open when the call is cancelled",
        SHELL,
        "        finally:\n            # However the run ended, also by the call being cancelled: it writes no more.\n            saved = None if place is None else place.close()\n",
        "        saved = None if place is None else place.close()\n",
    ),
    mutant(
        "an output that was saved in part counts as saved",
        SHELL,
        "            elif done.saved_bytes < done.output_bytes:\n",
        "            elif False:\n",
    ),
    mutant(
        "a place that took nothing is said to hold a part",
        SHELL,
        "            if saved is None or not done.saved_bytes:\n",
        "            if saved is None:\n",
    ),
    mutant(
        "an output that memory keeps whole is said not to be saved",
        SHELL,
        "        if place is not None and done.omitted_bytes:\n",
        "        if place is not None:\n",
    ),
    mutant(
        "the result does not say where the whole output is",
        SHELL,
        "                lines.append(complete.told())\n",
        "                pass\n",
    ),
    mutant(
        "nothing tells whatever bounds a result that the whole output is saved",
        SHELL,
        '        return ShellResult(output + "\\n".join(lines), complete)\n',
        '        return ShellResult(output + "\\n".join(lines), None)\n',
    ),
    mutant(
        "the whole output is called a result",
        SHELL,
        '                complete = replace(saved, what="output")\n',
        "                complete = saved\n",
    ),
    mutant(
        "the place is the one that was there when the tool was made",
        SHELL,
        "shell_tool(ctx.use(SANDBOX), shell, lambda: ctx.get(SPILL))",
        "shell_tool(ctx.use(SANDBOX), shell, lambda found=ctx.get(SPILL): found)",
    ),
    mutant(
        "a fault of a place fails the call that it was saving for",
        SHELL,
        "        except Exception as error:\n            # The call may be on its way out, cancelled. No fault of a place keeps it in.\n",
        "        except OSError as error:\n            # The call may be on its way out, cancelled. No fault of a place keeps it in.\n",
    ),
    mutant(
        "the saved output does not say which tool wrote it",
        SHELL,
        'f"{NAME}-output.txt"',
        '"output.txt"',
    ),
    mutant(
        "a fault of a place is written down by the tool without its trace",
        SHELL,
        "            fault = None if isinstance(error, OSError) else error\n",
        "            fault = None\n",
    ),
    mutant(
        "what the system refuses a place is written down by the tool with a trace",
        SHELL,
        "            fault = None if isinstance(error, OSError) else error\n",
        "            fault = error\n",
    ),
    mutant(
        "nothing is written down when the whole output cannot be saved",
        SHELL,
        '            logger.error(\n                "the whole output of a command could not be saved: %s", error, exc_info=fault\n            )\n',
        "            pass\n",
    ),
    mutant(
        "as much of an output is saved as the read tool looks at, and breaks come on top",
        SHELL,
        "MAX_SAVED_BYTES = 64 * 1024 * 1024 - 64 * 1024\n",
        "MAX_SAVED_BYTES = 64 * 1024 * 1024\n",
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
        "            elif last is None or last.data != written or not seen:",
        "            elif True:",
    ),
    mutant(
        "text of a source that replaces is put after the conversation",
        LOOP,
        '                if source.kind == "replace" and last is not None and seen:',
        "                if False:",
    ),
    mutant(
        "text of a source that appends takes the place of its old text",
        LOOP,
        '                if source.kind == "replace" and last is not None and seen:',
        "                if last is not None and seen:",
    ),
    mutant(
        "text that the model no longer sees is replaced all the same",
        LOOP,
        '                if source.kind == "replace" and last is not None and seen:',
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
    mutant(
        "text that was taken from the model is not given again while it stays the same",
        LOOP,
        "            elif last is None or last.data != written or not seen:",
        "            elif last is None or last.data != written:",
    ),
    mutant(
        "context that was taken from the model in a turn stays away",
        LOOP,
        "        if self.session.replaced != self._replaced:\n",
        "        if False:\n",
    ),
    mutant(
        "the sources of context are asked again before every request",
        LOOP,
        "        if self.session.replaced != self._replaced:\n",
        "        if True:\n",
    ),
    mutant(
        "word of something that happened is given again when context comes back",
        LOOP,
        '            if source.kind == "note" and not notes:\n                continue\n',
        "",
    ),
    mutant(
        "what the loop itself replaced makes it ask the sources again",
        LOOP,
        "        self._replaced = session.replaced\n",
        "",
    ),
    mutant(
        "a replacement of messages is not counted",
        SURFACE,
        "        self.replaced += 1\n",
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
        PROJECT,
        "            return list(reversed(up[: steps + 1]))",
        "            return list(reversed(up))",
    ),
    mutant(
        "without a repository every directory above is looked at",
        PROJECT,
        "    return [here]",
        "    return list(reversed(up))",
    ),
    mutant(
        "the repository that is furthest away is the project",
        PROJECT,
        "    for steps, above in enumerate(up):",
        "    for steps, above in reversed(list(enumerate(up))):",
    ),
    mutant(
        "what marks a repository is followed to see whether it is there",
        PROJECT,
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
    mutant(
        "the base program has no estimate of the size of a conversation",
        BASE_BUNDLE,
        "      - id: token-meter\n        plugin: columbina.token_meter\n",
        "",
    ),
    mutant(
        "the base program has no place for what is too large",
        BASE_BUNDLE,
        "      - id: spill\n        plugin: columbina.spill.local\n        config:\n          root: ${COLUMBINA_HOME}/spill\n",
        "",
    ),
    mutant(
        "the base program puts no limit on what a tool gives back",
        BASE_BUNDLE,
        "      - id: spill-policy\n        plugin: columbina.spill.policy\n",
        "",
    ),
    mutant(
        "the base program does not make a conversation shorter",
        BASE_BUNDLE,
        "      - id: compaction\n        plugin: columbina.compaction.basic\n",
        "",
    ),
    mutant(
        "the base program has no list of things to do",
        BASE_BUNDLE,
        "      - id: todo\n        plugin: columbina.todo\n",
        "",
    ),
    mutant(
        "a conversation at the terminal cannot be made shorter when the user asks",
        CLI_BUNDLE,
        "      - id: command-compact\n        plugin: columbina.commands.compact\n",
        "",
    ),
    mutant(
        "a conversation at the terminal cannot show the list of things to do",
        CLI_BUNDLE,
        "      - id: command-todos\n        plugin: columbina.commands.todos\n",
        "",
    ),
    mutant(
        "a conversation at the terminal cannot go on in a copy of its session",
        CLI_BUNDLE,
        "      - id: command-fork\n        plugin: columbina.commands.fork\n",
        "",
    ),
    mutant(
        "a conversation at the terminal is not told how much its model takes",
        CLI_BUNDLE,
        "            context_window: ${COLUMBINA_CONTEXT_WINDOW:-1048576}\n",
        "",
    ),
    mutant(
        "a run without an interface is not told how much its model takes",
        HEADLESS_BUNDLE,
        "            context_window: ${COLUMBINA_CONTEXT_WINDOW:-1048576}\n",
        "",
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
        APPS_COMMON,
        '        return "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"\n',
        '        return "(stopped)"\n',
    ),
    mutant(
        "a turn that the user stopped names the user",
        APPS_COMMON,
        '        return "(stopped)" if detail in (None, "user") else f"(stopped: {detail})"\n',
        '        return "(stopped)" if detail is None else f"(stopped: {detail})"\n',
    ),
    mutant(
        "a turn that ended with an error says nothing",
        APPS_COMMON,
        '    if end.reason == "error":\n',
        '    if end.reason == "no error":\n',
    ),
    mutant(
        "an error at the end of a turn is shown like any other end",
        CLI_TRANSCRIPT,
        '_BAD if data.reason == "error" else _NOTE',
        "_NOTE",
    ),
    mutant(
        "a reply that was cut off says nothing",
        APPS_COMMON,
        '    if end.reason == "max-tokens":\n',
        '    if end.reason == "no limit":\n',
    ),
    mutant(
        "a turn that was not allowed to go on says nothing",
        APPS_COMMON,
        '    if end.reason == "blocked":\n',
        '    if end.reason == "not blocked":\n',
    ),
    mutant(
        "a turn that the end of the program cut short says nothing",
        APPS_COMMON,
        '    if end.reason == "interrupted":\n',
        '    if end.reason == "not interrupted":\n',
    ),
    mutant(
        "a turn that was cut short where its session was copied says nothing",
        APPS_COMMON,
        '    if end.reason == "forked":\n',
        '    if end.reason == "not forked":\n',
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
    mutant(
        "a result that takes the place of another is shown as a call that ends again",
        CLI_TRANSCRIPT,
        "        if event.replaces:\n            return []\n        data = event.data\n",
        "        data = event.data\n",
    ),
    mutant(
        "the list of things to do is not shown when it is written",
        CLI_TRANSCRIPT,
        "        if isinstance(data, TodoWriteEvent):\n            return _things(data)\n",
        "",
    ),
    mutant(
        "the call that writes the list is shown with all that it writes",
        CLI_TRANSCRIPT,
        "        if data.name == TODO_TOOL and arguments is not None:\n",
        "        if False:\n",
    ),
    mutant(
        "a call for the list that cannot be read is shown by its name alone",
        CLI_TRANSCRIPT,
        "        if data.name == TODO_TOOL and arguments is not None:\n",
        "        if data.name == TODO_TOOL:\n",
    ),
    mutant(
        "a list that was emptied shows nothing",
        CLI_TRANSCRIPT,
        '    if not data.items:\n        return [_note("    (the list of things to do is empty)")]\n',
        "",
    ),
    mutant(
        "a thing that is being done does not stand out",
        CLI_TRANSCRIPT,
        '"in_progress": _DOING',
        '"in_progress": ""',
    ),
    mutant(
        "a thing that is done does not step back",
        CLI_TRANSCRIPT,
        '"completed": _NOTE}',
        '"completed": ""}',
    ),
    mutant(
        "nothing stands where a reply would grow while a conversation is made shorter",
        CLI_TRANSCRIPT,
        "            self._word = _SHORTER\n",
        "            self._word = None\n",
    ),
    mutant(
        "a conversation that is made shorter does not say why",
        CLI_TRANSCRIPT,
        "            return [] if why is None else [_note(why)]\n",
        "            return []\n",
    ),
    mutant(
        "the word about a conversation being made shorter stays when that is over",
        CLI_TRANSCRIPT,
        '            self._word = None\n            if data.why == "manual":\n',
        '            if data.why == "manual":\n',
    ),
    mutant(
        "what came of what the user asked for is told a second time",
        CLI_TRANSCRIPT,
        '            if data.why == "manual":\n                # The command that asked for it says what came of it.\n                return []\n',
        "",
    ),
    mutant(
        "what came of making a conversation shorter is not told",
        CLI_TRANSCRIPT,
        '            return [_note(f"({came[0].lower()}{came[1:]})")]\n',
        "            return []\n",
    ),
    mutant(
        "what came of making a conversation shorter begins like a sentence of its own",
        CLI_TRANSCRIPT,
        '            return [_note(f"({came[0].lower()}{came[1:]})")]\n',
        '            return [_note(f"({came})")]\n',
    ),
    mutant(
        "a conversation that the model refused says that it had come near the limit",
        CLI_TRANSCRIPT,
        '    "overflow": "(the model refused the conversation as too long. Making it shorter.)",\n',
        '    "overflow": "(the conversation has come near what the model takes. Making it shorter.)",\n',
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
        "        self._leaving = False\n        # A prompt stays on the screen with its line",
        "        # A prompt stays on the screen with its line",
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
    mutant(
        "a prompt that was put away takes the one after it off the screen as well",
        CLI_PROMPT,
        "        self._session.app.erase_when_done = False\n",
        "",
    ),
    mutant(
        "a prompt that is put away stays on the screen",
        CLI_PROMPT,
        "            app.erase_when_done = True\n            app.exit(result=None)\n",
        "            app.exit(result=None)\n",
    ),
    mutant(
        "whoever waits for a line goes on waiting when the prompt is put away",
        CLI_PROMPT,
        "            app.erase_when_done = True\n            app.exit(result=None)\n",
        "            app.erase_when_done = True\n",
    ),
    mutant(
        "a prompt that is not there cannot be put away",
        CLI_PROMPT,
        "        if app.is_running and not app.is_done:\n",
        "        if not app.is_done:\n",
    ),
    mutant(
        "a prompt cannot be put away twice",
        CLI_PROMPT,
        "        if app.is_running and not app.is_done:\n",
        "        if app.is_running:\n",
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
        "        elif self._turning:\n            agent.steer(line)\n",
        "        elif self._turning:\n            agent.followup(line)\n",
    ),
    mutant(
        "a line that is typed while the agent is at rest is said on the way",
        CLI_FRONTEND,
        "            agent.followup(line)\n            self._turning = True\n",
        "            agent.steer(line)\n            self._turning = True\n",
    ),
    mutant(
        "a command is carried out while a turn runs",
        CLI_FRONTEND,
        "            if self._turning:\n                waits =",
        "            if self._turning and not line:\n                waits =",
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
        "        if is_command(line):\n            if self._turning:",
        "        if is_command(typed):\n            if self._turning:",
        (
            "        line = line.strip()\n        if not line:\n            return\n",
            "        typed, line = line, line.strip()\n        if not line:\n            return\n",
        ),
    ),
    mutant(
        "the answer of a command is not shown",
        CLI_FRONTEND,
        "        if said:\n            # An empty line sets the answer apart from what comes after it.\n",
        "        if not said:\n            # An empty line sets the answer apart from what comes after it.\n",
    ),
    mutant(
        "the answer of a command has no empty line after it",
        CLI_FRONTEND,
        "            prompt.show([*(fit(part) for part in said.splitlines()), Text()])\n",
        "            prompt.show([fit(part) for part in said.splitlines()])\n",
    ),
    mutant(
        "a command that has nothing to say leaves an empty line",
        CLI_FRONTEND,
        "        if said:\n            # An empty line sets the answer apart from what comes after it.\n",
        "        if True:\n            # An empty line sets the answer apart from what comes after it.\n",
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
        "the agent that a conversation leaves is not closed",
        CLI_FRONTEND,
        "        self._adopt(agent, prompt)\n        await _ended(left)\n",
        "        self._adopt(agent, prompt)\n",
    ),
    mutant(
        "the agent of a conversation is closed before the one that it is to go over to is there",
        CLI_FRONTEND,
        "        agent = await opener(self._ctx, self._spec())\n        left = self._agent\n",
        "        left = self._agent\n        await _ended(left)\n        agent = await opener(self._ctx, self._spec())\n",
        (
            "        self._adopt(agent, prompt)\n        await _ended(left)\n",
            "        self._adopt(agent, prompt)\n",
        ),
    ),
    mutant(
        "a conversation does not go over to the agent that was created for it",
        CLI_FRONTEND,
        "        self._unwatch()\n        self._adopt(agent, prompt)\n        await _ended(left)\n",
        "        self._unwatch()\n        await _ended(left)\n",
    ),
    mutant(
        "the terminal does not offer to go over to another agent",
        CLI_FRONTEND,
        "    ctx.provide(SWITCH, cli.switch)\n",
        "",
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
        "    await asyncio.shield(agent.close())\n",
        "    pass\n",
    ),
    mutant(
        "the conversation goes on although the agent has stopped for good",
        CLI_FRONTEND,
        "                if self._gone.is_set():\n                    return 1\n",
        "                if self._gone.is_set():\n                    self._gone.clear()\n",
    ),
    mutant(
        "an agent that has stopped for good is taken for a user who left",
        CLI_FRONTEND,
        "                if self._gone.is_set():\n                    return 1\n",
        "                if self._gone.is_set():\n                    return 0\n",
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
        "        return status(\n            session,\n            self._model,\n            sandbox,\n            busy=self._prompt.busy,\n            mode=mode,\n            meter=self._ctx.get(TOKEN_METER),\n        )\n",
        '        return ""\n',
    ),
    mutant(
        "the line below the prompt names the model that the conversation began with",
        CLI_FRONTEND,
        "            self._model,\n            sandbox,\n",
        "            self._config.model,\n            sandbox,\n",
    ),
    mutant(
        "the line below the prompt does not go by the estimate that the program has",
        CLI_FRONTEND,
        "            meter=self._ctx.get(TOKEN_METER),\n",
        "",
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
        "    except Exception as error:\n        return error\n    return None\n",
        "    except Exception:\n        return None\n    return None\n",
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
        "        if isinstance(data, TurnStartEvent):\n            self._turning = True\n",
        "        if isinstance(data, TurnStartEvent):\n            pass\n",
    ),
    mutant(
        "what the last request sent is shown although there is an estimate",
        CLI_STATUS,
        '    sent, about = (_last_sent(session), "") if meter is None else (_held(session, meter), "~")\n',
        '    sent, about = (_last_sent(session), "")\n',
    ),
    mutant(
        "an estimate is shown as if the service had counted it",
        CLI_STATUS,
        '    sent, about = (_last_sent(session), "") if meter is None else (_held(session, meter), "~")\n',
        '    sent, about = (_last_sent(session), "") if meter is None else (_held(session, meter), "")\n',
    ),
    mutant(
        "what the service counted is marked as an estimate",
        CLI_STATUS,
        '    sent, about = (_last_sent(session), "") if meter is None else (_held(session, meter), "~")\n',
        '    sent, about = (_last_sent(session), "~") if meter is None else (_held(session, meter), "~")\n',
    ),
    mutant(
        "the conversation goes over only when the agent before it has ended",
        CLI_FRONTEND,
        "        self._unwatch()\n        self._adopt(agent, prompt)\n        await _ended(left)\n",
        "        self._unwatch()\n        await _ended(left)\n        self._adopt(agent, prompt)\n",
    ),
    mutant(
        "whoever waited for the agent before goes on waiting when the conversation has left it",
        CLI_FRONTEND,
        "        left = self._agent\n        self._unwatch()\n",
        "        left = self._agent\n",
    ),
    mutant(
        "what an agent does after the conversation left it is taken for the conversation's",
        CLI_FRONTEND,
        "            if agent is self._agent:\n                hear(heard)\n",
        "            if True:\n                hear(heard)\n",
    ),
    mutant(
        "half a line of the agent before stays above the prompt",
        CLI_FRONTEND,
        "        prompt.partial = self._transcript.partial\n        agent.ctx.on(SESSION_EVENT",
        "        agent.ctx.on(SESSION_EVENT",
    ),
    mutant(
        "an agent that is in a turn when the conversation goes over to it is taken to be at rest",
        CLI_FRONTEND,
        "        self._turning = True\n        self._mark(prompt)\n        self._watch(prompt, agent)\n\n    def _heard",
        "        self._turning = False\n        self._mark(prompt)\n        self._watch(prompt, agent)\n\n    def _heard",
    ),
    mutant(
        "nobody waits for the agent of a conversation to come to rest until it begins a turn",
        CLI_FRONTEND,
        "        self._mark(prompt)\n        self._watch(prompt, agent)\n\n    def _heard",
        "        self._mark(prompt)\n\n    def _heard",
    ),
    mutant(
        "an agent that the conversation has left says when the conversation is at rest",
        CLI_FRONTEND,
        "            if agent is self._agent:\n                self._turning = False\n",
        "            if True:\n                self._turning = False\n",
    ),
    mutant(
        "escape does not stop what a command has the agent do",
        CLI_FRONTEND,
        "        prompt.busy = self._turning or self._command is not None\n",
        "        prompt.busy = self._turning\n",
    ),
    mutant(
        "the prompt does not say that a command is being carried out when it begins",
        CLI_FRONTEND,
        "            self._command = asyncio.ensure_future(self._carry_out(prompt, agent, line))\n            self._mark(prompt)\n",
        "            self._command = asyncio.ensure_future(self._carry_out(prompt, agent, line))\n",
    ),
    mutant(
        "the prompt goes on saying that a command is carried out when it is over",
        CLI_FRONTEND,
        "                    self._command = None\n                    self._mark(prompt)\n",
        "                    self._command = None\n",
    ),
    mutant(
        "a line that is submitted while a command is carried out is taken at once",
        CLI_FRONTEND,
        "                if command is None and lines:\n",
        "                if lines:\n",
    ),
    mutant(
        "a command that is over is noticed only when a line is submitted",
        CLI_FRONTEND,
        "                waited = {typed, gone} if command is None else {typed, gone, command}\n",
        "                waited = {typed, gone}\n",
    ),
    mutant(
        "what a command let through is lost",
        CLI_FRONTEND,
        "                    command.result()\n",
        "",
    ),
    mutant(
        "a command that is being carried out is left to itself when the program leaves",
        CLI_FRONTEND,
        "            await self._drop_the_command()\n",
        "",
    ),
    mutant(
        "a command is waited for and not stopped when the program leaves",
        CLI_FRONTEND,
        "            command.cancel()\n            await asyncio.gather(command, return_exceptions=True)\n",
        "            await asyncio.gather(command, return_exceptions=True)\n",
    ),
    mutant(
        "the prompt stays on the screen when the conversation is over",
        CLI_FRONTEND,
        "                prompt.put_away()\n",
        "                pass\n",
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
        "                command.run(agent, rest.strip()), what=",
        "                command.run(agent, rest), what=",
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
        '        mark = "" if current is None else "* " if session.id == current else "  "\n',
        '        mark = "" if current is None else "  "\n',
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
        '        lines.append(f"{mark}{session.id.ljust(width)}  {_when(session.modified_ms)}  {said}")\n',
        '        lines.append(f"{mark}{session.id.ljust(width)}  {said}")\n',
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
    mutant(
        "the command that makes a conversation shorter fails on an agent that is busy",
        COMMAND_COMPACT,
        "        except AgentBusyError:\n            return _BUSY\n",
        "",
    ),
    mutant(
        "work that was stopped with the agent counts as a failure of the command",
        COMMAND_COMPACT,
        "        except cancellation.CancelledFromInside:\n            # The agent was told to stop, and the work with it. Nobody stopped this command.\n            return _STOPPED\n",
        "",
    ),
    mutant(
        "the command asks as if the conversation had come near what the model takes",
        COMMAND_COMPACT,
        'return await service.compact(agent, "manual")',
        'return await service.compact(agent, "pressure")',
    ),
    mutant(
        "one message is counted like several",
        COMMAND_COMPACT,
        '    return f"1 {one}" if count == 1 else f"{count} {several}"\n',
        '    return f"{count} {several}"\n',
    ),
    mutant(
        "a conversation that stayed as it was is said to have been made shorter",
        COMMAND_COMPACT,
        "    if not outcome.changed:\n",
        "    if False:\n",
    ),
    mutant(
        "a failure that changed nothing is told as nothing to do",
        COMMAND_COMPACT,
        '        if outcome.error is None:\n            return f"There is nothing',
        '        if True:\n            return f"There is nothing',
    ),
    mutant(
        "a summary that failed after results were cut is not mentioned",
        COMMAND_COMPACT,
        '    if outcome.error is None:\n        return f"{told} {sizes}"\n',
        '    if True:\n        return f"{told} {sizes}"\n',
    ),
    mutant(
        "results that were cut down are not mentioned",
        COMMAND_COMPACT,
        "    if outcome.trimmed:\n",
        "    if False:\n",
    ),
    mutant(
        "messages that gave way to a summary are not mentioned",
        COMMAND_COMPACT,
        "    if outcome.summarised:\n",
        "    if False:\n",
    ),
    mutant(
        "a session is copied although nothing can go over to the copy",
        COMMAND_FORK,
        "        if switch is None:\n            return _NO_WAY\n",
        "",
    ),
    mutant(
        "a session is copied while a turn runs in it",
        COMMAND_FORK,
        "        if TURN.of(session).in_turn:\n            return _RUNNING\n",
        "",
    ),
    mutant(
        "a session in which no turn has ended is copied",
        COMMAND_FORK,
        "        if not any(isinstance(event.data, TurnEndEvent) for event in session.events):\n            return _NOTHING\n",
        "",
    ),
    mutant(
        "what was done to a session since its last turn is not copied",
        COMMAND_FORK,
        "            return await agents.fork(scope, spec, session)\n",
        "            return await agents.fork(scope, spec, session, upto=len(session) - 2)\n",
    ),
    mutant(
        "the conversation goes on in a session that holds nothing",
        COMMAND_FORK,
        "            return await agents.fork(scope, spec, session)\n",
        "            return await agents.create(scope, spec)\n",
    ),
    mutant(
        "a command whose work was stopped stops whoever carries it out",
        SLASH,
        '            return await cancellation.guard(\n                command.run(agent, rest.strip()), what=f"the command {called}"\n            )\n',
        "            return await command.run(agent, rest.strip())\n",
    ),
    mutant(
        "a command whose work was stopped is told as one that failed",
        SLASH,
        '        except cancellation.CancelledFromInside:\n            return f"The command {called} was stopped."\n',
        "",
    ),
    mutant(
        "a command that was stopped is named with the capitals it was typed in",
        SLASH,
        '        called = f"{_MARK}{named.lower()}"\n',
        '        called = f"{_MARK}{named}"\n',
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
    mutant(
        "a result that is replaced later is reported a second time",
        HEADLESS,
        "        if committed.event.replaces:\n            # It takes the place of something that was reported when it happened.\n            return\n",
        "",
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
        '        if said.source != "user":\n            continue\n',
        '        if said.source == "nobody":\n            continue\n',
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
        "        if len(first) > _SHOWN_OF_AN_INPUT + 1:\n",
        "        if len(first) > 100 * _SHOWN_OF_AN_INPUT:\n",
    ),
    mutant(
        "a first input of several lines is shown with all of them",
        SESSION_STORE,
        '        first = " ".join(texts).strip().partition("\\n")[0].rstrip()\n',
        '        first = " ".join(texts).strip()\n',
    ),
    mutant(
        "a last line that was never finished makes a session unreadable",
        SESSION_STORE,
        '        if not line.endswith(b"\\n"):\n            # A last line that was never finished.\n            break\n',
        '        if not line.endswith(b"\\n"):\n            # A last line that was never finished.\n            pass\n',
    ),
    mutant(
        "the first user message is looked for only in the first event",
        SESSION_STORE,
        '        if event.get("type") != UserMessageEvent.event_type:\n            continue\n',
        '        if event.get("type") != UserMessageEvent.event_type:\n            break\n',
    ),
    mutant(
        "a file that does not begin like a session is listed",
        SESSION_STORE,
        "        header = decode_header(file.readline())\n        return header, _first_input(file)\n",
        '        file.readline()\n        return SessionHeader(id=path.parent.name, created_at=0, cwd=""), _first_input(file)\n',
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
        "        return stored_sessions(self.root / workspace_key(cwd), delegated=delegated)\n",
        '        return stored_sessions(self.root / workspace_key("."), delegated=delegated)\n',
    ),
    mutant(
        "the sessions of subagents are listed among the conversations",
        SESSION_STORE,
        "        if header.delegation_depth == 0 or delegated:\n",
        "        if True:\n",
    ),
    mutant(
        "whoever asks for every session gets the conversations only",
        SESSION_STORE,
        "        if header.delegation_depth == 0 or delegated:\n",
        "        if header.delegation_depth == 0:\n",
    ),
    mutant(
        "a session in a list does not say how deep it was delegated",
        SESSION_STORE,
        "            listed.append(Listed(entry.name, modified, first, header.delegation_depth))\n",
        "            listed.append(Listed(entry.name, modified, first))\n",
    ),
    mutant(
        "a session forgets how deep it was delegated",
        SESSION_SERVICE,
        "                delegation_depth=depth,\n",
        "",
    ),
    mutant(
        "a copy of a session that was delegated counts as delegated itself",
        SESSION_SERVICE,
        "        session = self._start(ctx, cwd, session_id, parent, 0, events)\n",
        "        session = self._start(ctx, cwd, session_id, parent, 1, events)\n",
    ),
    mutant(
        "whoever lists the sessions of a directory gets those of subagents as well",
        SESSION_SERVICE,
        "        return stored_sessions(self.root / workspace_key(cwd), delegated=delegated)\n",
        "        return stored_sessions(self.root / workspace_key(cwd), delegated=True)\n",
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

_METER = (
    mutant(
        "a space costs what a letter costs",
        TOKEN_METER,
        '    "space": 0.15,\n',
        '    "space": 0.24,\n',
    ),
    mutant(
        "a digit costs what a letter costs",
        TOKEN_METER,
        '    "digit": 0.50,\n',
        '    "digit": 0.24,\n',
    ),
    mutant(
        "a sign costs what a letter costs",
        TOKEN_METER,
        '    "sign": 0.70,\n',
        '    "sign": 0.24,\n',
    ),
    mutant(
        "a letter in a long run costs what any letter costs",
        TOKEN_METER,
        '    "dense": 0.85,\n',
        '    "dense": 0.24,\n',
    ),
    mutant(
        "a wide character costs a whole token",
        TOKEN_METER,
        '    "wide": 0.62,\n',
        '    "wide": 1.0,\n',
    ),
    mutant(
        "a run of letters and digits is long one character later",
        TOKEN_METER,
        "DENSE_RUN = 24\n",
        "DENSE_RUN = 25\n",
    ),
    mutant(
        "a run of letters and digits is long one character sooner",
        TOKEN_METER,
        "DENSE_RUN = 24\n",
        "DENSE_RUN = 23\n",
    ),
    mutant(
        "a message costs nothing beside its content",
        TOKEN_METER,
        "PER_MESSAGE = 3\n",
        "PER_MESSAGE = 0\n",
    ),
    mutant(
        "declaring tools costs nothing beside what is written about them",
        TOKEN_METER,
        "TOOLS_DECLARED = 210\n",
        "TOOLS_DECLARED = 0\n",
    ),
    mutant(
        "a part of a token counts as none",
        TOKEN_METER,
        "    return -(-hundredths // 100)\n",
        "    return hundredths // 100\n",
    ),
    mutant(
        "the digits of a long run count as its letters too",
        TOKEN_METER,
        "    dense = sum(len(run) - _without(run, _DIGITS) for run in _RUN.findall(raw))\n",
        "    dense = sum(len(run) for run in _RUN.findall(raw))\n",
    ),
    mutant(
        "no run of letters and digits is long",
        TOKEN_METER,
        "    dense = sum(len(run) - _without(run, _DIGITS) for run in _RUN.findall(raw))\n",
        "    dense = 0\n",
    ),
    mutant(
        "a letter counts as a sign",
        TOKEN_METER,
        '        "sign": len(raw) - beyond - digits - spaces - letters,\n',
        '        "sign": len(raw) - beyond - digits - spaces - dense,\n',
    ),
    mutant(
        "a character beyond ASCII counts as a sign as well",
        TOKEN_METER,
        '        "sign": len(raw) - beyond - digits - spaces - letters,\n',
        '        "sign": len(raw) - digits - spaces - letters,\n',
    ),
    mutant(
        "a wide character counts as another one as well",
        TOKEN_METER,
        '        "other": beyond - wide,\n',
        '        "other": beyond,\n',
    ),
    mutant(
        "no character is wide",
        TOKEN_METER,
        '    wide = 0 if not beyond else len(text) - len(_WIDE.sub("", text))\n',
        "    wide = 0\n",
    ),
    mutant(
        "the reasoning of a reply is not counted",
        TOKEN_METER,
        "            total += estimate(block.text)\n",
        "            total += estimate(block.text) if isinstance(block, TextBlock) else 0\n",
    ),
    mutant(
        "the arguments of a call are not counted",
        TOKEN_METER,
        "    return estimate(block.name) + estimate(block.arguments) + estimate(block.call_id)\n",
        "    return estimate(block.name) + estimate(block.call_id)\n",
    ),
    mutant(
        "the id of the call is not counted with a result",
        TOKEN_METER,
        "    if isinstance(message, ToolMessage):\n        total += estimate(message.call_id)\n",
        "",
    ),
    mutant(
        "no tools cost what declaring tools costs",
        TOKEN_METER,
        "    if not tools:\n        return 0\n",
        "",
    ),
    mutant(
        "what is said about a tool is not counted",
        TOKEN_METER,
        '            {"name": tool.name, "description": tool.description, "parameters": tool.parameters}\n',
        '            {"name": tool.name, "parameters": tool.parameters}\n',
    ),
    mutant(
        "the tools are not counted with the messages",
        TOKEN_METER,
        "        estimated = ledger.messages + ledger.tools\n",
        "        estimated = ledger.messages\n",
    ),
    mutant(
        "the count for one model is applied to another",
        TOKEN_METER,
        "        rate = ledger.rate if ledger.rate_of == ledger.model else None\n",
        "        rate = ledger.rate\n",
    ),
    mutant(
        "the estimate is not put right by what the provider counted",
        TOKEN_METER,
        "            tokens=estimated if rate is None else round(estimated * rate),\n",
        "            tokens=estimated,\n",
    ),
    mutant(
        "a reply that was cut short tells what its request held",
        TOKEN_METER,
        "        if isinstance(data, AssistantMessageEvent) and data.finish_reason is not None:\n",
        "        if isinstance(data, AssistantMessageEvent):\n",
    ),
    mutant(
        "what came from the cache was not sent",
        TOKEN_METER,
        "            sent = (\n                data.usage.input_tokens\n                + data.usage.cache_read_tokens\n                + data.usage.cache_write_tokens\n            )\n",
        "            sent = data.usage.input_tokens\n",
    ),
    mutant(
        "the tools were not a part of what the provider counted",
        TOKEN_METER,
        "            before = ledger.messages + ledger.tools\n",
        "            before = ledger.messages\n",
    ),
    mutant(
        "a count of a third of the estimate is believed",
        TOKEN_METER,
        "_BELIEVED = (0.5, 2.0)\n",
        "_BELIEVED = (0.3, 2.0)\n",
    ),
    mutant(
        "a count of three times the estimate is believed",
        TOKEN_METER,
        "_BELIEVED = (0.5, 2.0)\n",
        "_BELIEVED = (0.5, 3.0)\n",
    ),
    mutant(
        "a count that is not believed takes the one before it away",
        TOKEN_METER,
        "            if _BELIEVED[0] <= rate <= _BELIEVED[1]:\n                ledger.rate, ledger.rate_of = rate, ledger.model\n",
        "            believed = _BELIEVED[0] <= rate <= _BELIEVED[1]\n            ledger.rate, ledger.rate_of = (rate, ledger.model) if believed else (None, None)\n",
    ),
    mutant(
        "a message that was replaced goes on counting",
        TOKEN_METER,
        "                ledger.messages -= ledger.prices.pop(seq)\n",
        "                ledger.prices.pop(seq)\n",
    ),
    mutant(
        "the header of a request does not say which model counts",
        TOKEN_METER,
        "            ledger.model = (data.config.provider, data.config.model)\n",
        '            ledger.model = (data.config.provider, "")\n',
    ),
)

_SPILL = (
    mutant(
        "a line may be one character longer than the read tool shows",
        SPILL_LOCAL,
        "FOLD_AT = 2000\n",
        "FOLD_AT = 2001\n",
    ),
    mutant(
        "a name is taken with whatever it holds",
        SPILL_LOCAL,
        '    fit = _UNFIT.sub("_", name)[:_NAME_CHARACTERS]\n',
        "    fit = name[:_NAME_CHARACTERS]\n",
    ),
    mutant(
        "a name is taken at whatever length",
        SPILL_LOCAL,
        '    fit = _UNFIT.sub("_", name)[:_NAME_CHARACTERS]\n',
        '    fit = _UNFIT.sub("_", name)\n',
    ),
    mutant(
        "a name of nothing but dots is taken",
        SPILL_LOCAL,
        '    return fit if re.search("[A-Za-z0-9]", fit) else "output"\n',
        "    return fit\n",
    ),
    mutant(
        "a line is broken one character before it has to be",
        SPILL_LOCAL,
        "            end = min(at + max(FOLD_AT - self._length, 0), len(line))\n",
        "            end = min(at + max(FOLD_AT - self._length - 1, 0), len(line))\n",
    ),
    mutant(
        "no line is broken",
        SPILL_LOCAL,
        "            if self._length + len(line) > FOLD_AT:\n",
        "            if False:\n",
    ),
    mutant(
        "a file that is there is written over",
        SPILL_LOCAL,
        '        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)\n',
        '        flags = os.O_WRONLY | os.O_CREAT | getattr(os, "O_BINARY", 0)\n',
    ),
    mutant(
        "a line break is written the way the system writes one",
        SPILL_LOCAL,
        '        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)\n',
        "        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL\n",
    ),
    mutant(
        "what was written is counted in pieces",
        SPILL_LOCAL,
        "        self._written += len(data)\n",
        "        self._written += 1\n",
    ),
    mutant(
        "a file that was closed is written to",
        SPILL_LOCAL,
        '        if self._saved is not None:\n            raise ValueError(f"{self.locator} is closed")\n',
        "",
    ),
    mutant(
        "a file is closed as often as it is asked",
        SPILL_LOCAL,
        "        if self._saved is None:\n            try:\n",
        "        if True:\n            try:\n",
    ),
    mutant(
        "half a text stays where it was begun",
        SPILL_LOCAL,
        "            with contextlib.suppress(OSError):\n                Path(file.locator).unlink()\n",
        "",
    ),
    mutant(
        "a character that cannot be written keeps the text from being saved",
        SPILL_LOCAL,
        '            file.write(text.encode("utf-8", errors="replace"))\n',
        '            file.write(text.encode("utf-8"))\n',
    ),
    mutant(
        "nobody is told that lines were broken",
        SPILL_LOCAL,
        "                hint = _HINT + _FOLDED if self._fold.broken else _HINT\n",
        "                hint = _HINT\n",
    ),
    mutant(
        "the files of all sessions lie in one directory",
        SPILL_LOCAL,
        "        directory = workspace / session.id\n",
        "        directory = workspace\n",
    ),
    mutant(
        "only the last directory is made for its owner alone",
        SPILL_LOCAL,
        "        for level in (self.root, workspace, directory):\n",
        "        for level in (directory,):\n",
    ),
    mutant(
        "the directories are made for everybody",
        SPILL_LOCAL,
        "            level.mkdir(mode=0o700, parents=True, exist_ok=True)\n",
        "            level.mkdir(parents=True, exist_ok=True)\n",
    ),
    mutant(
        "a file is made for everybody",
        SPILL_LOCAL,
        "        self._descriptor = os.open(path, flags, 0o600)\n",
        "        self._descriptor = os.open(path, flags)\n",
    ),
    mutant(
        "a name that is taken is not drawn anew",
        SPILL_LOCAL,
        "        for _ in range(_DRAWS):\n",
        "        for _ in range(1):\n",
    ),
    mutant(
        "the files of all working directories lie in one directory",
        SPILL_LOCAL,
        "        return self.root / workspace_key(cwd)\n",
        '        return self.root / "all"\n',
    ),
    mutant(
        "what was saved may be written by the model",
        SPILL_LOCAL,
        '        if access.kind != "read":\n            return False\n',
        "",
    ),
    mutant(
        "the directory of what was saved cannot be listed",
        SPILL_LOCAL,
        "        return directory == access.where.path or directory in access.where.path.parents\n",
        "        return directory in access.where.path.parents\n",
    ),
    mutant(
        "what was saved for any working directory may be read",
        SPILL_LOCAL,
        "        directory = Path(os.path.realpath(self.directory(access.cwd)))\n",
        "        directory = Path(os.path.realpath(self.root))\n",
    ),
    mutant(
        "the newer files are removed and the older ones are kept",
        SPILL_LOCAL,
        "                    if entry.is_file(follow_symlinks=False) and status.st_mtime < before:\n",
        "                    if entry.is_file(follow_symlinks=False) and status.st_mtime > before:\n",
    ),
    mutant(
        "a file is kept for seconds and not for days",
        SPILL_LOCAL,
        "    before = time.time() - keep_days * _DAY\n",
        "    before = time.time() - keep_days\n",
    ),
    mutant(
        "the files that were removed are not counted",
        SPILL_LOCAL,
        "                        removed += 1\n",
        "                        pass\n",
    ),
    mutant(
        "the directory of a session stays when nothing is left in it",
        SPILL_LOCAL,
        "            with contextlib.suppress(OSError):\n                Path(session).rmdir()\n",
        "",
    ),
    mutant(
        "the directory of a working directory stays when nothing is left in it",
        SPILL_LOCAL,
        "        with contextlib.suppress(OSError):\n            Path(workspace).rmdir()\n",
        "",
    ),
    mutant(
        "files are kept for a day longer",
        SPILL_LOCAL,
        "    keep_days: float = Field(default=30, gt=0)\n",
        "    keep_days: float = Field(default=31, gt=0)\n",
    ),
    mutant(
        "old files that cannot be looked through keep nothing from being said",
        SPILL_LOCAL,
        '        logger.exception("the old files under %s could not be looked through", root)\n',
        "        pass\n",
    ),
    mutant(
        "the model is not let read what was saved",
        SPILL_LOCAL,
        '        seam.grants.add(ctx, "spill", store.grants)\n',
        "        pass\n",
    ),
    mutant(
        "old files are not removed when the program starts",
        SPILL_LOCAL,
        "        removed = await asyncio.to_thread(sweep, root, keep_days=keep_days)\n",
        "        removed = 0\n",
    ),
    mutant(
        "the length of a line goes on after a line break",
        SPILL_LOCAL,
        "            if at:\n                self._length = 0\n",
        "",
    ),
    mutant(
        "a carriage return before a line break makes a line too long",
        SPILL_LOCAL,
        '            if self._length + end - at == FOLD_AT and line.startswith("\\r", end):\n',
        "            if False:\n",
    ),
    mutant(
        "carriage returns are let through one after another",
        SPILL_LOCAL,
        '            if self._length + end - at == FOLD_AT and line.startswith("\\r", end):\n',
        '            if line.startswith("\\r", end):\n',
    ),
    mutant(
        "a line is broken and nobody knows",
        SPILL_LOCAL,
        "            self.broken = True\n",
        "            pass\n",
    ),
    mutant(
        "a byte that is no text is replaced by a sign",
        SPILL_LOCAL,
        '        self._decoder = _Utf8(errors="surrogateescape")\n',
        '        self._decoder = _Utf8(errors="replace")\n',
    ),
    mutant(
        "half a character at the very end is dropped",
        SPILL_LOCAL,
        '                self._take(self._decoder.decode(b"", True))\n',
        "                pass\n",
    ),
    mutant(
        "a character that comes in two pieces counts as several",
        SPILL_LOCAL,
        "        self._take(self._decoder.decode(data))\n",
        '        self._take(data.decode("utf-8", errors="surrogateescape"))\n',
    ),
    mutant(
        "a file that the system failed to close is closed again",
        SPILL_LOCAL,
        "                self._saved = Saved(self.locator, self._written, hint)\n                os.close(self._descriptor)\n",
        "                os.close(self._descriptor)\n                self._saved = Saved(self.locator, self._written, hint)\n",
    ),
)

_CAP = (
    mutant(
        "a result may be one token larger",
        SPILL_POLICY,
        "    max_tokens: int = Field(default=12_500, gt=0)\n",
        "    max_tokens: int = Field(default=12_501, gt=0)\n",
    ),
    mutant(
        "a tool that hands out pages is cut like any other",
        SPILL_POLICY,
        "        if not isinstance(verdict, Accept) or executed.invocation.tool.paged:\n",
        "        if not isinstance(verdict, Accept):\n",
    ),
    mutant(
        "a result of exactly the limit is cut",
        SPILL_POLICY,
        "        too_large = estimate(text) > config.max_tokens\n",
        "        too_large = estimate(text) >= config.max_tokens\n",
    ),
    mutant(
        "no result is too large",
        SPILL_POLICY,
        "        too_large = estimate(text) > config.max_tokens\n",
        "        too_large = False\n",
    ),
    mutant(
        "a result whose whole was saved by its tool is saved again",
        SPILL_POLICY,
        "        if isinstance(complete, Saved):\n",
        "        if False:\n",
    ),
    mutant(
        "a small result with a hole in it does not say where the whole is",
        SPILL_POLICY,
        '        if not too_large:\n            between = "" if text.endswith("\\n") else "\\n"\n            return Accept(text=f"{text}{between}{last}")\n',
        "        if not too_large:\n            return verdict\n",
    ),
    mutant(
        "each end gets all the room there is",
        SPILL_POLICY,
        "    half = max(room // 2, 0)\n",
        "    half = max(room, 0)\n",
    ),
    mutant(
        "the last line is not counted against the room",
        SPILL_POLICY,
        "    room = tokens - estimate(last) - estimate(widest) - 2\n",
        "    room = tokens - estimate(widest) - 2\n",
    ),
    mutant(
        "the beginning is cut where the room ends, also in the middle of a line",
        SPILL_POLICY,
        '    broken = text.rfind("\\n", low - low // 5, low)\n',
        "    broken = -1\n",
    ),
    mutant(
        "the beginning goes back to a line break however far that is",
        SPILL_POLICY,
        '    broken = text.rfind("\\n", low - low // 5, low)\n',
        '    broken = text.rfind("\\n", 0, low)\n',
    ),
    mutant(
        "the end is cut where the room begins, also in the middle of a line",
        SPILL_POLICY,
        '    broken = text.find("\\n", start, start + low // 5)\n',
        "    broken = -1\n",
    ),
    mutant(
        "the end goes on to a line break however far that is",
        SPILL_POLICY,
        '    broken = text.find("\\n", start, start + low // 5)\n',
        '    broken = text.find("\\n", start)\n',
    ),
    mutant(
        "the line between the ends says how long the text is",
        SPILL_POLICY,
        "    gap = _left_out(len(text) - len(head) - len(tail)) if counted else _UNCOUNTED\n",
        "    gap = _left_out(len(text)) if counted else _UNCOUNTED\n",
    ),
    mutant(
        "the end is taken from the whole text and may hold the beginning again",
        SPILL_POLICY,
        "    tail = _end(text[len(head) :], half)\n",
        "    tail = _end(text, half)\n",
    ),
    mutant(
        "a result that cannot be saved is said to be nowhere",
        SPILL_POLICY,
        '            return f"(The whole result could not be saved: {error.strerror or error}.)"\n',
        "            return _NOWHERE\n",
    ),
    mutant(
        "nothing is logged when a result cannot be saved",
        SPILL_POLICY,
        '            logger.error("a result of %s could not be saved: %s", named, error)\n',
        "            pass\n",
    ),
    mutant(
        "what is saved is named after nothing",
        SPILL_POLICY,
        '        named = f"{executed.invocation.tool.name}-result.txt"\n',
        '        named = "result.txt"\n',
    ),
    mutant(
        "the limit comes before the other checks of a result",
        SPILL_POLICY,
        "    ctx.on(POST_EXECUTE, within_the_limit, prepend=True)\n",
        "    ctx.on(POST_EXECUTE, within_the_limit)\n",
    ),
    mutant(
        "what the place says about how to read it is left out",
        SPILL_SEAM,
        'f"(The whole {self.what} has {self.bytes} bytes and is in {self.locator}. {self.hint})"',
        'f"(The whole {self.what} has {self.bytes} bytes and is in {self.locator}.)"',
    ),
    mutant(
        "what it is the whole of is not said",
        SPILL_SEAM,
        'f"(The whole {self.what} has {self.bytes} bytes and is in {self.locator}. {self.hint})"',
        'f"(The whole result has {self.bytes} bytes and is in {self.locator}. {self.hint})"',
    ),
    mutant(
        "what a tool said about the whole is said again",
        SPILL_POLICY,
        "            text = text.removesuffix(last)\n",
        "            pass\n",
    ),
    mutant(
        "a number is given of what a tool had left out",
        SPILL_POLICY,
        "            counted = False\n",
        "            counted = True\n",
    ),
    mutant(
        "the line between the ends never gives a number",
        SPILL_POLICY,
        "    gap = _left_out(len(text) - len(head) - len(tail)) if counted else _UNCOUNTED\n",
        "    gap = _UNCOUNTED\n",
    ),
)

_STORED_SESSIONS = (
    mutant(
        "what the user said on the way is left out of a turn that is told",
        APPS_SESSIONS,
        '_MARKS = {"user": ">", "steer": ">>"}',
        '_MARKS = {"user": ">"}',
    ),
    mutant(
        "what the user said on the way is told like a task",
        APPS_SESSIONS,
        '"steer": ">>"',
        '"steer": ">"',
    ),
    mutant(
        "a reply without words takes the place of the last answer",
        APPS_SESSIONS,
        "        elif isinstance(data, AssistantMessageEvent) and data.message.text.strip():\n",
        "        elif isinstance(data, AssistantMessageEvent):\n",
    ),
    mutant(
        "a turn that came to its end is said not to have",
        APPS_SESSIONS,
        "            ended = how_it_ended(data)\n",
        "            pass\n",
    ),
    mutant(
        "a turn that is still open says nothing about that",
        APPS_SESSIONS,
        '    ended: str | None = "(the turn did not come to its end)"\n',
        "    ended: str | None = None\n",
    ),
    mutant(
        "a long answer does not say that it had more",
        APPS_SESSIONS,
        "    if len(answer) > _ANSWER_LINES:\n",
        "    if False:\n",
    ),
    mutant(
        "an answer of just as many lines as are shown is said to have more",
        APPS_SESSIONS,
        "    if len(answer) > _ANSWER_LINES:\n",
        "    if len(answer) >= _ANSWER_LINES:\n",
    ),
    mutant(
        "a long answer is told whole",
        APPS_SESSIONS,
        "    lines.extend(answer[-_ANSWER_LINES:])\n",
        "    lines.extend(answer)\n",
    ),
    mutant(
        "one line before the end of an answer is told as one lines",
        APPS_SESSIONS,
        "{'line' if earlier == 1 else 'lines'}",
        "{'lines'}",
    ),
    mutant(
        "a copy does not say what it is a copy of",
        APPS_SESSIONS,
        '        lines.append(f"{what} the session {header.parent_session}")\n',
        "        pass\n",
    ),
    mutant(
        "a session without turns says nothing about that",
        APPS_SESSIONS,
        '    if not starts:\n        lines.append("(no turn has begun in this session)")\n',
        "",
    ),
    mutant(
        "the turns of a session are counted from nothing",
        APPS_SESSIONS,
        "enumerate(zip(starts, stops, strict=False), start=1)",
        "enumerate(zip(starts, stops, strict=False))",
    ),
    mutant(
        "a turn is told with the turns that follow it",
        APPS_SESSIONS,
        "    stops = [*starts[1:], len(stored.events)]\n",
        "    stops = [len(stored.events)] * len(starts)\n",
    ),
    mutant(
        "a copy holds the turn that was under way",
        APPS_SESSIONS,
        "        copy = sessions.fork(scope, cwd, source, upto=ends[-1], session_id=session_id)\n",
        "        copy = sessions.fork(scope, cwd, source, session_id=session_id)\n",
    ),
    mutant(
        "a copy ends where the first turn of the session ends",
        APPS_SESSIONS,
        "        copy = sessions.fork(scope, cwd, source, upto=ends[-1], session_id=session_id)\n",
        "        copy = sessions.fork(scope, cwd, source, upto=ends[0], session_id=session_id)\n",
    ),
    mutant(
        "a copy is given an id of its own although one was asked for",
        APPS_SESSIONS,
        "        copy = sessions.fork(scope, cwd, source, upto=ends[-1], session_id=session_id)\n",
        "        copy = sessions.fork(scope, cwd, source, upto=ends[-1])\n",
    ),
    mutant(
        "a session is listed when it was to be shown",
        APPS_SESSIONS,
        "    if what == LIST:\n",
        "    if what in (LIST, SHOW):\n",
    ),
    mutant(
        "a session is copied when it was to be shown",
        APPS_SESSIONS,
        "    if what == SHOW:\n        return shown(sessions.read(cwd, source))\n",
        "",
    ),
    mutant(
        "a list that no conversation asked for keeps a place for the star",
        COMMAND_SESSIONS,
        '        mark = "" if current is None else "* " if session.id == current else "  "\n',
        '        mark = "* " if session.id == current else "  "\n',
    ),
    mutant(
        "a list that no conversation asked for speaks of a star",
        COMMAND_SESSIONS,
        '    if current is None:\n        lines.append("A session is taken up with --session-id.")\n    else:\n',
        "    if True:\n",
    ),
    mutant(
        "what a session holds is printed as it was written",
        LAUNCHER,
        "        print(harmless(told))\n",
        "        print(told)\n",
    ),
    mutant(
        "an id that cannot name a session ends the program with a trace",
        LAUNCHER,
        "        except (SessionError, ValueError) as error:\n",
        "        except SessionError as error:\n",
    ),
    mutant(
        "a session that cannot be read ends the program with a trace",
        LAUNCHER,
        "        except (SessionError, ValueError) as error:\n",
        "        except ValueError as error:\n",
    ),
    mutant(
        "sessions that could not be looked at count as looked at",
        LAUNCHER,
        "            _tell(str(error))\n            return 2\n        # What a session holds",
        "            _tell(str(error))\n            return 0\n        # What a session holds",
    ),
    mutant(
        "why sessions could not be looked at is not said",
        LAUNCHER,
        "            _tell(str(error))\n            return 2\n        # What a session holds",
        "            return 2\n        # What a session holds",
    ),
    mutant(
        "the sessions of the current directory are looked at, whichever was named",
        LAUNCHER,
        "    cwd = Path() if args.cwd is None else Path(args.cwd)\n",
        "    cwd = Path()\n",
    ),
    mutant(
        "the directory whose sessions are meant has to be named",
        LAUNCHER,
        "    cwd = Path() if args.cwd is None else Path(args.cwd)\n",
        "    cwd = Path(args.cwd)\n",
    ),
    mutant(
        "all of a profile is loaded to look at its sessions",
        LAUNCHER,
        "        if entry.id in _KNOW_THE_LOG and not entry.disabled\n",
        "        if not entry.disabled\n",
    ),
    mutant(
        "an entry that is switched off is loaded to look at sessions",
        LAUNCHER,
        "        if entry.id in _KNOW_THE_LOG and not entry.disabled\n",
        "        if entry.id in _KNOW_THE_LOG\n",
    ),
    mutant(
        "a profile that stores no sessions is looked at all the same",
        LAUNCHER,
        "    if _KNOW_THE_LOG[0] not in [entry.id for entry in entries]:\n",
        "    if False:\n",
    ),
    mutant(
        "an entry that is called session and stores none is taken at its word",
        LAUNCHER,
        "        if root.get(SESSIONS) is None:\n            # What an entry is called",
        "        if False:\n            # What an entry is called",
    ),
    mutant(
        "a profile that stores no sessions is told as a success",
        LAUNCHER,
        '                "there are none to look at"\n            )\n            return 2\n',
        '                "there are none to look at"\n            )\n            return 0\n',
    ),
    mutant(
        "a session that was delegated is told as a copy",
        APPS_SESSIONS,
        '        what = "delegated by" if header.delegation_depth else "a copy of"\n',
        '        what = "a copy of"\n',
    ),
    mutant(
        "a copy is told as a session that was delegated",
        APPS_SESSIONS,
        '        what = "delegated by" if header.delegation_depth else "a copy of"\n',
        '        what = "delegated by"\n',
    ),
)

_TODO = (
    mutant(
        "a list that the model writes is not kept",
        TODO,
        "        run.session.append(TodoWriteEvent(call_id=run.call_id, items=items))\n",
        "        pass\n",
    ),
    mutant(
        "a new list is added to the old one",
        TODO,
        "        return Todos(items=data.items, written_at=event.seq)\n",
        "        return Todos(items=(*state.items, *data.items), written_at=event.seq)\n",
    ),
    mutant(
        "a list forgets where it was written",
        TODO,
        "        return Todos(items=data.items, written_at=event.seq)\n",
        "        return Todos(items=data.items)\n",
    ),
    mutant(
        "a thing that is in progress is marked like one that waits",
        TODO,
        '"in_progress": "[>]"',
        '"in_progress": "[ ]"',
    ),
    mutant(
        "a thing that is done is marked like one that waits",
        TODO,
        '"completed": "[x]"',
        '"completed": "[ ]"',
    ),
    mutant(
        "a list that was emptied is said to hold nothing of each kind",
        TODO,
        '    if not items:\n        return "The list is empty now."\n',
        "",
    ),
    mutant(
        "the things of one state are counted as the things of the others",
        TODO,
        "sum(item.status == status for item in items)",
        "sum(item.status != status for item in items)",
    ),
    mutant(
        "one thing is told as one things",
        TODO,
        '    things = "1 thing" if len(items) == 1 else f"{len(items)} things"\n',
        '    things = f"{len(items)} things"\n',
    ),
    mutant(
        "what is done is told as what waits",
        TODO,
        "_counted(counts['completed'], 'completed')",
        "_counted(counts['pending'], 'completed')",
    ),
    mutant(
        "what is in progress is told as what waits",
        TODO,
        "_counted(counts['in_progress'], 'in progress')",
        "_counted(counts['pending'], 'in progress')",
    ),
    mutant(
        "what waits is told as what is done",
        TODO,
        "_counted(counts['pending'], 'pending')",
        "_counted(counts['completed'], 'pending')",
    ),
    mutant(
        "a list may be as long as the model likes",
        TODO,
        "    if len(args.items) > MAX_ITEMS:\n",
        "    if False:\n",
    ),
    mutant(
        "a list of just the size that is kept is refused",
        TODO,
        "    if len(args.items) > MAX_ITEMS:\n",
        "    if len(args.items) >= MAX_ITEMS:\n",
    ),
    mutant(
        "the things of a list are counted from nothing where one says nothing",
        TODO,
        "enumerate(args.items, start=1)",
        "enumerate(args.items)",
    ),
    mutant(
        "the space around the words of a thing is kept",
        TODO,
        '        content = " ".join(thing.content.split())\n',
        "        content = thing.content\n",
    ),
    mutant(
        "a thing may go over several lines",
        TODO,
        '        content = " ".join(thing.content.split())\n',
        "        content = thing.content.strip()\n",
    ),
    mutant(
        "a thing that says nothing is kept",
        TODO,
        "        if not content:\n",
        "        if False:\n",
    ),
    mutant(
        "a thing may be as long as the model likes",
        TODO,
        "        if len(content) > MAX_CHARACTERS:\n",
        "        if False:\n",
    ),
    mutant(
        "a thing of just the length that is kept is refused",
        TODO,
        "        if len(content) > MAX_CHARACTERS:\n",
        "        if len(content) >= MAX_CHARACTERS:\n",
    ),
    mutant(
        "the things in progress are counted from nothing",
        TODO,
        "enumerate(items, start=1)",
        "enumerate(items)",
    ),
    mutant(
        "several things may be in progress at once",
        TODO,
        "    if len(doing) > 1:\n",
        "    if False:\n",
    ),
    mutant(
        "one thing in progress is one too many",
        TODO,
        "    if len(doing) > 1:\n",
        "    if len(doing) > 0:\n",
    ),
    mutant(
        "a model is told its list although it sees the call that wrote it",
        TODO,
        "    if not todos.items or todos.written_at is None or _seen(session, todos.written_at):\n",
        "    if not todos.items or todos.written_at is None:\n",
    ),
    mutant(
        "a list that was emptied is told again",
        TODO,
        "    if not todos.items or todos.written_at is None or _seen(session, todos.written_at):\n",
        "    if todos.written_at is None or _seen(session, todos.written_at):\n",
    ),
    mutant(
        "the reply that came last passes for the one that wrote the list",
        TODO,
        "        for seq in range(written_at - 1, -1, -1)\n",
        "        for seq in range(len(events) - 1, -1, -1)\n",
    ),
    mutant(
        "the list that is told again lacks its things",
        TODO,
        "        *shown(todos.items),\n",
        "",
    ),
    mutant(
        "the event of the list is not declared",
        TODO,
        "    ctx.use(SESSIONS).declare(ctx, TodoWriteEvent)\n",
        "",
    ),
    mutant(
        "a list that is told again stands beside the one that was told before",
        TODO,
        'ContextSource(_remembered, order=150, kind="replace")',
        'ContextSource(_remembered, order=150, kind="append")',
    ),
    mutant(
        "the list is told before what is told about the run",
        TODO,
        'ContextSource(_remembered, order=150, kind="replace")',
        'ContextSource(_remembered, order=50, kind="replace")',
    ),
    mutant(
        "the command shows a list that was emptied as no line at all",
        COMMAND_TODOS,
        '    return "\\n".join(shown(items)) if items else _NOTHING\n',
        '    return "\\n".join(shown(items))\n',
    ),
    mutant(
        "the command says that nothing was written down, whatever was",
        COMMAND_TODOS,
        '    return "\\n".join(shown(items)) if items else _NOTHING\n',
        "    return _NOTHING\n",
    ),
    mutant(
        "a list whose call the model no longer sees is not told again",
        TODO,
        "    return reply in session.surface\n",
        "    return True\n",
    ),
)

_COMPACTION = (
    mutant(
        "a reader must know the event that begins a compaction",
        COMPACTION_SEAM,
        '    event_type: ClassVar[str] = "compaction/start"\n    ignorable: ClassVar[bool] = True\n',
        '    event_type: ClassVar[str] = "compaction/start"\n',
    ),
    mutant(
        "a reader must know the event that keeps a summary",
        COMPACTION_SEAM,
        '    event_type: ClassVar[str] = "compaction/summary"\n    ignorable: ClassVar[bool] = True\n',
        '    event_type: ClassVar[str] = "compaction/summary"\n',
    ),
    mutant(
        "a reader must know the event that ends a compaction",
        COMPACTION_SEAM,
        '    event_type: ClassVar[str] = "compaction/end"\n    ignorable: ClassVar[bool] = True\n',
        '    event_type: ClassVar[str] = "compaction/end"\n',
    ),
    mutant(
        "the source of a summary is that of what the user says",
        COMPACTION_SEAM,
        'SUMMARY_SOURCE = "compaction"',
        'SUMMARY_SOURCE = "user"',
    ),
    mutant(
        "a compaction that changed nothing is counted",
        COMPACTION_SEAM,
        "        count = state.count + data.outcome.changed\n",
        "        count = state.count + 1\n",
    ),
    mutant(
        "a compaction that has ended stays open",
        COMPACTION_SEAM,
        'update={"count": count, "last": data.outcome, "open": False}',
        'update={"count": count, "last": data.outcome}',
    ),
    mutant(
        "how the last compaction ended is not kept",
        COMPACTION_SEAM,
        'update={"count": count, "last": data.outcome, "open": False}',
        'update={"count": count, "open": False}',
    ),
    mutant(
        "a compaction that was open when the session was taken up again stays open",
        COMPACTION_SEAM,
        "    if isinstance(data, EndSeedEvent) and state.open:\n",
        "    if False:\n",
    ),
    mutant(
        "taking a session up again makes an orphan of a compaction that had ended",
        COMPACTION_SEAM,
        "    if isinstance(data, EndSeedEvent) and state.open:\n",
        "    if isinstance(data, EndSeedEvent):\n",
    ),
    mutant(
        "a compaction that begins while another is open makes no orphan of it",
        COMPACTION_SEAM,
        'update={"open": True, "orphans": state.orphans + state.open}',
        'update={"open": True}',
    ),
    mutant(
        "trimming alone does not count as a change",
        COMPACTION_SEAM,
        "        return bool(self.trimmed or self.summarised)\n",
        "        return bool(self.summarised)\n",
    ),
    mutant(
        "a summary alone does not count as a change",
        COMPACTION_SEAM,
        "        return bool(self.trimmed or self.summarised)\n",
        "        return bool(self.trimmed)\n",
    ),
    mutant(
        "a result that the model has not read yet is cut down",
        TRIM,
        "    read = events[: replies[-1]] if replies else []\n",
        "    read = events\n",
    ),
    mutant(
        "a result of just the length that is let be is cut down",
        TRIM,
        "            if len(block.text) <= over\n",
        "            if len(block.text) < over\n",
    ),
    mutant(
        "no result is long enough to be cut down",
        TRIM,
        "            if len(block.text) <= over\n",
        "            if True\n",
    ),
    mutant(
        "the end of a result is not kept",
        TRIM,
        "    first, last = text[:head], text[len(text) - tail :]\n",
        '    first, last = text[:head], ""\n',
    ),
    mutant(
        "the beginning of a result is not kept",
        TRIM,
        "    first, last = text[:head], text[len(text) - tail :]\n",
        '    first, last = "", text[len(text) - tail :]\n',
    ),
    mutant(
        "the line about what was cut follows the beginning without a break",
        TRIM,
        '    between = "" if not first or first.endswith("\\n") else "\\n"\n',
        '    between = ""\n',
    ),
    mutant(
        "what a result is cut down to may be long itself",
        TRIM,
        "    return cut if len(cut) <= over else text\n",
        "    return cut\n",
    ),
    mutant(
        "a result is said to have lost all of its characters",
        TRIM,
        '    cut = f"{first}{between}{_left_out(count)}\\n{last}"\n',
        '    cut = f"{first}{between}{_left_out(len(text))}\\n{last}"\n',
    ),
    mutant(
        "ends that reach each other are both shown with a line between them",
        TRIM,
        "    if count <= 0:\n        # The two ends reach each other: nothing would be left out.\n        return text\n",
        "",
    ),
    mutant(
        "a result that was cut down no longer says whether its call failed",
        TRIM,
        'data.model_copy(update={"message": message})',
        "ToolResultEvent(message=message)",
    ),
    mutant(
        "a result that was cut down is put at the end of the conversation",
        TRIM,
        "session.append(cut.shorter, replaces=[cut.seq])",
        "session.append(cut.shorter)",
    ),
    mutant(
        "the results that were cut down are not counted",
        TRIM,
        "            put += 1\n",
        "            put += 0\n",
    ),
    mutant(
        "a result that the model no longer sees is cut down all the same",
        TRIM,
        "        if cut.seq in seen:\n",
        "        if True:\n",
    ),
    mutant(
        "what would be cut is said to be the whole result",
        TRIM,
        'found.append(Cut(event.seq, data, data.model_copy(update={"message": message})))',
        "found.append(Cut(event.seq, data, data))",
    ),
    mutant(
        "the context at the top is summed up with the conversation",
        REGION,
        "    while start < len(events) and (\n        isinstance(events[start].data, SystemMessageEvent)\n        or _source(events[start]).startswith(CONTEXT_PREFIX)\n    ):\n",
        "    while start < len(events) and isinstance(events[start].data, SystemMessageEvent):\n",
    ),
    mutant(
        "the instructions are summed up with the conversation",
        REGION,
        "    while start < len(events) and (\n        isinstance(events[start].data, SystemMessageEvent)\n        or _source(events[start]).startswith(CONTEXT_PREFIX)\n    ):\n",
        "    while start < len(events) and _source(events[start]).startswith(CONTEXT_PREFIX):\n",
    ),
    mutant(
        "one message more stays than was asked for",
        REGION,
        "    while end > start and kept < retain:\n",
        "    while end > start and kept <= retain:\n",
    ),
    mutant(
        "what is to stay is counted in messages",
        REGION,
        "        kept += sizes[end]\n",
        "        kept += 1\n",
    ),
    mutant(
        "a call stays without its results going with it",
        REGION,
        "    end = _before_the_call(events, min(end, last), start)\n",
        "    end = min(end, last)\n",
    ),
    mutant(
        "what the user said last goes although it need not",
        REGION,
        "    if human and human[-1] < end and sum(sizes[human[-1] :]) <= half:\n",
        "    if False:\n",
    ),
    mutant(
        "what the user said last stays whatever has come after it",
        REGION,
        "    if human and human[-1] < end and sum(sizes[human[-1] :]) <= half:\n",
        "    if human and human[-1] < end:\n",
    ),
    mutant(
        "what the user said last goes when just half of the window follows it",
        REGION,
        "    if human and human[-1] < end and sum(sizes[human[-1] :]) <= half:\n",
        "    if human and human[-1] < end and sum(sizes[human[-1] :]) < half:\n",
    ),
    mutant(
        "a steer is not something that the user said",
        REGION,
        'HUMAN = ("user", "steer")',
        'HUMAN = ("user",)',
    ),
    mutant(
        "whatever is in the role of the user counts as what the user said",
        REGION,
        "if at >= start and _source(event) in HUMAN]",
        "if at >= start and _source(event)]",
    ),
    mutant(
        "no more goes although so much has to go",
        REGION,
        "    while end < last and sum(sizes[start:end]) < shed:\n",
        "    while False:\n",
    ),
    mutant(
        "the last message goes if so much has to go",
        REGION,
        "    while end < last and sum(sizes[start:end]) < shed:\n",
        "    while end < len(events) - 1 and sum(sizes[start:end]) < shed:\n",
    ),
    mutant(
        "more goes than has to",
        REGION,
        "    while end < last and sum(sizes[start:end]) < shed:\n",
        "    while end < last and sum(sizes[start:end]) <= shed:\n",
    ),
    mutant(
        "a call goes and its results stay",
        REGION,
        "        while isinstance(events[end].data, ToolResultEvent):\n            end += 1\n",
        "",
    ),
    mutant(
        "an earlier summary is summed up by itself",
        REGION,
        "    if all(_source(event) == SUMMARY_SOURCE for event in chosen):\n",
        "    if not chosen:\n",
    ),
    mutant(
        "what goes is said to come to as much as the whole conversation",
        REGION,
        "    return Region(tuple(event.seq for event in chosen), sum(sizes[start:end]))\n",
        "    return Region(tuple(event.seq for event in chosen), sum(sizes))\n",
    ),
    mutant(
        "the model that sums up is not shown what stands before the part that goes",
        SUMMARY,
        "LlmRequest(config=config, messages=(*before, *shown, ask), tools=tools)",
        "LlmRequest(config=config, messages=(*shown, ask), tools=tools)",
    ),
    mutant(
        "the model is asked for a summary before it is shown what to sum up",
        SUMMARY,
        "LlmRequest(config=config, messages=(*before, *shown, ask), tools=tools)",
        "LlmRequest(config=config, messages=(ask, *before, *shown), tools=tools)",
    ),
    mutant(
        "the request for a summary declares no tools",
        SUMMARY,
        "LlmRequest(config=config, messages=(*before, *shown, ask), tools=tools)",
        "LlmRequest(config=config, messages=(*before, *shown, ask), tools=())",
    ),
    mutant(
        "what the requests for a summary cost is what the last one cost",
        SUMMARY,
        "        usage += call.usage\n",
        "        usage = call.usage\n",
    ),
    mutant(
        "a part that is too long for the model is not halved",
        SUMMARY,
        "            newer = _newer_half(shown, size) if failure.code == _TOO_LONG else ()\n",
        "            newer = ()\n",
    ),
    mutant(
        "any failure of the request for a summary halves the part",
        SUMMARY,
        "            newer = _newer_half(shown, size) if failure.code == _TOO_LONG else ()\n",
        "            newer = _newer_half(shown, size)\n",
    ),
    mutant(
        "a failure that may pass ends the summary at once",
        SUMMARY,
        "            elif failure.code in _PASSING and not waited:\n",
        "            elif False:\n",
    ),
    mutant(
        "a failure that may pass is waited out as often as it comes",
        SUMMARY,
        "            elif failure.code in _PASSING and not waited:\n",
        "            elif failure.code in _PASSING:\n",
    ),
    mutant(
        "a failure that cannot pass is waited out",
        SUMMARY,
        "            elif failure.code in _PASSING and not waited:\n",
        "            elif not waited:\n",
    ),
    mutant(
        "the wait is the same whatever the service asked for",
        SUMMARY,
        "                seconds = _WAIT_SECONDS if asked_for is None else asked_for / 1000\n",
        "                seconds = _WAIT_SECONDS\n",
    ),
    mutant(
        "the service is waited for as long as it says",
        SUMMARY,
        "                await pause(min(seconds, _LONGEST_WAIT_SECONDS))\n",
        "                await pause(seconds)\n",
    ),
    mutant(
        "a summary that was cut off is taken for a whole one",
        SUMMARY,
        '        if finish.reason == "max-tokens":\n',
        "        if False:\n",
    ),
    mutant(
        "the text of a reply is taken with the space around it",
        SUMMARY,
        "        text = call.message.text.strip()\n",
        "        text = call.message.text\n",
    ),
    mutant(
        "a reply without text is the end although tools were declared",
        SUMMARY,
        "        if not tools:\n",
        "        if True:\n",
    ),
    mutant(
        "a reply without text is asked for again with the tools",
        SUMMARY,
        "        # It called a tool, or it said nothing. Without tools it can only write.\n        tools = ()\n",
        "        # It called a tool, or it said nothing. Without tools it can only write.\n        pass\n",
    ),
    mutant(
        "a summary of the size of what it sums up is taken",
        SUMMARY,
        "    if size(carried(text)) >= region.tokens:\n",
        "    if size(carried(text)) > region.tokens:\n",
    ),
    mutant(
        "a summary is taken however long it is",
        SUMMARY,
        "    if size(carried(text)) >= region.tokens:\n",
        "    if False:\n",
    ),
    mutant(
        "nobody is told how many messages the model was not shown",
        SUMMARY,
        "dropped=len(region.seqs) - len(shown))",
        "dropped=0)",
    ),
    mutant(
        "the newer half of a part is less than half of it",
        SUMMARY,
        "    while at > 0 and kept + sizes[at - 1] <= half:\n",
        "    while at > 0 and kept + sizes[at - 1] < half:\n",
    ),
    mutant(
        "a half begins with a result whose call was dropped",
        SUMMARY,
        "    while at < len(messages) and isinstance(messages[at], ToolMessage):\n        at += 1\n",
        "",
    ),
    mutant(
        "a part of which nothing can be dropped is shown again",
        SUMMARY,
        "    return messages[at:] if at else ()\n",
        "    return messages[at:]\n",
    ),
    mutant(
        "a closing tag in a summary ends the frame around it",
        SUMMARY,
        '    body = summary.strip().replace("</summary", "<\\\\/summary")\n',
        "    body = summary.strip()\n",
    ),
    mutant(
        "a summary is carried with the space around it",
        SUMMARY,
        '    body = summary.strip().replace("</summary", "<\\\\/summary")\n',
        '    body = summary.replace("</summary", "<\\\\/summary")\n',
    ),
    mutant(
        "the headroom is a part of the window however large that is",
        BASIC,
        "    headroom = max(math.floor(min(config.headroom_tokens, window * config.headroom_ratio)), 1)\n",
        "    headroom = max(math.floor(window * config.headroom_ratio), 1)\n",
    ),
    mutant(
        "the headroom is so many tokens however small the window is",
        BASIC,
        "    headroom = max(math.floor(min(config.headroom_tokens, window * config.headroom_ratio)), 1)\n",
        "    headroom = max(config.headroom_tokens, 1)\n",
    ),
    mutant(
        "the headroom of a tiny window is nothing",
        BASIC,
        "    headroom = max(math.floor(min(config.headroom_tokens, window * config.headroom_ratio)), 1)\n",
        "    headroom = math.floor(min(config.headroom_tokens, window * config.headroom_ratio))\n",
    ),
    mutant(
        "a conversation may fill all that the headroom leaves",
        BASIC,
        "threshold=math.floor(min(window * config.threshold_ratio, window - reply - headroom)),",
        "threshold=math.floor(window - reply - headroom),",
    ),
    mutant(
        "a conversation may fill its part of the window whatever the headroom",
        BASIC,
        "threshold=math.floor(min(window * config.threshold_ratio, window - reply - headroom)),",
        "threshold=math.floor(window * config.threshold_ratio),",
    ),
    mutant(
        "the room for the reply counts as room for the conversation",
        BASIC,
        "threshold=math.floor(min(window * config.threshold_ratio, window - reply - headroom)),",
        "threshold=math.floor(min(window * config.threshold_ratio, window - headroom)),",
    ),
    mutant(
        "what stays is a part of the whole window, the room for the reply included",
        BASIC,
        "retain=math.floor((window - reply) * config.retain_ratio),",
        "retain=math.floor(window * config.retain_ratio),",
    ),
    mutant(
        "what the service said about its model counts for less than the config",
        BASIC,
        "    window = window or model.context_window\n",
        "    window = model.context_window or window\n",
    ),
    mutant(
        "a service that says what the messages resulted in is not understood",
        BASIC,
        '_REQUESTED = re.compile(r"(?:requested|resulted in) (\\d+) tokens")',
        '_REQUESTED = re.compile(r"requested (\\d+) tokens")',
    ),
    mutant(
        "the whole request is taken for the conversation",
        BASIC,
        "    messages = _MESSAGES.search(message) or requested\n",
        "    messages = requested\n",
    ),
    mutant(
        "a refusal that does not say how much was requested is read all the same",
        BASIC,
        "    if window is None or requested is None:\n",
        "    if window is None:\n",
    ),
    mutant(
        "a refusal that does not say how much the model takes is read all the same",
        BASIC,
        "    if window is None or requested is None:\n",
        "    if requested is None:\n",
    ),
    mutant(
        "ends that leave no room for the line between them are taken by the config",
        BASIC,
        "        if self.trim_head + self.trim_tail + _LINE_BETWEEN > self.trim_over:\n",
        "        if self.trim_head + self.trim_tail > self.trim_over:\n",
    ),
    mutant(
        "ends that leave just the room for the line between them are refused",
        BASIC,
        "        if self.trim_head + self.trim_tail + _LINE_BETWEEN > self.trim_over:\n",
        "        if self.trim_head + self.trim_tail + _LINE_BETWEEN >= self.trim_over:\n",
    ),
    mutant(
        "a conversation is made shorter for a step that does not begin",
        BASIC,
        "        if isinstance(decision, Enter) and await self._pressed(info.agent, coming):\n",
        "        if await self._pressed(info.agent, coming):\n",
    ),
    mutant(
        "a conversation is made shorter before every step",
        BASIC,
        "        if isinstance(decision, Enter) and await self._pressed(info.agent, coming):\n",
        "        if isinstance(decision, Enter):\n",
    ),
    mutant(
        "the service does not wait for what the other listeners say about a step",
        BASIC,
        "        ctx.on(PRE_STEP, service.before_a_step, prepend=True)\n",
        "        ctx.on(PRE_STEP, service.before_a_step)\n",
    ),
    mutant(
        "any failure of a request makes the conversation shorter",
        BASIC,
        "        if failed.failure.code != _TOO_LONG or self._refused.get(session) == step:\n",
        "        if self._refused.get(session) == step:\n",
    ),
    mutant(
        "a request is made again as often as it is refused",
        BASIC,
        "        if failed.failure.code != _TOO_LONG or self._refused.get(session) == step:\n",
        "        if failed.failure.code != _TOO_LONG:\n",
    ),
    mutant(
        "a request is made again although the model would see the same",
        BASIC,
        "        return Retry() if outcome.changed else await proceed(failed)\n",
        "        return Retry()\n",
    ),
    mutant(
        "what the service said when it refused is not read",
        BASIC,
        "        said = refusal(failed.failure.message)\n",
        "        said = None\n",
    ),
    mutant(
        "a window that leaves no room is watched all the same",
        BASIC,
        "        if limits is None or limits.threshold <= 0:\n",
        "        if limits is None:\n",
    ),
    mutant(
        "it is said before every step that a model is not watched",
        BASIC,
        "                self._told = True\n",
        "                pass\n",
    ),
    mutant(
        "a conversation that fills just what may be filled is made shorter",
        BASIC,
        "        if self._meter.measure(session).tokens <= limits.threshold:\n",
        "        if self._meter.measure(session).tokens < limits.threshold:\n",
    ),
    mutant(
        "failures in a row do not stop the service from trying",
        BASIC,
        "        return self._failures.get(session, 0) < _FAILURES_IN_A_ROW\n",
        "        return True\n",
    ),
    mutant(
        "the service gives up after one failure",
        BASIC,
        "_FAILURES_IN_A_ROW = 2",
        "_FAILURES_IN_A_ROW = 1",
    ),
    mutant(
        "the service gives up after three failures",
        BASIC,
        "_FAILURES_IN_A_ROW = 2",
        "_FAILURES_IN_A_ROW = 3",
    ),
    mutant(
        "two attempts work on one conversation at once",
        BASIC,
        "        if session in self._running:\n",
        "        if False:\n",
    ),
    mutant(
        "an attempt that has ended is still under way",
        BASIC,
        '        """Write down how an attempt ended, and keep count of the ones that failed."""\n        self._running.discard(session)\n',
        '        """Write down how an attempt ended, and keep count of the ones that failed."""\n',
    ),
    mutant(
        "cutting old results is never enough by itself",
        BASIC,
        "            enough = plan.target is not None and plan.size - saved <= plan.target\n",
        "            enough = False\n",
    ),
    mutant(
        "what cutting old results would save is not counted",
        BASIC,
        "            enough = plan.target is not None and plan.size - saved <= plan.target\n",
        "            enough = plan.target is not None and plan.size <= plan.target\n",
    ),
    mutant(
        "a summary is never enough",
        BASIC,
        "                        enough = plan.target is None or now <= plan.target\n",
        "                        enough = False\n",
    ),
    mutant(
        "old results are not cut although cutting them is enough",
        BASIC,
        "            if not summarised or not enough:\n",
        "            if not enough:\n",
    ),
    mutant(
        "what is left is not cut although the summary was not enough",
        BASIC,
        "            if not summarised or not enough:\n",
        "            if not summarised:\n",
    ),
    mutant(
        "an attempt that could do nothing is no failure",
        BASIC,
        '            if not (trimmed or summarised or error) and why != "manual":\n',
        "            if False:\n",
    ),
    mutant(
        "asking for a summary where nothing can go is a failure",
        BASIC,
        '            if not (trimmed or summarised or error) and why != "manual":\n',
        "            if not (trimmed or summarised or error):\n",
    ),
    mutant(
        "an attempt that the user stopped counts as a failure",
        BASIC,
        '"cancelled", counts=False)',
        '"cancelled", counts=True)',
    ),
    mutant(
        "a fault of the service ends the turn",
        BASIC,
        "        except Exception as failed:\n            logger.exception(",
        "        except SummaryError as failed:\n            logger.exception(",
    ),
    mutant(
        "every failed attempt counts, also one that the user asked for",
        BASIC,
        '        counts = why == "pressure"\n',
        "        counts = True\n",
    ),
    mutant(
        "failed attempts are not counted",
        BASIC,
        '        counts = why == "pressure"\n',
        "        counts = False\n",
    ),
    mutant(
        "the summary is asked for without the tools that were declared",
        BASIC,
        "        tools: tuple[ToolSchema, ...] = () if header is None else header.tools\n",
        "        tools: tuple[ToolSchema, ...] = ()\n",
    ),
    mutant(
        "a summary may be longer than any reply may be",
        BASIC,
        "        limit = min(headroom, model.max_output_tokens or headroom)\n",
        "        limit = headroom\n",
    ),
    mutant(
        "a summary may be longer than the headroom",
        BASIC,
        "        limit = min(headroom, model.max_output_tokens or headroom)\n",
        "        limit = model.max_output_tokens or headroom\n",
    ),
    mutant(
        "the message that carries a summary is taken for what the user said",
        BASIC,
        "UserMessageEvent(message=message, source=SUMMARY_SOURCE), replaces=region.seqs",
        "UserMessageEvent(message=message), replaces=region.seqs",
    ),
    mutant(
        "a summary follows the conversation and what it sums up stays",
        BASIC,
        "UserMessageEvent(message=message, source=SUMMARY_SOURCE), replaces=region.seqs",
        "UserMessageEvent(message=message, source=SUMMARY_SOURCE)",
    ),
    mutant(
        "an attempt says that it left the conversation as large as it was",
        BASIC,
        "            after=self._meter.measure(session).tokens,\n",
        "            after=before,\n",
    ),
    mutant(
        "an attempt that succeeds does not end the count of failures",
        BASIC,
        "            self._failures.pop(session, None)\n",
        "            pass\n",
    ),
    mutant(
        "the service does nothing by itself although the config says so",
        BASIC,
        "    if config.auto:\n",
        "    if False:\n",
    ),
    mutant(
        "the service works by itself although the config says not to",
        BASIC,
        "    if config.auto:\n",
        "    if True:\n",
    ),
    mutant(
        "what the service counted is not believed before the estimate",
        BASIC,
        "        scale = said.messages / measured.estimated\n",
        "        pass\n",
    ),
    mutant(
        "the model of a refusal is taken to be as large as the config says",
        BASIC,
        "    limits = budget(model, config, None if said is None else said.window)\n",
        "    limits = budget(model, config)\n",
    ),
    mutant(
        "no headroom is asked for after a refusal",
        BASIC,
        "    shed = said.requested - said.window + limits.headroom\n",
        "    shed = said.requested - said.window\n",
    ),
    mutant(
        "after a refusal the conversation may stay as large as it was",
        BASIC,
        "target=size - shed)",
        "target=size)",
    ),
    mutant(
        "the summary is asked of the model that the agent was created with",
        BASIC,
        "        return await agent.ctx.waterfall(REQUEST, draft, _as_drafted)\n",
        "        return agent.spec.model\n",
    ),
    mutant(
        "a result is cut down although that saves no tokens",
        BASIC,
        "[cut for cut in found if size(cut.shorter.message) < size(cut.whole.message)]",
        "[cut for cut in found if True]",
    ),
    mutant(
        "what was cut before the log refused an event is not counted",
        BASIC,
        "                for cut in found:\n                    # One at a time: what was cut stays counted if the next cannot be written.\n                    trimmed += cut_down(session, [cut])\n",
        "                trimmed = cut_down(session, found)\n",
    ),
    mutant(
        "a reply that calls a tool is taken for a summary if it says something beside",
        SUMMARY,
        "        if text and not call.message.tool_calls:\n",
        "        if text:\n",
    ),
    mutant(
        "a reply of nothing is waited out with the tools still on offer",
        SUMMARY,
        "        if failure is not None and failure.code == _EMPTY and tools:\n",
        "        if False:\n",
    ),
    mutant(
        "a reply of nothing is asked for again at once also where no tools were declared",
        SUMMARY,
        "        if failure is not None and failure.code == _EMPTY and tools:\n",
        "        if failure is not None and failure.code == _EMPTY:\n",
    ),
    mutant(
        "what a request for a summary cost is not written down",
        SUMMARY,
        '            session.append(AsideEvent(purpose="summary", usage=call.usage, failure=call.failure))\n',
        "            pass\n",
    ),
    mutant(
        "what requests beside the conversation cost is in no total",
        PROJECTION,
        "    if isinstance(data, (AssistantMessageEvent, AssistantAttemptEvent, AsideEvent)):\n",
        "    if isinstance(data, (AssistantMessageEvent, AssistantAttemptEvent)):\n",
    ),
    mutant(
        "the step before the one that is about to begin says which model counts",
        BASIC,
        "        coming = (info.turn, info.step, 1)\n",
        "        coming = (info.turn, info.step - 1, 1)\n",
    ),
    mutant(
        "the request that is made again after a refusal is taken for the one that was refused",
        BASIC,
        "        coming = (failed.turn, failed.step, failed.attempt + 1)\n",
        "        coming = (failed.turn, failed.step, failed.attempt)\n",
    ),
    mutant(
        "between two turns the request to come is taken for one of the turn before",
        BASIC,
        "        turn, step, attempt = coming or (TURN.of(agent.session).turn + 1, 1, 1)\n",
        "        turn, step, attempt = coming or (TURN.of(agent.session).turn, 1, 1)\n",
    ),
    mutant(
        "two that ask at the same moment both begin",
        BASIC,
        "        self._running.add(session)\n        config = self._config\n        try:\n            model = await self._model(agent, coming)\n",
        "        config = self._config\n        try:\n            model = await self._model(agent, coming)\n            self._running.add(session)\n",
    ),
    mutant(
        "an attempt that could not begin stays in the way",
        BASIC,
        "            # It has not begun. Whoever asks next finds nothing in the way.\n            self._running.discard(session)\n",
        "            # It has not begun. Whoever asks next finds nothing in the way.\n",
    ),
)

_BUDGET = (
    mutant(
        "a turn takes a step more than it has",
        BUDGET,
        "        if steps is not None and info.step > steps:\n",
        "        if steps is not None and info.step > steps + 1:\n",
    ),
    mutant(
        "a turn is ended a step before its last",
        BUDGET,
        "        if steps is not None and info.step > steps:\n",
        "        if steps is not None and info.step >= steps:\n",
    ),
    mutant(
        "the steps of the config count before those of the spec",
        BUDGET,
        "        steps = info.agent.spec.max_steps or config.max_steps\n",
        "        steps = config.max_steps or info.agent.spec.max_steps\n",
    ),
    mutant(
        "the steps that a spec names are not looked at",
        BUDGET,
        "        steps = info.agent.spec.max_steps or config.max_steps\n",
        "        steps = config.max_steps\n",
    ),
    mutant(
        "one step is told as one steps",
        BUDGET,
        "step{'' if steps == 1 else 's'}",
        "steps",
    ),
    mutant(
        "the tokens of a turn are those of the whole session",
        BUDGET,
        "        used = spent - begun.tokens\n",
        "        used = spent\n",
    ),
    mutant(
        "tokens that come to just the limit are not the limit",
        BUDGET,
        "        if config.max_tokens is not None and used >= config.max_tokens:\n",
        "        if config.max_tokens is not None and used > config.max_tokens:\n",
    ),
    mutant(
        "what was read from the cache is not counted as used",
        BUDGET,
        "        + usage.cache_read_tokens\n",
        "",
    ),
    mutant(
        "what the model wrote is not counted as used",
        BUDGET,
        "        + usage.output_tokens\n",
        "",
    ),
    mutant(
        "a turn goes by where the turn before it began",
        BUDGET,
        "        if begun is None or begun.turn != info.turn:\n",
        "        if begun is None:\n",
    ),
    mutant(
        "a turn that has run for its time takes another step",
        BUDGET,
        "        if config.max_seconds is not None and passed >= config.max_seconds:\n",
        "        if False:\n",
    ),
    mutant(
        "a step that the budget refuses is asked about by those after it",
        BUDGET,
        "            # Nobody else is asked about a step that does not begin.\n            return Reject(over)\n",
        "            await proceed(info)\n            return Reject(over)\n",
    ),
    mutant(
        "the budget is asked after those that were there before it",
        BUDGET,
        "    ctx.on(PRE_STEP, _Budget(config).before_a_step, prepend=True)\n",
        "    ctx.on(PRE_STEP, _Budget(config).before_a_step)\n",
    ),
)

_SKILLS = (
    mutant(
        "a file that does not begin with dashes is taken for a skill file",
        SKILL_FILES,
        "    if lines[0].rstrip() != _DASHES:\n",
        "    if False:\n",
    ),
    mutant(
        "a space after the first dashes makes a file no skill file",
        SKILL_FILES,
        "    if lines[0].rstrip() != _DASHES:\n",
        "    if lines[0] != _DASHES:\n",
    ),
    mutant(
        "a space after the dashes keeps them from closing the first lines",
        SKILL_FILES,
        "        if lines[at].rstrip() == _DASHES:\n",
        "        if lines[at] == _DASHES:\n",
    ),
    mutant(
        "dashes in the middle of a line close the first lines",
        SKILL_FILES,
        "        if lines[at].rstrip() == _DASHES:\n",
        "        if _DASHES in lines[at]:\n",
    ),
    mutant(
        "the text of a skill begins with the dashes that close its first lines",
        SKILL_FILES,
        'return "\\n".join(lines[1:at]), "\\n".join(lines[at + 1 :])',
        'return "\\n".join(lines[1:at]), "\\n".join(lines[at:])',
    ),
    mutant(
        "what a file gives is shown at whatever length",
        SKILL_FILES,
        '    return text if len(text) <= limit else f"{text[:limit]}..."\n',
        "    return text\n",
    ),
    mutant(
        "a value of just the length that is shown is cut all the same",
        SKILL_FILES,
        '    return text if len(text) <= limit else f"{text[:limit]}..."\n',
        '    return text if len(text) < limit else f"{text[:limit]}..."\n',
    ),
    mutant(
        "one character more of a value is shown than is said",
        SKILL_FILES,
        '    return text if len(text) <= limit else f"{text[:limit]}..."\n',
        '    return text if len(text) <= limit else f"{text[: limit + 1]}..."\n',
    ),
    mutant(
        "a list that a key gives is written out",
        SKILL_FILES,
        '    if isinstance(value, list):\n        return "a list"\n',
        "",
    ),
    mutant(
        "keys and values that a key gives are written out",
        SKILL_FILES,
        '    if isinstance(value, dict):\n        return "keys and values of its own"\n',
        "",
    ),
    mutant(
        "a key that is no text is asked for its letters",
        SKILL_FILES,
        "    if not isinstance(key, str):\n        return None\n",
        "",
    ),
    mutant(
        "a key in capital letters is no slip of the pen",
        SKILL_FILES,
        '    letters = key.lower().replace("-", "")',
        '    letters = key.replace("-", "")',
    ),
    mutant(
        "a key with dashes between its words is no slip of the pen",
        SKILL_FILES,
        'key.lower().replace("-", "").replace("_", "")',
        'key.lower().replace("_", "")',
    ),
    mutant(
        "a key with lines under its words is no slip of the pen",
        SKILL_FILES,
        '.replace("-", "").replace("_", "").replace("invok", "invoc")',
        '.replace("-", "").replace("invok", "invoc")',
    ),
    mutant(
        "a key that is spelled with a k is no slip of the pen",
        SKILL_FILES,
        '.replace("_", "").replace("invok", "invoc")\n',
        '.replace("_", "")\n',
    ),
    mutant(
        "the keys that say who can reach a skill are slips of themselves",
        SKILL_FILES,
        "    return None if key == meant else meant\n",
        "    return meant\n",
    ),
    mutant(
        "a key that is nearly one that says who can reach a skill is passed over",
        SKILL_FILES,
        "    return None if key == meant else meant\n",
        "    return None\n",
    ),
    mutant(
        "a slip of the key for people is said to be meant for the other key",
        SKILL_FILES,
        '"userinvocable": _USER}\n',
        '"userinvocable": _MODEL}\n',
    ),
    mutant(
        "a slip that names what a model may do is not known for one",
        SKILL_FILES,
        '_MEANT = {"disablemodelinvocation": _MODEL, "modelinvocable": _MODEL, ',
        '_MEANT = {"disablemodelinvocation": _MODEL, ',
    ),
    mutant(
        "a slip that was found is not told",
        SKILL_FILES,
        "        if meant is not None:\n",
        "        if False:\n",
    ),
    mutant(
        "a key that is a slip is shown at whatever length",
        SKILL_FILES,
        'f"the key {_shown(key)} is not known. What is meant is spelled {meant!r}"',
        'f"the key {key!r} is not known. What is meant is spelled {meant!r}"',
    ),
    mutant(
        "a key that is missing is not said to be missing",
        SKILL_FILES,
        '    if key not in fields:\n        raise SkillError(f"the key {key!r} is missing")\n',
        "",
    ),
    mutant(
        "a key with nothing after it is said to give something that is no text",
        SKILL_FILES,
        "    if value is None or (isinstance(value, str) and not value.strip()):\n",
        "    if isinstance(value, str) and not value.strip():\n",
    ),
    mutant(
        "a description of spaces alone says something",
        SKILL_FILES,
        "    if value is None or (isinstance(value, str) and not value.strip()):\n",
        "    if value is None or (isinstance(value, str) and not value):\n",
    ),
    mutant(
        "a number is taken for the text that a key gives",
        SKILL_FILES,
        "    if not isinstance(value, str):\n",
        "    if False:\n",
    ),
    mutant(
        "whatever a key gives counts as true or false",
        SKILL_FILES,
        "    if not isinstance(value, bool):\n",
        "    if False:\n",
    ),
    mutant(
        "a link that leads nowhere is followed all the same",
        SKILL_FILES,
        "    return Path(os.path.realpath(path, strict=True))\n",
        "    return Path(os.path.realpath(path, strict=False))\n",
    ),
    mutant(
        "a mark at the beginning of a file makes it no skill file",
        SKILL_FILES,
        '        text = content.decode("utf-8-sig")\n',
        '        text = content.decode("utf-8")\n',
    ),
    mutant(
        "the line ends of windows stay in the text of a skill",
        SKILL_FILES,
        '    front, rest = _split(text.replace("\\r\\n", "\\n"))\n',
        "    front, rest = _split(text)\n",
    ),
    mutant(
        "lists that lie too deep in one another are told as a value that cannot be read",
        SKILL_FILES,
        '    except RecursionError:\n        raise SkillError(f"the lines at the beginning of {FILE} are nested too deeply") from None\n',
        "",
    ),
    mutant(
        "an error of the reader of yaml that is none of its own goes through",
        SKILL_FILES,
        "    except Exception as error:\n",
        "    except ValueError as error:\n",
    ),
    mutant(
        "what the reader of yaml says is passed on at whatever length",
        SKILL_FILES,
        '    except yaml.YAMLError as error:\n        said = _cut(" ".join(str(error).split()), _SAID)\n',
        '    except yaml.YAMLError as error:\n        said = " ".join(str(error).split())\n',
    ),
    mutant(
        "what the reader of yaml says is passed on with its line breaks",
        SKILL_FILES,
        '    except yaml.YAMLError as error:\n        said = _cut(" ".join(str(error).split()), _SAID)\n',
        "    except yaml.YAMLError as error:\n        said = _cut(str(error), _SAID)\n",
    ),
    mutant(
        "why a value cannot be made is passed on at whatever length",
        SKILL_FILES,
        '        # thousands of digits, a tag before something that it does not go with.\n        said = _cut(" ".join(str(error).split()), _SAID)\n',
        '        # thousands of digits, a tag before something that it does not go with.\n        said = " ".join(str(error).split())\n',
    ),
    mutant(
        "first lines that say nothing are told as no keys and values",
        SKILL_FILES,
        "    if loaded is None:\n        loaded = {}\n",
        "",
    ),
    mutant(
        "a list at the beginning of a skill file is looked into for keys",
        SKILL_FILES,
        "    if not isinstance(loaded, dict):\n",
        "    if False:\n",
    ),
    mutant(
        "a name may be as long as its writer likes",
        SKILL_FILES,
        "    if len(name) > MAX_NAME or _NAME.fullmatch(name) is None:\n",
        "    if _NAME.fullmatch(name) is None:\n",
    ),
    mutant(
        "a name of just the length that is allowed is refused",
        SKILL_FILES,
        "    if len(name) > MAX_NAME or _NAME.fullmatch(name) is None:\n",
        "    if len(name) >= MAX_NAME or _NAME.fullmatch(name) is None:\n",
    ),
    mutant(
        "a name may hold whatever characters",
        SKILL_FILES,
        "    if len(name) > MAX_NAME or _NAME.fullmatch(name) is None:\n",
        "    if len(name) > MAX_NAME:\n",
    ),
    mutant(
        "a name need only begin like a name",
        SKILL_FILES,
        "    if len(name) > MAX_NAME or _NAME.fullmatch(name) is None:\n",
        "    if len(name) > MAX_NAME or _NAME.match(name) is None:\n",
    ),
    mutant(
        "a name may begin and end with a dash, and hold two in a row",
        SKILL_FILES,
        '_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")\n',
        '_NAME = re.compile(r"[a-z0-9-]+")\n',
    ),
    mutant(
        "a name that cannot be used is shown at whatever length",
        SKILL_FILES,
        'f"the name {_shown(name)} cannot be used: ',
        'f"the name {name!r} cannot be used: ',
    ),
    mutant(
        "a skill may be called otherwise than its directory",
        SKILL_FILES,
        "    if name != directory:\n",
        "    if False:\n",
    ),
    mutant(
        "a skill that a model may not load is one that it may",
        SKILL_FILES,
        "    model = not _flag(fields, _MODEL, default=False)\n",
        "    model = _flag(fields, _MODEL, default=True)\n",
    ),
    mutant(
        "a skill that says nothing about it may not be loaded by a model",
        SKILL_FILES,
        "    model = not _flag(fields, _MODEL, default=False)\n",
        "    model = not _flag(fields, _MODEL, default=True)\n",
    ),
    mutant(
        "a skill that says nothing about it cannot be handed over by a person",
        SKILL_FILES,
        "_flag(fields, _USER, default=True), rest.strip",
        "_flag(fields, _USER, default=False), rest.strip",
    ),
    mutant(
        "the empty lines around the text of a skill stay",
        SKILL_FILES,
        'rest.strip("\\n")\n',
        "rest\n",
    ),
    mutant(
        "a skill file of the user that lies behind a link is not read where it leads",
        SKILL_FILES,
        "        if root.own:\n            # The user's own: a file that is a link is read where it leads.\n            file = _followed(file)\n",
        "",
    ),
    mutant(
        "a directory without a skill file is one that cannot be read",
        SKILL_FILES,
        "        # No such file, or the link that was followed leads to a file and not to a directory.\n        return None\n",
        "        # No such file, or the link that was followed leads to a file and not to a directory.\n        raise\n",
    ),
    mutant(
        "a directory in the place of a skill file is told as a file that cannot be read",
        SKILL_FILES,
        '    except bound.NotAFileError:\n        raise SkillError(f"{FILE} is not a file") from None\n',
        "",
    ),
    mutant(
        "a link is blamed for a skill file of the user that somebody was at work on",
        SKILL_FILES,
        "        if root.own:\n            # Its links were followed a moment before, so somebody is at work there.\n            raise\n        raise SkillError(_LINK) from None\n",
        "        raise SkillError(_LINK) from None\n",
    ),
    mutant(
        "a skill file of a project that lies behind a link is told as one that cannot be read",
        SKILL_FILES,
        "        raise SkillError(_LINK) from None\n",
        "        raise\n",
    ),
    mutant(
        "a skill file that is too long is cut to the length that is allowed and read",
        SKILL_FILES,
        "        content = opened.read(MAX_BYTES + 1)\n",
        "        content = opened.read(MAX_BYTES)\n",
    ),
    mutant(
        "a skill file of any length is read",
        SKILL_FILES,
        "    if len(content) > MAX_BYTES:\n",
        "    if False:\n",
    ),
    mutant(
        "a skill file of just the length that is allowed is refused",
        SKILL_FILES,
        "    if len(content) > MAX_BYTES:\n",
        "    if len(content) >= MAX_BYTES:\n",
    ),
    mutant(
        "a skill file has to say the name of the directory that a link leads to",
        SKILL_FILES,
        "    description, model, user, text = parsed(content, name)\n",
        "    description, model, user, text = parsed(content, directory.name)\n",
    ),
    mutant(
        "who can reach a skill is told the wrong way round",
        SKILL_FILES,
        "    return Skill(name, description, directory, root, model, user), text\n",
        "    return Skill(name, description, directory, root, user, model), text\n",
    ),
    mutant(
        "a link in a project is followed",
        SKILL_FILES,
        "        if not root.own:\n            raise SkillError(_LINK)\n",
        "",
    ),
    mutant(
        "a link that leads nowhere is told as a skill that cannot be read",
        SKILL_FILES,
        '        try:\n            directory = _followed(there)\n        except FileNotFoundError:\n            raise SkillError("it is a link that leads nowhere") from None\n',
        "        directory = _followed(there)\n",
    ),
    mutant(
        "a skill behind a link goes by the name of the directory that the link leads to",
        SKILL_FILES,
        "    read = _read(directory, root, entry.name)\n",
        "    read = _read(directory, root, directory.name)\n",
    ),
    mutant(
        "a place of the user that is a link is not followed",
        SKILL_FILES,
        "        if root.own:\n            base = _followed(base)\n",
        "",
    ),
    mutant(
        "a root that is not there is an error",
        SKILL_FILES,
        "    except (FileNotFoundError, NotADirectoryError):\n        return Scan(root)\n",
        "    except (FileNotFoundError, NotADirectoryError):\n        raise\n",
    ),
    mutant(
        "a link is blamed for a place of the user that somebody was at work on",
        SKILL_FILES,
        "        if root.own:\n            # Its links were followed a moment before, so somebody is at work there.\n            raise\n        return Scan(",
        "        return Scan(",
    ),
    mutant(
        "a place of a project that is a link says nothing about it",
        SKILL_FILES,
        "        return Scan(root, invalid=(Invalid(base, root, _LINK),))\n",
        "        return Scan(root)\n",
    ),
    mutant(
        "a root with very many entries says nothing of those that it leaves out",
        SKILL_FILES,
        "    if more:\n",
        "    if False:\n",
    ),
    mutant(
        "every entry of a root is looked at, however many there are",
        SKILL_FILES,
        "        entries, more = bound.listing(base, _MAX_ENTRIES)\n",
        "        entries, more = bound.listing(base)\n",
    ),
    mutant(
        "a skill that cannot be used keeps a root from being looked into",
        SKILL_FILES,
        "            invalid.append(Invalid(there, root, str(error)))\n",
        "            raise\n",
    ),
    mutant(
        "a skill that cannot be read keeps a root from being looked into",
        SKILL_FILES,
        '            invalid.append(Invalid(there, root, f"it cannot be read: {error.strerror or error}"))\n',
        "            raise\n",
    ),
    mutant(
        "why a skill cannot be read is told with the number of the error",
        SKILL_FILES,
        'f"it cannot be read: {error.strerror or error}"',
        'f"it cannot be read: {error}"',
    ),
    mutant(
        "an error without words of the system says nothing of why a skill cannot be read",
        SKILL_FILES,
        'f"it cannot be read: {error.strerror or error}"',
        'f"it cannot be read: {error.strerror}"',
    ),
    mutant(
        "of two skills of one name the one that was found last is the skill",
        SKILL_FILES,
        "            if skill.name in skills:\n                shadowed.append(skill)\n            else:\n                skills[skill.name] = skill\n",
        "            skills[skill.name] = skill\n",
    ),
    mutant(
        "a skill that was passed over is not written down",
        SKILL_FILES,
        "            if skill.name in skills:\n                shadowed.append(skill)\n",
        "            if skill.name in skills:\n                pass\n",
    ),
    mutant(
        "skills come in the order in which they were found and not by name",
        SKILL_FILES,
        "    return Found(dict(sorted(skills.items())), tuple(shadowed), tuple(invalid))\n",
        "    return Found(dict(skills), tuple(shadowed), tuple(invalid))\n",
    ),
    mutant(
        "a skill that is gone when its text is asked for gives nothing and says nothing",
        SKILL_FILES,
        '    if read is None:\n        raise SkillError(f"{FILE} is no longer there")\n',
        "",
    ),
    mutant(
        "a skill is read anew by the name of the directory that it lies in",
        SKILL_FILES,
        "    read = _read(skill.directory, skill.root, skill.name)\n",
        "    read = _read(skill.directory, skill.root, skill.directory.name)\n",
    ),
    mutant(
        "the place of a project is taken for the user's own",
        SKILL_FILES,
        '        Root(project / ".columbina" / "skills", "project", own=False),\n',
        '        Root(project / ".columbina" / "skills", "project", own=True),\n',
    ),
    mutant(
        "the place that a project shares with other tools is taken for the user's own",
        SKILL_FILES,
        '        Root(project / ".agents" / "skills", "project, .agents", own=False),\n',
        '        Root(project / ".agents" / "skills", "project, .agents", own=True),\n',
    ),
    mutant(
        "the home of the program is looked into as if it were a project",
        SKILL_FILES,
        '        found.append(Root(home, "home", own=True))\n',
        '        found.append(Root(home, "home", own=False))\n',
    ),
    mutant(
        "further places for skills count before the others",
        SKILL_FILES,
        '    found.extend(Root(path, "extra", own=True) for path in extra)\n',
        '    found[:0] = [Root(path, "extra", own=True) for path in extra]\n',
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
    "process": MutantSet(("tests/test_process.py", "tests/test_process_overflow.py"), _PROCESS),
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
    "meter": MutantSet(("tests/token_meter",), _METER),
    "spill": MutantSet(("tests/spill",), _SPILL),
    "cap": MutantSet(("tests/spill/test_policy.py",), _CAP),
    "compaction": MutantSet(("tests/compaction",), _COMPACTION),
    "todo": MutantSet(("tests/todo", "tests/commands/test_todos.py"), _TODO),
    "sessions": MutantSet(("tests/apps/test_stored_sessions.py",), _STORED_SESSIONS),
    "budget": MutantSet(("tests/guard",), _BUDGET),
    "skills": MutantSet(("tests/skills",), _SKILLS),
}
"""The sets of mutants, by the name that ``mutate.py`` takes."""
