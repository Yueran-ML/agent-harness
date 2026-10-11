"""The command line and the service of the model: ``--provider``, what was stored, and ``setup``."""

import contextlib
import getpass
import json
import socket
import socketserver
import sys
import threading
from collections.abc import Generator, Iterator
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from typing import Any

import httpx
import pytest
import yaml

from columbina.apps.launcher import main
from columbina.apps.terminal_text import STAND_IN
from columbina.kernel import interpolate
from columbina.llm import CallConfig, credentials
from columbina.llm.adapters.openai_chat import Config
from columbina.llm.credentials import keep, stored
from support.launch import profile
from support.vault import Memory

KEY = "sk-test-0123456789"
OTHER = "sk-other-9876543210"
KEYS = "${COLUMBINA_HOME}/credentials.json"
OPENAI = {
    "provider": "openai",
    "base_url": "https://api.openai.com/v1",
    "api_key_env": "OPENAI_API_KEY",
    "api_key_file": KEYS,
    "reasoning_style": "openai",
    "max_tokens_field": "max_completion_tokens",
}
DEEPSEEK = {
    "provider": "deepseek",
    "base_url": "https://api.deepseek.com",
    "api_key_env": "DEEPSEEK_API_KEY",
    "api_key_file": KEYS,
    "reasoning_style": "deepseek",
}
NO_ADAPTER = (
    "columbina: the profile has no entry 'model' that loads "
    "columbina.llm.adapters.openai_chat, or it is disabled, so there is nothing to point at "
    "the service {}\n"
)
NAMES = "deepseek, openai, openrouter, moonshot, zhipu, siliconflow, ollama, lmstudio"


class Dump:
    """Has the command line say what it would load, and what it says instead where it refuses."""

    def __init__(self, work: Path, capsys: pytest.CaptureFixture[str]) -> None:
        self.work = work
        self.capsys = capsys

    def entries(self, *arguments: str, command: str = "run") -> dict[str, dict[str, Any]]:
        task = ["Do it."] if command == "run" else []
        code = main([command, *task, "--cwd", str(self.work), *arguments, "--dump-config"])
        captured = self.capsys.readouterr()
        assert (code, captured.err) == (0, "")
        return {entry["id"]: entry for entry in yaml.safe_load(captured.out)}

    def model(self, *arguments: str) -> tuple[dict[str, Any], dict[str, Any]]:
        """What the adapter is told, and what the frontend is told of the model."""
        entries = self.entries(*arguments)
        return entries["model"]["config"], entries["frontend"]["config"]["model"]

    def refused(self, *arguments: str) -> str:
        code = main(["run", "Do it.", "--cwd", str(self.work), *arguments, "--dump-config"])
        captured = self.capsys.readouterr()
        assert (code, captured.out) == (2, "")
        return captured.err


@pytest.fixture
def dump(home: Path, work: Path, capsys: pytest.CaptureFixture[str]) -> Dump:
    return Dump(work, capsys)


def chosen(home: Path, text: str) -> None:
    """Write down what a person chose, the way ``columbina setup`` does."""
    home.mkdir(parents=True, exist_ok=True)
    (home / "provider.yml").write_text(text, encoding="utf-8")


OPENAI_CHOSEN = (
    "use: openai\nservices:\n  openai:\n    model: gpt-test\n    context_window: 400000\n"
)


# A service that the command line names


def test_without_a_word_about_a_service_the_profile_is_as_it_is(dump: Dump) -> None:
    adapter, frontend = dump.model()

    assert adapter == DEEPSEEK | {"base_url": "${DEEPSEEK_BASE_URL:-https://api.deepseek.com}"}
    assert frontend == {
        "provider": "deepseek",
        "model": "${COLUMBINA_MODEL:-deepseek-flash}",
        "context_window": "${COLUMBINA_CONTEXT_WINDOW:-1048576}",
    }


def test_a_service_is_named_on_the_command_line(dump: Dump) -> None:
    adapter, frontend = dump.model("--provider", "openai", "--model", "gpt-test")

    assert adapter == OPENAI
    # What a profile says of the size of the window is said of the model of another service.
    assert frontend == {"provider": "openai", "model": "gpt-test"}


