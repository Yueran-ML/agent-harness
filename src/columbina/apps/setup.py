"""What ``columbina setup`` does: the service of the model is chosen, and its key is stored.

A model comes from a service, and a service wants a key. Both can be said
with environment variables and options, every time the program is started.
``columbina setup`` asks for them once, at a terminal, and writes the answers
down, so that the program finds them from then on.

What is asked
    The service, from a list: the ones that are known by name, see
    :mod:`columbina.llm.services`, and the ones that the user added. Any
    other service that speaks the OpenAI Chat Completions protocol is added
    under a name of the user's own, by its address. Then the model, how
    many tokens it takes where that is not known, and the key.

What is written
    Two files in the home of the program. ``provider.yml`` holds the
    services that were set up and the one that is in use. It holds no key.
    ``credentials.json`` says which services have a key and where each key
    is sent. The key itself goes into the keyring of the system where there
    is one, and into that file where there is none, see
    :mod:`columbina.llm.credentials`. Whoever typed the key is told which.
    Nothing is written before the last question is answered: asking is one
    step and storing another, see :func:`asked` and :func:`store`. So whoever
    leaves in the middle of the questions has changed nothing, and can be
    told so.

The key
    A key is typed without being shown, and it is never repeated: not on the
    screen, and not where something is wrong with it. It is never taken
    from the command line, where the shell would keep it in its history and
    every program could read it from the list of processes. It is stored for
    the address of its service as it is known here, and is sent nowhere
    else, whatever a variable of the environment says of the address later.

The environment goes first. A key in the variable of a service is used before
the one that is stored, and the variables that name the service, the model
and the size of its window go before what is written down here.

``columbina setup --forget NAME`` deletes what is stored for a service. With
``--json`` it says what it deleted in a document, for another program.

For another program
    A desktop application sets a service up without a terminal. It starts
    this program for a moment and talks to it over its standard input and
    output, in JSON. ``columbina setup --list`` says what is set up, see
    :func:`listed`. ``columbina setup --from-stdin`` reads one document that
    gives the answers to the questions all at once, see :func:`given`. They
    are stored by the step that stores what a person answered, and what was
    stored is said in a document again, see :func:`stored`. Neither of the
    two asks anything. ``columbina setup --check`` reads such a document too,
    and asks the service for the list of its models with the key that it
    gives, so that a key is seen to work before it is stored. It stores
    nothing, see :func:`checked`.

    The key comes in with that document and by no other way, and it goes
    out by none. What is said of the services and of what was stored holds
    no key and nothing that a key could be guessed from. What is said of a
    document that is not right repeats no part of the document.

What is shown is text for a person, and some of it was typed by a person or
read from a file. Whoever shows the text makes it fit for a terminal first.
"""

import contextlib
import json
import re
import secrets
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Literal, cast, get_args
from urllib.parse import urlsplit

import yaml
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from columbina import plain_yaml
from columbina.llm import LlmError, credentials
from columbina.llm.adapters.anthropic_messages import AnthropicMessages
from columbina.llm.adapters.anthropic_messages import Config as AnthropicConfig
from columbina.llm.adapters.openai_chat import Config as AdapterConfig
from columbina.llm.adapters.openai_chat import OpenAIChat
from columbina.llm.credentials import CredentialsError, origin_of, sendable, trimmed
from columbina.llm.services import PRESETS, Service, adapter_config, of_ones_own, preset
from columbina.llm.types import LlmFailure
from columbina.plain_yaml import PlainYamlError

__all__ = [
    "CHOICE",
    "IN_THE_WAY",
    "KEYS",
    "MAX_DOCUMENT_BYTES",
    "Answers",
    "Ask",
    "Chosen",
    "Own",
    "SetupError",
    "Show",
    "asked",
    "chosen",
    "forget",
    "given",
    "listed",
    "run",
    "service_named",
    "store",
    "stored",
]

CHOICE = "provider.yml"
"""The file in the home of the program that holds what was chosen."""

KEYS = "credentials.json"
"""The file in the home of the program that holds the keys."""

IN_THE_WAY = ("COLUMBINA_PROVIDER", "COLUMBINA_MODEL", "COLUMBINA_CONTEXT_WINDOW")
"""The environment variables that go before what is chosen here."""

MAX_DOCUMENT_BYTES = 64 * 1024
"""How long a document may be that says what is to be stored. A longer one is refused."""

Ask = Callable[[str], str]
"""Asks the person a question and returns the answer. ``EOFError`` if none can be given."""

Show = Callable[[str], None]
"""Shows a text to the person."""

