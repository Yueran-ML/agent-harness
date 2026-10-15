"""The command line of Columbina.

``columbina`` holds a conversation at the terminal, and ``columbina run TASK``
gives an agent one task. Without a command the first is meant, so
``columbina --continue`` is ``columbina chat --continue``. ``columbina serve``
holds a conversation for another program, which talks to it over its standard
input and output. ``columbina sessions``
lists, shows and copies the sessions that are stored, without a conversation.
``columbina hooks`` lists the hooks of a directory, and trusts those of its
project or takes that trust back. ``columbina setup`` asks which service the
model is to come from and for its key, and writes both down. For another
program it does that without a question: ``columbina setup --list`` says what
is set up, and ``columbina setup --from-stdin`` stores what a document on its
standard input says, each with one document of JSON on its standard output.
``columbina login github`` signs in to GitHub with a code that the user enters
there, ``columbina login --status`` says who is signed in, and
``columbina logout github`` deletes the sign-in, each with lines of JSON for
another program where ``--json`` asks for them. ``columbina doctor`` sees
whether a session could start, and says what is in the way: the service and
its key, the shell, or the sandbox.

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
import json
import logging
import os
import shutil
import sys
import threading
import time
import webbrowser
from collections.abc import Awaitable, Callable, Mapping, Sequence
from importlib import resources
from pathlib import Path
from typing import Any, NoReturn, cast, get_args

import yaml
from pydantic import ValidationError

import columbina
from columbina.apps import FRONTEND
from columbina.apps import doctor as health
from columbina.apps import hooks as written_hooks
from columbina.apps import sessions as stored_sessions
from columbina.apps import setup as chosen_service
from columbina.apps.serve import wire as serve_wire
from columbina.apps.serve.wire import WIRE, Wire
from columbina.apps.terminal_text import escaped, harmless
from columbina.github import oauth, signing, store
from columbina.github.app import GitHubError, web_of
from columbina.github.client import GitHub
from columbina.github.store import Tokens
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
_SANDBOX = "sandbox"
_INTERRUPTED = 130
_CHAT = "chat"
_RUN = "run"
_SERVE = "serve"
_SESSIONS = "sessions"
_KNOW_THE_LOG = ("session", "agent")
"""The entries that are enough to read a stored session: its log, and the events of the inbox."""
_HOOKS = "hooks"
_RUNS_THE_HOOKS = "columbina.hooks"
"""The plugin whose config says where the hook files and the record of trust lie."""
_NOT_A_COMMAND = ("-h", "--help", "--version")
"""What may stand first on the command line without meaning the conversation."""
_SETUP = "setup"
_LOGIN = "login"
_LOGOUT = "logout"
_GITHUB = "github"
_SHOWN_REPOSITORIES = 20
"""How many repositories a person is shown, before how many more there are."""
_STOPPED = "the sign-in was stopped"
_DOCTOR = "doctor"
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


def _held(entries: list[Entry], args: argparse.Namespace) -> None:
    """Hand what the command line says runs the commands to the entry of the sandbox."""
    if args.sandbox is None:
        return
    for position, entry in enumerate(entries):
        if entry.id == _SANDBOX and not entry.disabled:
            config = _config_of(entry)
            config["provider"] = args.sandbox.replace("$", "$$")
            entries[position] = entry.model_copy(update={"config": config})
            return
    raise _UsageError(
        f"the profile has no entry {_SANDBOX!r}, or it is disabled, so --sandbox has nothing "
        "to change"
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
    _held(entries, args)
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
            # Whoever is served says over the wire which session is to be open.
            if args.command != _SERVE and args.session_id is not None:
                config["session_id"] = args.session_id
            if args.model is not None:
                config["model"] = {**_call_of(config), "model": args.model.replace("$", "$$")}
            if args.command == _RUN:
                config["task"] = args.task
                if args.json is not None:
                    config["json_lines"] = args.json
            elif args.command == _CHAT and args.resume:
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
    entries: Sequence[Entry],
    env: Mapping[str, str],
    then: Callable[[Context], Awaitable[int]],
    wire: Wire | None = None,
) -> int:
    """Load the entries, do what is to be done with them, and unload everything again.

    Args:
        entries: The entries.
        env: The environment that their configs refer to.
        then: Does what is to be done, and returns the exit code.
        wire: The wire that the program speaks over, if it serves another one.
    """
    root = Context()
    try:
        if wire is not None:
            root.provide(WIRE, wire)
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


async def _run(entries: Sequence[Entry], env: Mapping[str, str], wire: Wire | None = None) -> int:
    """Load the entries, run the frontend, and unload everything again."""

    async def run(root: Context) -> int:
        frontend = root.get(FRONTEND)
        if frontend is None:
            plugin = next(entry.plugin for entry in entries if entry.id == _FRONTEND)
            _tell(f"the entry {_FRONTEND!r} loads {plugin}, which provides nothing to run the task")
            return 2
        return await frontend.run()

    return await _loaded(entries, env, run, wire)


async def _serve(entries: Sequence[Entry], env: Mapping[str, str]) -> int:
    """Take the standard input and output for the protocol, and then load and run.

    They are taken before a single plugin is loaded. A plugin that prints
    while it loads would otherwise put its words where the first answer
    belongs. From here on, what is printed goes to the standard error.
    """
    try:
        wire = serve_wire.taken()
    except OSError as error:
        _tell(
            "'columbina serve' talks over its standard input and output, and this process "
            f"has none: {error}"
        )
        return 2
    return await _run(entries, env, wire)


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


def _answer(document: Mapping[str, object]) -> None:
    """Write a document of JSON to the standard output, on a line of its own.

    A program reads it, and a person may look at it at a terminal. So what a
    terminal would act on is written by its number, which says the same to
    whoever reads the JSON.
    """
    sys.stdout.write(escaped(json.dumps(document, ensure_ascii=False)) + "\n")


def _from_another_program(option: str) -> bytes:
    """Return what another program wrote to the standard input, as the bytes that came.

    No more is read than a document may be long, and one byte. A document
    without end is not waited for.

    Args:
        option: The option of ``columbina setup`` that reads the document.

    Raises:
        SetupError: The standard input is a terminal, or there is none that
            can be read.
    """
    try:
        # Windows says of a standard input that leads nowhere that it is a terminal too.
        if sys.stdin.isatty():
            raise chosen_service.SetupError(
                f"'columbina setup {option}' reads what another program writes to its "
                "standard input, and this one is a terminal, or leads nowhere. A key that is "
                "typed at a terminal is shown. 'columbina setup' asks for it without showing it"
            )
        # The bytes, and not the text that the system would make of them: a document is
        # UTF-8 whatever the system takes its text for.
        return sys.stdin.buffer.read(chosen_service.MAX_DOCUMENT_BYTES + 1)
    except (AttributeError, OSError, ValueError):
        # A process that was started without a standard input has none, or one that is closed.
        raise chosen_service.SetupError(
            f"'columbina setup {option}' reads a document from its standard input, and this "
            "process has none that can be read"
        ) from None


def _setup_for_a_program(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Say what is set up, or store what a document says, for another program.

    The standard output holds one document of JSON and nothing else: what
    was asked for, or what is wrong, in a sentence. What a person would be
    shown goes to the standard error.

    Whoever interrupts the storing is answered nothing, since what was stored
    by then may be the key, or the key and what was chosen.
    """

    def show(told: str) -> None:
        # Written, and not printed: what is printed to a standard error that is not there
        # goes to the standard output, which is for the answer alone. And a standard error
        # that takes nothing does not keep the answer back. By then the key may be stored.
        with contextlib.suppress(AttributeError, OSError, ValueError):
            sys.stderr.write(harmless(told) + "\n")

    try:
        if args.list:
            answer = chosen_service.listed(home, env)
        elif args.check:
            document = _from_another_program("--check")
            answer = asyncio.run(chosen_service.checked(home, env, document, show))
        elif args.forget is not None:
            forgot = chosen_service.forget(home, args.forget, show)
            what = {"name": args.forget, "key": forgot.key, "chosen": forgot.chosen}
            answer: dict[str, object] = {"forgotten": what}
            if forgot.unasked is not None:
                answer["unasked"] = forgot.unasked
        else:
            answers = chosen_service.given(home, env, _from_another_program("--from-stdin"), show)
            kept = chosen_service.store(home, env, answers, show)
            answer = chosen_service.stored(answers, kept)
    except chosen_service.SetupError as error:
        _answer({"error": str(error)})
        return 2
    _answer(answer)
    # A service that listed no models is answered as an error is.
    return 2 if "error" in answer else 0


