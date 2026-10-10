"""The command line of Columbina.

``columbina`` holds a conversation at the terminal, and ``columbina run TASK``
gives an agent one task. Without a command the first is meant, so
``columbina --continue`` is ``columbina chat --continue``. ``columbina sessions``
lists, shows and copies the sessions that are stored, without a conversation.

Either way the program is built from layers, which are lists of plugin
entries: first the profile, then the patch of the user, then the patches named
on the command line, and last what the command line itself asks for. The
entry called ``frontend`` provides what is then run.

The command line changes only what it names. A setting that it leaves open
stays as the layers before it have it.

``columbina sessions`` builds the same layers and loads two of their entries,
the ones called ``session`` and ``agent``: they know what a log holds. So the
sessions that it finds are the ones that the profile stores, and no model is
needed to look at them.

What the agent may do is the business of the entry called ``permissions``:
``--permission`` names its preset, and each ``--allow`` adds a rule to the
rules that it has.
"""

import argparse
import asyncio
import contextlib
import logging
import os
import sys
from collections.abc import Awaitable, Callable, Mapping, Sequence
from importlib import resources
from pathlib import Path
from typing import Any, NoReturn, cast, get_args

import yaml

import columbina
from columbina.apps import FRONTEND
from columbina.apps import sessions as stored_sessions
from columbina.apps.terminal_text import harmless
from columbina.kernel import CompositionError, Context, Entry, Layer, flatten, mount, parse_layer
from columbina.permissions import Preset
from columbina.session import SessionError

__all__ = ["main"]

_FRONTEND = "frontend"
_PERMISSIONS = "permissions"
_INTERRUPTED = 130
_CHAT = "chat"
_RUN = "run"
_SESSIONS = "sessions"
_KNOW_THE_LOG = ("session", "agent")
"""The entries that are enough to read a stored session: its log, and the events of the inbox."""
_NOT_A_COMMAND = ("-h", "--help", "--version")
"""What may stand first on the command line without meaning the conversation."""


class _UsageError(Exception):
    """The command line asks for something that cannot be done."""


def _bundles() -> list[str]:
    """Return the names of the profiles that come with Columbina."""
    files = resources.files("columbina.bundles").iterdir()
    return sorted(file.name.removesuffix(".yml") for file in files if file.name.endswith(".yml"))


def _bundle(name: str) -> Layer:
    """Return the profile that comes with Columbina under ``name``."""
    if name not in _bundles():
        raise KeyError(name)
    text = resources.files("columbina.bundles").joinpath(f"{name}.yml").read_text(encoding="utf-8")
    return parse_layer(text, source=f"profile {name}")


def _file_layer(path: Path) -> Layer:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise _UsageError(f"{path} cannot be read: {error.strerror}") from error
    return parse_layer(text, source=str(path))


def _profile(name: str) -> Layer:
    """Return the first layer: a file if ``name`` is the path of one, or else a named profile."""
    path = Path(name)
    if path.is_file():
        return _file_layer(path)
    try:
        return _bundle(name)
    except KeyError:
        known = ", ".join(_bundles())
        raise _UsageError(
            f"there is no profile {name!r}. The profiles that come with Columbina are: {known}"
        ) from None


def _config_of(entry: Entry) -> dict[str, Any]:
    """Return a copy of the config of an entry, which the command line is about to add to."""
    config: object = entry.config
    if config is None:
        return {}
    if not isinstance(config, dict):
        raise _UsageError(f"the config of the entry {entry.id!r} is not a mapping")
    return dict(cast("dict[str, Any]", config))


def _permit(entries: list[Entry], args: argparse.Namespace) -> None:
    """Hand the preset and the rules of the command line to the entry of the permissions."""
    if args.permission is None and not args.allow:
        return
    for position, entry in enumerate(entries):
        if entry.id == _PERMISSIONS and not entry.disabled:
            config = _config_of(entry)
            if args.permission is not None:
                config["preset"] = args.permission
            if args.allow:
                rules: object = config.get("allow") or []
                if not isinstance(rules, list):
                    raise _UsageError(
                        f"the entry {_PERMISSIONS!r} has rules under 'allow' that are not a list"
                    )
                config["allow"] = [*cast("list[object]", rules), *args.allow]
            entries[position] = entry.model_copy(update={"config": config})
            return
    raise _UsageError(
        f"the profile has no entry {_PERMISSIONS!r}, or it is disabled, so --permission and "
        "--allow have nothing to change"
    )


def _flattened(args: argparse.Namespace, home: Path) -> list[Entry]:
    """Return the entries of the profile, with the patches of the user and of the command line."""
    layers = [_profile(args.profile)]
    mine = home / "config.yml"
    if mine.is_file():
        layers.append(_file_layer(mine))
    layers.extend(_file_layer(Path(patch)) for patch in args.patch)
    return flatten(layers, lookup=_bundle)


