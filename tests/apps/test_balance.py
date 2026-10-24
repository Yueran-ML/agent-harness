"""``columbina balance``: how much money the account of a service has left, as DeepSeek says."""

import json
import os
import socket
from collections.abc import Iterator
from pathlib import Path

import pytest

from columbina.apps import setup
from columbina.apps.launcher import main
from support.model_service import Service, serving

KEY = "sk-balance-0123456789"
BALANCE = {
    "is_available": True,
    "balance_infos": [
        {
            "currency": "CNY",
            "total_balance": "110.00",
            "granted_balance": "10.00",
            "topped_up_balance": "100.00",
        }
    ],
}


@pytest.fixture
def service(home: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Service]:
    """DeepSeek on this machine, with a key in its variable."""
    with serving() as service:
        service.answers(200, BALANCE)
        monkeypatch.setenv("DEEPSEEK_BASE_URL", service.address)
        monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
        yield service


async def balance(home: Path, name: str | None = None) -> dict[str, object]:
    return await setup.balance(home, dict(os.environ), name)


async def test_deepseek_says_what_its_account_has_left(home: Path, service: Service) -> None:
    answer = await balance(home)

    assert answer == {
        "service": "deepseek",
        "title": "DeepSeek",
        "supported": True,
        "available": True,
        "balances": [
            {"currency": "CNY", "total": "110.00", "granted": "10.00", "topped_up": "100.00"}
        ],
    }
    assert service.asked == [("/v1/user/balance", f"Bearer {KEY}")]


async def test_what_a_service_leaves_out_is_none_of_its_money(home: Path, service: Service) -> None:
    service.answers(
        200,
        {"is_available": False, "balance_infos": [{"currency": "USD", "total_balance": "0.00"}]},
    )

    answer = await balance(home)

    assert answer["available"] is False
    assert answer["balances"] == [
        {"currency": "USD", "total": "0.00", "granted": "0", "topped_up": "0"}
    ]


async def test_a_service_that_cannot_be_asked_with_its_key_is_said_to_be_one_and_is_not_asked(
    home: Path, service: Service
) -> None:
    assert await balance(home, "openai") == {
        "service": "openai",
        "title": "OpenAI",
        "supported": False,
    }
    assert service.asked == []


async def test_the_service_in_use_is_asked_where_none_is_named(
    home: Path, service: Service
) -> None:
    home.mkdir(parents=True, exist_ok=True)
    (home / setup.CHOICE).write_text("use: anthropic\n", encoding="utf-8")

    answer = await balance(home)

    assert (answer["service"], answer["supported"]) == ("anthropic", False)


async def test_a_service_that_is_not_there_is_refused(home: Path) -> None:
    with pytest.raises(setup.SetupError, match="there is no service named 'nowhere'"):
        await balance(home, "nowhere")


async def test_a_key_that_the_service_refuses_is_said_without_the_key(
    home: Path, service: Service
) -> None:
    service.answers(401, {"error": {"message": "Authentication Fails, Your api key is invalid"}})

    answer = await balance(home)

    assert answer["kind"] == "key"
    assert "Authentication Fails" in str(answer["error"])
    assert KEY not in json.dumps(answer)


async def test_what_cannot_be_read_as_a_balance_is_said_to_be_so(
    home: Path, service: Service
) -> None:
    service.answers(200, {"money": "plenty"})

    answer = await balance(home)

    assert answer["kind"] == "unknown"
    assert str(answer["error"]).startswith("the service sent a balance that cannot be read")


async def test_a_service_that_cannot_be_reached_is_said_to_be_so_with_a_word_of_the_proxy(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with socket.socket() as taken:
        taken.bind(("127.0.0.1", 0))
        port = taken.getsockname()[1]
    monkeypatch.setenv("DEEPSEEK_BASE_URL", f"http://127.0.0.1:{port}")
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)

    answer = await balance(home)

    assert answer["kind"] == "network"
    assert str(answer["error"]).endswith(
        "If the service is reached through a proxy, see that the proxy is on"
    )


# At the command line


def test_a_person_is_told_the_balance(
    home: Path, service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["balance"]) == 0

    assert (
        capsys.readouterr().out
        == "DeepSeek: 110.00 CNY, of which 100.00 were paid in and 10.00 were given.\n"
    )


def test_a_person_is_told_that_the_account_cannot_pay_or_holds_nothing(
    home: Path, service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    service.answers(200, {"is_available": False, "balance_infos": []})

    assert main(["balance"]) == 0

    assert (
        capsys.readouterr().out
        == "DeepSeek names no money in its account.\nThe account cannot pay for calls.\n"
    )


def test_a_person_is_told_of_a_service_that_cannot_be_asked(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["balance", "--service", "openai"]) == 0

    assert (
        capsys.readouterr().out
        == "OpenAI does not tell a program how much money its account has left.\n"
    )


def test_a_person_is_told_why_the_service_could_not_be_asked(
    home: Path, service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    service.answers(401, {"error": {"message": "Authentication Fails"}})

    assert main(["balance"]) == 2

    told = capsys.readouterr()
    assert told.out == ""
    assert told.err.startswith(
        "columbina: DeepSeek could not be asked how much money its account has left: "
    )


def test_a_program_is_answered_in_json(
    home: Path, service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["balance", "--json"]) == 0
    assert json.loads(capsys.readouterr().out)["balances"][0]["total"] == "110.00"

    service.answers(401, {"error": {"message": "Authentication Fails"}})
    assert main(["balance", "--json"]) == 2
    assert json.loads(capsys.readouterr().out)["kind"] == "key"


def test_a_service_that_is_not_there_is_said_in_either_way(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["balance", "--json", "--service", "nowhere"]) == 2
    assert json.loads(capsys.readouterr().out) == {"error": "there is no service named 'nowhere'"}

    assert main(["balance", "--service", "nowhere"]) == 2
    assert capsys.readouterr().err == "columbina: there is no service named 'nowhere'\n"
