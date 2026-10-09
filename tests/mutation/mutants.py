"""Small faults to put into the code, to see whether the tests notice.

A mutant is a replacement of text in one source file. ``mutate.py`` puts each
one in, runs the tests that ought to notice, and takes it out again. A mutant
that the tests do not notice points at something they do not check.

The text to replace must occur exactly once in its file. ``test_mutants.py``
checks that for every mutant, so a list cannot go stale without a test failing.

Left out on purpose are mutants that change nothing that can be observed. One
example: counting the lines after the wanted ones on the slow path of the
reader instead of the fast one.
"""

from dataclasses import dataclass

LOOP = "src/tack/agent_loop/loop.py"
INBOX = "src/tack/agent/inbox.py"
CANCELLATION = "src/tack/cancellation.py"
READ = "src/tack/builtin_tools/read.py"


@dataclass(frozen=True, slots=True)
class Mutant:
    """One fault.

    Attributes:
        name: What goes wrong, in a few words.
        path: The file, from the root of the repository.
        changes: Pairs of the text to replace and the text to put there.
    """

    name: str
    path: str
    changes: tuple[tuple[str, str], ...]


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
        "                else:\n"
        "                    await until_running(0)\n"
        "                    await session.flush()\n",
        "                else:\n                    await session.flush()\n",
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
        "        # A thread cannot be cancelled. It is told to stop, and does so at its next piece.\n"
        "        stopped.set()\n",
        "        pass\n",
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

SETS: dict[str, MutantSet] = {
    "loop": MutantSet(("tests/agent_loop", "tests/agent", "tests/test_cancellation.py"), _LOOP),
    "read": MutantSet(("tests/builtin_tools",), _READ),
}
"""The sets of mutants, by the name that ``mutate.py`` takes."""
