"""Setting up the assistant in WeCom: keeping the bot, saying how it is doing, and forgetting it.

A person sets the bot up at a terminal with ``columbina wecom --setup``, and another program,
such as the desktop application, with ``columbina wecom --from-stdin``, which reads the bot as
one document of JSON from the standard input. The Secret is never an argument of the command
line, where it would stay in the history of the shell.

Before a bot is kept, it signs in to WeCom once: a Bot ID and a Secret that do not belong
together are not kept, and neither is a bot whose API is not set to the long connection. A bot
has one connection at a time, so the sign-in closes one that a program elsewhere holds.

An assistant that runs goes on with the bot that it read when it started, and writes down
whom it answers and in which session. So the bot is kept or deleted only while no assistant
runs, and under the lock that a running assistant holds, which no assistant can take
meanwhile. So is whether its agent may change the files of its directory, which another
program gives as a document that says that alone.
"""

import contextlib
import json
from collections.abc import Callable, Generator, Mapping
from pathlib import Path
from typing import Any, cast

from columbina.apps import thinking
from columbina.feishu.store import within
from columbina.llm.credentials import sendable, trimmed
from columbina.session.lock import acquire
from columbina.wecom import store
from columbina.wecom.app import WecomError, address
from columbina.wecom.link import Link, Opener

__all__ = [
    "NEXT",
    "Ask",
    "at_a_terminal",
    "checked",
    "forgotten",
    "kept_from",
    "made_writable",
    "read_given",
    "said",
    "status_of",
]

_LONGEST_ID = 128
"""How long a Bot ID may be, and more than one is."""


def _bot_id(given: object) -> str:
    """Return a Bot ID as it was given, or say what is wrong with it.

    Raises:
        WecomError: It is no Bot ID.
    """
    bot_id = given.strip() if isinstance(given, str) else ""
    if not bot_id or len(bot_id) > _LONGEST_ID or not sendable(bot_id):
        raise WecomError(
            "the Bot ID is the one that the page of the bot shows under the API, in the long "
            "connection",
            code="INVALID",
        )
    return bot_id


def _secret(given: object) -> str:
    """Return a Secret as it can be sent, or say what is wrong with it.

    Raises:
        WecomError: It cannot be one.
    """
    secret = trimmed(given) if isinstance(given, str) else ""
    if not secret or not sendable(secret):
        raise WecomError(
            "the Secret is the one that the page of the bot shows next to the Bot ID",
            code="INVALID",
        )
    return secret


async def checked(
    bot_id: str, secret: str, env: Mapping[str, str], *, opener: Opener | None = None
) -> None:
    """Sign the bot in to WeCom once, and say whether WeCom takes it.

    Raises:
        WecomError: WeCom does not take it, or could not be reached.
    """
    link = Link(
        address(env),
        bot_id,
        secret,
        heard=lambda frame: None,
        told=lambda state, why: None,
        env=env,
    )
    if opener is not None:
        link.opener = opener
    await link.check()


async def kept_from(
    home: Path,
    given: Mapping[str, object],
    env: Mapping[str, str],
    *,
    opener: Opener | None = None,
) -> dict[str, object]:
    """Check a bot with WeCom and keep it, and return what to answer.

    A document that holds ``writable`` alone says whether the agent of the assistant may
    change the files of its directory, and changes that alone, see :func:`made_writable`.

    Args:
        home: The home of the program.
        given: The bot: ``bot_id`` and ``secret``.
        env: The environment.
        opener: Opens the WebSocket of the check, for tests.

    Returns:
        What was kept, and where the Secret went: ``keyring`` for the keyring of the system,
        ``file`` for the file of keys, where it lies as it is and not encrypted. The secret
        itself is not said.

    Raises:
        WecomError: The bot is not one, or WeCom does not take it, or it cannot be kept, or an
            assistant runs.
    """
    if set(given) == {"writable"}:
        return made_writable(home, given["writable"], env)
    if thinking.is_a_choice(given):
        return made_thinking(home, given)
    bot_id = _bot_id(given.get("bot_id"))
    secret = _secret(given.get("secret"))
    place = address(env)
    with _alone(home, make=True):
        await checked(bot_id, secret, env, opener=opener)
        kept = store.keep(home, store.Settings(bot_id=bot_id), secret, place)
    return {
        "stored": {"bot_id": bot_id},
        "secret_in": "file" if kept.keyring is None else "keyring",
    }


def made_writable(home: Path, writable: object, env: Mapping[str, str]) -> dict[str, object]:
    """Keep whether the agent of the assistant may change the files of its directory.

    A running assistant writes down what it read of the bot when it started, so this is
    changed only while none runs, as the bot is. A directory that lies in the home of the
    program, among what the program keeps, is not one that the agent may change.

    Returns:
        What is kept now.

    Raises:
        WecomError: What is given is not ``true`` or ``false``, the directory lies in the home
            of the program, no bot is set up, what is kept cannot be read or written, or an
            assistant runs.
    """
    if not isinstance(writable, bool):
        raise WecomError("whether the assistant may change files is true or false", code="INVALID")
    folder = store.directory(home, env)
    if writable and within(folder, home):
        raise WecomError(
            f"the directory of the assistant, {folder}, lies in the home of the program, where "
            f"it may change no file. Name another with {store.ASSISTANT_HOME}",
            code="FOLDER",
        )
    with _alone(home, make=False):
        chosen = store.settings(home)
        if chosen is None:
            raise WecomError("no bot of WeCom is set up", code="NONE")
        store.save(home, chosen.model_copy(update={"writable": writable}))
    return {"writable": writable}


