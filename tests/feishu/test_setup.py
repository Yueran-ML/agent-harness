"""Setting up the assistant in Feishu: ``columbina.feishu.setup``."""

import json
from collections.abc import Callable
from pathlib import Path

import httpx
import pytest

from columbina.feishu import store
from columbina.feishu.app import FeishuError, Host
from columbina.feishu.client import TOKEN
from columbina.feishu.link import ENDPOINT
from columbina.feishu.setup import (
    NEXT,
    at_a_terminal,
    checked,
    forgotten,
    kept_from,
    read_given,
    said,
    status_of,
)
from columbina.session.lock import acquire
from support.fake_feishu import SECRET
from support.vault import Memory

FEISHU = Host("feishu", "https://open.feishu.cn")
APP = {"domain": "feishu", "app_id": "cli_a1b2c3d4", "app_secret": SECRET}


Answer = Callable[[httpx.Request], httpx.Response]


def answering(status: int, body: object = None) -> Answer:
    return lambda request: httpx.Response(status, json=body)


connection_given = answering(200, {"code": 0, "data": {"URL": "wss://x/ws?service_id=1"}})
elsewhere = answering(200, {"code": 1000040351, "msg": "Incorrect domain name"})
"""What a platform answers for a connection of an app of the other one."""


def platform(
    code: int = 0,
    *,
    connection: Answer = connection_given,
    asked: list[httpx.Request] | None = None,
) -> httpx.MockTransport:
    """The open platform: a token for the app or a refusal, and the address of a connection."""

    def answer(request: httpx.Request) -> httpx.Response:
        if asked is not None:
            asked.append(request)
        if request.url.path == ENDPOINT:
            return connection(request)
        assert request.url.path == TOKEN
        if code:
            return httpx.Response(400, json={"code": code, "msg": "app secret invalid"})
        return httpx.Response(200, json={"code": 0, "tenant_access_token": "t-1", "expire": 7200})

    return httpx.MockTransport(answer)


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


async def test_an_app_that_feishu_takes_is_kept(home: Path) -> None:
    answer = await kept_from(home, APP, {}, transport=platform())

    assert answer == {
        "stored": {"domain": "feishu", "app_id": "cli_a1b2c3d4"},
        "secret_in": "file",
    }
    assert store.settings(home) == store.Settings(domain="feishu", app_id="cli_a1b2c3d4")
    assert store.secret(home, FEISHU) == SECRET


async def test_the_answer_says_that_the_secret_went_into_the_keyring(home: Path) -> None:
    Memory().install()

    answer = await kept_from(home, {**APP, "app_secret": f"  {SECRET}\n"}, {}, transport=platform())

    assert answer["secret_in"] == "keyring"
    assert store.secret(home, FEISHU) == SECRET


async def test_an_app_id_is_kept_without_the_blanks_around_it(home: Path) -> None:
    answer = await kept_from(home, {**APP, "app_id": " cli_a1b2c3d4\n"}, {}, transport=platform())

    assert answer["stored"] == {"domain": "feishu", "app_id": "cli_a1b2c3d4"}
    settings = store.settings(home)
    assert settings is not None
    assert settings.app_id == "cli_a1b2c3d4"


async def test_an_app_without_a_domain_is_in_feishu(home: Path) -> None:
    given = {"app_id": "cli_a1b2c3d4", "app_secret": SECRET}

    answer = await kept_from(home, given, {}, transport=platform())

    assert answer["stored"] == {"domain": "feishu", "app_id": "cli_a1b2c3d4"}


@pytest.mark.parametrize(
    ("given", "said_of_it"),
    [
        ({**APP, "domain": "dingtalk"}, "the app is in feishu or lark"),
        ({**APP, "app_id": "12345"}, "begins with cli_"),
        ({**APP, "app_id": None}, "begins with cli_"),
        ({**APP, "app_secret": ""}, "next to the App ID"),
        ({**APP, "app_secret": "two halves"}, "next to the App ID"),
        ({**APP, "app_secret": 3}, "next to the App ID"),
    ],
)
async def test_what_is_no_app_is_not_kept(
    home: Path, given: dict[str, object], said_of_it: str
) -> None:
    with pytest.raises(FeishuError) as refused:
        await kept_from(home, given, {}, transport=platform())

    assert refused.value.code == "INVALID"
    assert said_of_it in str(refused.value)
    assert store.settings(home) is None