def _entries_that_know_the_log(args: argparse.Namespace, home: Path) -> list[Entry]:
    """Work out the entries that are enough to look at stored sessions."""
    entries = [
        entry
        for entry in _flattened(args, home)
        if entry.id in _KNOW_THE_LOG and not entry.disabled
    ]
    if _KNOW_THE_LOG[0] not in [entry.id for entry in entries]:
        raise _UsageError(
            f"the profile has no entry {_KNOW_THE_LOG[0]!r}, or it is disabled, so there are "
            "no stored sessions to look at"
        )
    return entries


def _entries(args: argparse.Namespace, home: Path) -> list[Entry]:
    """Work out the plugin entries that the command line asks for."""
    if args.command == _CHAT and args.resume and args.session_id is not None:
        raise _UsageError(
            "--continue takes up the session that was used last, and --session-id names one: "
            "give one of the two"
        )
    entries = _flattened(args, home)
    _permit(entries, args)
    for position, entry in enumerate(entries):
        if entry.id == _FRONTEND:
            if entry.disabled:
                raise _UsageError(
                    f"the entry {_FRONTEND!r} is disabled, so there is nothing to run the task"
                )
            config = _config_of(entry)
            if args.cwd is not None:
                if not Path(args.cwd).is_dir():
                    # Otherwise an agent would be sent to work where nothing is.
                    raise _UsageError(f"the working directory {args.cwd!r} is not a directory")
                config["cwd"] = str(Path(args.cwd).resolve())
            config.setdefault("cwd", str(Path().resolve()))
            if args.session_id is not None:
                config["session_id"] = args.session_id
            if args.model is not None:
                config["model"] = {**config.get("model", {}), "model": args.model}
            if args.command == _RUN:
                config["task"] = args.task
                if args.json is not None:
                    config["json_lines"] = args.json
            elif args.resume:
                config["resume"] = True
                # Which session to take up is said here. One that a profile names is not it.
                config.pop("session_id", None)
            entries[position] = entry.model_copy(update={"config": config})
            return entries
    raise _UsageError(
        f"the profile has no entry {_FRONTEND!r}, so there is nothing to run the task"
    )


def _dump(entries: Sequence[Entry]) -> str:
    """Return the entries as YAML, with the references to the environment as they were written."""
    plain = [
        entry.model_dump(exclude_defaults=True) | {"id": entry.id, "plugin": entry.plugin}
        for entry in entries
    ]
    ordered = [
        {key: item[key] for key in ("id", "plugin", "config", "disabled") if key in item}
        for item in plain
    ]
    return yaml.safe_dump(ordered, sort_keys=False, allow_unicode=True)


def _tell(message: str) -> None:
    """Say something to whoever started the program.

    What is said may name a file or repeat what a file holds, so it is made fit for a
    terminal first.
    """
    print(harmless(f"columbina: {message}"), file=sys.stderr)


class _Fit(logging.Formatter):
    """Writes a record of the log so that a terminal shows it and does not act on it."""

    def format(self, record: logging.LogRecord) -> str:
        """Return the record as text, with what a terminal would act on made harmless."""
        return harmless(super().format(record))


class _Parser(argparse.ArgumentParser):
    """Reads the command line, and says what is wrong with it in a form fit for a terminal.

    What is wrong is said with the words that were given, and whatever starts
    the program may have them from somewhere else. The commands are read by
    parsers of this kind too.
    """

    def error(self, message: str) -> NoReturn:
        """Say what is wrong with the command line, and end the program."""
        super().error(harmless(message))


async def _loaded(
    entries: Sequence[Entry], env: Mapping[str, str], then: Callable[[Context], Awaitable[int]]
) -> int:
    """Load the entries, do what is to be done with them, and unload everything again."""
    root = Context()
    try:
        composition = mount(root, entries, env=env)
        await root.settled()
        problems = composition.problems()
        if problems:
            for problem in problems:
                _tell(str(problem))
            return 2
        return await then(root)
    finally:
        await root.fiber.dispose()


async def _run(entries: Sequence[Entry], env: Mapping[str, str]) -> int:
    """Load the entries, run the frontend, and unload everything again."""

    async def run(root: Context) -> int:
        frontend = root.get(FRONTEND)
        if frontend is None:
            plugin = next(entry.plugin for entry in entries if entry.id == _FRONTEND)
            _tell(f"the entry {_FRONTEND!r} loads {plugin}, which provides nothing to run the task")
            return 2
        return await frontend.run()

    return await _loaded(entries, env, run)


async def _look(entries: Sequence[Entry], env: Mapping[str, str], args: argparse.Namespace) -> int:
    """Load what knows the log, and list, show or copy the sessions that are stored."""
    cwd = Path() if args.cwd is None else Path(args.cwd)

    async def look(root: Context) -> int:
        try:
            told = await stored_sessions.run(
                root, cwd.resolve(), args.what, getattr(args, "id", None), args.copy_id
            )
        except (SessionError, ValueError) as error:
            _tell(str(error))
            return 2
        # What a session holds was written by users and by models.
        print(harmless(told))
        return 0

    return await _loaded(entries, env, look)


