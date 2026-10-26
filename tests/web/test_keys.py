"""The keys of the services that search the web, and which service is asked."""

import logging
from pathlib import Path

import pytest

from columbina.llm.credentials import CredentialsError
from columbina.web import keys


def test_a_key_that_is_kept_is_found_and_deleted(tmp_path: Path) -> None:
    kept = keys.keep(tmp_path, "tavily", " tvly-1\n")

    assert kept.keyring is None
    assert keys.key_of(tmp_path, {}, "tavily") == "tvly-1"
    assert keys.key_of(tmp_path, {}, "brave") is None
    assert bool(keys.forget(tmp_path, "tavily"))
    assert keys.key_of(tmp_path, {}, "tavily") is None
    assert not keys.forget(tmp_path, "tavily")


def test_the_variable_goes_before_what_is_kept(tmp_path: Path) -> None:
    keys.keep(tmp_path, "brave", "kept")

    assert keys.key_of(tmp_path, {"BRAVE_API_KEY": " given "}, "brave") == "given"
    assert keys.key_of(tmp_path, {"BRAVE_API_KEY": "  "}, "brave") == "kept"


def test_a_key_that_cannot_be_had_counts_as_none(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    (tmp_path / keys.FILE).write_text("not json", encoding="utf-8")

    with caplog.at_level(logging.WARNING, "columbina.web"):
        assert keys.key_of(tmp_path, {}, "tavily") is None

    assert "the key of tavily for searching cannot be had" in caplog.text


def test_the_first_service_with_a_key_searches(tmp_path: Path) -> None:
    assert keys.chosen(tmp_path, {}, "auto") == ("duckduckgo", None)
    assert keys.chosen(tmp_path, {"BRAVE_API_KEY": "b"}, "auto") == ("brave", "b")
    keys.keep(tmp_path, "tavily", "t")
    assert keys.chosen(tmp_path, {"BRAVE_API_KEY": "b"}, "auto") == ("tavily", "t")


def test_a_service_that_the_config_names_searches(tmp_path: Path) -> None:
    keys.keep(tmp_path, "tavily", "t")

    assert keys.chosen(tmp_path, {}, "duckduckgo") == ("duckduckgo", None)
    assert keys.chosen(tmp_path, {}, "brave") == ("brave", None)
    assert keys.chosen(tmp_path, {}, "tavily") == ("tavily", "t")


def test_what_is_kept_is_listed_without_a_key(tmp_path: Path) -> None:
    keys.keep(tmp_path, "tavily", "secret-key")

    listed = keys.listed(tmp_path, {"BRAVE_API_KEY": "b"})

    assert listed == {
        "services": [
            {"name": "tavily", "key": "kept", "variable": "TAVILY_API_KEY"},
            {"name": "brave", "key": "variable", "variable": "BRAVE_API_KEY"},
        ]
    }
    assert keys.listed(tmp_path / "none", {})["services"][1]["key"] == "none"


def test_what_is_no_key_is_not_kept(tmp_path: Path) -> None:
    with pytest.raises(CredentialsError):
        keys.keep(tmp_path, "brave", "two words")
