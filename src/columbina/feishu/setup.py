"""Setting up the assistant in Feishu: keeping the app, saying how it is doing, and forgetting it.

A person sets the app up at a terminal with ``columbina feishu --setup``, and another program,
such as the desktop application, with ``columbina feishu --from-stdin``, which reads the app
as one document of JSON from the standard input. The App Secret is never an argument of the
command line, where it would stay in the history of the shell.

Before an app is kept, Feishu is asked for a token in its name, and for the address of a
connection. An App ID and an App Secret that do not belong together are not kept, and neither
is an app of Lark that is said to be one of Feishu, or the other way round: either platform
gives a token to an app of the other, and only the address tells them apart.

An assistant that runs goes on with the app that it read when it started, and writes down
whom it answers and in which session. So the app is kept or deleted only while no assistant
runs, and under the lock that a running assistant holds, which no assistant can take
meanwhile. So is whether its agent may change the files of its directory, which another
program gives as a document that says that alone.
"""

import contextlib
import json
import re
from collections.abc import Callable, Generator, Mapping
from pathlib import Path
from typing import Any, cast

import httpx

from columbina.feishu import store
from columbina.feishu.app import DOMAINS, Domain, FeishuError, host
from columbina.feishu.client import Feishu
from columbina.feishu.link import platform_checked
from columbina.llm.credentials import sendable, trimmed
from columbina.session.lock import acquire

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

_APP_ID = re.compile(r"cli_[A-Za-z0-9]{4,60}")
"""What an App ID looks like."""


def _app_id(given: object) -> str:
    """Return an App ID as it was given, or say what is wrong with it.

    Raises:
        FeishuError: It is no App ID.
    """
    app_id = given.strip() if isinstance(given, str) else ""
    if _APP_ID.fullmatch(app_id) is None:
        raise FeishuError(
            "the App ID is the one that the developer console shows under Credentials & Basic "
            "Info, and begins with cli_",
            code="INVALID",
        )
    return app_id


def _secret(given: object) -> str:
    """Return an App Secret as it can be sent, or say what is wrong with it.

    Raises:
        FeishuError: It cannot be one.
    """
    secret = trimmed(given) if isinstance(given, str) else ""
    if not secret or not sendable(secret):
        raise FeishuError(
            "the App Secret is the one that the developer console shows next to the App ID",
            code="INVALID",
        )
    return secret


def _domain(given: object) -> Domain:
    """Return the place that the app is in.

    Raises:
        FeishuError: It is neither Feishu nor Lark.
    """
    if given not in DOMAINS:
        raise FeishuError(f"the app is in {' or '.join(DOMAINS)}", code="INVALID")
    return given


