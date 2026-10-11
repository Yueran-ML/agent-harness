"""API keys: what one looks like, and the file in which a user keeps them."""

import json
import os
import stat
import string
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

from columbina.llm import credentials
from columbina.llm.credentials import (
    CredentialsError,
    forget,
    keep,
    key_for,
    sendable,
    stored,
    trimmed,
)

KEY = "sk-test-0123456789"


@pytest.mark.parametrize(
    "found",
    [KEY, f"{KEY}\n", f"{KEY}\r\n", f" \t{KEY}", f"\n {KEY} \n", KEY + chr(0xA0)],
    ids=["as it is", "a line break", "a line break of Windows", "blanks", "both", "a hard blank"],
)
def test_what_is_around_a_key_is_no_part_of_it(found: str) -> None:
    assert trimmed(found) == KEY


def test_what_is_inside_a_key_stays() -> None:
    assert trimmed(" sk test\n0123 ") == "sk test\n0123"
    assert trimmed(" \n\t") == ""


def test_a_key_of_visible_characters_can_be_sent() -> None:
    every = string.ascii_letters + string.digits + string.punctuation

    assert sendable(KEY)
    assert sendable(every)
    # The first and the last of the characters that can be seen.
    assert sendable("!")
    assert sendable("~")


@pytest.mark.parametrize(
    "key",
    [
        "",
        " ",
        "sk test",
        "sk\ntest",
        "sk\ttest",
        "sk" + chr(0x1F),
        "sk" + chr(0x7F),
        "sk" + chr(0xE9),
        chr(0x201C) + "sk" + chr(0x201D),
        "sk" + chr(0x200B),
        "sk" + chr(0xFEFF),
    ],
    ids=[
        "nothing",
        "a blank alone",
        "a blank",
        "a line break",
        "a tab",
        "the last control character before the blank",
        "the control character after the tilde",
        "a letter with an accent",
        "the quotation marks of a word processor",
        "a blank of no width",
        "the mark at the head of a file",
    ],
)
def test_a_key_with_anything_else_in_it_cannot_be_sent(key: str) -> None:
    assert not sendable(key)


# The file of keys

OTHER = "sk-other-9876543210"
NOT_WRITTEN_BY_SETUP = (
    "is not what 'columbina setup' writes, so it is not used. Delete it, and enter the keys "
    "again with 'columbina setup'"
)


@pytest.fixture
def file(tmp_path: Path) -> Path:
    return tmp_path / "home" / "credentials.json"


def written(path: Path, content: object) -> str:
    """Put a file there by hand, as its user's alone where a file can be that."""
    text = content if isinstance(content, str) else json.dumps(content)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    path.chmod(0o600)
    return text


def seen_as(mode: int) -> Callable[[int], os.stat_result]:
    """What a system with modes says of every open file: that it has this mode."""

    def fstat(descriptor: int) -> os.stat_result:
        return os.stat_result((stat.S_IFREG | mode, 0, 0, 1, 0, 0, 0, 0, 0, 0))

    return fstat


def test_where_there_is_no_file_there_are_no_keys(file: Path) -> None:
    assert stored(file) == {}
    assert key_for(file, "deepseek") is None
    assert not file.parent.exists()


def test_a_key_that_is_kept_is_found_again(file: Path) -> None:
    keep(file, "deepseek", KEY)

    assert key_for(file, "deepseek") == KEY
    assert key_for(file, "openai") is None
    assert stored(file) == {"deepseek": KEY}


def test_the_file_is_a_text_that_can_be_read_and_ends_with_its_line(file: Path) -> None:
    keep(file, "deepseek", KEY)

    assert file.read_bytes() == (
        b'{\n  "keys": {\n    "deepseek": "sk-test-0123456789"\n  },\n  "version": 1\n}\n'
    )


def test_a_key_for_another_service_leaves_the_first_one_alone(file: Path) -> None:
    keep(file, "deepseek", KEY)

    keep(file, "openai", OTHER)

    assert stored(file) == {"deepseek": KEY, "openai": OTHER}


def test_a_new_key_takes_the_place_of_the_one_that_was_kept(file: Path) -> None:
    keep(file, "deepseek", KEY)

    keep(file, "deepseek", OTHER)

    assert stored(file) == {"deepseek": OTHER}


def test_what_is_around_a_key_is_not_kept(file: Path) -> None:
    keep(file, "deepseek", f"  {KEY}\r\n")

    assert stored(file) == {"deepseek": KEY}