_NAME = re.compile(r"[a-z][a-z0-9_-]{0,31}")
_LOCAL = frozenset({"localhost", "127.0.0.1", "::1"})
_ENDPOINT = "/chat/completions"
_SAID_UNDER = ("service", "base_url", "model", "context_window", "api_key", "effort")
"""The names under which a document says what is to be stored."""
Kept = Literal["default", "off", "low", "high", "max"]
"""A level of effort that is stored for a service. ``ultra`` holds for one run, and is not."""
_KEPT: tuple[Kept, ...] = get_args(Kept)
_NOT_A_KEY = (
    "what the document says under 'api_key' cannot be a key: there is a character in it that "
    "no key has, such as a blank or a line break in the middle. Nothing of it was kept"
)
_UNREACHABLE = "If the service is reached through a proxy, see that the proxy is on"
_BY_DEFAULT = PRESETS[0].name
"""The service that a profile talks to where nothing was chosen: the first that is known by name."""
_NOT_ENCRYPTED = (
    "This address is not encrypted. The key and the conversation cross the network as they are."
)
_AGAIN = "Run 'columbina setup' again, or delete the file"
_NOT_READ: dict[plain_yaml.Kind, str] = {
    "syntax": "is not YAML",
    "depth": "is nested too deeply",
    "value": "gives a value that cannot be read: {said}",
    "alias": "uses a value again by its name, which is not read here",
}
"""What is said of a file of what was chosen that cannot be read, after its name."""


class SetupError(Exception):
    """What was asked for cannot be done. The text says why, and begins without a capital."""


class Own(BaseModel):
    """What a user said of one service.

    Attributes:
        base_url: Its address. A service that is not known by name has one.
            For one that is known, it stands in the place of the address
            that is known.
        model: The model to use.
        context_window: How many tokens that model takes, if the user said.
        effort: How hard the model is to think, if the user chose that.
            ``default`` asks for nothing, where a profile would ask for a level.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    base_url: str | None = None
    model: str | None = None
    context_window: int | None = Field(default=None, gt=0)
    effort: Kept | None = None


class Chosen(BaseModel):
    """What ``columbina setup`` wrote down.

    Attributes:
        use: The service that is in use, or ``None`` if the profile decides.
        services: What the user said of each service that was set up.
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    use: str | None = None
    services: dict[str, Own] = Field(default_factory=dict[str, Own])


def chosen(home: Path) -> Chosen:
    """Read what was chosen.

    Args:
        home: The home of the program.

    Returns:
        What the file holds. Nothing is chosen if there is no file.

    Raises:
        SetupError: The file cannot be read or is not what
            ``columbina setup`` writes.
    """
    path = home / CHOICE
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return Chosen()
    except (OSError, UnicodeDecodeError) as error:
        raise SetupError(f"{path} cannot be read: {_reason(error)}") from error
    try:
        # Read plainly: a file that this program wrote builds no values out of values.
        data = plain_yaml.load(text)
    except PlainYamlError as error:
        why = _NOT_READ[error.kind].format(said=error.said)
        raise SetupError(f"{path} {why}. {_AGAIN}") from None
    try:
        return Chosen.model_validate({} if data is None else data)
    except ValidationError as error:
        first = error.errors()[0]
        place = ".".join(str(part) for part in first["loc"]) or "the whole of it"
        raise SetupError(
            f"{path} is not what 'columbina setup' writes: {place}: {first['msg']}. {_AGAIN}"
        ) from error


def _reason(error: OSError | UnicodeDecodeError) -> str:
    if isinstance(error, OSError):
        return str(error.strerror)
    return "it is not text in UTF-8"


def _own(chosen: Chosen) -> list[Service]:
    """Return the services that the user added, in the order of their names."""
    return [
        service_named(name, chosen)
        for name, said in sorted(chosen.services.items())
        if preset(name) is None and said.base_url is not None
    ]


def _all(chosen: Chosen) -> list[Service]:
    """Return every service, in the order in which a list shows them."""
    return [service_named(service.name, chosen) for service in PRESETS] + _own(chosen)


def service_named(name: str, chosen: Chosen) -> Service:
    """Return the service of a name, with what the user said of it.

    Args:
        name: The name of a service that is known, or of one that the user
            added.
        chosen: What was chosen.

    Raises:
        SetupError: There is no service of this name.
    """
    known, said = preset(name), chosen.services.get(name)
    if known is None:
        if said is None or said.base_url is None:
            names = ", ".join(service.name for service in (*PRESETS, *_own(chosen)))
            raise SetupError(
                f"there is no service {name!r}. The services are: {names}. Another one is "
                "added with 'columbina setup'"
            )
        own = of_ones_own(name, said.base_url)
        return replace(
            own, model=said.model, context_window=said.context_window, effort=said.effort
        )
    if said is None:
        return known
    # What is known of the window is known of one model.
    same = said.model is None or said.model == known.model
    base_url = said.base_url or known.base_url
    # At another address, such as a mirror, a key is not made on the page of the service,
    # and the account that pays may be another one.
    home = base_url == known.base_url
    return replace(
        known,
        base_url=base_url,
        model=said.model or known.model,
        context_window=said.context_window or (known.context_window if same else None),
        key_page=known.key_page if home else None,
        effort=said.effort,
        balance=known.balance if home else None,
        drops_unbound_thinking=known.drops_unbound_thinking and home,
    )


