"""The command line of Columbina.

``columbina`` holds a conversation at the terminal, and ``columbina run TASK``
gives an agent one task. Without a command the first is meant, so
``columbina --continue`` is ``columbina chat --continue``. ``columbina sessions``
lists, shows and copies the sessions that are stored, without a conversation.
``columbina hooks`` lists the hooks of a directory, and trusts those of its
project or takes that trust back. ``columbina setup`` asks which service the
model is to come from and for its key, and writes both down.

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

``columbina hooks`` builds the same layers and loads none of their entries.
It reads the config of the one that has the hooks run, so the files that it
shows and the record that it writes are the ones that a session of the
profile goes by.

What the agent may do is the business of the entry called ``permissions``:
``--permission`` names its preset, and each ``--allow`` adds a rule to the
rules that it has.

Where the model comes from is the business of two entries: the one called
``model``, which loads the adapter, and the frontend, which says which model
of which provider it talks to. A service is a name for what both of them
need, see :mod:`columbina.llm.services`. The service that
``columbina setup`` wrote down is put into the profile before the patches
are applied, so a patch still has the last word. ``--provider`` and the
variable ``COLUMBINA_PROVIDER`` are what the command line asks for, and are
put in last. Either way only a profile whose entry ``model`` loads the
adapter for the OpenAI Chat Completions protocol is changed.
"""

import argparse
import asyncio
import contextlib
import getpass
import logging
import os
import sys
from collections.abc import Awaitable, Callable, Mapping, Sequence
from importlib import resources
from pathlib import Path
from typing import Any, NoReturn, cast, get_args

import yaml
from pydantic import ValidationError

import columbina
from columbina.apps import FRONTEND
from columbina.apps import hooks as written_hooks
from columbina.apps import sessions as stored_sessions
from columbina.apps import setup as chosen_service
from columbina.apps.terminal_text import harmless
from columbina.hooks import Config as HooksConfig
from columbina.kernel import (
    CompositionError,
    Context,
    Entry,
    Insert,
    Layer,
    flatten,
    interpolate,
    mount,
    parse_layer,
)
from columbina.llm.services import PRESETS, Service, adapter_config
from columbina.permissions import Preset
from columbina.session import SESSIONS, SessionError

__all__ = ["main"]

_FRONTEND = "frontend"
_PERMISSIONS = "permissions"
_INTERRUPTED = 130
_CHAT = "chat"
_RUN = "run"
_SESSIONS = "sessions"
_KNOW_THE_LOG = ("session", "agent")
"""The entries that are enough to read a stored session: its log, and the events of the inbox."""
_HOOKS = "hooks"
_RUNS_THE_HOOKS = "columbina.hooks"
"""The plugin whose config says where the hook files and the record of trust lie."""
_NOT_A_COMMAND = ("-h", "--help", "--version")
"""What may stand first on the command line without meaning the conversation."""
_SETUP = "setup"
_MODEL = "model"
_SPEAKS_OPENAI = "columbina.llm.adapters.openai_chat"
"""The plugin that talks to every service that has a name."""
_STORED_KEYS = "${COLUMBINA_HOME}/" + chosen_service.KEYS
"""Where the keys lie that the user stored, as a config says it."""


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


