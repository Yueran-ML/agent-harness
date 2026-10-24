"""Setting up the assistant in QQ: keeping the bot, saying how it is doing, and forgetting it.

A person sets the bot up at a terminal with ``columbina qq --setup``, and another program, such
as the desktop application, with ``columbina qq --from-stdin``, which reads the bot as one
document of JSON from the standard input. The AppSecret is never an argument of the command
line, where it would stay in the history of the shell.

Before a bot is kept, its AppID and AppSecret are exchanged once for a token: two that do not
belong together are not kept.

An assistant that runs goes on with the bot that it read when it started, and writes down whom
it answers and in which sessions. So the bot is kept or deleted only while no assistant runs,
and under the lock that a running assistant holds, which no assistant can take meanwhile.
"""

import contextlib
import json
from collections.abc import Callable, Generator, Mapping
from pathlib import Path
from typing import Any, cast

import httpx

from columbina.llm.credentials import sendable, trimmed
from columbina.qq import store
from columbina.qq.app import Api, QqError, token_url
from columbina.session.lock import acquire

__all__ = [
    "NEXT",
    "Ask",
    "at_a_terminal",
    "checked",
    "forgotten",
    "kept_from",
    "read_given",
    "status_of",
]

_LONGEST_ID = 64
"""How long an AppID may be, and more than one is."""


def _app_id(given: object) -> str:
    """Return an AppID as it was given, or say what is wrong with it.

    Raises:
        QqError: It is no AppID.
    """
    app_id = given.strip() if isinstance(given, str) else ""
    if not app_id or len(app_id) > _LONGEST_ID or not sendable(app_id):
        raise QqError(
            "the AppID is the one that the page of the bot on the open platform of QQ shows "
            "under its development settings",
            code="INVALID",
        )
    return app_id


def _secret(given: object) -> str:
    """Return an AppSecret as it can be sent, or say what is wrong with it.

    Raises:
        QqError: It cannot be one.
    """
    secret = trimmed(given) if isinstance(given, str) else ""
    if not secret or not sendable(secret):
        raise QqError(
            "the AppSecret is the one that the page of the bot shows next to the AppID",
            code="INVALID",
        )
    return secret


async def checked(
    app_id: str,
    secret: str,
    env: Mapping[str, str],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> None:
    """Ask QQ once for a token of the bot, and so whether it takes the AppID and the AppSecret.

    Raises:
        QqError: QQ does not take them, or could not be reached.
    """
    api = Api(app_id, secret, env, transport=transport)
    try:
        await api.token()
    finally:
        await api.aclose()


async def kept_from(
    home: Path,
    given: Mapping[str, object],
    env: Mapping[str, str],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> dict[str, object]:
    """Check a bot with QQ and keep it, and return what to answer.

    Args:
        home: The home of the program.
        given: The bot: ``app_id`` and ``secret``.
        env: The environment.
        transport: Sends the request of the check in place of the network, for tests.

    Returns:
        What was kept, and where the AppSecret went: ``keyring`` for the keyring of the system,
        ``file`` for the file of keys, where it lies as it is and not encrypted. The secret
        itself is not said.

    Raises:
        QqError: The bot is not one, or QQ does not take it, or it cannot be kept, or an
            assistant runs.
    """
    app_id = _app_id(given.get("app_id"))
    secret = _secret(given.get("secret"))
    with _alone(home, make=True):
        await checked(app_id, secret, env, transport=transport)
        kept = store.keep(home, store.Settings(app_id=app_id), secret, token_url(env))
    return {
        "stored": {"app_id": app_id},
        "secret_in": "file" if kept.keyring is None else "keyring",
    }


@contextlib.contextmanager
def _alone(home: Path, *, make: bool) -> Generator[None]:
    """Hold the lock of the assistant while its bot is changed, so that none runs meanwhile.

    Raises:
        QqError: An assistant runs, or the directory of the assistant cannot be used.
    """
    folder = home / store.FOLDER
    if not make and not folder.is_dir():
        yield
        return
    try:
        folder.mkdir(parents=True, exist_ok=True)
        held = acquire(folder / store.LOCK)
    except OSError as error:
        raise QqError(f"{folder} cannot be used: {error.strerror}", code="STORE") from None
    if held is None:
        raise QqError(
            "an assistant in QQ runs on this computer, and goes on with the bot that it read when "
            "it started. Stop it, and then change the bot",
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

    ``folder`` is the directory of the assistant.

    Raises:
        QqError: What is kept cannot be read.
    """
    chosen = store.settings(home)
    folder = str(store.directory(home, env))
    if chosen is None:
        return {"set_up": False, "running": _running(home), "folder": folder}
    running = _running(home)
    told = store.status(home)
    said: dict[str, object] = {
        "set_up": True,
        "app_id": chosen.app_id,
        "paired": chosen.owner is not None,
        "folder": folder,
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
        QqError: An assistant runs, or the AppSecret cannot be taken out of the keyring of the
            system.
    """
    with _alone(home, make=False):
        return {"forgotten": store.forget(home)}


def read_given(text: str) -> dict[str, Any]:
    """Read the bot that another program gave as JSON.

    Raises:
        QqError: It is no object of JSON.
    """
    try:
        given: object = json.loads(text)
    except ValueError:
        given = None
    if not isinstance(given, dict):
        raise QqError("the standard input holds no object of JSON", code="INVALID")
    return cast("dict[str, Any]", given)


Ask = Callable[[str], str]
"""Asks a person something at a terminal, and returns the answer."""

NEXT = (
    "Next, run 'columbina qq' and leave it running. Then find the bot in QQ, and send it the "
    "code of pairing that 'columbina qq' shows. Put the bot in a group, and it answers whoever "
    "mentions it there."
)
"""What a person does after the bot is kept."""


async def at_a_terminal(
    home: Path,
    env: Mapping[str, str],
    ask: Ask,
    ask_unseen: Ask,
    show: Callable[[str], None],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> int:
    """Ask a person at a terminal for the bot, check it with QQ, and keep it.

    Returns:
        0 where the bot was kept, and 2 where it was not.
    """
    show(
        "The assistant answers you in QQ through a bot of your own. Make one on the open "
        "platform of QQ, q.qq.com: its page shows its AppID and its AppSecret under the "
        "development settings. Set its events to come by WebSocket."
    )
    given = {"app_id": ask("AppID: "), "secret": ask_unseen("AppSecret: ")}
    try:
        answer = await kept_from(home, given, env, transport=transport)
    except QqError as error:
        show(f"The bot is not kept: {error}.")
        return 2
    if answer["secret_in"] == "keyring":
        where_it_is = "The AppSecret is in the keyring of the system."
    else:
        where_it_is = (
            "No keyring of the system took the AppSecret, so it lies as it is in "
            f"{home / store.CREDENTIALS}."
        )
    show(f"QQ takes the bot, and it is kept. {where_it_is}\n\n{NEXT}")
    return 0