def _kept(path: Path, text: str) -> None:
    """Put a file of this text in the place of the one that is there, whole or not at all."""
    beside = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        try:
            beside.write_text(text, encoding="utf-8", newline="\n")
            beside.replace(path)
        finally:
            beside.unlink(missing_ok=True)
    except OSError as error:
        raise SetupError(f"{path} cannot be written: {error.strerror}") from error


def _written(home: Path, chosen: Chosen) -> None:
    data = chosen.model_dump(exclude_none=True)
    text = yaml.safe_dump(data, sort_keys=False, allow_unicode=True)
    _kept(home / CHOICE, f"# Written by 'columbina setup'. It holds no key.\n{text}")


def _keys(home: Path) -> dict[str, str]:
    try:
        return credentials.stored(home / KEYS)
    except CredentialsError as error:
        raise SetupError(str(error)) from None


def _in_its_variable(service: Service, env: Mapping[str, str]) -> bool:
    """Whether the variable of a service holds a key."""
    return service.key_env is not None and bool(trimmed(env.get(service.key_env, "")))


def _is_stored(service: Service, keys: Mapping[str, str]) -> bool:
    """Whether a key is stored for a service, for the place that its address leads to."""
    return keys.get(service.name) == origin_of(service.base_url)


def _of_the_key(service: Service, keys: Mapping[str, str], env: Mapping[str, str]) -> str:
    """Say where the key of a service comes from."""
    if _in_its_variable(service, env):
        return f"its key is in the variable {service.key_env}"
    if _is_stored(service, keys):
        return "a key is stored"
    if service.name in keys:
        return "a key is stored for another address"
    return "no key" if service.key_env is not None else "asks for no key"


def _listed(chosen: Chosen, keys: Mapping[str, str], env: Mapping[str, str]) -> str:
    """Say which services there are, where each has its key from, and which one is in use."""
    services = _all(chosen)
    names = max(len(service.name) for service in services)
    titles = max(len(service.title) for service in services)
    lines = ["The model can come from one of these services.", ""]
    lines.extend(
        f"  {number:>2}  {service.name:<{names}}  {service.title:<{titles}}  "
        f"{_of_the_key(service, keys, env)}"
        for number, service in enumerate(services, 1)
    )
    lines.append("")
    in_use = next((service for service in services if service.name == chosen.use), None)
    if in_use is None:
        lines.append(
            "None of them is chosen yet. The profiles that come with Columbina talk to "
            f"{PRESETS[0].name}."
        )
    else:
        model = "no model" if in_use.model is None else f"the model {in_use.model}"
        lines.append(f"In use: {in_use.name}, with {model}.")
    lines.append(
        "A service that is not listed is added under a name of your own, if it speaks the "
        "OpenAI Chat Completions protocol."
    )
    return "\n".join(lines)


def _key_in_a_word(service: Service, keys: Mapping[str, str], env: Mapping[str, str]) -> str:
    """Say in one word where the key of a service comes from, for a program."""
    if _in_its_variable(service, env):
        return "environment"
    if _is_stored(service, keys):
        return "stored"
    return "stored-for-another-address" if service.name in keys else "none"


def _told_of(
    service: Service, keys: Mapping[str, str], env: Mapping[str, str]
) -> dict[str, object]:
    """Return what a program is told of one service, see :func:`listed`."""
    return {
        "name": service.name,
        "title": service.title,
        "base_url": service.base_url,
        "own": preset(service.name) is None,
        "model": service.model,
        "context_window": service.context_window,
        "key_env": service.key_env,
        "key": _key_in_a_word(service, keys, env),
        "key_page": service.key_page,
    }


def listed(home: Path, env: Mapping[str, str]) -> dict[str, object]:
    """Say which services there are and which one is in use, for a program to read.

    What is returned is written out as one document of JSON::

        {"use": "openai",
         "services": [{"name": "openai", "title": "OpenAI",
                       "base_url": "https://api.openai.com/v1", "own": false,
                       "model": "gpt-test", "context_window": 400000,
                       "key_env": "OPENAI_API_KEY", "key": "stored",
                       "key_page": "https://platform.openai.com/api-keys"}]}

    ``use`` names the service that is in use, and is ``null`` where none of
    the services is. The services stand in the order of the list that a
    person is shown: the ones that are known by name, and then the ones
    that the user added, for which ``own`` is true. A service is chosen by
    its ``name``, is called its ``title`` and is at its ``base_url``.
    ``model`` and ``context_window`` are what is known of a service or was
    said of it, and ``null`` where nothing is. ``key_env`` names the
    variable that may hold the key of the service, and is ``null`` for a
    service that asks for no key. ``key`` says where the key comes from:
    ``environment`` for that variable, which goes first, ``stored``,
    ``stored-for-another-address`` for a key that is stored and is not sent
    to the address that the service has now, or ``none``. ``key_page`` is
    the page of the service on which a key is made, and ``null`` where none
    is known, as for a service of the user's own.

    No key is said, and nothing that a key could be guessed from. The
    keyring of the system is not asked for anything.

    Args:
        home: The home of the program.
        env: The environment.

    Raises:
        SetupError: What is stored cannot be read.
    """
    before = chosen(home)
    keys = _keys(home)
    services = _all(before)
    names = {service.name for service in services}
    return {
        "use": before.use if before.use in names else None,
        "services": [_told_of(service, keys, env) for service in services],
    }