@pytest.mark.parametrize("nothing", ["", "  \n"], ids=["nothing", "blanks"])
def test_nothing_is_not_kept_as_a_key(file: Path, nothing: str) -> None:
    keep(file, "deepseek", KEY)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", nothing)

    assert str(refused.value) == "there is no key to keep: what was given is empty"
    assert stored(file) == {"deepseek": KEY}


@pytest.mark.parametrize(
    "found",
    ["sk-other 9876543210", "sk-other\n9876543210", chr(0x201C) + OTHER + chr(0x201D)],
    ids=["a blank", "a line break", "the quotation marks of a word processor"],
)
def test_what_cannot_be_sent_is_not_kept_as_a_key_and_is_not_repeated(
    file: Path, found: str
) -> None:
    keep(file, "deepseek", KEY)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", found)

    assert str(refused.value) == (
        "this cannot be a key: there is a character in it that no key has, such as a blank, a "
        "line break or a quotation mark of a word processor"
    )
    assert stored(file) == {"deepseek": KEY}


def test_a_key_that_is_forgotten_is_gone_and_the_others_stay(file: Path) -> None:
    keep(file, "deepseek", KEY)
    keep(file, "openai", OTHER)

    assert forget(file, "openai")

    assert stored(file) == {"deepseek": KEY}


