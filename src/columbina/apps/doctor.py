"""What ``columbina doctor`` does: whether a session could start, and what is in the way.

A session needs a few things that are not in the hands of this program: a
service that answers and takes the key, a shell for the commands, and, where
the commands are to run in containers, a Docker engine that has the image.
Each of them is looked at the way a session would come to use it: from the
same profile with the same patches, in the same environment, with the same
words where something is wrong.

Nothing is left behind. No container is made, no session is stored, and the
model is asked for no reply. The one request that leaves this machine asks
the service for the list of its models, with the key, see
:meth:`columbina.llm.adapters.openai_chat.OpenAIChat.models` and, for
Anthropic, :meth:`columbina.llm.adapters.anthropic_messages.AnthropicMessages.models`.

Each finding is a line: whether it is all right, what it is about, and what
was found. ``ok`` is as a session needs it, ``warn`` lets a session start
and is worth a look, and ``fail`` is in the way of a session.

What is said holds no key. The text of a failure of the service comes
through the adapter, which cuts the key out of it. What is said repeats what
files and services wrote, so whoever shows it makes it fit for a terminal.
"""

import platform
import sys
import textwrap
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, TypeVar, cast

from pydantic import BaseModel, ValidationError

import columbina
from columbina import shells
from columbina.builtin_tools.shell import NAME as SHELL
from columbina.kernel import CompositionError, Entry, interpolate
from columbina.llm import CallConfig, LlmError
from columbina.llm.adapters.anthropic_messages import AnthropicMessages
from columbina.llm.adapters.anthropic_messages import Config as AnthropicConfig
from columbina.llm.adapters.openai_chat import Config as AdapterConfig
from columbina.llm.adapters.openai_chat import OpenAIChat
from columbina.llm.credentials import CredentialsError, stored, trimmed
from columbina.sandbox import Config as SandboxConfig
from columbina.sandbox import SandboxUnavailableError
from columbina.sandbox.docker import Docker

__all__ = ["Finding", "State", "looked", "shown"]

State = Literal["ok", "warn", "fail"]

_FRONTEND = "frontend"
_SPEAKS_OPENAI = "columbina.llm.adapters.openai_chat"
_SPEAKS_ANTHROPIC = "columbina.llm.adapters.anthropic_messages"
_SANDBOX = "columbina.sandbox"
_COLUMNS = (6, 9)
"""How wide the state and the subject of a line are."""


@dataclass(frozen=True, slots=True)
class Finding:
    """What was found of one thing that a session needs.

    Attributes:
        state: ``ok`` where it is as a session needs it, ``warn`` where a
            session may start and something is worth a look, and ``fail``
            where a session would not get far.
        about: What it is about, in a word.
        said: What was found, in a sentence for a person.
    """

    state: State
    about: str
    said: str


class _ConfigError(Exception):
    """A config of the profile cannot be read the way the plugin that it is for reads it."""


_Read = TypeVar("_Read", bound=BaseModel)


def _why(error: CompositionError | ValidationError) -> str:
    """Say in a line what is wrong with a config: where, and what."""
    if isinstance(error, CompositionError):
        return str(error)
    first = error.errors()[0]
    place = ".".join(str(part) for part in first["loc"])
    return f"{place}: {first['msg']}" if place else first["msg"]


def _config(entry: Entry, kind: type[_Read], env: Mapping[str, str]) -> _Read:
    """Return the config of an entry, filled in and read as its plugin reads it.

    Raises:
        _ConfigError: It refers to a variable that is not set, or it is not
            what the plugin takes.
    """
    try:
        filled: Any = interpolate(entry.config, env)
        return kind.model_validate({} if filled is None else filled)
    except (CompositionError, ValidationError) as error:
        raise _ConfigError(
            f"the entry {entry.id!r} has a config that cannot be read: {_why(error)}"
        ) from error


async def looked(entries: Sequence[Entry], env: Mapping[str, str], cwd: Path) -> list[Finding]:
    """Look at what a session of the entries needs, and say what was found.

    Args:
        entries: The entries of the session, as the command line made them.
        env: The environment that their configs refer to.
        cwd: The working directory of the session.
    """
    return [*await _served(entries, env), shell(), await _held(entries, env, cwd)]


async def _served(entries: Sequence[Entry], env: Mapping[str, str]) -> list[Finding]:
    """Say which model of which service a session asks, and whether the service answers."""
    # The command line has seen to it that there is a frontend, and that its config is a mapping.
    frontend = next(entry for entry in entries if entry.id == _FRONTEND)
    said = cast("dict[str, Any]", frontend.config).get("model")
    try:
        call = CallConfig.model_validate(interpolate(said, env))
    except (CompositionError, ValidationError) as error:
        why = _why(error)
        return [Finding("fail", "model", f"the frontend names no model that can be read: {why}")]
    try:
        adapters: list[AdapterConfig | AnthropicConfig] = [
            _config(entry, AdapterConfig, env)
            if entry.plugin == _SPEAKS_OPENAI
            else _config(entry, AnthropicConfig, env)
            for entry in entries
            if entry.plugin in (_SPEAKS_OPENAI, _SPEAKS_ANTHROPIC) and not entry.disabled
        ]
    except _ConfigError as error:
        return [Finding("fail", "model", str(error))]
    config = next((one for one in adapters if one.provider == call.provider), None)
    if config is None:
        return [
            Finding(
                "warn",
                "model",
                f"{call.model} of {call.provider}, which no entry that loads {_SPEAKS_OPENAI} "
                f"or {_SPEAKS_ANTHROPIC} is for, so no service is asked",
            )
        ]
    model = Finding("ok", "model", f"{call.model} of {call.provider}, at {config.base_url}")
    return [model, await _answered(config, call.model, env)]