def made_thinking(home: Path, given: Mapping[str, object]) -> dict[str, object]:
    """Keep which model the assistant asks and how hard it thinks, from a document of those alone.

    A running assistant goes on with what it read when it started, so this is changed only while
    none runs, as the bot is.

    Returns:
        What is kept now.

    Raises:
        WecomError: What is given is no model or no level, no bot is set up, what is kept
            cannot be read or written, or an assistant runs.
    """
    try:
        model, effort = thinking.chosen(given)
    except ValueError as problem:
        raise WecomError(str(problem), code="INVALID") from None
    with _alone(home, make=False):
        chosen = store.settings(home)
        if chosen is None:
            raise WecomError("no bot of WeCom is set up", code="NONE")
        store.save(home, chosen.model_copy(update={"model": model, "effort": effort}))
    return {"model": model, "effort": effort}


@contextlib.contextmanager
def _alone(home: Path, *, make: bool) -> Generator[None]:
    """Hold the lock of the assistant while its bot is changed, so that none runs meanwhile.

    Args:
        home: The home of the program.
        make: Whether the directory of the assistant is made where it is not there. Where it
            is not there and is not made, no assistant runs, and there is nothing to hold.

    Raises:
        WecomError: An assistant runs, or the directory of the assistant cannot be used.
    """
    folder = home / store.FOLDER
    if not make and not folder.is_dir():
        yield
        return
    try:
        folder.mkdir(parents=True, exist_ok=True)
        held = acquire(folder / store.LOCK)
    except OSError as error:
        raise WecomError(f"{folder} cannot be used: {error.strerror}", code="STORE") from None
    if held is None:
        raise WecomError(
            "an assistant in WeCom runs on this computer, and goes on with the bot that it read "
            "when it started. Stop it, and then change the bot",
            code="RUNNING",
        )
    try:
        yield
    finally:
        held.release()


def _running(home: Path) -> bool:
    """Tell whether an assistant runs: whether somebody holds its lock."""
    folder = home / store.FOLDER
    if not folder.is_dir():
        return False
    held = acquire(folder / store.LOCK)
    if held is None:
        return True
    held.release()
    return False


def status_of(home: Path, env: Mapping[str, str]) -> dict[str, object]:
    """Say which bot is set up and how its assistant is doing, for a program to read.

    ``folder`` is the directory of the assistant, and ``writable`` whether its agent may change
    the files there.

    Raises:
        WecomError: What is kept cannot be read.
    """
    chosen = store.settings(home)
    folder = str(store.directory(home, env))
    if chosen is None:
        return {"set_up": False, "running": _running(home), "folder": folder}
    running = _running(home)
    told = store.status(home)
    said: dict[str, object] = {
        "set_up": True,
        "bot_id": chosen.bot_id,
        "paired": chosen.owner is not None,
        "folder": folder,
        "writable": chosen.writable,
        "model": chosen.model,
        "effort": chosen.effort,
        "running": running,
        "state": None,
        "code": None,
        "error": None,
    }
    if told is not None and (running or told.state == "failed"):
        said["state"] = told.state if running else "failed"
        said["code"] = told.code if running else None
        said["error"] = told.error
    return said


def forgotten(home: Path) -> dict[str, object]:
    """Delete the bot, and say whether there was one.

    Raises:
        WecomError: An assistant runs, or the Secret cannot be taken out of the keyring of the
            system.
    """
    with _alone(home, make=False):
        return {"forgotten": store.forget(home)}


def said(answer: Mapping[str, object]) -> str:
    """Return a document of JSON as a line."""
    return json.dumps(answer, ensure_ascii=False)


def read_given(text: str) -> dict[str, Any]:
    """Read the bot that another program gave as JSON.

    Raises:
        WecomError: It is no object of JSON.
    """
    try:
        given: object = json.loads(text)
    except ValueError:
        given = None
    if not isinstance(given, dict):
        raise WecomError("the standard input holds no object of JSON", code="INVALID")
    return cast("dict[str, Any]", given)


Ask = Callable[[str], str]
"""Asks a person something at a terminal, and returns the answer."""

NEXT = (
    "Next, run 'columbina wecom' and leave it running. Then find the bot in WeCom, and send it "
    "the code of pairing that 'columbina wecom' shows."
)
"""What a person does after the bot is kept."""


async def at_a_terminal(
    home: Path,
    env: Mapping[str, str],
    ask: Ask,
    ask_unseen: Ask,
    show: Callable[[str], None],
    *,
    opener: Opener | None = None,
) -> int:
    """Ask a person at a terminal for the bot, check it with WeCom, and keep it.

    Returns:
        0 where the bot was kept, and 2 where it was not.
    """
    show(
        "The assistant answers you in WeCom through a smart bot of your own. In the admin "
        "console of WeCom, make one under Smart bots, and on its page set its API to the long "
        "connection: the page then shows its Bot ID and its Secret."
    )
    given = {"bot_id": ask("Bot ID: "), "secret": ask_unseen("Secret: ")}
    try:
        answer = await kept_from(home, given, env, opener=opener)
    except WecomError as error:
        show(f"The bot is not kept: {error}.")
        return 2
    if answer["secret_in"] == "keyring":
        where_it_is = "The Secret is in the keyring of the system."
    else:
        where_it_is = (
            "No keyring of the system took the Secret, so it lies as it is in "
            f"{home / store.CREDENTIALS}."
        )
    show(f"WeCom takes the bot, and it is kept. {where_it_is}\n\n{NEXT}")
    return 0