def test_a_conversation_takes_the_service_like_a_task(dump: Dump) -> None:
    entries = dump.entries("--provider", "openai", "--model", "gpt-test", command="chat")

    assert entries["frontend"]["plugin"] == "columbina.apps.cli"
    assert entries["model"]["config"] == OPENAI
    assert entries["frontend"]["config"]["model"] == {"provider": "openai", "model": "gpt-test"}


def test_only_the_entries_of_the_model_are_changed(dump: Dump) -> None:
    before = dump.entries()
    after = dump.entries("--provider", "ollama", "--model", "qwen3:14b")

    assert list(after) == list(before)
    assert {name for name in before if after[name] != before[name]} == {"model", "frontend"}
    assert after["model"]["plugin"] == before["model"]["plugin"]
    assert after["model"]["config"] == {
        "provider": "ollama",
        "base_url": "http://localhost:11434/v1",
        "api_key_file": KEYS,
        "api_key_required": False,
    }


def test_naming_the_service_of_the_profiles_changes_nothing_that_is_loaded(
    dump: Dump, home: Path
) -> None:
    env = {"COLUMBINA_HOME": str(home)}

    by_itself = dump.model()
    named = dump.model("--provider", "deepseek")

    assert named[0] == DEEPSEEK
    assert named[1] == {
        "provider": "deepseek",
        "model": "deepseek-flash",
        "context_window": 1048576,
    }
    assert Config.model_validate(interpolate(named[0], env)) == Config.model_validate(
        interpolate(by_itself[0], env)
    )
    assert CallConfig.model_validate(interpolate(named[1], env)) == CallConfig.model_validate(
        interpolate(by_itself[1], env)
    )


def test_a_service_without_a_model_of_its_own_needs_one_named(
    dump: Dump, monkeypatch: pytest.MonkeyPatch
) -> None:
    refused = dump.refused("--provider", "openrouter")
    by_the_option = dump.model("--provider", "openrouter", "--model", "some/model")
    monkeypatch.setenv("COLUMBINA_MODEL", "another/model")
    by_the_variable = dump.model("--provider", "openrouter")

    assert refused == (
        "columbina: no model is named for the service openrouter, and it has none that is used "
        "by default. Name one with --model, or choose one with 'columbina setup'\n"
    )
    assert by_the_option[1] == {"provider": "openrouter", "model": "some/model"}
    # The variable is named, the way a profile names it, and not what it holds.
    assert by_the_variable[1] == {"provider": "openrouter", "model": "${COLUMBINA_MODEL}"}