async def test_an_app_that_feishu_does_not_take_is_not_kept(home: Path) -> None:
    with pytest.raises(FeishuError) as refused:
        await kept_from(home, APP, {}, transport=platform(code=10014))

    assert refused.value.number == 10014
    assert store.settings(home) is None


async def test_an_app_is_checked_by_asking_for_its_token_and_then_for_a_connection() -> None:
    asked: list[httpx.Request] = []

    await checked("feishu", "cli_a1b2c3d4", SECRET, {}, transport=platform(asked=asked))

    assert [str(request.url) for request in asked] == [
        f"https://open.feishu.cn{TOKEN}",
        f"https://open.feishu.cn{ENDPOINT}",
    ]
    assert json.loads(asked[1].content) == {"AppID": "cli_a1b2c3d4", "AppSecret": SECRET}


async def test_no_connection_is_asked_for_an_app_without_a_token() -> None:
    asked: list[httpx.Request] = []

    with pytest.raises(FeishuError):
        await checked("feishu", "cli_a1b2c3d4", SECRET, {}, transport=platform(10014, asked=asked))

    assert [request.url.path for request in asked] == [TOKEN]


@pytest.mark.parametrize(("domain", "other"), [("feishu", "Lark"), ("lark", "Feishu")])
async def test_an_app_of_the_other_platform_is_not_kept(
    home: Path, domain: str, other: str
) -> None:
    # Either platform gives a token to an app of the other: only the connection tells them apart.
    given = {**APP, "domain": domain}
    with pytest.raises(FeishuError) as refused:
        await kept_from(home, given, {}, transport=platform(connection=elsewhere))

    assert refused.value.code == "WRONG_PLATFORM"
    assert str(refused.value).endswith(f"Set it up as an app of {other}")
    assert store.settings(home) is None


@pytest.mark.parametrize(
    "connection",
    [
        pytest.param(answering(200, {"code": 1, "msg": "system busy"}), id="busy"),
        pytest.param(answering(502), id="no answer"),
        pytest.param(answering(200, {"code": 1000040344, "msg": "no"}), id="refused otherwise"),
    ],
)
async def test_an_app_whose_connection_cannot_be_told_of_now_is_kept_all_the_same(
    home: Path, connection: Answer
) -> None:
    answer = await kept_from(home, APP, {}, transport=platform(connection=connection))

    assert answer["stored"] == {"domain": "feishu", "app_id": "cli_a1b2c3d4"}
    assert store.secret(home, FEISHU) == SECRET


