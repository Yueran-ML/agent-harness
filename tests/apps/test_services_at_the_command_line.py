"""The command line and the service of the model: ``--provider``, what was stored, and ``setup``."""

import contextlib
import getpass
import io
import json
import logging
import os
import re
import socket
import socketserver
import subprocess
import sys
import threading
from collections.abc import Callable, Generator, Iterator
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
from support.documents import one_document, said, written_by_a_program
from support.launch import profile
from support.vault import Memory, Refusing

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
    "pictures": True,
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
    "columbina.llm.adapters.openai_chat or columbina.llm.adapters.anthropic_messages, or it "
    "is disabled, so there is nothing to point at the service {}\n"
)
NAMES = "deepseek, openai, anthropic, openrouter, moonshot, zhipu, siliconflow, ollama, lmstudio"


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


def test_claude_is_talked_to_by_the_adapter_of_anthropic(dump: Dump) -> None:
    entries = dump.entries("--provider", "anthropic")

    assert entries["model"]["plugin"] == "columbina.llm.adapters.anthropic_messages"
    assert entries["model"]["config"] == {
        "provider": "anthropic",
        "base_url": "https://api.anthropic.com",
        "api_key_env": "ANTHROPIC_API_KEY",
        "api_key_file": KEYS,
        "drop_unbound_thinking": True,
    }
    assert entries["frontend"]["config"]["model"] == {
        "provider": "anthropic",
        "model": "claude-opus-5-5",
        "context_window": 1_000_000,
    }


def test_claude_that_was_chosen_is_talked_to_and_another_service_named_goes_before_it(
    dump: Dump, home: Path
) -> None:
    chosen(home, "use: anthropic\nservices:\n  anthropic:\n    model: claude-sonnet-5-5\n")

    claude = dump.entries()
    named = dump.entries("--provider", "openai", "--model", "gpt-test")

    assert claude["model"]["plugin"] == "columbina.llm.adapters.anthropic_messages"
    assert claude["frontend"]["config"]["model"]["model"] == "claude-sonnet-5-5"
    # The adapter of the protocol of the service that is named, whatever was chosen.
    assert named["model"]["plugin"] == "columbina.llm.adapters.openai_chat"
    assert named["model"]["config"] == OPENAI


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


