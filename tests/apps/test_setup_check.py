"""``columbina setup --check``: a service is asked for its models with a key. Nothing is kept."""

import json
import os
import socket
from collections.abc import Iterator
from pathlib import Path

import pytest

from columbina.apps import setup
from columbina.apps.launcher import main
from columbina.llm.adapters import openai_chat
from columbina.llm.credentials import keep, origin_of, spellings, stored
from support.documents import one_document, said, written_by_a_program
from support.model_service import Service, serving
from support.vault import Memory

KEY = "sk-check-0123456789"
STORED = "sk-stored-9876543210"
IN_A_VARIABLE = "sk-variable-5555555555"
MODELS = ("deepseek-flash", "deepseek-pro")


@pytest.fixture
def service(home: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Service]:
    """DeepSeek, as the profiles of Columbina reach it, on this machine."""
    with serving(*MODELS) as service:
        monkeypatch.setenv("DEEPSEEK_BASE_URL", service.address)
        yield service


async def check(home: Path, **document: object) -> dict[str, object]:
    shown: list[str] = []
    return await setup.checked(home, dict(os.environ), said(**document), shown.append)


def nowhere(text: str, *keys: str) -> bool:
    """Whether no key is in a text, in any of the spellings in which a text repeats one."""
    return not any(spelling in text for key in keys for spelling in spellings(key))


async def test_a_key_that_is_checked_lists_the_models_and_is_kept_nowhere(
    home: Path, service: Service
) -> None:
    vault = Memory().install()

    answer = await check(home, service="deepseek", api_key=KEY)

    assert answer == {"models": list(MODELS)}
    assert service.asked == [("/v1/models", f"Bearer {KEY}")]
    assert vault.held == {}
    assert not (home / "credentials.json").exists()
    assert not (home / "provider.yml").exists()