def _address(typed: str) -> str | None:
    """Return the address that was typed, as it is stored, or ``None`` if it is none."""
    if not typed.isprintable() or " " in typed:
        return None
    try:
        parts = urlsplit(typed)
        named = parts.hostname
    except ValueError:
        return None
    # A name and a password in an address would be written down with it. The key is asked for.
    if parts.scheme not in ("http", "https") or not named or "@" in parts.netloc:
        return None
    if parts.query or parts.fragment:
        return None
    return typed.rstrip("/").removesuffix(_ENDPOINT)


def _encrypted(address: str) -> bool:
    """Whether what is sent to an address is encrypted, or does not leave the machine."""
    parts = urlsplit(address)
    host = parts.hostname or ""
    return parts.scheme == "https" or host in _LOCAL or host.endswith(".localhost")


def _added(name: str, ask: Ask, show: Show) -> Service | None:
    """Ask for the address of a service that is not listed. ``None`` if the person goes back."""
    show(f"{name} is not in the list, so it is added as a service of your own.")
    while True:
        typed = ask(f"Its address, up to but without {_ENDPOINT} (Enter goes back): ").strip()
        if not typed:
            return None
        address = _address(typed)
        if address is None:
            show(
                "An address begins with http:// or https:// and names a machine, such as "
                "https://api.example.com/v1."
            )
            continue
        if not _encrypted(address):
            show(_NOT_ENCRYPTED)
        return of_ones_own(name, address)


def _service(chosen: Chosen, ask: Ask, show: Show) -> Service:
    """Ask which service the model is to come from."""
    services = _all(chosen)
    by_name = {service.name: service for service in services}
    default = chosen.use if chosen.use in by_name else PRESETS[0].name
    while True:
        typed = ask(f"Service, by its number or its name [{default}]: ").strip() or default
        if typed in by_name:
            return by_name[typed]
        if typed.isdecimal():
            if 1 <= int(typed) <= len(services):
                return services[int(typed) - 1]
            show(f"There is no service with the number {typed}.")
            continue
        if _NAME.fullmatch(typed) is None:
            show(
                "A name of your own begins with a small letter and goes on with small "
                "letters, digits, '-' and '_'. It has 32 characters at most."
            )
            continue
        added = _added(typed, ask, show)
        if added is not None:
            return added


def _model(service: Service, ask: Ask, show: Show) -> str:
    """Ask which model of the service to use."""
    hint = "" if service.model is None else f" [{service.model}]"
    while True:
        typed = ask(f"Model{hint}: ").strip() or service.model
        if typed and typed.isprintable():
            return typed
        if typed:
            show("The name of a model has no characters in it that cannot be seen.")
        else:
            show(f"{service.name} has no model that is used by default, so one has to be named.")


def _window(ask: Ask, show: Show) -> int | None:
    """Ask how many tokens the model takes. ``None`` if the person does not say."""
    while True:
        typed = ask("How many tokens the model takes, if you know (Enter if not): ").strip()
        if not typed:
            return None
        digits = typed.replace(",", "").replace("_", "")
        if digits.isdecimal() and int(digits) > 0:
            return int(digits)
        show("A number of tokens is a number, such as 128000.")


def _key(
    service: Service, keys: Mapping[str, str], env: Mapping[str, str], ask: Ask, show: Show
) -> str | None:
    """Ask for the key of the service. ``None`` if none is to be stored."""
    if _in_its_variable(service, env):
        show(f"The variable {service.key_env} holds a key, and it goes before one that is stored.")
        question = "A key to store as well, or Enter for none: "
    elif _is_stored(service, keys):
        show(f"A key is stored for {service.name}.")
        question = "A new key, or Enter to keep the one that is stored: "
    elif service.name in keys:
        show(
            f"The key that is stored for {service.name} is for {keys[service.name]}, and it "
            "is not sent to this address."
        )
        question = "A key for this address, or Enter to store none: "
    elif service.key_env is None:
        show(f"{service.title} asks for no key.")
        question = "A key, if yours asks for one, or Enter for none: "
    else:
        question = f"The API key of {service.name}, or Enter to store none: "
    show("A key is not shown while it is typed.")
    while True:
        typed = trimmed(ask(question))
        if not typed:
            return None
        if sendable(typed):
            return typed
        show(
            "That cannot be a key: there is a character in it that no key has, such as a "
            "blank or a line break in the middle. Nothing of it was kept."
        )


@dataclass(frozen=True, slots=True)
class Answers:
    """What a person answered or a document said, ready to be stored.

    Attributes:
        service: The service that was chosen.
        model: The model that was named for it.
        chosen: What is to be written down as chosen.
        key: The key that was typed or given, or ``None`` if none is to be
            stored. It is left out where the answers are shown.
        without_a_key: Whether the service wants a key, and none was typed
            or given and none is there.
    """

    service: Service
    model: str
    chosen: Chosen
    key: str | None = field(repr=False)
    without_a_key: bool