def test_what_is_deleted_for_another_program_is_said_in_a_document(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chosen(home, OPENAI_CHOSEN)
    keep(home / "credentials.json", "openai", KEY, "https://api.openai.com")

    first = main(["setup", "--forget", "openai", "--json"])
    told = capsys.readouterr()
    again = main(["setup", "--forget", "openai", "--json"])
    refused = capsys.readouterr()

    assert first == 0
    assert one_document(told.out) == {"forgotten": {"name": "openai", "key": True, "chosen": True}}
    # What a person would be shown goes where no program looks for its answer.
    assert told.err.startswith("The key of openai is deleted.\n")
    assert again == 2
    assert one_document(refused.out) == {"error": "nothing is stored for 'openai'"}


def test_a_key_alone_or_what_was_said_alone_is_said_to_be_deleted(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    keep(home / "credentials.json", "openai", KEY, "https://api.openai.com")
    main(["setup", "--forget", "openai", "--json"])
    key_alone = one_document(capsys.readouterr().out)
    chosen(home, OPENAI_CHOSEN)
    main(["setup", "--forget", "openai", "--json"])
    said_alone = one_document(capsys.readouterr().out)

    assert key_alone == {"forgotten": {"name": "openai", "key": True, "chosen": False}}
    assert said_alone == {"forgotten": {"name": "openai", "key": False, "chosen": True}}


def test_another_program_is_told_where_the_keyring_could_not_be_asked(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    vault = Memory().install()
    vault.locked = True
    keep(home / "credentials.json", "openai", KEY, "https://api.openai.com")

    code = main(["setup", "--forget", "openai", "--json"])

    assert code == 0
    assert one_document(capsys.readouterr().out) == {
        "forgotten": {"name": "openai", "key": True, "chosen": False},
        "unasked": "Failed to unlock the collection",
    }


def test_json_goes_with_forget_alone(home: Path, capsys: pytest.CaptureFixture[str]) -> None:
    code = main(["setup", "--json"])

    captured = capsys.readouterr()
    assert code == 2
    assert captured.out == ""
    assert captured.err.startswith("columbina: --json goes with --forget.")
    assert stored(home / "credentials.json") == {}


# The command that another program runs


NOT_AN_OBJECT = "the document is not an object of JSON"
NO_KEYRING_TOOK_IT = (
    "No keyring of this system took the key: the keyring that is set up here keeps nothing. "
)


def test_setup_tells_another_program_what_is_set_up_in_one_document_of_json(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    chosen(home, OPENAI_CHOSEN)
    keep(home / "credentials.json", "openai", KEY, "https://api.openai.com")
    monkeypatch.setenv("DEEPSEEK_API_KEY", OTHER)

    code = main(["setup", "--list"])

    captured = capsys.readouterr()
    assert (code, captured.err) == (0, "")
    told = one_document(captured.out)
    assert captured.out.startswith('{"use": "openai", "services": [{"name": "deepseek", ')
    assert told["use"] == "openai"
    assert [service["name"] for service in told["services"]] == NAMES.split(", ")
    assert told["services"][:2] == [
        {
            "name": "deepseek",
            "title": "DeepSeek",
            "base_url": "https://api.deepseek.com",
            "own": False,
            "model": "deepseek-flash",
            "context_window": 1048576,
            "key_env": "DEEPSEEK_API_KEY",
            "key": "environment",
            "key_page": "https://platform.deepseek.com/api_keys",
        },
        {
            "name": "openai",
            "title": "OpenAI",
            "base_url": "https://api.openai.com/v1",
            "own": False,
            "model": "gpt-test",
            "context_window": 400000,
            "key_env": "OPENAI_API_KEY",
            "key": "stored",
            "key_page": "https://platform.openai.com/api-keys",
        },
    ]
    assert KEY not in captured.out
    assert OTHER not in captured.out


def test_setup_stores_what_another_program_writes_and_answers_in_one_document_of_json(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(
        monkeypatch, said(service="openai", model="gpt-test", context_window=400000, api_key=KEY)
    )

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == (
        '{"stored": {"name": "openai", "base_url": "https://api.openai.com/v1", '
        '"model": "gpt-test", "context_window": 400000}, "key_in": "file"}\n'
    )
    # What a person would be shown goes where no program looks for its answer.
    assert captured.err == (
        "\nStored. Columbina talks to openai now, with the model gpt-test.\n"
        f"What was chosen is in {home / 'provider.yml'}.\n"
        f"{NO_KEYRING_TOOK_IT}It is in {home / 'credentials.json'}, as it is and not encrypted. "
        "Keep that file to yourself.\n"
    )
    assert KEY not in captured.out + captured.err
    assert yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8")) == {
        "use": "openai",
        "services": {"openai": {"model": "gpt-test", "context_window": 400000}},
    }
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}
    assert KEY.encode() in (home / "credentials.json").read_bytes()


def test_a_key_from_another_program_that_a_keyring_took_is_said_nowhere_and_lies_in_no_file(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    vault = Memory().install()
    caplog.set_level(logging.DEBUG)
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert one_document(captured.out)["key_in"] == "keyring"
    assert captured.err.endswith(
        "The key is in the keyring of this system (vault Memory). No file of Columbina holds it.\n"
    )
    assert KEY not in captured.out + captured.err + caplog.text
    assert [record for record in caplog.records if record.name.startswith("columbina")] == []
    for found in home.iterdir():
        assert KEY.encode() not in found.read_bytes()
    assert [provider for _, provider in vault.held] == ["openai"]


def test_a_keyring_that_repeats_the_key_where_it_fails_is_not_repeated_to_another_program(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    caplog: pytest.LogCaptureFixture,
) -> None:
    Refusing(lambda given: RuntimeError(f"cannot keep {given}")).install()
    caplog.set_level(logging.DEBUG)
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert one_document(captured.out)["key_in"] == "file"
    assert "No keyring of this system took the key: cannot keep [key]. It is in " in captured.err
    assert KEY not in captured.out + captured.err + caplog.text


def test_setup_stores_no_key_where_another_program_writes_none(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test"))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert captured.out == (
        '{"stored": {"name": "openai", "base_url": "https://api.openai.com/v1", '
        '"model": "gpt-test", "context_window": null}, "key_in": null}\n'
    )
    assert captured.err.endswith(
        "No key is stored for openai. Set the variable OPENAI_API_KEY before the program is "
        "started, or run this again.\n"
    )
    assert not (home / "credentials.json").exists()


@pytest.mark.parametrize(
    ("document", "problem"),
    [
        (b"", NOT_AN_OBJECT),
        (b'{"service": "openai", "model": "gpt-test", "api_key": "' + KEY.encode(), NOT_AN_OBJECT),
        (
            said(service="openai", model="gpt-test", key=KEY),
            "the document says something under a name that is not read. The names that are read "
            "are: service, base_url, model, context_window, api_key, effort",
        ),
        (
            said(service="openai", model="gpt-test", api_key=f"{KEY} {KEY}"),
            "what the document says under 'api_key' cannot be a key: there is a character in it "
            "that no key has, such as a blank or a line break in the middle. Nothing of it was "
            "kept",
        ),
        (
            said(service=KEY, model="gpt-test"),
            "there is no service of the name that the document gives, and the document gives no "
            "address to add one by",
        ),
    ],
    ids=[
        "nothing",
        "cut off in the key",
        "the key under a name that is not read",
        "what cannot be a key",
        "the key for a service",
    ],
)
def test_a_document_that_is_not_right_is_answered_in_json_and_nothing_is_stored(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    document: bytes,
    problem: str,
) -> None:
    written_by_a_program(monkeypatch, document)

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {"error": problem}
    assert captured.err == ""
    assert KEY not in captured.out
    assert not home.exists()


def test_what_cannot_be_stored_for_another_program_is_answered_in_json(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    replace = Path.replace

    def only_the_keys(self: Path, target: Path) -> Path:
        if Path(target).name == "provider.yml":
            raise PermissionError(13, "Permission denied")
        return replace(self, target)

    monkeypatch.setattr(Path, "replace", only_the_keys)
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {
        "error": f"{home / 'provider.yml'} cannot be written: Permission denied. The key was "
        f"stored all the same, in {home / 'credentials.json'}"
    }
    assert captured.err == ""
    assert KEY not in captured.out


@pytest.mark.parametrize("asked_for", ["--list", "--from-stdin"])
def test_what_is_set_up_and_cannot_be_read_is_answered_in_json(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    asked_for: str,
) -> None:
    chosen(home, "use: [")
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))

    code = main(["setup", asked_for])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {
        "error": f"{home / 'provider.yml'} is not YAML. Run 'columbina setup' again, or delete "
        "the file"
    }
    assert captured.err == ""
    assert not (home / "credentials.json").exists()


AT_A_TERMINAL = (
    "'columbina setup --from-stdin' reads what another program writes to its standard input, "
    "and this one is a terminal, or leads nowhere. A key that is typed at a terminal is shown. "
    "'columbina setup' asks for it without showing it"
)


class Keyboard(io.TextIOWrapper):
    """A standard input that is a terminal."""

    def isatty(self) -> bool:
        return True


def test_setup_reads_no_document_at_a_terminal_where_a_key_would_be_shown(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    typed = io.BytesIO(said(service="openai", model="gpt-test", api_key=KEY))
    monkeypatch.setattr(sys, "stdin", Keyboard(typed, encoding="utf-8"))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {"error": AT_A_TERMINAL}
    assert captured.err == ""
    # Nothing was read of what would have been typed.
    assert typed.tell() == 0
    assert not home.exists()


def test_a_standard_input_that_leads_nowhere_is_answered_in_json_and_stores_nothing(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    with Path(os.devnull).open("rb") as nowhere:
        monkeypatch.setattr(sys, "stdin", io.TextIOWrapper(nowhere))

        code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    # Windows says of what leads nowhere that it is a terminal. Elsewhere it is read, and
    # holds nothing.
    problem = AT_A_TERMINAL if sys.platform == "win32" else NOT_AN_OBJECT
    assert code == 2
    assert one_document(captured.out) == {"error": problem}
    assert captured.err == ""
    assert not home.exists()


class Broken(io.RawIOBase):
    """A way in that fails whoever reads from it."""

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        raise OSError(5, "Input/output error")


def closed() -> io.TextIOWrapper:
    stream = io.TextIOWrapper(io.BytesIO(b"{}"))
    stream.close()
    return stream


@pytest.mark.parametrize(
    "standard_input",
    [
        lambda: None,
        closed,
        lambda: io.TextIOWrapper(io.BufferedReader(Broken())),
        lambda: io.StringIO("{}"),
    ],
    ids=["none", "one that is closed", "one that fails", "one that gives text and no bytes"],
)
def test_setup_without_a_standard_input_that_can_be_read_answers_in_json(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    standard_input: Callable[[], object],
) -> None:
    monkeypatch.setattr(sys, "stdin", standard_input())

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {
        "error": "'columbina setup --from-stdin' reads a document from its standard input, and "
        "this process has none that can be read"
    }
    assert captured.err == ""
    assert not home.exists()


def test_a_document_without_end_is_read_no_further_than_one_may_be_long(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    pipe = written_by_a_program(monkeypatch, b" " * 1_000_000)

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 2
    assert one_document(captured.out) == {
        "error": "the document is longer than 65536 bytes, which is more than is read"
    }
    assert pipe.tell() == 65537


class Deaf:
    """A standard error that takes nothing: whoever was to read it is gone."""

    def write(self, text: str) -> int:
        raise OSError(32, "Broken pipe")


@pytest.mark.parametrize(
    "standard_error",
    [lambda: None, closed, Deaf],
    ids=["none", "one that is closed", "one that nobody reads any more"],
)
def test_what_was_stored_is_answered_also_where_a_person_could_be_shown_nothing(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    standard_error: Callable[[], object],
) -> None:
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))
    monkeypatch.setattr(sys, "stderr", standard_error())

    code = main(["setup", "--from-stdin"])

    out = capsys.readouterr().out
    assert code == 0
    # Nothing of what a person would be shown went the way of the answer instead.
    assert out == (
        '{"stored": {"name": "openai", "base_url": "https://api.openai.com/v1", '
        '"model": "gpt-test", "context_window": null}, "key_in": "file"}\n'
    )
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}


def test_a_document_is_read_as_the_bytes_that_came_and_not_as_the_system_reads_text(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    model = "mod" + chr(0xE8) + "le"
    written_by_a_program(monkeypatch, said(service="openai", model=model))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert one_document(captured.out)["stored"]["model"] == model
    # A letter beyond ASCII goes out as it is, as in every other line of JSON of the program.
    assert f'"model": "{model}"' in captured.out
    assert yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8")) == {
        "use": "openai",
        "services": {"openai": {"model": model}},
    }


@pytest.mark.parametrize(
    "together",
    [
        ["--list", "--from-stdin"],
        ["--list", "--forget", "openai"],
        ["--forget", "openai", "--from-stdin"],
    ],
    ids=["to list and to store", "to list and to forget", "to forget and to store"],
)
def test_setup_does_one_of_the_three_at_a_time(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    together: list[str],
) -> None:
    chosen(home, OPENAI_CHOSEN)
    written_by_a_program(monkeypatch, said(service="deepseek", model="deepseek-flash"))

    with pytest.raises(SystemExit) as stopped:
        main(["setup", *together])

    captured = capsys.readouterr()
    assert stopped.value.code == 2
    assert captured.out == ""
    assert "not allowed with argument" in captured.err
    assert (home / "provider.yml").read_text(encoding="utf-8") == OPENAI_CHOSEN


def test_no_option_of_setup_takes_a_key(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as stopped:
        main(["setup", "--help"])

    told = " ".join(capsys.readouterr().out.split())
    assert stopped.value.code == 0
    # A key on the command line would stay in the history of the shell, and every program
    # could read it from the list of processes.
    assert told.startswith(
        "usage: columbina setup [-h] [--forget NAME | --list | --from-stdin | --check] [--json] "
        "options: "
    )


def test_what_another_program_is_told_cannot_steer_a_terminal(
    home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    escape, turned = chr(0x1B), chr(0x202E)
    model = f"ti{escape}[2J{turned}ny"
    mine = {"base_url": "http://localhost:8000/v1", "model": model}
    chosen(home, yaml.safe_dump({"use": "mine", "services": {"mine": mine}}))

    code = main(["setup", "--list"])

    out = capsys.readouterr().out
    assert code == 0
    assert escape not in out
    assert turned not in out
    assert '"model": "ti\\u001b[2J\\u202eny"' in out
    # A program reads the same from it.
    assert one_document(out)["services"][-1]["model"] == model


def test_what_a_person_would_be_shown_of_the_storing_cannot_steer_a_terminal(
    home: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    turned = chr(0x202E)
    odd = tmp_path / f"home{turned}x"
    monkeypatch.setenv("COLUMBINA_HOME", str(odd))
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 0
    assert turned not in captured.out + captured.err
    assert f"What was chosen is in {tmp_path / f'home{STAND_IN}x' / 'provider.yml'}." in (
        captured.err
    )
    assert (odd / "provider.yml").exists()


def test_what_another_program_had_stored_is_what_the_next_run_loads(
    dump: Dump, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    written_by_a_program(
        monkeypatch, said(service="openai", model="gpt-test", context_window=400000, api_key=KEY)
    )
    assert main(["setup", "--from-stdin"]) == 0
    capsys.readouterr()

    assert dump.model() == (
        OPENAI,
        {"provider": "openai", "model": "gpt-test", "context_window": 400000},
    )


def test_what_another_program_had_stored_is_what_it_is_told_is_set_up(
    home: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    written_by_a_program(
        monkeypatch,
        said(service="my-server", base_url="http://localhost:8000/v1", model="tiny", api_key=KEY),
    )
    assert main(["setup", "--from-stdin"]) == 0
    was_stored = one_document(capsys.readouterr().out)["stored"]

    code = main(["setup", "--list"])

    told = one_document(capsys.readouterr().out)
    assert code == 0
    assert told["use"] == "my-server"
    assert told["services"][-1] == {
        "name": "my-server",
        "title": "http://localhost:8000/v1",
        "base_url": "http://localhost:8000/v1",
        "own": True,
        "model": "tiny",
        "context_window": None,
        "key_env": "COLUMBINA_API_KEY",
        "key": "stored",
        "key_page": None,
    }
    assert was_stored == {name: told["services"][-1][name] for name in was_stored}


def test_storing_for_another_program_that_is_interrupted_answers_nothing(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(monkeypatch, said(service="openai", model="gpt-test", api_key=KEY))
    kept = credentials.keep

    def kept_and_then_interrupted(path: Path, provider: str, key: str, origin: str) -> None:
        kept(path, provider, key, origin)
        raise KeyboardInterrupt

    monkeypatch.setattr(credentials, "keep", kept_and_then_interrupted)

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert code == 130
    # The key is stored by now. Nothing is answered that is not so.
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}
    assert (captured.out, captured.err) == ("", "")


def as_a_process(
    home: Path, *arguments: str, written: bytes = b""
) -> subprocess.CompletedProcess[bytes]:
    """Run ``columbina setup`` as a process of its own, the way a desktop application does."""
    env = {
        name: value
        for name, value in os.environ.items()
        if not re.search("KEY|TOKEN|SECRET|PASSWORD", name.upper())
    }
    # The name of this one has KEY in it, so it was left out with the others, and is put
    # back: the process stores a key, and not among the passwords of whoever runs the tests.
    env["PYTHON_KEYRING_BACKEND"] = "keyring.backends.null.Keyring"
    env["COLUMBINA_HOME"] = str(home)
    return subprocess.run(
        [sys.executable, "-m", "columbina", "setup", *arguments],
        input=written,
        capture_output=True,
        env=env,
        timeout=120,
        check=False,
    )


@pytest.mark.timeout(300)
def test_a_real_process_is_told_what_to_store_and_asked_what_is_set_up_over_its_pipes(
    home: Path,
) -> None:
    # A letter that is two bytes in UTF-8, and something else in what Windows takes text for.
    model = "mod" + chr(0xE8) + "le"

    storing = as_a_process(
        home, "--from-stdin", written=said(service="openai", model=model, api_key=KEY)
    )
    listing = as_a_process(home, "--list")
    refusing = as_a_process(home, "--from-stdin", written=b'{"api_key": "' + KEY.encode())

    assert (storing.returncode, listing.returncode, refusing.returncode) == (0, 0, 2)
    assert one_document(storing.stdout.decode("utf-8").replace("\r\n", "\n")) == {
        "stored": {
            "name": "openai",
            "base_url": "https://api.openai.com/v1",
            "model": model,
            "context_window": None,
        },
        "key_in": "file",
    }
    assert f"with the model {model}." in storing.stderr.decode("utf-8")
    told = one_document(listing.stdout.decode("utf-8").replace("\r\n", "\n"))
    assert (told["use"], told["services"][1]["model"], told["services"][1]["key"]) == (
        "openai",
        model,
        "stored",
    )
    assert one_document(refusing.stdout.decode("utf-8").replace("\r\n", "\n")) == {
        "error": NOT_AN_OBJECT
    }
    assert (listing.stderr, refusing.stderr) == (b"", b"")
    for heard in (storing, listing, refusing):
        assert KEY.encode() not in heard.stdout + heard.stderr
    assert stored(home / "credentials.json") == {"openai": "https://api.openai.com"}


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


# How hard the model thinks

CHOSEN_TWO = (
    "use: openai\nservices:\n  openai:\n    model: gpt-test\n"
    "  deepseek:\n    model: deepseek-v4-pro\n    context_window: 500000\n"
)


def what_was_chosen(home: Path) -> Any:
    return yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8"))


def test_a_level_of_effort_is_stored_alone_and_all_else_that_was_chosen_stays(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    chosen(home, CHOSEN_TWO)
    written_by_a_program(monkeypatch, said(service="deepseek", effort=" MAX "))

    code = main(["setup", "--from-stdin"])

    captured = capsys.readouterr()
    assert (code, captured.out, captured.err) == (
        0,
        '{"effort": {"name": "deepseek", "effort": "max"}}\n',
        "",
    )
    # The service in use stays the one it was, and so do the model and the window of the other.
    assert what_was_chosen(home) == {
        "use": "openai",
        "services": {
            "openai": {"model": "gpt-test"},
            "deepseek": {"model": "deepseek-v4-pro", "context_window": 500000, "effort": "max"},
        },
    }


def test_a_level_is_stored_for_a_service_that_nothing_was_chosen_of(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    written_by_a_program(monkeypatch, said(service="deepseek", effort="default"))

    code = main(["setup", "--from-stdin"])

    assert code == 0
    assert one_document(capsys.readouterr().out) == {
        "effort": {"name": "deepseek", "effort": "default"}
    }
    # The default is a choice: it asks for nothing even where a profile asks for a level.
    assert what_was_chosen(home) == {"services": {"deepseek": {"effort": "default"}}}


@pytest.mark.parametrize(
    ("document", "error"),
    [
        (
            said(service="deepseek", effort="ultra"),
            "ultra holds for one run, and is not stored. "
            "The levels that are stored are: default, off, low, high, max",
        ),
        (
            said(service="deepseek", effort="sk-test-0123456789"),
            "what the document says under 'effort' is no level of effort. "
            "The levels that are stored are: default, off, low, high, max",
        ),
        (said(service="deepseek", effort="  "), "the document names no level of effort"),
        (said(service="  ", effort="max"), "the document names no service"),
        (said(service="deepseek", effort=3), "what the document says under 'effort' is not text"),
        (
            said(service="nowhere", effort="max"),
            "there is no service 'nowhere'. The services are: "
            f"{NAMES}. Another one is added with 'columbina setup'",
        ),
    ],
    ids=["ultra", "a key for a level", "no level", "no service", "a number", "no such service"],
)
def test_a_level_that_is_not_stored_is_answered_in_json_and_nothing_is_stored(
    home: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
    document: bytes,
    error: str,
) -> None:
    chosen(home, CHOSEN_TWO)
    written_by_a_program(monkeypatch, document)

    code = main(["setup", "--from-stdin"])

    assert (code, one_document(capsys.readouterr().out)) == (2, {"error": error})
    assert (home / "provider.yml").read_text(encoding="utf-8") == CHOSEN_TWO


def test_a_model_that_is_stored_keeps_the_level_and_a_document_may_say_both(
    home: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    chosen(home, "services:\n  deepseek:\n    model: deepseek-v4-pro\n    effort: low\n")
    written_by_a_program(monkeypatch, said(service="deepseek", model="deepseek-flash"))
    assert main(["setup", "--from-stdin"]) == 0
    kept = what_was_chosen(home)
    written_by_a_program(
        monkeypatch, said(service="deepseek", model="deepseek-v4-pro", effort="off")
    )
    assert main(["setup", "--from-stdin"]) == 0
    both = what_was_chosen(home)
    capsys.readouterr()

    assert kept["services"]["deepseek"] == {"model": "deepseek-flash", "effort": "low"}
    assert both["services"]["deepseek"] == {"model": "deepseek-v4-pro", "effort": "off"}


def test_the_level_that_was_stored_goes_before_the_profile_and_a_patch_before_it(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    asking_low = {"id": "frontend", "config": {"model": {"reasoning": "low"}}}
    low = tmp_path / "low.yml"
    low.write_text(yaml.safe_dump({"extends": "headless", "patch": [asking_low]}), "utf-8")
    patch = tmp_path / "patch.yml"
    patch.write_text(yaml.safe_dump([asking_low]), encoding="utf-8")
    reasoned: list[object] = []
    patched: list[object] = []
    for effort in (None, "max", "default"):
        level = "" if effort is None else f"    effort: {effort}\n"
        chosen(home, f"use: deepseek\nservices:\n  deepseek:\n    model: deepseek-flash\n{level}")
        reasoned.append(dump.model("--profile", str(low))[1].get("reasoning"))
        patched.append(dump.model("--patch", str(patch))[1].get("reasoning"))

    # Where nothing was chosen the profile decides, and the default asks for nothing at all.
    assert reasoned == ["low", "max", None]
    # A patch still has the last word.
    assert patched == ["low", "low", "low"]


def profile_asking(tmp_path: Path, **asked: Any) -> str:
    """Write a profile whose frontend asks a model of DeepSeek, and what else is given of it.

    Returns where the profile is.
    """
    model = {"provider": "deepseek", "model": "deepseek-flash", **asked}
    asking = {"id": "frontend", "config": {"model": model}}
    written = tmp_path / "asking.yml"
    written.write_text(yaml.safe_dump({"extends": "headless", "patch": [asking]}), "utf-8")
    return str(written)


def test_a_level_holds_for_the_service_of_the_profile_where_no_service_is_chosen(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    low = profile_asking(tmp_path, reasoning="low")
    reasoned: list[object] = []
    for services in (
        "deepseek: {effort: max}",
        "deepseek: {effort: default}",
        "openai: {effort: high}",
    ):
        chosen(home, f"services:\n  {services}\n")
        reasoned.append(dump.model("--profile", low)[1].get("reasoning"))

    # Nothing chose the service, so the profile talks to the one that it names, and the level
    # that was kept for that one holds. One that was kept for another service does not.
    assert reasoned == ["max", None, "low"]


def test_a_profile_that_names_its_service_in_no_way_that_can_be_read_is_left_as_it_is(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    chosen(home, "services:\n  deepseek: {effort: max}\n")
    odd = profile_asking(tmp_path, provider=["deepseek"], reasoning="low")

    assert dump.model("--profile", odd)[1] == {
        "provider": ["deepseek"],
        "model": "deepseek-flash",
        "reasoning": "low",
    }


def test_a_level_of_another_service_is_not_asked_of_one_that_the_command_line_names(
    dump: Dump, home: Path, tmp_path: Path
) -> None:
    low = profile_asking(tmp_path, reasoning="low")
    reasoned: list[object] = []
    for text in (
        "use: deepseek\nservices:\n  deepseek: {model: deepseek-flash, effort: max}\n",
        "services:\n  deepseek: {effort: max}\n",
        "services:\n  deepseek: {effort: max}\n  openai: {model: gpt-test, effort: high}\n",
    ):
        chosen(home, text)
        named = dump.model("--profile", low, "--provider", "openai", "--model", "gpt-test")[1]
        reasoned.append(named.get("reasoning"))

    # The profile decides for a service that nothing was kept for, and a level that was kept
    # for the service that is named holds.
    assert reasoned == ["low", "low", "high"]


def test_a_level_is_stored_for_a_service_of_ones_own_as_well(dump: Dump, home: Path) -> None:
    chosen(
        home,
        # Written as setup writes it: unquoted, YAML would read the level as false.
        "use: mine\nservices:\n  mine:\n    base_url: http://localhost:9/v1\n"
        "    model: m\n    effort: 'off'\n",
    )

    assert dump.model()[1]["reasoning"] == "off"