def _setup(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Ask for the service of the model and its key and store them, or do what an option says."""

    def show(told: str) -> None:
        # What is told names files, and repeats what a person typed or a file holds.
        print(harmless(told))

    def asked(question: str) -> str:
        return input(harmless(question))

    def asked_unseen(question: str) -> str:
        return getpass.getpass(harmless(question))

    if args.json and args.forget is None:
        raise _UsageError(
            "--json goes with --forget. What setup does for another program besides is "
            "answered in JSON as it is"
        )
    if args.list or args.from_stdin or args.check or args.json:
        return _setup_for_a_program(args, home, env)
    if args.forget is not None:
        chosen_service.forget(home, args.forget, show)
        return 0
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        raise _UsageError(
            "'columbina setup' asks its questions at a terminal, and this is none. A key can "
            "be given in the environment variable of its service instead"
        )
    try:
        answers = chosen_service.asked(home, env, asked, asked_unseen, show)
    except (EOFError, KeyboardInterrupt):
        # The line of the question that was not answered is still open. Nothing is stored
        # before the last question is answered, so this much can be said for certain.
        print()
        _tell("nothing was stored")
        return _INTERRUPTED
    # Whoever interrupts the storing itself is told nothing of what was stored: it may be
    # the key, or the key and what was chosen.
    chosen_service.store(home, env, answers, show)
    return 0


def _doctor(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """See whether a session of the command line could start, and say what is in the way.

    Returns:
        1 if something is in the way of a session, and 0 otherwise.
    """
    try:
        entries = _entries(args, home, env)
    except (_UsageError, CompositionError, chosen_service.SetupError) as error:
        # Nothing else of a session can be looked at then, but for the shell of this machine.
        findings = [health.Finding("fail", "profile", str(error)), health.shell()]
    else:
        mine = home / "config.yml"
        patches = [*([str(mine)] if mine.is_file() else []), *args.patch]
        said = f"{args.profile}, with {', '.join(patches)}" if patches else args.profile
        cwd = Path() if args.cwd is None else Path(args.cwd)
        looked = asyncio.run(health.looked(entries, env, cwd.resolve()))
        findings = [health.Finding("ok", "profile", said), *looked]
    width = shutil.get_terminal_size().columns
    # What was found repeats what files and services wrote.
    print(harmless(health.shown(findings, width)))
    return 1 if any(finding.state == "fail" for finding in findings) else 0


def _opened(url: str, env: Mapping[str, str], platform: str = sys.platform) -> bool:
    """Open a page in a browser of the desktop, where there is a desktop. Whether it opened.

    A terminal without a desktop is given no browser: one of the terminal would take it
    over while the program waits.
    """
    if platform not in ("win32", "darwin") and not (
        env.get("DISPLAY") or env.get("WAYLAND_DISPLAY")
    ):
        return False
    try:
        return webbrowser.open(url)
    except (webbrowser.Error, OSError):
        return False


def _sentence(said: str) -> str:
    """Return what was said as a sentence of its own: with a capital and a full stop."""
    said = said[:1].upper() + said[1:]
    return said if said.endswith(".") else f"{said}."


def _when_the_input_closes() -> asyncio.Event:
    """Return an event that is set when the standard input comes to its end.

    Another program stops a sign-in by closing the standard input of the command. Where
    there is no standard input to watch, the event is never set.
    """
    loop = asyncio.get_running_loop()
    closed = asyncio.Event()
    try:
        descriptor = sys.stdin.fileno()
    except (AttributeError, OSError, ValueError):
        return closed

    def watch() -> None:
        # The descriptor, and not sys.stdin: a thread that is still in a read of a stream of
        # Python when the program ends holds its lock, and Python cannot end cleanly.
        with contextlib.suppress(OSError):
            while os.read(descriptor, 4096):
                pass
        with contextlib.suppress(RuntimeError):
            loop.call_soon_threadsafe(closed.set)

    threading.Thread(target=watch, name="columbina-input", daemon=True).start()
    return closed


class _Stopped(Exception):  # noqa: N818 - how the waiting is told to stop, and no error
    """The program that waits for the sign-in closed the standard input."""


async def _waited(github: GitHub, code: oauth.Code, stop: asyncio.Event) -> Tokens | None:
    """Wait until the user entered the code, and return the tokens, or ``None`` if stopped.

    The stop is seen between two asks for the tokens, and never in the middle of one.
    """

    async def pause(seconds: float) -> None:
        with contextlib.suppress(TimeoutError):
            await asyncio.wait_for(stop.wait(), seconds)
        if stop.is_set():
            raise _Stopped

    try:
        return await oauth.wait_for_the_user(github.http, github.app, code, sleep=pause)
    except _Stopped:
        return None


async def _person(found: signing.Who, home: Path) -> dict[str, object]:
    """Return who signed in, for another program: the names and the picture."""
    picture = await asyncio.to_thread(signing.avatar, home)
    return {"login": found.login, "name": found.name, "avatar": picture}


def _say_who(found: signing.Who) -> str:
    named = f" ({found.name})" if found.name else ""
    return f"Signed in to GitHub as @{found.login}{named}."


def _say_the_reach(found: signing.Who, reached: list[str], install_url: str) -> None:
    """Say who signed in, and which repositories Columbina may reach in that name."""
    print(harmless(_say_who(found)))
    if not reached:
        print(
            harmless(
                "Columbina may reach no repository of yours yet. Choose them on GitHub: "
                f"{install_url}"
            )
        )
        return
    print("Columbina may reach these repositories:")
    for name in reached[:_SHOWN_REPOSITORIES]:
        print(harmless(f"  {name}"))
    if len(reached) > _SHOWN_REPOSITORIES:
        print(f"  and {len(reached) - _SHOWN_REPOSITORIES} more")
    print(harmless(f"Choose others on GitHub: {install_url}"))


def _login(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Sign in to GitHub, or say who is signed in."""
    if args.status:
        return _status(args, home, env)
    if args.service is None:
        raise _UsageError(
            "say where to sign in, as in 'columbina login github', or ask who is signed in "
            "with 'columbina login --status'"
        )
    if not args.json:
        return _login_at_a_terminal(home, env)
    try:
        return asyncio.run(_login_for_a_program(home, env))
    except KeyboardInterrupt:
        _answer({"error": _STOPPED, "code": "STOPPED"})
        return _INTERRUPTED


async def _login_for_a_program(home: Path, env: Mapping[str, str]) -> int:
    """Sign in to GitHub, and say how it goes in lines of JSON.

    The first line holds the code and the page to enter it on, and the last one how the
    sign-in ended. Closing the standard input stops the waiting for the user.
    """
    github = GitHub(home, env)
    try:
        app = github.app
        code = await oauth.ask_for_a_code(github.http, app)
        expires_in = max(0, round(code.expires_at - time.time()))
        _answer({"code": code.user_code, "url": code.verification_uri, "expires_in": expires_in})
        # The program that reads it shows the code while this one waits.
        sys.stdout.flush()
        tokens = await _waited(github, code, _when_the_input_closes())
        if tokens is None:
            _answer({"error": _STOPPED, "code": "STOPPED"})
            return _INTERRUPTED
        found = await signing.keep(github, home, tokens)
        answer: dict[str, object] = {
            "signed_in": await _person(found, home),
            "install_url": app.install_url,
        }
        try:
            answer["repositories"] = await signing.reach(github)
        except GitHubError as error:
            answer["offline"] = str(error)
        _answer(answer)
        return 0
    except GitHubError as error:
        _answer({"error": str(error), "code": error.code})
        return 2
    finally:
        await github.aclose()


def _login_at_a_terminal(home: Path, env: Mapping[str, str]) -> int:
    """Sign in to GitHub, and say how it goes to a person."""
    kept: list[signing.Who] = []

    async def signed_in() -> int:
        github = GitHub(home, env)
        try:
            app = github.app
            before = await asyncio.to_thread(store.signed, home, app)
            code = await oauth.ask_for_a_code(github.http, app)
            minutes = max(1, round((code.expires_at - time.time()) / 60))
            if before is not None:
                print(
                    harmless(
                        f"You are signed in to GitHub as @{before.login}. A new sign-in "
                        "takes its place."
                    )
                )
            # The code first: the browser may come to the front over the terminal.
            print(harmless(f"Enter this code on GitHub to sign in:\n\n    {code.user_code}\n"))
            page = f"The page to enter it on is {code.verification_uri}"
            opened = _opened(code.verification_uri, env)
            print(harmless(f"{page}, and it was opened in your browser." if opened else f"{page}."))
            print(f"The code holds for {minutes} minutes. Ctrl+C stops the waiting.")
            tokens = await oauth.wait_for_the_user(github.http, app, code)
            found = await signing.keep(github, home, tokens)
            kept.append(found)
            try:
                reached = await signing.reach(github)
            except GitHubError as error:
                print(harmless(_say_who(found)))
                _tell(f"GitHub could not be asked which repositories Columbina may reach: {error}")
                return 0
            _say_the_reach(found, reached, app.install_url)
            return 0
        finally:
            await github.aclose()

    try:
        return asyncio.run(signed_in())
    except GitHubError as error:
        _tell(str(error))
        return 2
    except KeyboardInterrupt:
        # The line of the wait is still open.
        print()
        if kept:
            _tell(f"signed in to GitHub as @{kept[0].login}. The repositories were not listed")
        else:
            _tell(_STOPPED)
        return _INTERRUPTED


async def _looked_up(home: Path, env: Mapping[str, str]) -> dict[str, object]:
    """Say who is signed in, and which repositories the app may reach, for another program.

    Raises:
        GitHubError: What is kept cannot be read, or a variable names a
            GitHub that cannot be one.
    """
    github = GitHub(home, env)
    try:
        try:
            app = github.app
        except GitHubError as error:
            if error.code != "NOT_REGISTERED":
                raise
            return {"signed_in": None, "registered": False, "why": str(error)}
        answer: dict[str, object] = {"signed_in": None, "install_url": app.install_url}
        said = await asyncio.to_thread(store.signed, home, app)
        if said is None:
            return answer
        found = signing.Who(said.login, said.name)
        try:
            found = await signing.who(github)
            answer["repositories"] = await signing.reach(github)
        except GitHubError as error:
            if error.code == "STORE":
                raise
            if error.code == "SIGNED_OUT":
                answer["ended"] = str(error)
                return answer
            answer["offline"] = str(error)
        answer["signed_in"] = await _person(found, home)
        return answer
    finally:
        await github.aclose()


def _status(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Say who is signed in to GitHub, and what Columbina may reach in that name."""
    try:
        answer = asyncio.run(_looked_up(home, env))
    except GitHubError as error:
        if args.json:
            _answer({"error": str(error), "code": error.code})
        else:
            _tell(str(error))
        return 2
    if args.json:
        _answer(answer)
        return 0
    signed = answer["signed_in"]
    if answer.get("registered") is False:
        print(harmless(_sentence(str(answer["why"]))))
    elif "ended" in answer:
        ended = _sentence(str(answer["ended"])).removesuffix(".")
        print(harmless(f"{ended} with 'columbina login github'."))
    elif not isinstance(signed, dict):
        print("Nobody is signed in to GitHub. Sign in with 'columbina login github'.")
    else:
        person = cast("dict[str, Any]", signed)
        found = signing.Who(person["login"], person["name"])
        if "offline" in answer:
            print(
                harmless(
                    f"{_say_who(found)} GitHub could not be asked which repositories "
                    f"Columbina may reach: {answer['offline']}"
                )
            )
        else:
            repositories = cast("list[str]", answer["repositories"])
            _say_the_reach(found, repositories, str(answer["install_url"]))
    return 0


def _logout(args: argparse.Namespace, home: Path, env: Mapping[str, str]) -> int:
    """Delete the sign-in to GitHub that this machine keeps."""
    try:
        there = store.forget(home)
    except GitHubError as error:
        if args.json:
            _answer({"error": str(error), "code": error.code})
        else:
            _tell(str(error))
        return 2
    try:
        web = web_of(env)
    except GitHubError:
        # The sign-in is gone all the same, and its right was given on GitHub itself.
        web = "https://github.com"
    revoke = signing.revoke_url(web)
    if args.json:
        _answer({"signed_out": there, "revoke": revoke})
    elif there:
        print(
            harmless(
                "Signed out of GitHub on this machine. The app keeps the right that you gave it "
                f"until you take it back on GitHub: {revoke}"
            )
        )
    else:
        print("Nobody was signed in to GitHub.")
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


def _where(command: argparse.ArgumentParser, profile: str) -> None:
    """Add the arguments that say which program is meant, which model it asks, and where it works.

    Args:
        command: The command.
        profile: The profile that it loads unless another one is named.
    """
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
    command.add_argument(
        "--sandbox",
        metavar="NAME",
        help="what runs the commands of the agent: none, for this machine as it is, or "
        "docker, for a container",
    )


def _shared(command: argparse.ArgumentParser, profile: str, *, session: bool = True) -> None:
    """Add the arguments that every command takes that has an agent work.

    Args:
        command: The command.
        profile: The profile that it loads unless another one is named.
        session: Whether the command line can name the session.
    """
    _where(command, profile)
    if session:
        command.add_argument(
            "--session-id", metavar="ID", help="the session to start or to continue"
        )
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
    serve = commands.add_parser(
        _SERVE,
        help="hold a conversation for another program, over the standard input and output",
    )
    _shared(serve, "serve", session=False)
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
    # No option takes a key. On the command line it would stay in the history of the shell.
    instead = setup.add_mutually_exclusive_group()
    instead.add_argument(
        "--forget",
        metavar="NAME",
        help="delete what is stored for a service, its key and what was said of it, and stop",
    )
    instead.add_argument(
        "--list",
        action="store_true",
        help="say what is set up in one document of JSON, for another program, and stop",
    )
    instead.add_argument(
        "--from-stdin",
        action="store_true",
        help="store what a document of JSON on the standard input says, for another program, "
        "and answer in one document of JSON",
    )
    instead.add_argument(
        "--check",
        action="store_true",
        help="ask the service that a document of JSON on the standard input names for its "
        "models, with the key that it gives, store nothing, and answer in one document of JSON",
    )
    setup.add_argument(
        "--json",
        action="store_true",
        help="with --forget: say what was deleted in one document of JSON, for another program",
    )
    login = commands.add_parser(
        _LOGIN,
        help="sign in to GitHub, so that the agent may work in the repositories that you "
        "choose for it",
    )
    login.add_argument(
        "service", nargs="?", choices=[_GITHUB], metavar="SERVICE", help="where: github"
    )
    login.add_argument(
        "--status",
        action="store_true",
        help="say who is signed in, and which repositories Columbina may reach, and stop",
    )
    login.add_argument(
        "--json",
        action="store_true",
        help="say it in lines of JSON, for another program. Closing the standard input stops "
        "the waiting for the user",
    )
    logout = commands.add_parser(
        _LOGOUT, help="delete the sign-in to GitHub that this machine keeps"
    )
    logout.add_argument("service", choices=[_GITHUB], metavar="SERVICE", help="where: github")
    logout.add_argument(
        "--json",
        action="store_true",
        help="say what was deleted in one document of JSON, for another program",
    )
    doctor = commands.add_parser(
        _DOCTOR,
        help="see whether a session could start: the service and its key, the shell, and "
        "the sandbox",
    )
    _where(doctor, "cli")
    # What a session asks of the command line besides, and the doctor does not.
    doctor.set_defaults(session_id=None, permission=None, allow=[], resume=False)
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
    # Where the user keeps what several tools read, skills for one.
    env.setdefault("COLUMBINA_AGENTS_HOME", str(Path.home() / ".agents"))
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
        if args.command == _LOGIN:
            return _login(args, home, env)
        if args.command == _LOGOUT:
            return _logout(args, home, env)
        if args.command == _DOCTOR:
            return _doctor(args, home, env)
        entries = _entries(args, home, env)
        if args.dump_config:
            dumped = _dump(entries)
            # A file or another program gets the entries as they are, to be read again.
            sys.stdout.write(harmless(dumped) if sys.stdout.isatty() else dumped)
            return 0
        if args.command == _SERVE:
            return asyncio.run(_serve(entries, env))
        return asyncio.run(_run(entries, env))
    except (_UsageError, CompositionError, chosen_service.SetupError) as error:
        _tell(str(error))
        return 2
    except KeyboardInterrupt:
        return _INTERRUPTED
    finally:
        log.removeHandler(handler)
