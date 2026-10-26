"""What ``columbina search-key`` does: the key of a service that searches the web.

``columbina search-key`` says of Tavily and of Brave whether a key is there for
it, and where: in its variable, kept, or none. ``columbina search-key NAME``
keeps a key for one of them: typed without being shown at a terminal, and
read from the standard input otherwise, as another program hands it over.
``--forget`` deletes the key that is kept. With ``--json`` what is said is a
document of JSON on a line of its own, for another program. A key is never
repeated, and never taken from the command line.

See :mod:`columbina.web.keys`.
"""

from collections.abc import Callable, Mapping
from pathlib import Path

from columbina.llm.credentials import CredentialsError
from columbina.web import keys

__all__ = ["SEARCH_KEY", "run"]

SEARCH_KEY = "search-key"
"""The name of the command."""

_MOST = 4096
"""How many characters of the standard input are read for a key at most."""


def run(
    engine: keys.Keyed | None,
    *,
    forget: bool,
    as_json: bool,
    home: Path,
    env: Mapping[str, str],
    ask_unseen: Callable[[str], str] | None,
    read_input: Callable[[int], str],
    say: Callable[[str], None],
    answer: Callable[[Mapping[str, object]], None],
    tell: Callable[[str], None],
) -> int:
    """Carry out the command, and return its exit code.

    Args:
        engine: The service whose key is kept or deleted, or ``None`` to
            say what is there.
        forget: Whether the key is to be deleted.
        as_json: Whether what is said is a document of JSON.
        home: The home of the program.
        env: The environment of the program.
        ask_unseen: Asks for a key without showing it, at a terminal, or
            ``None`` where there is none.
        read_input: Reads at most so many characters of the standard input.
        say: Says a line to a person.
        answer: Writes a document of JSON.
        tell: Says what is wrong.
    """

    def failed(said: str) -> int:
        if as_json:
            answer({"error": said})
        else:
            tell(said)
        return 2

    try:
        if engine is None:
            listed = keys.listed(home, env)
            if as_json:
                answer(listed)
            else:
                for one in listed["services"]:
                    where = {"variable": f"in {one['variable']}", "kept": "kept", "none": "none"}
                    say(f"{one['name']}: {where[one['key']]}")
            return 0
        if forget:
            gone = keys.forget(home, engine)
            if as_json:
                answer({"forgot": bool(gone)})
            else:
                say(
                    f"The key of {engine} was deleted." if gone else f"No key of {engine} was kept."
                )
            return 0
        typed = (
            ask_unseen(f"The key of {engine}: ") if ask_unseen is not None else read_input(_MOST)
        )
        kept = keys.keep(home, engine, typed)
    except CredentialsError as error:
        return failed(str(error))
    where = "the file of keys" if kept.keyring is None else kept.keyring
    if as_json:
        answer({"kept": engine, "keyring": kept.keyring})
    else:
        say(f"The key of {engine} is kept in {where}.")
    return 0