def test_the_variable_names_a_service_and_the_option_goes_before_it(
    dump: Dump, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_PROVIDER", "moonshot")

    by_the_variable = dump.model("--model", "kimi-test")
    by_the_option = dump.model("--provider", "zhipu", "--model", "glm-test")
    monkeypatch.setenv("COLUMBINA_PROVIDER", "")
    by_nothing = dump.model()

    assert by_the_variable[0]["provider"] == by_the_variable[1]["provider"] == "moonshot"
    assert by_the_option[0]["provider"] == by_the_option[1]["provider"] == "zhipu"
    assert by_nothing[0]["provider"] == by_nothing[1]["provider"] == "deepseek"
    assert by_nothing[1]["model"] == "${COLUMBINA_MODEL:-deepseek-flash}"


def test_what_is_known_of_the_size_of_a_window_is_known_of_one_model(
    dump: Dump, monkeypatch: pytest.MonkeyPatch
) -> None:
    its_own = dump.model("--provider", "deepseek", "--model", "deepseek-flash")
    another = dump.model("--provider", "deepseek", "--model", "deepseek-v4-pro")
    monkeypatch.setenv("COLUMBINA_MODEL", "deepseek-flash")
    its_own_by_the_variable = dump.model("--provider", "deepseek")
    option_before_variable = dump.model("--provider", "deepseek", "--model", "deepseek-v4-pro")
    monkeypatch.setenv("COLUMBINA_MODEL", "deepseek-v4-pro")
    another_by_the_variable = dump.model("--provider", "deepseek")

    assert its_own[1] == {
        "provider": "deepseek",
        "model": "deepseek-flash",
        "context_window": 1048576,
    }
    assert another[1] == {"provider": "deepseek", "model": "deepseek-v4-pro"}
    assert its_own_by_the_variable[1] == {
        "provider": "deepseek",
        "model": "${COLUMBINA_MODEL}",
        "context_window": 1048576,
    }
    assert option_before_variable[1] == another[1]
    assert another_by_the_variable[1] == {"provider": "deepseek", "model": "${COLUMBINA_MODEL}"}


def test_the_variable_of_the_size_of_the_window_goes_before_what_is_known(
    dump: Dump, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("COLUMBINA_CONTEXT_WINDOW", "5000")

    known = dump.model("--provider", "deepseek")
    not_known = dump.model("--provider", "openai", "--model", "gpt-test")
    monkeypatch.setenv("COLUMBINA_CONTEXT_WINDOW", "")
    empty = dump.model("--provider", "openai", "--model", "gpt-test")

    assert known[1]["context_window"] == "${COLUMBINA_CONTEXT_WINDOW}"
    assert not_known[1]["context_window"] == "${COLUMBINA_CONTEXT_WINDOW}"
    assert "context_window" not in empty[1]


def test_the_variable_of_a_service_that_names_another_address_goes_on_counting(
    dump: Dump, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("DEEPSEEK_BASE_URL", "https://mirror.example.test/v1")

    assert dump.model("--provider", "deepseek")[0]["base_url"] == "https://mirror.example.test/v1"
    assert dump.model("--provider", "openai", "--model", "x")[0]["base_url"] == OPENAI["base_url"]


def test_a_dollar_sign_in_the_name_of_a_model_stands_for_itself(dump: Dump, home: Path) -> None:
    name = "a${HOME}b$$c"

    by_itself = dump.model("--model", name)
    with_a_service = dump.model("--provider", "moonshot", "--model", name)
    chosen(home, "use: openai\nservices:\n  openai:\n    model: 'stored${HOME}'\n")
    as_stored = dump.model()

    assert by_itself[1]["model"] == with_a_service[1]["model"] == "a$${HOME}b$$$$c"
    assert interpolate(by_itself[1]["model"], {}) == name
    assert interpolate(as_stored[1]["model"], {}) == "stored${HOME}"


def test_a_service_that_is_not_there_is_refused_with_the_ones_that_are(
    dump: Dump, home: Path
) -> None:
    named = dump.refused("--provider", "nowhere")
    chosen(home, "use: gone\nservices:\n  mine:\n    base_url: http://localhost:8000/v1\n")
    stored_one = dump.refused()

    assert named == (
        f"columbina: there is no service 'nowhere'. The services are: {NAMES}. Another one is "
        "added with 'columbina setup'\n"
    )
    assert stored_one == (
        f"columbina: there is no service 'gone'. The services are: {NAMES}, mine. Another one "
        "is added with 'columbina setup'\n"
    )


def test_the_help_says_which_services_there_are(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as of_a_task:
        main(["run", "--help"])
    task = " ".join(capsys.readouterr().out.split())
    with pytest.raises(SystemExit) as of_the_program:
        main(["--help"])
    program = " ".join(capsys.readouterr().out.split())

    assert (of_a_task.value.code, of_the_program.value.code) == (0, 0)
    assert (
        f"--provider NAME the service that the model comes from: {NAMES}, or one that was "
        "added with 'columbina setup'"
    ) in task
    assert "setup choose the service that the model comes from, and store the API key for it" in (
        program
    )


# What was stored


def test_what_was_stored_is_what_a_run_talks_to(dump: Dump, home: Path) -> None:
    chosen(home, OPENAI_CHOSEN)

    adapter, frontend = dump.model()

    assert adapter == OPENAI
    assert frontend == {"provider": "openai", "model": "gpt-test", "context_window": 400000}


def test_a_service_of_ones_own_is_talked_to_at_its_address(dump: Dump, home: Path) -> None:
    chosen(
        home,
        "use: mine\nservices:\n  mine:\n    base_url: http://10.0.0.5:8000/v1\n    model: tiny\n",
    )

    adapter, frontend = dump.model()

    assert adapter == {
        "provider": "mine",
        "base_url": "http://10.0.0.5:8000/v1",
        "api_key_env": "COLUMBINA_API_KEY",
        "api_key_file": KEYS,
        "api_key_required": False,
    }
    assert frontend == {"provider": "mine", "model": "tiny"}


def test_the_command_line_and_the_variables_go_before_what_was_stored(
    dump: Dump, home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    chosen(home, OPENAI_CHOSEN)

    another_service = dump.model("--provider", "deepseek")
    another_model = dump.model("--model", "gpt-other")
    monkeypatch.setenv("COLUMBINA_MODEL", "gpt-of-the-variable")
    model_of_the_variable = dump.model()
    monkeypatch.setenv("COLUMBINA_PROVIDER", "moonshot")
    service_of_the_variable = dump.model()

    assert another_service[0] == DEEPSEEK
    assert another_service[1] == {
        "provider": "deepseek",
        "model": "deepseek-flash",
        "context_window": 1048576,
    }
    assert another_model == (OPENAI, {"provider": "openai", "model": "gpt-other"})
    assert model_of_the_variable == (OPENAI, {"provider": "openai", "model": "${COLUMBINA_MODEL}"})
    assert service_of_the_variable[0]["provider"] == "moonshot"
    assert service_of_the_variable[1] == {"provider": "moonshot", "model": "${COLUMBINA_MODEL}"}


def test_a_service_that_is_named_has_the_model_that_was_stored_for_it(
    dump: Dump, home: Path
) -> None:
    chosen(home, OPENAI_CHOSEN.replace("use: openai", "use: deepseek"))

    in_use = dump.model()
    named = dump.model("--provider", "openai")

    assert in_use[0] == DEEPSEEK
    assert named == (OPENAI, {"provider": "openai", "model": "gpt-test", "context_window": 400000})


def test_a_patch_goes_before_what_was_stored_and_the_command_line_before_a_patch(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    chosen(home, OPENAI_CHOSEN)
    patched = {"provider": "patched", "base_url": "http://localhost:9/v1"}
    patch = tmp_path / "patch.yml"
    patch.write_text(
        yaml.safe_dump(
            [
                {"id": "model", "config": patched},
                {"id": "frontend", "config": {"model": {"provider": "patched", "model": "m"}}},
            ]
        ),
        encoding="utf-8",
    )

    by_the_patch = dump.model("--patch", str(patch))
    by_the_option = dump.model("--patch", str(patch), "--provider", "deepseek")
    (home / "config.yml").write_text(patch.read_text(encoding="utf-8"), encoding="utf-8")
    by_the_patch_of_the_user = dump.model()

    assert by_the_patch == (patched, {"provider": "patched", "model": "m"})
    assert by_the_patch_of_the_user == by_the_patch
    assert by_the_option[0] == DEEPSEEK
    assert by_the_option[1] == {
        "provider": "deepseek",
        "model": "deepseek-flash",
        "context_window": 1048576,
    }


def test_a_profile_that_talks_to_its_model_in_another_way_is_left_as_it_is(
    home: Path, work: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen(home, OPENAI_CHOSEN)
    scripted = profile(tmp_path, {"text": "Hello."})

    code = main(["run", "Hi.", "--profile", scripted, "--cwd", str(work), "--no-json"])

    captured = capsys.readouterr()
    assert (code, captured.out, captured.err) == (0, "Hello.\n", "")


def test_a_service_cannot_be_named_for_a_profile_without_the_adapter(
    home: Path,
    work: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scripted = profile(tmp_path, {"text": "Hello."})
    run = ["run", "Hi.", "--profile", scripted, "--cwd", str(work)]

    by_the_option = main([*run, "--provider", "openai", "--model", "gpt-test"])
    said = capsys.readouterr()
    monkeypatch.setenv("COLUMBINA_PROVIDER", "moonshot")
    by_the_variable = main([*run, "--model", "kimi-test"])

    assert (by_the_option, said.out, said.err) == (2, "", NO_ADAPTER.format("openai"))
    assert (by_the_variable, capsys.readouterr().err) == (2, NO_ADAPTER.format("moonshot"))


def test_an_adapter_that_is_switched_off_is_no_adapter_to_point(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: model\n  disabled: true\n", encoding="utf-8")
    (home / "config.yml").parent.mkdir(parents=True, exist_ok=True)
    (home / "config.yml").write_text(patch.read_text(encoding="utf-8"), encoding="utf-8")

    refused = dump.refused("--provider", "openai", "--model", "gpt-test")
    chosen(home, OPENAI_CHOSEN)
    stored_only = dump.entries()

    assert refused == NO_ADAPTER.format("openai")
    # What was stored was put into the profile before the patch switched its adapter off.
    assert stored_only["model"] == {
        "id": "model",
        "plugin": "columbina.llm.adapters.openai_chat",
        "config": OPENAI,
        "disabled": True,
    }


def test_a_profile_whose_adapter_is_switched_off_is_not_pointed_at_what_was_stored(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    chosen(home, OPENAI_CHOSEN)
    off = tmp_path / "off.yml"
    off.write_text(
        yaml.safe_dump({"extends": "headless", "patch": [{"id": "model", "disabled": True}]}),
        encoding="utf-8",
    )

    entries = dump.entries("--profile", str(off))

    assert entries["model"]["config"]["provider"] == "deepseek"
    assert entries["frontend"]["config"]["model"]["provider"] == "deepseek"


def test_what_was_chosen_and_cannot_be_read_stops_a_run_and_nothing_else(
    dump: Dump, home: Path, work: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen(home, "use: [")

    refused = dump.refused()
    # Looking at the sessions that are stored talks to no model.
    sessions = main(["sessions", "list", "--cwd", str(work)])

    assert refused == (
        f"columbina: {home / 'provider.yml'} is not YAML. Run 'columbina setup' again, or "
        "delete the file\n"
    )
    assert (sessions, capsys.readouterr().err) == (0, "")


def test_what_the_frontend_says_of_the_model_has_to_be_a_mapping(
    dump: Dump, tmp_path: Path
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: frontend\n  config:\n    model: just-a-name\n", encoding="utf-8")
    not_a_mapping = "columbina: the entry 'frontend' has a 'model' that is not a mapping\n"

    assert dump.refused("--patch", str(patch), "--model", "x") == not_a_mapping
    assert dump.refused("--patch", str(patch), "--provider", "openai", "--model", "x") == (
        not_a_mapping
    )


def test_a_frontend_that_says_nothing_of_the_model_is_told_what_the_command_line_says(
    dump: Dump, tmp_path: Path
) -> None:
    patch = tmp_path / "patch.yml"
    patch.write_text("- id: frontend\n  config: {}\n", encoding="utf-8")

    assert dump.model("--patch", str(patch), "--model", "x")[1] == {"model": "x"}
    assert dump.model("--patch", str(patch), "--provider", "openai", "--model", "x")[1] == {
        "provider": "openai",
        "model": "x",
    }


# The command that asks


class Terminal:
    """A person at a terminal: answers what is asked, and keeps the questions."""

    def __init__(
        self, monkeypatch: pytest.MonkeyPatch, *answers: str | type[BaseException], key: str = ""
    ) -> None:
        self.answers = list(answers)
        self.key = key
        self.asked: list[str] = []
        self.asked_unseen: list[str] = []
        monkeypatch.setattr(sys.stdin, "isatty", lambda: True)
        monkeypatch.setattr(sys.stdout, "isatty", lambda: True)
        monkeypatch.setattr("builtins.input", self.answer)
        monkeypatch.setattr(getpass, "getpass", self.answer_unseen)

    def answer(self, question: str = "") -> str:
        self.asked.append(question)
        answer = self.answers.pop(0) if self.answers else EOFError
        if isinstance(answer, str):
            return answer
        raise answer

    def answer_unseen(self, question: str = "") -> str:
        self.asked_unseen.append(question)
        return self.key


def test_setup_asks_at_a_terminal_and_stores_the_answers(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    terminal = Terminal(monkeypatch, "openai", "gpt-test", "400000", key=KEY)

    code = main(["setup"])

    captured = capsys.readouterr()
    assert (code, captured.err) == (0, "")
    assert terminal.asked == [
        "Service, by its number or its name [deepseek]: ",
        "Model: ",
        "How many tokens the model takes, if you know (Enter if not): ",
    ]
    assert terminal.asked_unseen == ["The API key of openai, or Enter to store none: "]
    assert captured.out.startswith(
        "The model can come from one of these services.\n\n   1  deepseek"
    )
    assert captured.out.endswith(
        "\nStored. Columbina talks to openai now, with the model gpt-test.\n"
        f"What was chosen is in {home / 'provider.yml'}.\n"
        "No keyring of this system took the key: the keyring that is set up here keeps nothing. "
        f"It is in {home / 'credentials.json'}, as it is and not encrypted. Keep that file to "
        "yourself.\n"
    )
    assert KEY not in captured.out
    assert yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8")) == {
        "use": "openai",
        "services": {"openai": {"model": "gpt-test", "context_window": 400000}},
    }
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}
    assert KEY.encode() in (home / "credentials.json").read_bytes()


def test_setup_says_which_keyring_took_the_key(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    vault = Memory().install()
    Terminal(monkeypatch, "openai", "gpt-test", "400000", key=KEY)

    code = main(["setup"])

    captured = capsys.readouterr()
    assert (code, captured.err) == (0, "")
    assert captured.out.endswith(
        f"What was chosen is in {home / 'provider.yml'}.\n"
        "The key is in the keyring of this system (vault Memory). No file of Columbina holds "
        "it.\n"
    )
    assert KEY not in captured.out
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}
    assert KEY.encode() not in (home / "credentials.json").read_bytes()
    assert [provider for _, provider in vault.held] == ["openai"]


def test_what_setup_stored_is_what_the_next_run_loads(
    dump: Dump, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    Terminal(monkeypatch, "openai", "gpt-test", "400000", key=KEY)
    assert main(["setup"]) == 0
    capsys.readouterr()

    assert dump.model() == (
        OPENAI,
        {"provider": "openai", "model": "gpt-test", "context_window": 400000},
    )


@pytest.mark.parametrize("terminals", [(False, False), (True, False), (False, True)])
def test_setup_asks_nothing_where_there_is_no_terminal(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    terminals: tuple[bool, bool],
) -> None:
    terminal = Terminal(monkeypatch, "openai", "gpt-test", "", key=KEY)
    monkeypatch.setattr(sys.stdin, "isatty", lambda: terminals[0])
    monkeypatch.setattr(sys.stdout, "isatty", lambda: terminals[1])

    code = main(["setup"])

    captured = capsys.readouterr()
    assert (code, captured.out) == (2, "")
    assert captured.err == (
        "columbina: 'columbina setup' asks its questions at a terminal, and this is none. A "
        "key can be given in the environment variable of its service instead\n"
    )
    assert terminal.asked == terminal.asked_unseen == []
    assert not home.exists()


@pytest.mark.parametrize("stopped", [EOFError, KeyboardInterrupt])
def test_setup_that_is_left_in_the_middle_stores_nothing_and_says_so(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    stopped: type[BaseException],
) -> None:
    Terminal(monkeypatch, "openai", "gpt-test", stopped, key=KEY)

    code = main(["setup"])

    captured = capsys.readouterr()
    assert code == 130
    # The line of the question that was left open is ended first.
    assert captured.out.endswith("protocol.\n\n\n")
    assert captured.err == "columbina: nothing was stored\n"
    assert not home.exists()


def test_what_setup_shows_and_asks_is_fit_for_a_terminal(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    escape = chr(0x1B)
    said = {"base_url": "http://localhost:8000/v1", "model": f"ti{escape}[2Jny"}
    chosen(home, yaml.safe_dump({"use": "mine", "services": {"mine": said}}))
    terminal = Terminal(monkeypatch, "", "tiny", "")

    code = main(["setup"])

    captured = capsys.readouterr()
    assert code == 0
    assert f"In use: mine, with the model ti{STAND_IN}[2Jny." in captured.out
    assert terminal.asked[1] == f"Model [ti{STAND_IN}[2Jny]: "
    assert escape not in captured.out + "".join(terminal.asked + terminal.asked_unseen)


def test_setup_says_what_is_wrong_with_what_was_stored(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    chosen(home, "use: [")
    terminal = Terminal(monkeypatch, "", "")

    code = main(["setup"])

    captured = capsys.readouterr()
    assert (code, captured.out) == (2, "")
    assert captured.err == (
        f"columbina: {home / 'provider.yml'} is not YAML. Run 'columbina setup' again, or "
        "delete the file\n"
    )
    assert terminal.asked == []


def test_what_is_stored_for_a_service_is_deleted_from_the_command_line(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen(home, OPENAI_CHOSEN)
    keep(home / "credentials.json", "openai", KEY, "https://api.openai.com")

    first = main(["setup", "--forget", "openai"])
    told = capsys.readouterr()
    again = main(["setup", "--forget", "openai"])
    refused = capsys.readouterr()

    assert (first, told.err) == (0, "")
    assert told.out == (
        "The key of openai is deleted.\n"
        "What was said of openai is deleted.\n"
        "openai is no longer in use. The profile says again where the model comes from.\n"
    )
    assert (again, refused.out) == (2, "")
    assert refused.err == "columbina: nothing is stored for 'openai'\n"
    assert stored(home / "credentials.json") == {}


# From the stored key to the service


class _Service(BaseHTTPRequestHandler):
    """Stands in for a service: keeps each request and answers it with one streamed word."""

    requests: list[tuple[str, str | None, dict[str, Any]]]

    def do_POST(self) -> None:
        length = int(self.headers["Content-Length"])
        body = json.loads(self.rfile.read(length))
        self.requests.append((self.path, self.headers["Authorization"], body))
        events: tuple[dict[str, Any], ...] = (
            {"choices": [{"index": 0, "delta": {"content": "Hello."}, "finish_reason": None}]},
            {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
        )
        reply = "".join(f"data: {json.dumps(event)}\n\n" for event in events) + "data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        self.wfile.write(reply.encode())

    def log_message(self, format: str, *args: object) -> None:
        """Say nothing of a request on the screen of the test."""


class _OnThisMachine(socketserver.ThreadingTCPServer):
    """The server of the stand-in. It asks nobody for the name of the machine that it runs on.

    The server of ``http.server`` asks for that name when it starts, to have something to
    call itself. On some machines the answer takes half a minute, and a test has less.
    """

    daemon_threads = True


@contextlib.contextmanager
def serving(requests: list[tuple[str, str | None, dict[str, Any]]]) -> Generator[str]:
    """Run the stand-in for a service on this machine, and give its address."""
    handler = type("Handler", (_Service,), {"requests": requests})
    with _OnThisMachine(("127.0.0.1", 0), handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_address[1]}/v1"
        finally:
            server.shutdown()
            thread.join(timeout=5)


@pytest.fixture
def service() -> Iterator[tuple[str, list[tuple[str, str | None, dict[str, Any]]]]]:
    """A service on this machine. Its address, and the requests that it was sent."""
    requests: list[tuple[str, str | None, dict[str, Any]]] = []
    with serving(requests) as address:
        yield address, requests


def test_the_stand_in_for_a_service_asks_nobody_for_the_name_of_its_machine(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def asked(*name: str) -> str:
        raise AssertionError(f"the name of the machine was asked for: {name}")

    # On the machines that run the tests for macOS, the answer to either question takes
    # longer than a test may take.
    monkeypatch.setattr(socket, "getfqdn", asked)
    monkeypatch.setattr(socket, "gethostbyaddr", asked)
    requests: list[tuple[str, str | None, dict[str, Any]]] = []

    with serving(requests) as address:
        answer = httpx.post(
            f"{address}/chat/completions",
            json={"model": "tiny"},
            headers={"Authorization": "Bearer made-up"},
            timeout=5,
        )

    assert answer.status_code == 200
    assert "Hello." in answer.text
    assert requests == [("/v1/chat/completions", "Bearer made-up", {"model": "tiny"})]


@pytest.mark.parametrize("in_a_keyring", [False, True], ids=["in the file", "in a keyring"])
def test_a_run_talks_to_the_service_that_was_chosen_with_the_key_that_was_stored(
    home: Path,
    work: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    service: tuple[str, list[tuple[str, str | None, dict[str, Any]]]],
    in_a_keyring: bool,
) -> None:
    vault = Memory().install() if in_a_keyring else None
    address, requests = service
    terminal = Terminal(monkeypatch, "local", address, "tiny", "32768", key=KEY)
    assert main(["setup"]) == 0
    capsys.readouterr()
    run = ["run", "Say hello.", "--cwd", str(work), "--no-json"]

    with_the_stored_key = main([*run, "--session-id", "s-1"])
    first = capsys.readouterr()
    monkeypatch.setenv("COLUMBINA_API_KEY", OTHER)
    with_the_key_of_the_variable = main([*run, "--session-id", "s-2"])
    second = capsys.readouterr()

    assert terminal.asked[1].startswith("Its address")
    assert (with_the_stored_key, first.out, first.err) == (0, "Hello.\n", "")
    assert (with_the_key_of_the_variable, second.out, second.err) == (0, "Hello.\n", "")
    assert [(path, key, body["model"]) for path, key, body in requests] == [
        ("/v1/chat/completions", f"Bearer {KEY}", "tiny"),
        ("/v1/chat/completions", f"Bearer {OTHER}", "tiny"),
    ]
    # The key lies where it was stored, and nowhere else in what the program wrote down:
    # in the file of keys, or in the keyring and then in no file at all.
    holding = [
        found.relative_to(home).as_posix()
        for found in home.rglob("*")
        if found.is_file()
        and (KEY.encode() in found.read_bytes() or OTHER.encode() in found.read_bytes())
    ]
    assert holding == ([] if in_a_keyring else ["credentials.json"])
    assert len([found for found in home.rglob("*") if found.is_file()]) > 3
    if vault is not None:
        assert [provider for _, provider in vault.held] == ["local"]


# What a review found


def test_setup_that_is_interrupted_while_it_stores_does_not_say_that_nothing_was_stored(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    Terminal(monkeypatch, "openai", "gpt-test", "", key=KEY)
    kept = credentials.keep

    def kept_and_then_interrupted(path: Path, provider: str, key: str, origin: str) -> None:
        kept(path, provider, key, origin)
        raise KeyboardInterrupt

    monkeypatch.setattr(credentials, "keep", kept_and_then_interrupted)

    code = main(["setup"])

    captured = capsys.readouterr()
    assert code == 130
    # The key is stored by now. Nothing is said that is not so.
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}
    assert "nothing was stored" not in captured.err + captured.out
    assert captured.err == ""


def test_a_stored_key_does_not_follow_a_variable_that_sends_the_requests_elsewhere(
    home: Path,
    work: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    service: tuple[str, list[tuple[str, str | None, dict[str, Any]]]],
) -> None:
    address, requests = service
    keep(home / "credentials.json", "deepseek", KEY, "https://api.deepseek.com")
    monkeypatch.setenv("DEEPSEEK_BASE_URL", address)
    run = ["run", "Say hello.", "--cwd", str(work), "--no-json"]

    with_the_stored_key = main([*run, "--session-id", "s-1"])
    first = capsys.readouterr()
    # A key that comes from the environment, like the address, goes where the address leads.
    monkeypatch.setenv("DEEPSEEK_API_KEY", OTHER)
    with_the_key_of_the_variable = main([*run, "--session-id", "s-2"])
    second = capsys.readouterr()

    assert with_the_stored_key == 1
    assert "was stored for https://api.deepseek.com" in first.out + first.err
    assert "It is not sent there." in first.out + first.err
    assert KEY not in first.out + first.err
    assert (with_the_key_of_the_variable, second.out) == (0, "Hello.\n")
    # The one request that was made carries the key of the variable. The stored one went nowhere.
    assert [key for _path, key, _body in requests] == [f"Bearer {OTHER}"]
