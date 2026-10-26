"""The keys of the services that search the web: where they are kept, and which service is asked.

A key of Tavily or of Brave is put in once with ``columbina search-key``, and
it is kept as the key of a model is: in the keyring of the system where there
is one, and in the file ``search-credentials.json`` in the home of the program
otherwise, see :mod:`columbina.llm.credentials`. A variable of the
environment goes before what is kept: ``TAVILY_API_KEY`` and ``BRAVE_API_KEY``.

Which service is asked is said by the config of the plugin, and by default it
is the first that has a key: Tavily, then Brave. Where neither has one,
DuckDuckGo is asked, which needs none.
"""

import logging
from collections.abc import Mapping
from pathlib import Path
from typing import Literal, get_args

from columbina.llm import credentials
from columbina.web.search import DUCKDUCKGO, ENGINES, Engine

__all__ = [
    "FILE",
    "KEYED",
    "VARIABLES",
    "Chosen",
    "Keyed",
    "chosen",
    "forget",
    "keep",
    "key_of",
    "listed",
]

logger = logging.getLogger("columbina.web")

FILE = "search-credentials.json"
"""The file of the keys, in the home of the program."""

Keyed = Literal["tavily", "brave"]
"""The services that search with a key."""

KEYED: tuple[Keyed, ...] = get_args(Keyed)
"""Those services, in the order in which they are chosen."""

VARIABLES: Mapping[Keyed, str] = {"tavily": "TAVILY_API_KEY", "brave": "BRAVE_API_KEY"}
"""The variable of the environment that holds the key of each one."""

Chosen = Literal["auto", "duckduckgo", "tavily", "brave"]
"""What the config names: a service, or ``auto`` for the first that has a key."""


def _origin(engine: Keyed) -> str:
    return credentials.origin_of(ENGINES[engine])


def key_of(home: Path, env: Mapping[str, str], engine: Keyed) -> str | None:
    """Return the key of a service: the one of its variable, or the one that is kept.

    A key that is kept and cannot be had is said in the log, and counts as none.
    """
    given = credentials.trimmed(env.get(VARIABLES[engine], ""))
    if given:
        return given
    try:
        return credentials.key_for(home / FILE, engine, _origin(engine))
    except credentials.CredentialsError as error:
        logger.warning("the key of %s for searching cannot be had: %s", engine, error)
        return None


def chosen(home: Path, env: Mapping[str, str], wanted: Chosen) -> tuple[Engine, str | None]:
    """Return the service that searches, and its key."""
    if wanted == "duckduckgo":
        return DUCKDUCKGO, None
    if wanted != "auto":
        return wanted, key_of(home, env, wanted)
    for engine in KEYED:
        key = key_of(home, env, engine)
        if key is not None:
            return engine, key
    return DUCKDUCKGO, None


def keep(home: Path, engine: Keyed, key: str) -> credentials.Kept:
    """Keep the key of a service.

    Raises:
        CredentialsError: It could not be kept, or it is no key.
    """
    return credentials.keep(home / FILE, engine, key, _origin(engine))


def forget(home: Path, engine: Keyed) -> credentials.Forgotten:
    """Delete the key that is kept for a service.

    Raises:
        CredentialsError: It could not be deleted.
    """
    return credentials.forget(home / FILE, engine)


def listed(home: Path, env: Mapping[str, str]) -> dict[str, list[dict[str, str]]]:
    """Say of each service whether it has a key, and where, with no key in what is said.

    Raises:
        CredentialsError: The file of the keys cannot be read.
    """
    kept = credentials.stored(home / FILE)
    services: list[dict[str, str]] = []
    for engine in KEYED:
        variable = VARIABLES[engine]
        if env.get(variable, "").strip():
            where = "variable"
        elif engine in kept:
            where = "kept"
        else:
            where = "none"
        services.append({"name": engine, "key": where, "variable": variable})
    return {"services": services}