async def test_claude_is_asked_for_its_models_in_the_protocol_of_anthropic(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with serving("claude-opus-5-5", "claude-haiku-5-5") as claude:
        monkeypatch.setenv("ANTHROPIC_BASE_URL", claude.address.removesuffix("/v1"))

        answer = await check(home, service="anthropic", api_key=KEY)

    assert answer == {"models": ["claude-opus-5-5", "claude-haiku-5-5"]}
    assert claude.asked == [("/v1/models?limit=1000", None)]
    assert claude.heard[0]["x-api-key"] == KEY
    assert claude.heard[0]["anthropic-version"] == "2023-06-01"


async def test_the_blanks_around_a_key_that_is_checked_are_not_sent(
    home: Path, service: Service
) -> None:
    await check(home, service="deepseek", api_key=f"  {KEY}\n")

    assert service.asked == [("/v1/models", f"Bearer {KEY}")]


async def test_without_a_key_the_one_that_a_session_would_send_is_checked(
    home: Path, service: Service, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(home / "credentials.json", "deepseek", STORED, origin_of(service.address))

    await check(home, service="deepseek")
    monkeypatch.setenv("DEEPSEEK_API_KEY", IN_A_VARIABLE)
    await check(home, service="deepseek")

    assert service.asked == [
        ("/v1/models", f"Bearer {STORED}"),
        ("/v1/models", f"Bearer {IN_A_VARIABLE}"),
    ]


async def test_the_key_that_is_given_goes_before_the_one_that_is_stored(
    home: Path, service: Service, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(home / "credentials.json", "deepseek", STORED, origin_of(service.address))
    monkeypatch.setenv("DEEPSEEK_API_KEY", IN_A_VARIABLE)

    await check(home, service="deepseek", api_key=KEY)

    assert service.asked == [("/v1/models", f"Bearer {KEY}")]
    assert stored(home / "credentials.json") == {"deepseek": origin_of(service.address)}


async def test_without_any_key_the_service_is_not_asked(home: Path, service: Service) -> None:
    answer = await check(home, service="deepseek")

    assert answer["kind"] == "key"
    assert "no API key" in str(answer["error"])
    assert service.asked == []


async def test_a_service_of_ones_own_is_checked_at_its_address_and_not_added(
    home: Path, service: Service
) -> None:
    answer = await check(home, service="mine", base_url=service.address)

    assert answer == {"models": list(MODELS)}
    # A service of one's own is sent a key only where one is found.
    assert service.asked == [("/v1/models", None)]
    assert not (home / "provider.yml").exists()


@pytest.mark.parametrize(
    ("status", "body", "kind"),
    [
        (401, {"error": {"message": f"Authentication Fails, {KEY} is invalid"}}, "key"),
        (403, {"error": {"message": "This key may not list models"}}, "forbidden"),
        (402, {"error": {"message": "Insufficient Balance"}}, "forbidden"),
        (404, b"404 page not found", "address"),
        (429, {"error": {"message": "Rate limit reached"}}, "busy"),
        (503, b"Service Unavailable", "busy"),
        (400, {"error": {"message": "Bad request"}}, "unknown"),
        (200, {"object": "nothing that lists models"}, "unknown"),
    ],
)
async def test_a_service_that_lists_no_models_is_said_to_have_done_so_and_why(
    home: Path, service: Service, status: int, body: object, kind: str
) -> None:
    service.answers(status, body)

    answer = await check(home, service="deepseek", api_key=KEY)

    assert answer["kind"] == kind
    assert answer["error"]
    assert nowhere(json.dumps(answer), KEY)


async def test_a_service_that_cannot_be_reached_is_said_to_be_so_with_a_word_on_proxies(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with socket.socket() as closed:
        closed.bind(("127.0.0.1", 0))
        port = closed.getsockname()[1]
    monkeypatch.setenv("DEEPSEEK_BASE_URL", f"http://127.0.0.1:{port}/v1")

    answer = await check(home, service="deepseek", api_key=KEY)

    assert answer["kind"] == "network"
    assert str(answer["error"]).endswith(
        ". If the service is reached through a proxy, see that the proxy is on"
    )
    assert ".." not in str(answer["error"])


async def test_a_service_that_does_not_answer_in_time_is_said_to_be_so(
    home: Path, service: Service, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(openai_chat, "_LIST_SECONDS", 0.2)
    service.waits(1.0)

    answer = await check(home, service="deepseek", api_key=KEY)

    assert answer["kind"] == "timeout"


async def test_a_document_without_a_service_is_refused(home: Path) -> None:
    with pytest.raises(setup.SetupError, match="names no service"):
        await check(home, api_key=KEY)


async def test_a_key_that_cannot_be_one_is_refused_without_being_repeated(home: Path) -> None:
    with pytest.raises(setup.SetupError) as refused:
        await check(home, service="deepseek", api_key="sk-two halves")

    assert "cannot be a key" in str(refused.value)
    assert "halves" not in str(refused.value)


# The command


def test_the_command_answers_with_the_models_in_one_document(
    service: Service, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(monkeypatch, said(service="deepseek", api_key=KEY))

    code = main(["setup", "--check"])

    captured = capsys.readouterr()
    assert code == 0
    assert one_document(captured.out) == {"models": list(MODELS)}
    assert nowhere(captured.out + captured.err, KEY)


def test_the_command_answers_a_refusal_as_an_error(
    service: Service, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    service.answers(401, {"error": {"message": f"Authentication Fails, {KEY} is invalid"}})
    written_by_a_program(monkeypatch, said(service="deepseek", api_key=KEY))

    code = main(["setup", "--check"])

    captured = capsys.readouterr()
    answer = one_document(captured.out)
    assert code == 2
    assert answer["kind"] == "key"
    assert nowhere(captured.out + captured.err, KEY)


def test_the_command_says_what_is_wrong_with_a_document(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(monkeypatch, b"[]")

    code = main(["setup", "--check"])

    assert code == 2
    assert one_document(capsys.readouterr().out) == {
        "error": "the document is not an object of JSON"
    }


def test_the_command_reads_no_terminal(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("sys.stdin.isatty", lambda: True)

    code = main(["setup", "--check"])

    assert code == 2
    assert (
        "'columbina setup --check' reads what another program writes"
        in one_document(capsys.readouterr().out)["error"]
    )


def test_a_check_is_one_thing_that_setup_does_and_no_more(home: Path) -> None:
    with pytest.raises(SystemExit) as refused:
        main(["setup", "--check", "--list"])

    assert refused.value.code == 2


def test_a_listing_is_still_no_error(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert main(["setup", "--list"]) == 0
    assert "services" in one_document(capsys.readouterr().out)