def _answers(
    before: Chosen,
    keys: Mapping[str, str],
    env: Mapping[str, str],
    service: Service,
    model: str,
    window: int | None,
    key: str | None,
    effort: Kept | None = None,
) -> Answers:
    """Put together what is to be stored of a service, whoever said it and how.

    Args:
        before: What was chosen until now.
        keys: Which services a key is stored for, and where each key is sent.
        env: The environment.
        service: The service that was chosen.
        model: The model that was named for it.
        window: How many tokens that model takes, where that is to be
            written down.
        key: The key to store, or ``None`` for none.
        effort: How hard the model is to think, or ``None`` to keep what was
            chosen of that before.
    """
    said = before.services.get(service.name)
    if preset(service.name) is None:
        address = service.base_url
    else:
        # The address of a service that is known is written down only where the user changed it.
        address = None if said is None else said.base_url
    if effort is None and said is not None:
        effort = said.effort
    services = before.services | {
        service.name: Own(base_url=address, model=model, context_window=window, effort=effort)
    }
    there = _in_its_variable(service, env) or _is_stored(service, keys)
    return Answers(
        service=service,
        model=model,
        chosen=Chosen(use=service.name, services=services),
        key=key,
        without_a_key=service.key_required and key is None and not there,
    )


def asked(home: Path, env: Mapping[str, str], ask: Ask, ask_unseen: Ask, show: Show) -> Answers:
    """Ask for the service, the model and the key. Nothing is stored here.

    Args:
        home: The home of the program.
        env: The environment.
        ask: Asks a question whose answer may be seen.
        ask_unseen: Asks for a key, which is not shown while it is typed.
        show: Shows a text.

    Raises:
        SetupError: What is stored cannot be read.
        EOFError: No answer can be given any more.
    """
    before = chosen(home)
    keys = _keys(home)
    show(_listed(before, keys, env))
    show("")
    service = _service(before, ask, show)
    model = _model(service, ask, show)
    said = before.services.get(service.name)
    if model == service.model and service.context_window is not None:
        # The size is known already, and stays as it was said.
        window = None if said is None else said.context_window
    else:
        window = _window(ask, show)
    key = _key(service, keys, env, ask_unseen, show)
    return _answers(before, keys, env, service, model, window, key)


def _said_in(document: bytes) -> dict[str, object]:
    """Return what a document says, by the names that it says it under.

    Raises:
        SetupError: The document is too long, is no object of JSON, or says
            something under a name that is not read.
    """
    if len(document) > MAX_DOCUMENT_BYTES:
        raise SetupError(
            f"the document is longer than {MAX_DOCUMENT_BYTES} bytes, which is more than is read"
        )
    read: object = None
    # Whatever the reading fails with names a place in the document, repeats a sign of it or
    # holds all of it, and a key may stand anywhere. So it is neither quoted nor held on to.
    with contextlib.suppress(ValueError, RecursionError):
        read = json.loads(document)
    if not isinstance(read, dict):
        raise SetupError("the document is not an object of JSON")
    said = cast("dict[str, object]", read)
    if not said.keys() <= set(_SAID_UNDER):
        raise SetupError(
            "the document says something under a name that is not read. The names that are "
            f"read are: {', '.join(_SAID_UNDER)}"
        )
    return said


def _text(said: Mapping[str, object], name: str) -> str | None:
    """Return the text that a document gives under a name, without the blanks around it.

    Returns:
        The text, or ``None`` if the document gives nothing there, or
        nothing but blanks.

    Raises:
        SetupError: What the document gives there is no text.
    """
    found = said.get(name)
    if found is None:
        return None
    if not isinstance(found, str):
        raise SetupError(f"what the document says under {name!r} is not text")
    return found.strip() or None


def _effort(said: Mapping[str, object]) -> Kept | None:
    """Return the level of effort that a document says, or ``None`` if it does not say.

    Raises:
        SetupError: What the document gives there is no level that is stored.
    """
    found = _text(said, "effort")
    if found is None:
        return None
    named = found.lower()
    for level in _KEPT:
        if level == named:
            return level
    stored = f"The levels that are stored are: {', '.join(_KEPT)}"
    if named == "ultra":
        raise SetupError(f"ultra holds for one run, and is not stored. {stored}")
    raise SetupError(f"what the document says under 'effort' is no level of effort. {stored}")


def _tokens(said: Mapping[str, object]) -> int | None:
    """Return how many tokens a document says the model takes, or ``None`` if it does not say.

    Raises:
        SetupError: What the document gives there is no number of tokens.
    """
    found = said.get("context_window")
    if found is None:
        return None
    # True and false are whole numbers to Python, and are no numbers of tokens.
    if isinstance(found, bool) or not isinstance(found, int) or found <= 0:
        raise SetupError(
            "what the document says under 'context_window' is not a number of tokens, such as "
            "128000"
        )
    return found