def _call_of(config: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of what the config of the frontend says of the model."""
    said: object = config.get("model")
    if said is None:
        return {}
    if not isinstance(said, dict):
        raise _UsageError(f"the entry {_FRONTEND!r} has a 'model' that is not a mapping")
    return dict(cast("dict[str, Any]", said))


def _served(
    entries: list[Entry], service: Service, env: Mapping[str, str], model: str | None
) -> bool:
    """Point the adapter of the entries and their frontend at a service.

    The config of the adapter is written anew as a whole: what a profile
    says of one service says nothing of another. Of what the frontend says
    of the model, the provider, the model and the size of its window change
    and the rest stays.

    Args:
        entries: The entries. They are changed in place.
        service: The service.
        env: The environment. The variables that name the model and the size
            of its window go before what is known of the service.
        model: The model that the command line names, if it names one.

    Returns:
        Whether there was an adapter to point. Nothing is changed where
        there is none.

    Raises:
        _UsageError: Nothing names a model, and the service has none that
            is used by default.
    """
    at = next(
        (
            position
            for position, entry in enumerate(entries)
            if entry.id == _MODEL and entry.plugin == _SPEAKS_OPENAI and not entry.disabled
        ),
        None,
    )
    if at is None:
        return False
    named = model or env.get("COLUMBINA_MODEL") or service.model
    if named is None:
        raise _UsageError(
            f"no model is named for the service {service.name}, and it has none that is used "
            "by default. Name one with --model, or choose one with 'columbina setup'"
        )
    config = adapter_config(service, _STORED_KEYS, env)
    entries[at] = entries[at].model_copy(update={"config": config})
    for position, entry in enumerate(entries):
        if entry.id == _FRONTEND and not entry.disabled:
            config = _config_of(entry)
            call = _call_of(config)
            call["provider"] = service.name
            # A dollar sign of a name stands for itself. The variable is named, and not
            # what it holds, the way a profile names it.
            call["model"] = (
                "${COLUMBINA_MODEL}"
                if model is None and env.get("COLUMBINA_MODEL")
                else named.replace("$", "$$")
            )
            if env.get("COLUMBINA_CONTEXT_WINDOW"):
                call["context_window"] = "${COLUMBINA_CONTEXT_WINDOW}"
            elif service.context_window is not None and named == service.model:
                call["context_window"] = service.context_window
            else:
                # What is known of the size is known of one model, and what a profile says
                # of it is said of the model of another service.
                call.pop("context_window", None)
            config["model"] = call
            entries[position] = entry.model_copy(update={"config": config})
    return True


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


def _flattened(
    args: argparse.Namespace,
    home: Path,
    first: Callable[[list[Entry]], object] | None = None,
) -> list[Entry]:
    """Return the entries of the profile, with the patches of the user and of the command line.

    Args:
        args: What the command line says.
        home: The home of the program.
        first: Changes the entries of the profile before a patch is applied
            to them.
    """
    entries = flatten([_profile(args.profile)], lookup=_bundle)
    if first is not None:
        first(entries)
    layers = [Layer(patch=(Insert(insert=tuple(entries)),))]
    mine = home / "config.yml"
    if mine.is_file():
        layers.append(_file_layer(mine))
    layers.extend(_file_layer(Path(patch)) for patch in args.patch)
    return flatten(layers)


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


def _config_of_the_hooks(
    args: argparse.Namespace, home: Path, env: Mapping[str, str]
) -> HooksConfig:
    """Work out where a session of the profile has its hooks from."""
    for entry in _flattened(args, home):
        # What an entry is called says nothing about what it loads.
        if entry.plugin == _RUNS_THE_HOOKS and not entry.disabled:
            try:
                config = interpolate(entry.config, env)
            except CompositionError as error:
                raise CompositionError(f"entry {entry.id!r}: {error}") from error
            try:
                return HooksConfig.model_validate({} if config is None else config)
            except ValidationError as error:
                raise _UsageError(
                    f"{entry.id} ({entry.plugin}) has an invalid config: {error}"
                ) from error
    raise _UsageError(
        f"the profile runs no hooks: none of its entries loads {_RUNS_THE_HOOKS}, or the one "
        "that does is disabled. So there are no hooks to look at"
    )


def _entries(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> list[Entry]:
    """Work out the plugin entries that the command line asks for."""
    if args.command == _CHAT and args.resume and args.session_id is not None:
        raise _UsageError(
            "--continue takes up the session that was used last, and --session-id names one: "
            "give one of the two"
        )
    stored = chosen_service.chosen(home)

    def as_chosen(profile: list[Entry]) -> None:
        # A profile that talks to its model in another way is left as it is.
        if stored.use is not None:
            service = chosen_service.service_named(stored.use, stored)
            _served(profile, service, env, args.model)

    entries = _flattened(args, home, as_chosen)
    _permit(entries, args)
    named = args.provider or env.get("COLUMBINA_PROVIDER")
    if named:
        service = chosen_service.service_named(named, stored)
        if not _served(entries, service, env, args.model):
            raise _UsageError(
                f"the profile has no entry {_MODEL!r} that loads {_SPEAKS_OPENAI}, or it is "
                f"disabled, so there is nothing to point at the service {named}"
            )
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
                config["model"] = {**_call_of(config), "model": args.model.replace("$", "$$")}
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
        if root.get(SESSIONS) is None:
            # What an entry is called says nothing about what its plugin provides.
            _tell(
                f"the entry {_KNOW_THE_LOG[0]!r} of this profile provides no sessions, so "
                "there are none to look at"
            )
            return 2
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


def _hooks(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """List the hooks of a working directory, or change whether those of its project are trusted."""
    config = _config_of_the_hooks(args, home, env)
    cwd = Path() if args.cwd is None else Path(args.cwd)
    if not cwd.is_dir():
        raise _UsageError(f"the working directory {args.cwd!r} is not a directory")

    def show(told: str) -> None:
        # What a hook file holds was written by the user, or by whoever wrote the project.
        print(harmless(told))
        # Whoever goes on after this may go by it that the text has left the program.
        sys.stdout.flush()

    try:
        written_hooks.run(config, cwd.resolve(), args.what, show)
    except written_hooks.HooksError as error:
        _tell(str(error))
        return 2
    except OSError as error:
        _tell(f"what there is to say about the hooks could not be written: {error}")
        return 2
    return 0


def _setup(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Ask for the service of the model and its key and store them, or delete what is stored."""

    def show(told: str) -> None:
        # What is told names files, and repeats what a person typed or a file holds.
        print(harmless(told))

    def asked(question: str) -> str:
        return input(harmless(question))

    def asked_unseen(question: str) -> str:
        return getpass.getpass(harmless(question))

    if args.forget is not None:
        chosen_service.forget(home, args.forget, show)
        return 0
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        raise _UsageError(
            "'columbina setup' asks its questions at a terminal, and this is none. A key can "
            "be given in the environment variable of its service instead"
        )
    try:
        chosen_service.run(home, env, asked, asked_unseen, show)
    except (EOFError, KeyboardInterrupt):
        # The line of the question that was not answered is still open.
        print()
        _tell("nothing was stored")
        return _INTERRUPTED
    return 0


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
        "--provider",
        metavar="NAME",
        help="the service that the model comes from: "
        + ", ".join(service.name for service in PRESETS)
        + ", or one that was added with 'columbina setup'",
    )
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