def _from_the_profile(command: argparse.ArgumentParser, profile: str) -> None:
    """Add the arguments that say which program is meant and where it works."""
    command.add_argument(
        "--profile",
        default=profile,
        metavar="NAME",
        help="the profile to load: a name that comes with Columbina, or the path of a file",
    )
    command.add_argument(
        "--patch",
        action="append",
        default=[],
        metavar="FILE",
        help="a file of changes to the profile; may be given several times",
    )


def _shared(command: argparse.ArgumentParser, profile: str) -> None:
    """Add the arguments that every command takes that has an agent work."""
    _from_the_profile(command, profile)
    command.add_argument("--model", metavar="ID", help="the model to use")
    command.add_argument(
        "--cwd",
        metavar="DIR",
        help="the working directory of the agent; the current directory if nothing names one",
    )
    command.add_argument("--session-id", metavar="ID", help="the session to start or to continue")
    command.add_argument(
        "--permission",
        choices=get_args(Preset),
        help="how much the agent may do without anybody being asked",
    )
    command.add_argument(
        "--allow",
        action="append",
        default=[],
        metavar="RULE",
        help="a rule that lets calls go ahead, such as 'run(git status*)'; "
        "may be given several times",
    )
    command.add_argument(
        "--dump-config",
        action="store_true",
        help="print the plugin entries that would be loaded, and stop",
    )


def _stored(command: argparse.ArgumentParser) -> None:
    """Add what ``columbina sessions`` takes: what to do, and with which session."""
    whats = command.add_subparsers(dest="what", metavar="WHAT", required=True)
    listing = whats.add_parser(
        stored_sessions.LIST, help="list the sessions of the working directory"
    )
    showing = whats.add_parser(
        stored_sessions.SHOW, help="tell what happened in a session, turn by turn"
    )
    copying = whats.add_parser(
        stored_sessions.FORK,
        help="copy a session up to its last whole turn, and print the id of the copy",
    )
    showing.add_argument("id", metavar="ID", help="the session to show")
    copying.add_argument("id", metavar="ID", help="the session to copy")
    for what in (listing, showing, copying):
        _from_the_profile(what, "cli")
        what.add_argument(
            "--cwd",
            metavar="DIR",
            help="the working directory whose sessions are meant; the current one by default",
        )
        what.set_defaults(copy_id=None)
    copying.add_argument(
        "--session-id",
        dest="copy_id",
        metavar="ID",
        help="the id that the copy is to have; one is made up by default",
    )


def _parser() -> argparse.ArgumentParser:
    parser = _Parser(
        prog="columbina",
        description="A plugin-based agent harness. Without a command, a conversation begins.",
    )
    parser.add_argument("--version", action="version", version=f"columbina {columbina.__version__}")
    commands = parser.add_subparsers(dest="command", metavar="COMMAND", required=True)
    chat = commands.add_parser(_CHAT, help="hold a conversation at the terminal")
    _shared(chat, "cli")
    chat.add_argument(
        "--continue",
        dest="resume",
        action="store_true",
        help="take up the session of this directory that was used last",
    )
    run = commands.add_parser(_RUN, help="run one task and print the answer")
    run.add_argument("task", metavar="TASK", help="what the agent is asked to do")
    _shared(run, "headless")
    run.add_argument(
        "--json",
        action=argparse.BooleanOptionalAction,
        help="report the run as lines of JSON, or as the answer alone",
    )
    stored = commands.add_parser(
        _SESSIONS, help="list, show or copy the sessions that are stored, without a conversation"
    )
    _stored(stored)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    """Run the command line and return the exit code of the program."""
    given = list(sys.argv[1:] if argv is None else argv)
    if not given or (given[0].startswith("-") and given[0] not in _NOT_A_COMMAND):
        given.insert(0, _CHAT)
    args = _parser().parse_args(given)
    for stream in (sys.stdout, sys.stderr):
        # Text goes out as UTF-8 also where the system would choose something else.
        with contextlib.suppress(AttributeError, ValueError):
            stream.reconfigure(encoding="utf-8", errors="replace")  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    env = dict(os.environ)
    env.setdefault("COLUMBINA_HOME", str(Path.home() / ".columbina"))
    log = logging.getLogger("columbina")
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(_Fit("columbina: %(message)s"))
    # A conversation shows what is logged above its prompt, and keeps it from here for as
    # long as the prompt is there. Before and after that, the log goes here for it too.
    log.addHandler(handler)
    try:
        home = Path(env["COLUMBINA_HOME"])
        if args.command == _SESSIONS:
            return asyncio.run(_look(_entries_that_know_the_log(args, home), env, args))
        entries = _entries(args, home)
        if args.dump_config:
            dumped = _dump(entries)
            # A file or another program gets the entries as they are, to be read again.
            sys.stdout.write(harmless(dumped) if sys.stdout.isatty() else dumped)
            return 0
        return asyncio.run(_run(entries, env))
    except (_UsageError, CompositionError) as error:
        _tell(str(error))
        return 2
    except KeyboardInterrupt:
        return _INTERRUPTED
    finally:
        log.removeHandler(handler)