def _meant(name: str, address: str | None, before: Chosen, show: Show) -> Service:
    """Return the service that a document means by a name, at the address that it gives for it.

    Args:
        name: The name that the document gives.
        address: The address that it gives, or ``None`` if it gives none.
        before: What was chosen until now.
        show: Shows a text. An address that is not encrypted is named as one.

    Raises:
        SetupError: There is no service of the name and no address to add
            one by, an address is given for a service that is known by name,
            or the name or the address cannot be one.
    """
    there = next((service for service in _all(before) if service.name == name), None)
    if address is None:
        if there is None:
            raise SetupError(
                "there is no service of the name that the document gives, and the document "
                "gives no address to add one by"
            )
        return there
    if preset(name) is not None:
        raise SetupError(
            "the document gives an address for a service that is known by name, and the "
            "address of such a service is not changed this way"
        )
    if there is None and _NAME.fullmatch(name) is None:
        raise SetupError(
            "the document adds a service under a name that cannot be one. A name of your own "
            "begins with a small letter and goes on with small letters, digits, '-' and '_'. "
            "It has 32 characters at most"
        )
    place = _address(address)
    if place is None:
        raise SetupError(
            "what the document says under 'base_url' is no address. An address begins with "
            "http:// or https:// and names a machine, such as https://api.example.com/v1"
        )
    if not _encrypted(place):
        show(_NOT_ENCRYPTED)
    own = of_ones_own(name, place)
    if there is None:
        return own
    # It is the service that is there, at another address. What is known of its model stays.
    return replace(own, model=there.model, context_window=there.context_window)


def given(home: Path, env: Mapping[str, str], document: bytes, show: Show) -> Answers:
    """Read the service, the model and the key from a document. Nothing is stored here.

    The document says at once what a person answers to the questions. It is
    an object of JSON, in UTF-8::

        {"service": "my-server", "base_url": "http://10.0.0.5:8000/v1",
         "model": "tiny", "context_window": 32768, "api_key": "sk-..."}

    ``service`` names the service: one that is known by name, or one of the
    user's own. With ``base_url`` a service of the user's own is added under
    that name, or the one that is there is given another address. A service
    that is known by name takes no address. ``model`` names the model.
    ``context_window`` says how many tokens that model takes. Without it,
    what was said of the size for this model stays. ``api_key`` gives the
    key. Without it no key is stored, and one that is stored stays.
    ``effort`` says how hard the model is to think. Without it, what was
    chosen of that stays. A document that says nothing but the service and
    the level is read by :func:`effort_stored`.

    The blanks around a text are no part of it, as with what is typed, and
    a text that is empty counts as not given.

    What is said of a document that is not right repeats no part of it. A
    key may stand anywhere in such a document.

    Args:
        home: The home of the program.
        env: The environment.
        document: The document, as the bytes that came.
        show: Shows a text. An address that is not encrypted is named as one.

    Raises:
        SetupError: The document is not right, or what is stored cannot be
            read.
    """
    said = _said_in(document)
    name, address = _text(said, "service"), _text(said, "base_url")
    model, key = _text(said, "model"), _text(said, "api_key")
    window, effort = _tokens(said), _effort(said)
    if name is None:
        raise SetupError("the document names no service")
    if model is None:
        raise SetupError("the document names no model")
    if not model.isprintable():
        raise SetupError(
            "the name of the model that the document gives has a character in it that cannot "
            "be seen"
        )
    if key is not None and not sendable(key):
        raise SetupError(_NOT_A_KEY)
    before = chosen(home)
    keys = _keys(home)
    service = _meant(name, address, before, show)
    earlier = before.services.get(name)
    if window is None and earlier is not None and model == service.model:
        # The document does not say the size, so what was said of it for this model stays.
        window = earlier.context_window
    return _answers(before, keys, env, service, model, window, key, effort)


def effort_stored(home: Path, document: bytes) -> dict[str, object] | None:
    """Store how hard the model of a service is to think, where that is all that a document says.

    The document is an object of JSON, in UTF-8::

        {"service": "deepseek", "effort": "max"}

    Only the level of that service changes. The service that is in use stays
    the one it was, and so do the model, the window and the address of this
    one. ``default`` is stored as well: it asks for nothing, even where the
    profile asks for a level. ``ultra`` holds for one run, and is not stored.

    Returns:
        What was stored, for a program to read, or ``None`` where the
        document says more than the service and the level::

            {"effort": {"name": "deepseek", "effort": "max"}}

    Raises:
        SetupError: The document is not right, there is no such service, or
            what was chosen cannot be read or written.
    """
    said = _said_in(document)
    if said.keys() != {"service", "effort"}:
        return None
    name, effort = _text(said, "service"), _effort(said)
    if name is None:
        raise SetupError("the document names no service")
    if effort is None:
        raise SetupError("the document names no level of effort")
    before = chosen(home)
    service_named(name, before)
    earlier = before.services.get(name) or Own()
    services = before.services | {name: earlier.model_copy(update={"effort": effort})}
    _written(home, Chosen(use=before.use, services=services))
    return {"effort": {"name": name, "effort": effort}}


