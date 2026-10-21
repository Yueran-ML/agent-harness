"""``columbina doctor``: whether a session could start, and what is in the way."""

import asyncio
import json
import platform
import re
import socket
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest
import yaml

import columbina
from columbina import shells
from columbina.apps import doctor
from columbina.apps.doctor import Finding, State, looked, shown
from columbina.apps.launcher import main
from columbina.builtin_tools.shell import NAME as SHELL
from columbina.kernel import Entry
from columbina.llm.credentials import CredentialsError, keep, origin_of
from support.model_service import Service, listing, serving

KEY = "sk-doctor-0123456789"
MODEL = "deepseek-flash"
"""The model that the profiles of Columbina ask for, where nothing names another."""
WINDOWS = sys.platform == "win32"
FAKE_DOCKER = Path(__file__).resolve().parents[1] / "support" / "fake_docker.py"
NONE = (
    "ok",
    "none: commands run on this machine as it is. --sandbox docker runs them in containers",
)
"""What is said of a sandbox that runs the commands on this machine."""


@pytest.fixture(autouse=True)
def wide(monkeypatch: pytest.MonkeyPatch) -> None:
    """A terminal wide enough that no finding goes on in a second line."""
    monkeypatch.setenv("COLUMNS", "400")


@pytest.fixture
def service(home: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Service]:
    """The service that the profiles of Columbina talk to, on this machine."""
    with serving(MODEL) as service:
        monkeypatch.setenv("DEEPSEEK_BASE_URL", service.address)
        yield service


def asked(capsys: pytest.CaptureFixture[str], *arguments: str) -> tuple[int, list[str]]:
    """Run the doctor, and return its exit code and the lines that it printed."""
    capsys.readouterr()
    code = main(["doctor", *arguments])
    return code, capsys.readouterr().out.splitlines()


def found(lines: list[str]) -> dict[str, tuple[str, str]]:
    """Return the findings among the lines: the state and the words, by what they are about."""
    findings: dict[str, tuple[str, str]] = {}
    for line in lines[2:-2]:
        state, about, said = line.split(maxsplit=2)
        findings[about] = (state, said)
    return findings


def patched(tmp_path: Path, *entries: dict[str, Any]) -> str:
    """Write a patch with these entries, and return its path."""
    path = tmp_path / "patch.yml"
    path.write_text(yaml.safe_dump({"patch": list(entries)}), encoding="utf-8")
    return str(path)


# What is looked at


def test_where_all_is_there_a_session_can_start(
    service: Service,
    work: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)

    code, lines = asked(capsys, "--cwd", str(work))

    assert code == 0
    python = platform.python_version()
    assert lines[:2] == [f"columbina {columbina.__version__}, Python {python}, {sys.platform}", ""]
    findings = found(lines)
    assert list(findings) == ["profile", "model", "service", "shell", "sandbox"]
    assert findings["profile"] == ("ok", "cli")
    assert findings["model"] == ("ok", f"{MODEL} of deepseek, at {service.address}")
    state, said = findings["service"]
    assert state == "ok"
    assert re.fullmatch(
        rf"answered in \d+\.\d s to the key in the variable DEEPSEEK_API_KEY, and lists {MODEL}",
        said,
    )
    assert findings["shell"] == ("ok", shells.find())
    assert findings["sandbox"] == NONE
    assert lines[-2:] == ["", "A session can start."]
    assert service.asked == [("/v1/models", f"Bearer {KEY}")]