async def test_the_app_is_neither_kept_nor_deleted_while_an_assistant_runs(home: Path) -> None:
    store.keep(home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"), SECRET, FEISHU)
    (home / store.FOLDER).mkdir()
    held = acquire(home / store.FOLDER / store.LOCK)
    assert held is not None
    try:
        with pytest.raises(FeishuError) as kept:
            await kept_from(home, {**APP, "app_id": "cli_other1"}, {}, transport=platform())
        with pytest.raises(FeishuError) as deleted:
            forgotten(home)
    finally:
        held.release()

    assert (kept.value.code, deleted.value.code) == ("RUNNING", "RUNNING")
    assert "Stop it" in str(kept.value)
    settings = store.settings(home)
    assert settings is not None
    assert settings.app_id == "cli_a1b2c3d4"
    assert store.secret(home, FEISHU) == SECRET


async def test_a_directory_of_the_assistant_that_cannot_be_used_keeps_nothing(home: Path) -> None:
    home.mkdir()
    (home / store.FOLDER).write_text("a file where the directory would be", encoding="utf-8")

    with pytest.raises(FeishuError) as refused:
        await kept_from(home, APP, {}, transport=platform())

    assert refused.value.code == "STORE"
    assert store.settings(home) is None


def test_nothing_set_up_is_said(home: Path) -> None:
    assert status_of(home, {}) == {"set_up": False, "running": False}


def test_an_app_that_is_set_up_and_not_running_is_said(home: Path) -> None:
    store.keep(
        home,
        store.Settings(domain="lark", app_id="cli_a1b2c3d4"),
        SECRET,
        Host("lark", "https://open.larksuite.com"),
    )
    (home / store.FOLDER).mkdir()

    assert status_of(home, {}) == {
        "set_up": True,
        "domain": "lark",
        "app_id": "cli_a1b2c3d4",
        "console": "https://open.larksuite.com/app",
        "paired": False,
        "running": False,
        "state": None,
        "code": None,
        "error": None,
    }


def test_a_running_assistant_says_how_it_is_doing(home: Path) -> None:
    store.keep(home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"), SECRET, FEISHU)
    store.tell(home, 7, "connected", paired=False, code="123456", error=None)
    held = acquire(home / store.FOLDER / store.LOCK)

    told = status_of(home, {})

    assert held is not None
    held.release()
    assert (told["running"], told["state"], told["code"]) == (True, "connected", "123456")


def test_an_assistant_that_failed_says_why_after_it_ended(home: Path) -> None:
    store.keep(home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"), SECRET, FEISHU)
    store.tell(home, 7, "failed", paired=True, code="123456", error="Feishu refuses it")

    told = status_of(home, {})

    assert (told["running"], told["state"], told["code"], told["error"]) == (
        False,
        "failed",
        None,
        "Feishu refuses it",
    )


def test_an_assistant_that_stopped_says_nothing_of_its_last_run(home: Path) -> None:
    store.keep(home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"), SECRET, FEISHU)
    store.tell(home, 7, "stopped", paired=True, error="old")

    told = status_of(home, {})

    assert (told["state"], told["error"]) == (None, None)


def test_a_forgotten_app_is_said_to_be_gone(home: Path) -> None:
    store.keep(home, store.Settings(domain="feishu", app_id="cli_a1b2c3d4"), SECRET, FEISHU)

    assert forgotten(home) == {"forgotten": True}
    assert forgotten(home) == {"forgotten": False}


def test_an_app_that_another_program_gives_is_read() -> None:
    assert read_given(json.dumps(APP)) == APP


@pytest.mark.parametrize("text", ["not json", "[1]", ""])
def test_what_is_no_object_is_no_app(text: str) -> None:
    with pytest.raises(FeishuError, match="no object of JSON"):
        read_given(text)


def test_an_answer_is_one_line_of_json() -> None:
    assert said({"stored": {"app_id": "cli_1"}, "说": "是"}) == (
        '{"stored": {"app_id": "cli_1"}, "说": "是"}'
    )


async def test_a_person_sets_the_app_up_at_a_terminal(home: Path) -> None:
    answers = iter(["", "cli_a1b2c3d4"])
    shown: list[str] = []

    code = await at_a_terminal(
        home,
        {},
        lambda question: next(answers),
        lambda question: SECRET,
        shown.append,
        transport=platform(),
    )

    assert code == 0
    assert store.secret(home, FEISHU) == SECRET
    assert shown[0].startswith("The assistant answers you in Feishu")
    assert f"lies as it is in {home / store.CREDENTIALS}" in shown[1]
    assert shown[1].endswith(NEXT)


async def test_a_person_is_told_where_the_keyring_took_the_secret(home: Path) -> None:
    Memory().install()
    answers = iter(["lark", "cli_a1b2c3d4"])
    shown: list[str] = []

    await at_a_terminal(
        home,
        {},
        lambda question: next(answers),
        lambda question: SECRET,
        shown.append,
        transport=platform(),
    )

    assert "The App Secret is in the keyring of the system." in shown[1]
    settings = store.settings(home)
    assert settings is not None
    assert settings.domain == "lark"


async def test_an_app_that_is_not_kept_is_said_to_be_so_at_a_terminal(home: Path) -> None:
    answers = iter(["feishu", "cli_a1b2c3d4"])
    shown: list[str] = []

    code = await at_a_terminal(
        home,
        {},
        lambda question: next(answers),
        lambda question: SECRET,
        shown.append,
        transport=platform(code=10014),
    )

    assert code == 2
    assert shown[-1].startswith("The app is not kept: ")


async def test_an_app_of_lark_that_is_said_to_be_in_feishu_is_not_kept_at_a_terminal(
    home: Path,
) -> None:
    answers = iter(["", "cli_a1b2c3d4"])
    shown: list[str] = []

    code = await at_a_terminal(
        home,
        {},
        lambda question: next(answers),
        lambda question: SECRET,
        shown.append,
        transport=platform(connection=elsewhere),
    )

    assert code == 2
    assert shown[-1] == (
        "The app is not kept: Feishu does not know this app, which is one of Lark (Incorrect "
        "domain name, code 1000040351). Set it up as an app of Lark."
    )
    assert store.settings(home) is None