async def _answered(
    config: AdapterConfig | AnthropicConfig, model: str, env: Mapping[str, str]
) -> Finding:
    """Ask the service for its models, and say how that went."""
    adapter = (
        AnthropicMessages(config) if isinstance(config, AnthropicConfig) else OpenAIChat(config)
    )
    began = time.monotonic()
    try:
        ids = await adapter.models()
    except LlmError as error:
        failure = error.failure
        if failure.status == 404:
            listed_at = "v1/models" if isinstance(config, AnthropicConfig) else "models"
            return Finding(
                "warn",
                "service",
                f"it has no list of models at {config.base_url}/{listed_at}, so whether it "
                "takes the key shows only when a session asks it",
            )
        return Finding("fail", "service", failure.message)
    finally:
        await adapter.aclose()
    seconds = time.monotonic() - began
    answered = f"answered in {seconds:.1f} s {_key(config, env)}"
    # Ollama lists a model with its tag, and takes it without the tag that is meant by default.
    if model in ids or f"{model}:latest" in ids:
        return Finding("ok", "service", f"{answered}, and lists {model}")
    return Finding(
        "warn",
        "service",
        f"{answered}, and does not list {model} among its {len(ids)} models. A session asks "
        "for it all the same, and the service may refuse it",
    )


def _key(config: AdapterConfig | AnthropicConfig, env: Mapping[str, str]) -> str:
    """Say which key the adapter sent: the one of the variable, the stored one, or none."""
    variable, file = config.api_key_env, config.api_key_file
    if variable is not None and trimmed(env.get(variable, "")):
        return f"to the key in the variable {variable}"
    if file is None:
        return "without a key"
    try:
        places = stored(file)
    except CredentialsError:
        # It was read for the request a moment ago, and something has changed it since.
        return "to a stored key"
    if config.provider in places:
        # A key that was stored for another place fails the request, and this one did not.
        return f"to the key that is stored for {config.provider}"
    return "without a key"


def shell() -> Finding:
    """Say which shell runs the commands and the hooks on this machine."""
    found = shells.find()
    if found is None:
        return Finding(
            "fail",
            "shell",
            f"{SHELL} is not installed: no command can run on this machine, and no hook at all",
        )
    return Finding("ok", "shell", found)


async def _held(entries: Sequence[Entry], env: Mapping[str, str], cwd: Path) -> Finding:
    """Say where the commands run, and whether they could."""
    entry = next(
        (entry for entry in entries if entry.plugin == _SANDBOX and not entry.disabled), None
    )
    if entry is None:
        return Finding(
            "warn",
            "sandbox",
            f"no entry loads {_SANDBOX}: commands run on this machine, and nothing holds them",
        )
    try:
        config = _config(entry, SandboxConfig, env)
    except _ConfigError as error:
        return Finding("fail", "sandbox", str(error))
    if config.provider == "none":
        return Finding(
            "ok",
            "sandbox",
            "none: commands run on this machine as it is. --sandbox docker runs them in containers",
        )
    if config.provider != "docker":
        return Finding(
            "warn", "sandbox", f"{config.provider}: a provider of another plugin, not looked at"
        )
    provider = Docker(config.docker)
    try:
        engine = await provider.ready(cwd)
    except SandboxUnavailableError as error:
        return Finding("fail", "sandbox", f"docker: {error}")
    finally:
        await provider.close()
    return Finding(
        "ok",
        "sandbox",
        f"docker: Docker {engine.version} on {engine.system}, with the image {config.docker.image}",
    )


def shown(findings: Sequence[Finding], width: int) -> str:
    """Return the findings as text for a terminal of a width, under what runs them.

    A finding that is longer than a line goes on under its own words.
    """
    python = platform.python_version()
    lines = [f"columbina {columbina.__version__}, Python {python}, {sys.platform}", ""]
    state, about = _COLUMNS
    indent = " " * (2 + state + about)
    lines.extend(
        textwrap.fill(
            finding.said,
            width=max(width - 1, len(indent) + 20),
            initial_indent=f"  {finding.state:<{state}}{finding.about:<{about}}",
            subsequent_indent=indent,
            break_long_words=False,
            break_on_hyphens=False,
        )
        for finding in findings
    )
    failed = sum(finding.state == "fail" for finding in findings)
    warned = sum(finding.state == "warn" for finding in findings)
    lines.append("")
    if failed:
        lines.append(f"{_things(failed)} in the way of a session.")
    elif warned:
        lines.append(f"A session can start. {_things(warned)} worth a look.")
    else:
        lines.append("A session can start.")
    return "\n".join(lines)


def _things(count: int) -> str:
    return "1 thing is" if count == 1 else f"{count} things are"