def _why(failure: LlmFailure) -> str:
    """Say in a word why a service listed no models, for a program to choose its words by."""
    if failure.code == "TRANSPORT":
        return "network"
    if failure.code == "TIMEOUT":
        return "timeout"
    if failure.code == "QUOTA" or failure.status == 403:
        return "forbidden"
    if failure.code == "AUTH":
        return "key"
    if failure.status == 404:
        return "address"
    if failure.code in ("RATE_LIMIT", "SERVER"):
        return "busy"
    return "unknown"


async def checked(
    home: Path, env: Mapping[str, str], document: bytes, show: Show
) -> dict[str, object]:
    """Ask a service for the list of its models, with the key that a document gives.

    Nothing is stored. The document is one that :func:`given` reads, and it
    needs no model::

        {"service": "deepseek", "api_key": "sk-..."}

    Without ``api_key`` the key is looked for as a session looks for it: in
    the variable of the service, and then among the stored keys. A service
    of the user's own may be given by its ``base_url``, as in :func:`given`,
    and is not added.

    Args:
        home: The home of the program.
        env: The environment.
        document: The document, as the bytes that came.
        show: Shows a text. An address that is not encrypted is named as one.

    Returns:
        ``{"models": [...]}``, the ids that the service lists in its order, or
        ``{"error": "...", "kind": "..."}`` where it listed none. The kind is
        ``network``, ``timeout``, ``key``, ``forbidden`` (a key without the
        right, or an account without the money), ``address``, ``busy`` or
        ``unknown``. Neither holds the key.

    Raises:
        SetupError: The document is not right, or what is stored cannot be
            read.
    """
    said = _said_in(document)
    name, address, key = _text(said, "service"), _text(said, "base_url"), _text(said, "api_key")
    if name is None:
        raise SetupError("the document names no service")
    if key is not None and not sendable(key):
        raise SetupError(_NOT_A_KEY)
    service = _meant(name, address, chosen(home), show)
    config = adapter_config(service, str(home / KEYS), env)
    adapter: OpenAIChat | AnthropicMessages
    if service.protocol == "anthropic":
        adapter = AnthropicMessages(AnthropicConfig.model_validate(config), secret=key)
    else:
        adapter = OpenAIChat(AdapterConfig.model_validate(config), secret=key)
    try:
        models = await adapter.models()
    except LlmError as error:
        kind = _why(error.failure)
        said_why = error.failure.message
        return {
            "error": f"{said_why.rstrip('.')}. {_UNREACHABLE}" if kind == "network" else said_why,
            "kind": kind,
        }
    finally:
        await adapter.aclose()
    return {"models": models}


async def balance(home: Path, env: Mapping[str, str], name: str | None) -> dict[str, object]:
    """Ask a service how much money the account of its key has left, for a program to read.

    The service is the one of the name, or else the one in use, or else
    the one that a profile talks to where nothing was chosen. Its key is
    looked for as a session looks for it.

    Returns:
        ``{"service": ..., "title": ..., "supported": false}`` for a service
        that cannot be asked with its key. Otherwise ``"supported": true``
        and either ``"available"``, whether the account can pay for calls,
        with ``"balances"``, a list of ``{"currency", "total", "granted",
        "topped_up"}`` as the service wrote the numbers, or ``"error"`` and
        ``"kind"`` as :func:`checked` gives them. None holds the key.

    Raises:
        SetupError: There is no service of the name, or what is stored
            cannot be read.
    """
    before = chosen(home)
    named = name or before.use or _BY_DEFAULT
    service = next((one for one in _all(before) if one.name == named), None)
    if service is None:
        raise SetupError(f"there is no service named {named!r}")
    said: dict[str, object] = {"service": service.name, "title": service.title}
    if service.balance is None:
        return {**said, "supported": False}
    adapter = OpenAIChat(
        AdapterConfig.model_validate(adapter_config(service, str(home / KEYS), env))
    )
    try:
        kept = await adapter.balance()
    except LlmError as error:
        kind = _why(error.failure)
        told = error.failure.message
        return {
            **said,
            "supported": True,
            "error": f"{told.rstrip('.')}. {_UNREACHABLE}" if kind == "network" else told,
            "kind": kind,
        }
    finally:
        await adapter.aclose()
    amounts = [
        {
            "currency": one.currency,
            "total": one.total,
            "granted": one.granted,
            "topped_up": one.topped_up,
        }
        for one in kept.amounts
    ]
    return {**said, "supported": True, "available": kept.available, "balances": amounts}