async def checked(
    domain: Domain,
    app_id: str,
    secret: str,
    env: Mapping[str, str],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> None:
    """Ask the platform for a token of the app, and whether the app is one of its own.

    A token is given only where the App ID and the App Secret belong together, and either
    platform gives one to an app of the other as well.

    Raises:
        FeishuError: It gave no token, or the app is one of the other platform.
    """
    client = Feishu(host(domain, env), app_id, secret, env, transport=transport)
    try:
        await client.token()
        await platform_checked(client.host, app_id, secret, client.http)
    finally:
        await client.aclose()


async def kept_from(
    home: Path,
    given: Mapping[str, object],
    env: Mapping[str, str],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> dict[str, object]:
    """Check an app with Feishu and keep it, and return what to answer.

    A document that holds ``writable`` alone says whether the agent of the assistant may
    change the files of its directory, and changes that alone, see :func:`made_writable`.

    Args:
        home: The home of the program.
        given: The app: ``domain``, ``app_id`` and ``app_secret``.
        env: The environment.
        transport: The transport of the HTTP client, for tests.

    Returns:
        What was kept, and where the App Secret went: ``keyring`` for the keyring of the
        system, ``file`` for the file of keys, where it lies as it is and not encrypted.
        The secret itself is not said.

    Raises:
        FeishuError: The app is not one, or Feishu does not take it, or it cannot be kept, or an
            assistant runs.
    """
    if set(given) == {"writable"}:
        return made_writable(home, given["writable"], env)
    domain = _domain(given.get("domain", "feishu"))
    app_id = _app_id(given.get("app_id"))
    secret = _secret(given.get("app_secret"))
    with _alone(home, make=True):
        await checked(domain, app_id, secret, env, transport=transport)
        kept = store.keep(
            home, store.Settings(domain=domain, app_id=app_id), secret, host(domain, env)
        )
    return {
        "stored": {"domain": domain, "app_id": app_id},
        "secret_in": "file" if kept.keyring is None else "keyring",
    }


def made_writable(home: Path, writable: object, env: Mapping[str, str]) -> dict[str, object]:
    """Keep whether the agent of the assistant may change the files of its directory.

    A running assistant writes down what it read of the app when it started, so this is
    changed only while none runs, as the app is. A directory that lies in the home of the
    program, among what the program keeps, is not one that the agent may change.

    Returns:
        What is kept now.

    Raises:
        FeishuError: What is given is not ``true`` or ``false``, the directory lies in the home
            of the program, no app is set up, what is kept cannot be read or written, or an
            assistant runs.
    """
    if not isinstance(writable, bool):
        raise FeishuError("whether the assistant may change files is true or false", code="INVALID")
    folder = store.directory(home, env)
    if writable and store.within(folder, home):
        raise FeishuError(
            f"the directory of the assistant, {folder}, lies in the home of the program, where "
            f"it may change no file. Name another with {store.ASSISTANT_HOME}",
            code="FOLDER",
        )
    with _alone(home, make=False):
        chosen = store.settings(home)
        if chosen is None:
            raise FeishuError("no app of Feishu is set up", code="NONE")
        store.save(home, chosen.model_copy(update={"writable": writable}))
    return {"writable": writable}


@contextlib.contextmanager
def _alone(home: Path, *, make: bool) -> Generator[None]:
    """Hold the lock of the assistant while its app is changed, so that none runs meanwhile.

    Args:
        home: The home of the program.
        make: Whether the directory of the assistant is made where it is not there. Where it
            is not there and is not made, no assistant runs, and there is nothing to hold.

    Raises:
        FeishuError: An assistant runs, or the directory of the assistant cannot be used.
    """
    folder = home / store.FOLDER
    if not make and not folder.is_dir():
        yield
        return
    try:
        folder.mkdir(parents=True, exist_ok=True)
        held = acquire(folder / store.LOCK)
    except OSError as error:
        raise FeishuError(f"{folder} cannot be used: {error.strerror}", code="STORE") from None
    if held is None:
        raise FeishuError(
            "an assistant in Feishu runs on this computer, and goes on with the app that it "
            "read when it started. Stop it, and then change the app",
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
    """Say which app is set up and how its assistant is doing, for a program to read.

    ``folder`` is the directory of the assistant, and ``writable`` whether its agent may change
    the files there.

    Raises:
        FeishuError: What is kept cannot be read.
    """
    chosen = store.settings(home)
    folder = str(store.directory(home, env))
    if chosen is None:
        return {"set_up": False, "running": _running(home), "folder": folder}
    running = _running(home)
    told = store.status(home)
    said: dict[str, object] = {
        "set_up": True,
        "domain": chosen.domain,
        "app_id": chosen.app_id,
        "console": host(chosen.domain, env).console,
        "paired": chosen.owner is not None,
        "folder": folder,
        "writable": chosen.writable,
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
    """Delete the app, and say whether there was one.

    Raises:
        FeishuError: An assistant runs, or the App Secret cannot be taken out of the keyring of
            the system.
    """
    with _alone(home, make=False):
        return {"forgotten": store.forget(home)}


def said(answer: Mapping[str, object]) -> str:
    """Return a document of JSON as a line."""
    return json.dumps(answer, ensure_ascii=False)


def read_given(text: str) -> dict[str, Any]:
    """Read the app that another program gave as JSON.

    Raises:
        FeishuError: It is no object of JSON.
    """
    try:
        given: object = json.loads(text)
    except ValueError:
        given = None
    if not isinstance(given, dict):
        raise FeishuError("the standard input holds no object of JSON", code="INVALID")
    return cast("dict[str, Any]", given)


Ask = Callable[[str], str]
"""Asks a person something at a terminal, and returns the answer."""

NEXT = (
    "Next, run 'columbina feishu' and leave it running. Meanwhile, in the developer console:\n"
    "1. Under Permissions, give the app im:message.p2p_msg:readonly, im:message:send_as_bot, "
    "im:message.reactions:write_only and im:message:readonly; to read documents also "
    "docx:document:readonly, wiki:wiki:readonly and sheets:spreadsheet:readonly.\n"
    "2. Under Events & Callbacks, choose to receive events through the persistent connection "
    "(this is saved only while 'columbina feishu' runs), and add the event "
    "im.message.receive_v1 (Receive messages).\n"
    "3. Publish a version of the app, with yourself in its availability.\n"
    "4. Find the bot in Feishu, and send it the code of pairing that 'columbina feishu' shows."
)
"""What a person does after the app is kept."""


async def at_a_terminal(
    home: Path,
    env: Mapping[str, str],
    ask: Ask,
    ask_unseen: Ask,
    show: Callable[[str], None],
    *,
    transport: httpx.AsyncBaseTransport | None = None,
) -> int:
    """Ask a person at a terminal for the app, check it with Feishu, and keep it.

    Returns:
        0 where the app was kept, and 2 where it was not.
    """
    show(
        "The assistant answers you in Feishu through an app of your own. Make one in the "
        "developer console, https://open.feishu.cn/app for Feishu or "
        "https://open.larksuite.com/app for Lark: a custom app with the ability of a bot."
    )
    where = ask("Is the app in Feishu or in Lark? [feishu] ").strip().lower() or "feishu"
    given = {"domain": where, "app_id": ask("App ID: "), "app_secret": ask_unseen("App Secret: ")}
    try:
        answer = await kept_from(home, given, env, transport=transport)
    except FeishuError as error:
        show(f"The app is not kept: {error}.")
        return 2
    if answer["secret_in"] == "keyring":
        where_it_is = "The App Secret is in the keyring of the system."
    else:
        where_it_is = (
            "No keyring of the system took the App Secret, so it lies as it is in "
            f"{home / store.CREDENTIALS}."
        )
    show(f"Feishu takes the app, and it is kept. {where_it_is}\n\n{NEXT}")
    return 0