def test_a_key_that_was_never_kept_cannot_be_forgotten(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(file, "deepseek", KEY)
    nowhere = file.with_name("none.json")

    def never(self: Path, target: Path) -> Path:
        raise AssertionError(f"{self} was put in the place of {target}")

    # Nothing is written where nothing changes.
    monkeypatch.setattr(Path, "replace", never)

    assert not forget(file, "openai")
    assert not forget(nowhere, "deepseek")
    assert not nowhere.exists()


def test_nothing_is_left_beside_the_file(file: Path) -> None:
    keep(file, "deepseek", KEY)
    keep(file, "openai", OTHER)
    forget(file, "openai")

    assert [found.name for found in file.parent.iterdir()] == ["credentials.json"]


def test_the_file_is_made_anew_and_for_its_user_alone(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    made: list[tuple[str, int, int]] = []
    real = os.open

    def recorded(path: str | Path, flags: int, mode: int = 0o777) -> int:
        made.append((Path(path).parent.name, flags & (os.O_CREAT | os.O_EXCL), mode))
        return real(path, flags, mode)

    monkeypatch.setattr(os, "open", recorded)

    keep(file, "deepseek", KEY)

    # One file, in the directory of the keys, which must not be there yet, and with the mode
    # of a file that only its user reads and writes.
    assert made == [("home", os.O_CREAT | os.O_EXCL, 0o600)]


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_the_file_has_the_mode_of_a_file_of_its_user_alone(file: Path) -> None:
    keep(file, "deepseek", KEY)

    assert stat.S_IMODE(file.stat().st_mode) == 0o600


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_a_file_that_others_can_read_is_not_used(file: Path) -> None:
    written(file, {"version": 1, "keys": {"deepseek": KEY}})
    file.chmod(0o640)

    with pytest.raises(CredentialsError, match="can be read by other users of this machine"):
        stored(file)


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_a_file_that_others_could_read_is_its_users_alone_once_it_is_written_anew(
    file: Path,
) -> None:
    written(file, {"version": 1, "keys": {"deepseek": KEY}})
    file.chmod(0o644)

    keep(file, "openai", OTHER)

    assert stat.S_IMODE(file.stat().st_mode) == 0o600
    assert stored(file) == {"deepseek": KEY, "openai": OTHER}


@pytest.mark.parametrize(
    ("mode", "used"),
    [
        (0o600, True),
        (0o400, True),
        (0o700, True),
        (0o640, False),
        (0o620, False),
        (0o610, False),
        (0o604, False),
        (0o602, False),
        (0o601, False),
        (0o666, False),
    ],
)
def test_a_file_is_used_only_if_nobody_else_may_do_anything_with_it(
    file: Path, monkeypatch: pytest.MonkeyPatch, mode: int, used: bool
) -> None:
    written(file, {"version": 1, "keys": {"deepseek": KEY}})
    # As a system with modes sees a file, also where this is run on one without them.
    monkeypatch.setattr(credentials, "_HAS_MODES", True)
    monkeypatch.setattr(os, "fstat", seen_as(mode))

    if used:
        assert stored(file) == {"deepseek": KEY}
        assert key_for(file, "deepseek") == KEY
        return
    for read in (lambda: stored(file), lambda: key_for(file, "deepseek")):
        with pytest.raises(CredentialsError) as refused:
            read()
        assert str(refused.value) == (
            f"the file of stored keys, {file}, can be read by other users of this machine, "
            f"so it is not used. Make it yours alone with: chmod 600 {file}"
        )


def test_where_files_have_no_such_modes_none_is_asked_about(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, {"version": 1, "keys": {"deepseek": KEY}})
    monkeypatch.setattr(credentials, "_HAS_MODES", False)
    monkeypatch.setattr(os, "fstat", seen_as(0o666))

    assert stored(file) == {"deepseek": KEY}


def test_one_who_writes_the_file_anew_reads_it_whoever_else_can(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, {"version": 1, "keys": {"deepseek": KEY, "moonshot": KEY}})
    monkeypatch.setattr(credentials, "_HAS_MODES", True)
    monkeypatch.setattr(os, "fstat", seen_as(0o644))

    keep(file, "openai", OTHER)
    assert forget(file, "moonshot")

    assert json.loads(file.read_text(encoding="utf-8")) == {
        "version": 1,
        "keys": {"deepseek": KEY, "openai": OTHER},
    }


@pytest.mark.parametrize(
    "content",
    [
        "",
        f"deepseek = {KEY}",
        json.dumps([KEY]),
        json.dumps({"keys": {"deepseek": KEY}}),
        json.dumps({"version": 2, "keys": {"deepseek": KEY}}),
        json.dumps({"version": "1", "keys": {"deepseek": KEY}}),
        json.dumps({"version": 1}),
        json.dumps({"version": 1, "keys": [KEY]}),
        json.dumps({"version": 1, "keys": {"deepseek": 5}}),
        json.dumps({"version": 1, "keys": {"deepseek": None}}),
        json.dumps({"version": 1, "keys": {"deepseek": KEY}, "more": KEY}),
    ],
    ids=[
        "nothing",
        "no json",
        "a list",
        "no version",
        "a version that is not known",
        "a version in quotation marks",
        "no keys",
        "keys in a list",
        "a number for a key",
        "nothing for a key",
        "something beside the keys",
    ],
)
def test_a_file_of_another_shape_is_not_used_and_nothing_of_it_is_repeated(
    file: Path, content: str
) -> None:
    written(file, content)
    reads = (
        lambda: stored(file),
        lambda: key_for(file, "deepseek"),
        lambda: keep(file, "openai", OTHER),
        lambda: forget(file, "deepseek"),
    )

    for read in reads:
        with pytest.raises(CredentialsError) as refused:
            read()

        assert str(refused.value) == f"the file of stored keys, {file}, {NOT_WRITTEN_BY_SETUP}"
        # The error that said what is wrong with the content holds the content.
        assert refused.value.__cause__ is None
        assert refused.value.__context__ is None
    assert file.read_text(encoding="utf-8") == content


def test_what_is_no_file_cannot_be_read(file: Path) -> None:
    file.mkdir(parents=True)

    for read in (lambda: stored(file), lambda: keep(file, "deepseek", KEY)):
        with pytest.raises(CredentialsError) as refused:
            read()

        assert str(refused.value).startswith(f"the file of stored keys, {file}, cannot be read: ")
        assert not str(refused.value).endswith("None")
    assert list(file.iterdir()) == []


def test_a_file_that_cannot_be_put_in_its_place_leaves_what_was_there(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(file, "deepseek", KEY)

    def refused_by_the_system(self: Path, target: Path) -> Path:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(Path, "replace", refused_by_the_system)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "openai", OTHER)

    assert str(refused.value) == (
        f"the file of stored keys, {file}, cannot be written: Permission denied"
    )
    monkeypatch.undo()
    assert stored(file) == {"deepseek": KEY}
    # What was written beside the file held the keys, and is gone.
    assert [found.name for found in file.parent.iterdir()] == ["credentials.json"]


def test_a_file_whose_directory_cannot_be_made_cannot_be_written(tmp_path: Path) -> None:
    in_the_way = tmp_path / "home"
    in_the_way.write_text("a file where the directory would be", encoding="utf-8")
    file = in_the_way / "credentials.json"

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", KEY)

    assert str(refused.value).startswith(f"the file of stored keys, {file}, cannot be ")
    assert in_the_way.read_text(encoding="utf-8") == "a file where the directory would be"