def _written(command: argparse.ArgumentParser) -> None:
    """Add what ``columbina hooks`` takes: what to do with the hooks of a directory."""
    whats = command.add_subparsers(dest="what", metavar="WHAT", required=True)
    helps = {
        written_hooks.LIST: "list the hooks that count in the working directory, and say "
        "whether those of its project are trusted",
        written_hooks.TRUST: "trust the hook file of the project as it is now, and show what "
        "runs from then on",
        written_hooks.DISTRUST: "take that trust back",
    }
    for name, text in helps.items():
        what = whats.add_parser(name, help=text)
        _from_the_profile(what, "cli")
        what.add_argument(
            "--cwd",
            metavar="DIR",
            help="the working directory whose hooks are meant; the current one by default",
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
    written = commands.add_parser(
        _HOOKS,
        help="list the hooks of a directory, and trust those of its project or trust them no more",
    )
    _written(written)
    setup = commands.add_parser(
        _SETUP,
        help="choose the service that the model comes from, and store the API key for it",
    )
    setup.add_argument(
        "--forget",
        metavar="NAME",
        help="delete what is stored for a service, its key and what was said of it, and stop",
    )
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
        if args.command == _HOOKS:
            return _hooks(args, home, env)
        if args.command == _SETUP:
            return _setup(args, home, env)
        entries = _entries(args, home, env)
        if args.dump_config:
            dumped = _dump(entries)
            # A file or another program gets the entries as they are, to be read again.
            sys.stdout.write(harmless(dumped) if sys.stdout.isatty() else dumped)
            return 0
        return asyncio.run(_run(entries, env))
    except (_UsageError, CompositionError, chosen_service.SetupError) as error:
        _tell(str(error))
        return 2
    except KeyboardInterrupt:
        return _INTERRUPTED
    finally:
        log.removeHandler(handler)
