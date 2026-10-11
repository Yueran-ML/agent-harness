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

``columbina setup --forget NAME`` deletes what is stored for a service.

What is shown is text for a person, and some of it was typed by a person or
read from a file. Whoever shows the text makes it fit for a terminal first.
"""

import re
import secrets
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field, replace
from pathlib import Path
from urllib.parse import urlsplit

import yaml
from pydantic import BaseModel, ConfigDict, Field, ValidationError

from columbina import plain_yaml
from columbina.llm import credentials
from columbina.llm.credentials import CredentialsError, origin_of, sendable, trimmed
from columbina.llm.services import PRESETS, Service, of_ones_own, preset
from columbina.plain_yaml import PlainYamlError

__all__ = [
    "CHOICE",
    "IN_THE_WAY",
    "KEYS",
    "Answers",
    "Ask",
    "Chosen",
    "Own",
    "SetupError",
    "Show",
    "asked",
    "chosen",
    "forget",
    "run",
    "service_named",
    "store",
]

CHOICE = "provider.yml"
"""The file in the home of the program that holds what was chosen."""

KEYS = "credentials.json"
"""The file in the home of the program that holds the keys."""

IN_THE_WAY = ("COLUMBINA_PROVIDER", "COLUMBINA_MODEL", "COLUMBINA_CONTEXT_WINDOW")
"""The environment variables that go before what is chosen here."""

Ask = Callable[[str], str]
"""Asks the person a question and returns the answer. ``EOFError`` if none can be given."""

Show = Callable[[str], None]
"""Shows a text to the person."""

_NAME = re.compile(r"[a-z][a-z0-9_-]{0,31}")
_LOCAL = frozenset({"localhost", "127.0.0.1", "::1"})
_ENDPOINT = "/chat/completions"
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
    """

    model_config = ConfigDict(extra="forbid", frozen=True)

    base_url: str | None = None
    model: str | None = None
    context_window: int | None = Field(default=None, gt=0)


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
        return replace(own, model=said.model, context_window=said.context_window)
    if said is None:
        return known
    # What is known of the window is known of one model.
    same = said.model is None or said.model == known.model
    return replace(
        known,
        base_url=said.base_url or known.base_url,
        model=said.model or known.model,
        context_window=said.context_window or (known.context_window if same else None),
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
            show(
                "This address is not encrypted. The key and the conversation cross the "
                "network as they are."
            )
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
    """What a person answered, ready to be stored.

    Attributes:
        service: The service that was chosen.
        model: The model that was named for it.
        chosen: What is to be written down as chosen.
        key: The key that was typed, or ``None`` if none is to be stored. It
            is left out where the answers are shown.
        without_a_key: Whether the service wants a key, and none was typed
            and none is there.
    """

    service: Service
    model: str
    chosen: Chosen
    key: str | None = field(repr=False)
    without_a_key: bool


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
    if preset(service.name) is None:
        address = service.base_url
    else:
        # The address of a service that is known is written down only where the user changed it.
        address = None if said is None else said.base_url
    services = before.services | {
        service.name: Own(base_url=address, model=model, context_window=window)
    }
    there = _in_its_variable(service, env) or _is_stored(service, keys)
    return Answers(
        service=service,
        model=model,
        chosen=Chosen(use=service.name, services=services),
        key=key,
        without_a_key=service.key_required and key is None and not there,
    )


def store(home: Path, env: Mapping[str, str], answers: Answers, show: Show) -> None:
    """Store what a person answered, and say what was stored.

    The key is stored first. What was chosen can be said again without a
    loss, and a key that was typed and thrown away cannot.

    Args:
        home: The home of the program.
        env: The environment.
        answers: What was answered.
        show: Shows a text.

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


def run(home: Path, env: Mapping[str, str], ask: Ask, ask_unseen: Ask, show: Show) -> None:
    """Ask for the service, the model and the key, and store the answers.

    Raises:
        SetupError: What is stored cannot be read, or the answers cannot be
            stored.
        EOFError: No answer can be given any more. Nothing was stored.
    """
    store(home, env, asked(home, env, ask, ask_unseen, show), show)


def forget(home: Path, name: str, show: Show) -> None:
    """Delete what is stored for a service: its key, and what was said of it.

    Args:
        home: The home of the program.
        name: The name of the service.
        show: Shows a text.

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
