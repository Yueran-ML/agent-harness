"""Mutants of the client of the engine of video, and of ``columbina video engine``.

They live apart from ``mutants.py`` for the reason that ``batch_seventeen.py`` gives, and are
added to its sets the same way.
"""

from mutation.mutants import MutantSet, mutant, on_windows

ENGINE = "src/columbina/video/engine.py"
VIDEO_COMMAND = "src/columbina/apps/video_command.py"

_ADDRESSES = (
    on_windows(
        mutant(
            "an address of another machine is connected to",
            ENGINE,
            r'_PIPE = re.compile(r"\\\\\.\\pipe\\columbina-engine-[0-9a-f]{16}")',
            r'_PIPE = re.compile(r"\\\\[^\\]+\\pipe\\columbina-engine-[0-9a-f]{16}")',
        )
    ),
    on_windows(
        mutant(
            "an address with more after the name of an engine's is connected to",
            ENGINE,
            "        return _PIPE.fullmatch(address) is not None",
            "        return _PIPE.match(address) is not None",
        )
    ),
    mutant(
        "an address that engine.lock names is opened unlooked at",
        ENGINE,
        '_address(cast("dict[str, object]", said).get("address")) if isinstance(said, dict)',
        'cast("dict[str, object]", said).get("address") if isinstance(said, dict)',
    ),
    mutant(
        "an address that a started engine says it listens at is opened unlooked at",
        ENGINE,
        '    address = _address(told.get("listening"))',
        '    address = told.get("listening")',
    ),
)

_PROGRAM = (
    mutant(
        "an engine in the working directory is run",
        ENGINE,
        "        and programs.trusted(program, apart)\n",
        "        and True\n",
    ),
    mutant(
        "a path of the engine relative to the working directory is run",
        ENGINE,
        "        program.is_absolute()\n",
        "        True\n",
    ),
    mutant(
        "COLUMBINA_ENGINE goes before the config",
        ENGINE,
        '    named = configured or os.environ.get("COLUMBINA_ENGINE")\n'
        "    if not named:\n"
        "        return programs.found(",
        '    named = os.environ.get("COLUMBINA_ENGINE") or configured\n'
        "    if not named:\n"
        "        return programs.found(",
    ),
    mutant(
        "the command takes the engine of its working directory",
        VIDEO_COMMAND,
        '    program = program_of(None, os.environ.get("PATH", ""), [Path.cwd()])',
        '    program = program_of(None, os.environ.get("PATH", ""), [])',
    ),
)

_PROTOCOL = (
    mutant(
        "answers are taken in the order of the requests, whatever their ids",
        ENGINE,
        "            asked = self._asked.pop(number, None) if type(number) is int else None",
        "            asked = (\n"
        "                self._asked.pop(next(iter(self._asked)), None)\n"
        "                if self._asked and type(number) is int\n"
        "                else None\n"
        "            )",
    ),
    mutant(
        "an error with an id of null fails the newest request",
        ENGINE,
        "                number = next(iter(self._asked), None)",
        "                number = next(reversed(self._asked), None)",
    ),
    mutant(
        "a gap in the revisions is not fetched again",
        ENGINE,
        "            if revision > self._revision + 1:",
        "            if False:",
    ),
    mutant(
        "a change that the project being fetched holds is told as well",
        ENGINE,
        "isinstance(revision, int) or self._fetching or revision <= self._revision:",
        "isinstance(revision, int) or revision <= self._revision:",
    ),
    mutant(
        "an answer of project.get takes the revision back behind the changes told",
        ENGINE,
        "                self._revision = max(self._revision, revision)",
        "                self._revision = revision",
    ),
    mutant(
        "the next id of an answer behind the changes told is taken",
        ENGINE,
        "if isinstance(next_id, int) and (revision == self._revision or asked.fetching):",
        "if isinstance(next_id, int):",
    ),
    mutant(
        "the project fetched again is not told",
        ENGINE,
        '                self._tell(Changed("project.fetched", told))',
        "                pass",
    ),
    mutant(
        "an engine that was started anew is taken to count on from the last",
        ENGINE,
        '        self._revision = _whole(hello, "revision")',
        '        self._revision = max(self._revision, _whole(hello, "revision"))',
    ),
    mutant(
        "a request over the limit is sent",
        ENGINE,
        "        if len(written) > _WRITE_BYTES:",
        "        if False:",
    ),
    mutant(
        "a line over the limit is read",
        ENGINE,
        "            if end - start > _READ_BYTES:",
        "            if False:",
    ),
    mutant(
        "a line that does not end is gathered without limit",
        ENGINE,
        "        if self._looked > _READ_BYTES:",
        "        if False:",
    ),
)

_LIFECYCLE = (
    mutant(
        "an engine that finds the project held is said to have failed",
        ENGINE,
        "        if code == 3 and isinstance(held, dict):",
        "        if code == 2 and isinstance(held, dict):",
    ),
    mutant(
        "an engine that is starting elsewhere is said to have failed",
        ENGINE,
        '        if code == 3 and told.get("contended") is True:',
        "        if False:",
    ),
    mutant(
        "why an engine could not read the project is not said",
        ENGINE,
        '        if code == 2 and isinstance(told.get("error"), str):',
        "        if False:",
    ),
    mutant(
        "engine.lock is not read again after a held engine named nothing that answers",
        ENGINE,
        '                if wire is None and kind == "held":',
        "                if False:",
    ),
    mutant(
        "an engine that refused is tried again",
        ENGINE,
        "                if error.code != UNREACHABLE or self._closed:",
        "                if self._closed:",
    ),
    mutant(
        "an engine that yields is raced",
        ENGINE,
        "            self._yielded = True",
        "            pass",
    ),
    mutant(
        "an engine that yielded to none that comes is waited for long past its while",
        ENGINE,
        "        until = loop.time() + _YIELD_SECONDS",
        "        until = loop.time() + 10 * _YIELD_SECONDS",
    ),
    mutant(
        "a connection that was lost is not made again",
        ENGINE,
        "            if self._wire is None:\n                await self._reach()",
        "            if False:\n                await self._reach()",
    ),
    mutant(
        "a closed engine connects again",
        ENGINE,
        "        self._closed = True\n        self._drop()",
        "        self._drop()",
    ),
    mutant(
        "an attach that was given up leaves its connection open",
        ENGINE,
        "        except BaseException:\n            self._drop()\n            raise",
        "        except BaseException:\n            raise",
    ),
    mutant(
        "the logs of engines are kept for ever",
        ENGINE,
        '        for old in sorted(cache.glob("engine-*.log"))[: 1 - _KEPT_LOGS]:',
        "        for old in []:",
    ),
)

_ENVIRONMENT = (
    mutant(
        "the libraries of FFmpeg come after the others",
        ENGINE,
        '    environment[variable] = f"{libraries}{os.pathsep}{before}" if before',
        '    environment[variable] = f"{before}{os.pathsep}{libraries}" if before',
    ),
    mutant(
        "an engine that is started is not given the libraries of FFmpeg",
        ENGINE,
        "            env=_environment(),",
        "            env=None,",
    ),
    mutant(
        "the environment of this program is changed",
        ENGINE,
        "    environment = dict(os.environ)",
        "    environment = os.environ",
    ),
)

ENGINE_SETS: dict[str, MutantSet] = {
    "engine-client": MutantSet(
        ("tests/video/test_engine.py", "tests/apps/test_video_command.py"),
        (*_ADDRESSES, *_PROGRAM, *_PROTOCOL, *_LIFECYCLE, *_ENVIRONMENT),
    ),
}
"""The sets of this file, by the name that ``mutate.py`` takes."""