def store(
    home: Path, env: Mapping[str, str], answers: Answers, show: Show
) -> credentials.Kept | None:
    """Store what a person answered, and say what was stored.

    The key is stored first. What was chosen can be said again without a
    loss, and a key that was typed and thrown away cannot.

    Args:
        home: The home of the program.
        env: The environment.
        answers: What was answered.
        show: Shows a text.

    Returns:
        Where the key was kept, or ``None`` if the answers held none.

    Raises:
        SetupError: The answers cannot be stored. Where the key was stored
            and what was chosen was not, the text says so.
    """
    service = answers.service
    kept: credentials.Kept | None = None
    if answers.key is not None:
        try:
            place = origin_of(service.base_url)
            kept = credentials.keep(home / KEYS, service.name, answers.key, place)
        except CredentialsError as error:
            raise SetupError(str(error)) from None
    try:
        _written(home, answers.chosen)
    except SetupError as error:
        if kept is None:
            raise
        where = home / KEYS if kept.keyring is None else kept.keyring
        raise SetupError(f"{error}. The key was stored all the same, in {where}") from None
    show("")
    show(f"Stored. Columbina talks to {service.name} now, with the model {answers.model}.")
    show(f"What was chosen is in {home / CHOICE}.")
    if kept is not None and kept.keyring is not None:
        show(f"The key is in {kept.keyring}. No file of Columbina holds it.")
    elif kept is not None:
        show(
            f"No keyring of this system took the key: {kept.said}. It is in {home / KEYS}, "
            "as it is and not encrypted. Keep that file to yourself."
        )
    elif answers.without_a_key:
        show(
            f"No key is stored for {service.name}. Set the variable {service.key_env} before "
            "the program is started, or run this again."
        )
    for variable in IN_THE_WAY:
        if variable in env:
            show(f"The variable {variable} is set, and it goes before what is stored here.")
    named = service.base_url_env
    elsewhere = "" if named is None else env.get(named, "")
    if elsewhere and origin_of(elsewhere) != origin_of(service.base_url):
        show(
            f"The variable {named} is set, and it sends the requests for "
            f"{service.name} to another address. A stored key is not sent there."
        )
    return kept


def stored(answers: Answers, kept: credentials.Kept | None) -> dict[str, object]:
    """Say what was stored, for a program to read.

    What is returned is written out as one document of JSON::

        {"stored": {"name": "my-server",
                    "base_url": "http://10.0.0.5:8000/v1",
                    "model": "tiny", "context_window": 32768},
         "key_in": "keyring"}

    ``stored`` says of the service that is in use now what :func:`listed`
    says of it under the same names. ``key_in`` says where the key went
    that came with the answers: ``keyring`` for the keyring of the system,
    ``file`` for the file of keys, where it lies as it is and not
    encrypted, and ``null`` where no key came. The key itself is not said.

    Args:
        answers: What was stored.
        kept: Where its key was kept, as :func:`store` returned it.
    """
    service = service_named(answers.service.name, answers.chosen)
    key_in = None
    if kept is not None:
        key_in = "file" if kept.keyring is None else "keyring"
    return {
        "stored": {
            "name": service.name,
            "base_url": service.base_url,
            "model": service.model,
            "context_window": service.context_window,
        },
        "key_in": key_in,
    }


def run(home: Path, env: Mapping[str, str], ask: Ask, ask_unseen: Ask, show: Show) -> None:
    """Ask for the service, the model and the key, and store the answers.

    Raises:
        SetupError: What is stored cannot be read, or the answers cannot be
            stored.
        EOFError: No answer can be given any more. Nothing was stored.
    """
    store(home, env, asked(home, env, ask, ask_unseen, show), show)


@dataclass(frozen=True, slots=True)
class Forgot:
    """What was deleted of a service, see :func:`forget`.

    Attributes:
        key: Whether a key of the service was deleted.
        chosen: Whether what was said of the service was deleted, or it was
            no longer to be in use.
        unasked: Why the keyring of the system could not be asked whether it
            holds an earlier key of the service, or ``None`` if it was asked
            or there is none.
    """

    key: bool
    chosen: bool
    unasked: str | None = None


def forget(home: Path, name: str, show: Show) -> Forgot:
    """Delete what is stored for a service: its key, and what was said of it.

    Args:
        home: The home of the program.
        name: The name of the service.
        show: Shows a text.

    Returns:
        What was deleted.

    Raises:
        SetupError: Nothing is stored for the service, or what is stored
            cannot be read or written.
    """
    before = chosen(home)
    try:
        forgotten = credentials.forget(home / KEYS, name)
    except CredentialsError as error:
        raise SetupError(str(error)) from None
    was_set_up = name in before.services or before.use == name
    if not forgotten.was_there and not was_set_up:
        raise SetupError(f"nothing is stored for {name!r}")
    if was_set_up:
        services = {other: said for other, said in before.services.items() if other != name}
        _written(home, Chosen(use=None if before.use == name else before.use, services=services))
    if forgotten.was_there:
        show(f"The key of {name} is deleted.")
    if forgotten.unasked is not None:
        # A key that went into the file once the keyring took none may have one before it
        # that lies in the keyring still.
        show(
            f"The keyring of this system could not be asked whether it holds an earlier key "
            f"of {name}: {forgotten.unasked}. Run this again once it can be asked."
        )
    if name in before.services:
        show(f"What was said of {name} is deleted.")
    if before.use == name:
        show(f"{name} is no longer in use. The profile says again where the model comes from.")
    return Forgot(key=forgotten.was_there, chosen=was_set_up, unasked=forgotten.unasked)