def test_the_patches_of_the_profile_are_named_with_it(
    home: Path, service: Service, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    home.mkdir()
    (home / "config.yml").write_text("patch: []\n", encoding="utf-8")
    mine = patched(tmp_path)

    code, lines = asked(capsys, "--profile", "headless", "--patch", mine)

    assert found(lines)["profile"] == ("ok", f"headless, with {home / 'config.yml'}, {mine}")
    # No key is stored and none is in a variable.
    assert code == 1


def test_the_model_that_the_command_line_names_is_looked_for(
    service: Service, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
    service.answers(200, listing(MODEL, "deepseek-pro"))

    code, lines = asked(capsys, "--model", "deepseek-pro")

    findings = found(lines)
    assert findings["model"] == ("ok", f"deepseek-pro of deepseek, at {service.address}")
    assert findings["service"][1].endswith(", and lists deepseek-pro")
    assert code == 0


# The service


def test_without_a_key_the_service_is_not_asked(
    service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines = asked(capsys)

    assert code == 1
    assert found(lines)["service"] == (
        "fail",
        "there is no API key for deepseek: the environment variable DEEPSEEK_API_KEY is not "
        "set, and no key is stored. Enter one with 'columbina setup', or set the variable",
    )
    assert lines[-1] == "1 thing is in the way of a session."
    assert service.asked == []


def test_a_key_that_the_service_refuses_is_in_the_way_and_is_not_shown(
    service: Service, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
    service.answers(401, {"error": {"message": f"Authentication Fails, {KEY} is invalid"}})

    code, lines = asked(capsys)

    assert code == 1
    assert found(lines)["service"] == ("fail", "Authentication Fails, [key] is invalid")
    assert KEY not in "\n".join(lines)


def test_the_stored_key_is_named_where_it_is_the_one_that_was_sent(
    home: Path, service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    keep(home / "credentials.json", "deepseek", KEY, origin_of(service.address))

    code, lines = asked(capsys)

    assert code == 0
    assert found(lines)["service"][1].startswith("answered in ")
    assert " s to the key that is stored for deepseek, and lists " in found(lines)["service"][1]
    assert service.asked == [("/v1/models", f"Bearer {KEY}")]


def test_a_service_that_does_not_list_the_model_is_worth_a_look(
    service: Service, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
    service.answers(200, listing("deepseek-pro", "deepseek-other"))

    code, lines = asked(capsys)

    state, said = found(lines)["service"]
    assert state == "warn"
    assert said.endswith(
        f", and does not list {MODEL} among its 2 models. A session asks for it all the same, "
        "and the service may refuse it"
    )
    assert lines[-1] == "A session can start. 1 thing is worth a look."
    assert code == 0


def test_a_model_is_listed_with_the_tag_that_is_meant_without_one(
    service: Service, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
    service.answers(200, listing(f"{MODEL}:latest"))

    code, lines = asked(capsys)

    assert found(lines)["service"][0] == "ok"
    assert code == 0


def test_a_service_without_a_list_of_models_is_worth_a_look(
    service: Service, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)
    service.answers(404, b"404 page not found")

    code, lines = asked(capsys)

    assert found(lines)["service"] == (
        "warn",
        f"it has no list of models at {service.address}/models, so whether it takes the key "
        "shows only when a session asks it",
    )
    assert code == 0


def test_a_service_that_cannot_be_reached_is_in_the_way(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    with socket.socket() as taken:
        taken.bind(("127.0.0.1", 0))
        port = taken.getsockname()[1]
    # Nothing listens there now.
    monkeypatch.setenv("DEEPSEEK_BASE_URL", f"http://127.0.0.1:{port}/v1")
    monkeypatch.setenv("DEEPSEEK_API_KEY", KEY)

    code, lines = asked(capsys)

    state, said = found(lines)["service"]
    assert state == "fail"
    assert said.startswith("ConnectError")
    assert code == 1


# What the profile says


def test_a_profile_that_cannot_be_made_is_in_the_way_and_the_shell_is_still_looked_at(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines = asked(capsys, "--provider", "nosuch")

    assert code == 1
    findings = found(lines)
    assert list(findings) == ["profile", "shell"]
    state, said = findings["profile"]
    assert state == "fail"
    assert said.startswith("there is no service 'nosuch'. The services are: deepseek, ")
    assert findings["shell"] == ("ok", shells.find())


@pytest.mark.parametrize(
    ("model", "why"),
    [
        ({"provider": "deepseek"}, "model: Field required"),
        ({"provider": "deepseek", "model": "${COLUMBINA_TEST_UNSET}"}, "environment variable "
         "'COLUMBINA_TEST_UNSET' is not set"),
        ("deepseek-flash", "Input should be a valid dictionary or instance of CallConfig"),
    ],
    ids=["a field is missing", "a variable is not set", "it is no mapping"],
)  # fmt: skip
def test_a_frontend_that_names_no_model_that_can_be_read_is_in_the_way(
    service: Service,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    model: object,
    why: str,
) -> None:
    patch = patched(tmp_path, {"id": "frontend", "config": {"model": model}})

    code, lines = asked(capsys, "--patch", patch)

    assert found(lines)["model"] == ("fail", f"the frontend names no model that can be read: {why}")
    assert "service" not in found(lines)
    assert code == 1


def test_an_adapter_whose_config_cannot_be_read_is_in_the_way(
    service: Service, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    config = {"provider": "deepseek", "base_url": service.address, "connect_timeout_seconds": 0}
    patch = patched(tmp_path, {"id": "model", "config": config})

    code, lines = asked(capsys, "--patch", patch)

    assert found(lines)["model"] == (
        "fail",
        "the entry 'model' has a config that cannot be read: connect_timeout_seconds: Input "
        "should be greater than 0",
    )
    assert code == 1


@pytest.mark.parametrize(
    "patch",
    [
        {"id": "frontend", "config": {"model": {"provider": "elsewhere", "model": "tiny"}}},
        {"id": "model", "disabled": True},
    ],
    ids=["another provider", "the adapter is disabled"],
)
def test_a_model_that_no_adapter_of_a_service_is_for_is_not_asked_for(
    service: Service, tmp_path: Path, capsys: pytest.CaptureFixture[str], patch: dict[str, Any]
) -> None:
    code, lines = asked(capsys, "--patch", patched(tmp_path, patch))

    state, said = found(lines)["model"]
    assert state == "warn"
    assert said.endswith(
        ", which no entry that loads columbina.llm.adapters.openai_chat or "
        "columbina.llm.adapters.anthropic_messages is for, so no service is asked"
    )
    assert "service" not in found(lines)
    assert service.asked == []
    assert code == 0


# Where the commands run


def test_a_sandbox_of_another_plugin_is_not_looked_at(
    service: Service, capsys: pytest.CaptureFixture[str]
) -> None:
    code, lines = asked(capsys, "--sandbox", "podman")

    assert found(lines)["sandbox"] == (
        "warn",
        "podman: a provider of another plugin, not looked at",
    )
    # The key is missing, and that is in the way.
    assert code == 1


def test_a_profile_without_a_sandbox_holds_no_command(
    service: Service, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    patch = patched(tmp_path, {"id": "sandbox", "disabled": True})

    _, lines = asked(capsys, "--patch", patch)

    assert found(lines)["sandbox"] == (
        "warn",
        "no entry loads columbina.sandbox: commands run on this machine, and nothing holds them",
    )


def test_a_sandbox_whose_config_cannot_be_read_is_in_the_way(
    service: Service, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    config = {"provider": "docker", "docker": {"network": "host"}}
    patch = patched(tmp_path, {"id": "sandbox", "config": config})

    _, lines = asked(capsys, "--patch", patch)

    assert found(lines)["sandbox"] == (
        "fail",
        "the entry 'sandbox' has a config that cannot be read: docker.network: Input should be "
        "'none' or 'bridge'",
    )


def test_a_machine_without_its_shell_runs_no_command(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(shells, "find", lambda: None)

    assert doctor.shell() == Finding(
        "fail",
        "shell",
        f"{SHELL} is not installed: no command can run on this machine, and no hook at all",
    )


# The engine of the containers, with a stand-in for the docker program


@pytest.fixture
async def engine(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """The stand-in for the docker program, in the place of the program that a config names.

    It answers as ``told.json`` in its home says. It is started in the event loop of the
    test, and no real engine is asked.
    """
    home = tmp_path / "engine"
    home.mkdir()
    program = tmp_path / "bin" / ("docker.exe" if WINDOWS else "docker")
    program.parent.mkdir()
    program.write_bytes(b"")
    program.chmod(0o755)
    monkeypatch.setenv("COLUMBINA_FAKE_DOCKER", str(home))
    loop = asyncio.get_running_loop()
    really_start: Any = loop.subprocess_exec

    async def start(factory: Any, name: str, *arguments: str, **options: Any) -> Any:
        assert name == str(program), f"{name} is no stand-in"
        return await really_start(factory, sys.executable, str(FAKE_DOCKER), *arguments, **options)

    monkeypatch.setattr(loop, "subprocess_exec", start)
    return program


def entries(address: str, sandbox: dict[str, Any] | None) -> list[Entry]:
    """The entries of a session that asks a service on this machine for no key."""
    model = {"provider": "here", "model": "tiny"}
    adapter = {"provider": "here", "base_url": address, "api_key_required": False}
    return [
        Entry(id="frontend", plugin="columbina.apps.headless", config={"model": model}),
        Entry(id="model", plugin="columbina.llm.adapters.openai_chat", config=adapter),
        Entry(id="sandbox", plugin="columbina.sandbox", config=sandbox),
    ]


async def test_containers_that_could_run_commands_are_named_by_their_engine_and_image(
    engine: Path, service: Service, work: Path
) -> None:
    service.answers(200, listing("tiny"))
    docker = {"program": str(engine), "image": "debian:bookworm-slim"}

    findings = await looked(
        entries(service.address, {"provider": "docker", "docker": docker}), {}, work
    )

    assert findings[0] == Finding("ok", "model", f"tiny of here, at {service.address}")
    assert re.fullmatch(r"answered in \d+\.\d s without a key, and lists tiny", findings[1].said)
    assert findings[2].about == "shell"
    assert findings[3] == Finding(
        "ok",
        "sandbox",
        "docker: Docker 29.8.2 on Docker Desktop, with the image debian:bookworm-slim",
    )
    assert service.asked == [("/v1/models", None)]


async def test_containers_that_could_not_run_a_command_are_in_the_way(
    engine: Path, service: Service, work: Path, tmp_path: Path
) -> None:
    (tmp_path / "engine" / "told.json").write_text(json.dumps({"image": "missing"}), "utf-8")
    sandbox = {"provider": "docker", "docker": {"program": str(engine)}}

    findings = await looked(entries(service.address, sandbox), {}, work)

    assert findings[-1] == Finding(
        "fail",
        "sandbox",
        "docker: the image python:3.12 is not on this machine, and no image is fetched for a "
        "command. Fetch it with: docker pull python:3.12",
    )


# Claude


def claude(base_url: str) -> list[Entry]:
    """The entries of a session that asks Claude, at a place on this machine, for no key."""
    model = {"provider": "claude", "model": "claude-tiny"}
    adapter = {"provider": "claude", "base_url": base_url, "api_key_required": False}
    return [
        Entry(id="frontend", plugin="columbina.apps.headless", config={"model": model}),
        Entry(id="model", plugin="columbina.llm.adapters.anthropic_messages", config=adapter),
    ]


async def test_claude_is_asked_for_its_models_in_the_protocol_of_anthropic(
    service: Service, work: Path
) -> None:
    service.answers(200, listing("claude-tiny"))
    base_url = service.address.removesuffix("/v1")

    findings = await looked(claude(base_url), {}, work)

    assert findings[0] == Finding("ok", "model", f"claude-tiny of claude, at {base_url}")
    assert re.fullmatch(
        r"answered in \d+\.\d s without a key, and lists claude-tiny", findings[1].said
    )
    assert service.asked == [("/v1/models?limit=1000", None)]
    assert service.heard[0]["anthropic-version"] == "2023-06-01"


async def test_claude_without_a_list_of_models_is_worth_a_look(
    service: Service, work: Path
) -> None:
    service.answers(404, {"type": "error", "error": {"type": "not_found_error", "message": "no"}})
    base_url = service.address.removesuffix("/v1")

    findings = await looked(claude(base_url), {}, work)

    assert findings[1] == Finding(
        "warn",
        "service",
        f"it has no list of models at {base_url}/v1/models, so whether it takes the key shows "
        "only when a session asks it",
    )


# Which key was sent


@pytest.mark.parametrize(
    ("stored", "said"),
    [
        (None, "without a key"),
        ([("other", None)], "without a key"),
        (CredentialsError("changed"), "to a stored key"),
    ],
    ids=["no file", "for another service", "changed meanwhile"],
)
async def test_a_service_that_asks_for_no_key_is_asked_with_what_there_is(
    service: Service,
    work: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    stored: list[tuple[str, str | None]] | Exception | None,
    said: str,
) -> None:
    service.answers(200, listing("tiny"))
    file = tmp_path / "credentials.json"
    if isinstance(stored, list):
        for name, place in stored:
            keep(file, name, "sk-stored-0123", place or origin_of(service.address))
    elif isinstance(stored, Exception):

        def broken(path: Path) -> dict[str, str]:
            raise stored

        monkeypatch.setattr(doctor, "stored", broken)
    # A sandbox whose entry says nothing has the config that its plugin has by default.
    session = entries(service.address, None)
    adapter = {**session[1].config, "api_key_env": "COLUMBINA_TEST_KEY", "api_key_file": str(file)}
    session[1] = session[1].model_copy(update={"config": adapter})

    findings = await looked(session, {}, work)

    assert re.fullmatch(rf"answered in \d+\.\d s {said}, and lists tiny", findings[1].said)
    assert findings[3] == Finding("ok", "sandbox", NONE[1])
    assert service.asked == [("/v1/models", None)]


async def test_a_variable_that_holds_only_blanks_holds_no_key(
    service: Service, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    service.answers(200, listing("tiny"))
    monkeypatch.setenv("COLUMBINA_TEST_KEY", "  ")
    session = entries(service.address, None)
    adapter = {**session[1].config, "api_key_env": "COLUMBINA_TEST_KEY"}
    session[1] = session[1].model_copy(update={"config": adapter})

    findings = await looked(session, {"COLUMBINA_TEST_KEY": "  "}, work)

    assert re.fullmatch(r"answered in \d+\.\d s without a key, and lists tiny", findings[1].said)
    assert service.asked == [("/v1/models", None)]


async def test_a_key_that_was_stored_for_another_place_is_in_the_way(
    service: Service, work: Path, tmp_path: Path
) -> None:
    file = tmp_path / "credentials.json"
    keep(file, "here", "sk-stored-0123", "https://elsewhere.test")
    session = entries(service.address, None)
    adapter = {**session[1].config, "api_key_file": str(file)}
    session[1] = session[1].model_copy(update={"config": adapter})

    findings = await looked(session, {}, work)

    assert findings[1].state == "fail"
    assert findings[1].said.startswith(
        "the key that is stored for here was stored for https://elsewhere.test, and the "
        "requests for here now go to "
    )
    assert service.asked == []


# The text


def test_a_long_finding_goes_on_under_its_own_words() -> None:
    address = "https://example.test/an/address/that/is/longer/than/a/line/of/the/terminal"
    long = Finding("fail", "service", "abc " * 30 + address)

    text = shown([long], width=60)

    lines = text.splitlines()[2:-2]
    assert lines[0].startswith("  fail  service  abc abc ")
    # The last column is left free: some terminals go on to the next line where it is not.
    assert all(len(line) <= 59 for line in lines[:-1])
    assert all(line.startswith(" " * 17) for line in lines[1:])
    # An address is not broken in two, where it is longer than a line.
    assert lines[-1] == " " * 17 + address


def test_a_narrow_terminal_still_gets_some_words_in_a_line() -> None:
    text = shown([Finding("ok", "shell", "one two three four five six")], width=10)

    assert text.splitlines()[2] == "  ok    shell    one two three four"


@pytest.mark.parametrize(
    ("states", "last"),
    [
        (["ok", "ok"], "A session can start."),
        (["ok", "warn"], "A session can start. 1 thing is worth a look."),
        (["warn", "warn"], "A session can start. 2 things are worth a look."),
        (["fail", "warn"], "1 thing is in the way of a session."),
        (["fail", "fail", "warn"], "2 things are in the way of a session."),
    ],
)
def test_the_last_line_counts_what_is_in_the_way_and_what_is_worth_a_look(
    states: list[State], last: str
) -> None:
    findings = [Finding(state, "x", "y") for state in states]

    assert shown(findings, width=80).splitlines()[-1] == last


@pytest.mark.parametrize("option", ["--session-id", "--permission", "--allow", "--dump-config"])
def test_the_doctor_takes_no_option_of_a_session(
    service: Service, capsys: pytest.CaptureFixture[str], option: str
) -> None:
    # Were the option taken, the doctor would look at the stand-ins and at nothing real.
    with pytest.raises(SystemExit) as stopped:
        main(["doctor", option, "x"])

    assert stopped.value.code == 2
    assert f"unrecognized arguments: {option} x" in capsys.readouterr().err
