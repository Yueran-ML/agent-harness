"""API keys: what one looks like, and where a user keeps them: a keyring, or a file."""

import importlib.util
import json
import os
import stat
import string
import subprocess
import sys
from collections.abc import Callable, Iterator
from pathlib import Path

import keyring
import pytest
from keyring.backends import fail, null
from keyring.errors import PasswordSetError

from columbina.llm import credentials
from columbina.llm.credentials import (
    CredentialsError,
    Kept,
    forget,
    keep,
    key_for,
    origin_of,
    sendable,
    stored,
    trimmed,
)
from support.links import link_file
from support.vault import Dropping, Locked, Memory, Refusing, Rewriting, Waiting, settled

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
DEEPSEEK = "https://api.deepseek.com"
OPENAI = "https://api.openai.com"
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


def holding(**keys: str) -> dict[str, object]:
    """What a file of keys holds that has these keys, each for the place of DeepSeek."""
    return {
        "version": 1,
        "keys": {name: {"key": key, "sent_to": DEEPSEEK} for name, key in keys.items()},
    }


def on_disk(path: Path) -> dict[str, tuple[str | None, str]]:
    """The keys of a file as they lie in it: each with the place that it is sent to.

    Where the file says of a key only that there is one, there is ``None`` for the key.
    """
    held = json.loads(path.read_text(encoding="utf-8"))
    assert held["version"] == 1
    return {name: (entry.get("key"), entry["sent_to"]) for name, entry in held["keys"].items()}


def seen_as(mode: int) -> Callable[[int], os.stat_result]:
    """What a system says of every open file: that it is of this kind and has this mode."""

    def fstat(descriptor: int) -> os.stat_result:
        kind = mode if stat.S_IFMT(mode) else stat.S_IFREG | mode
        return os.stat_result((kind, 0, 0, 1, 0, 0, 0, 0, 0, 0))

    return fstat


def test_where_there_is_no_file_there_are_no_keys(file: Path) -> None:
    assert stored(file) == {}
    assert key_for(file, "deepseek", DEEPSEEK) is None
    assert not file.parent.exists()


def test_a_key_that_is_kept_is_found_again_for_the_place_that_it_was_kept_for(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert key_for(file, "openai", DEEPSEEK) is None
    # Whoever asks what is stored is told for whom and for where, and is handed no key.
    assert stored(file) == {"deepseek": DEEPSEEK}
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


def test_a_key_is_not_handed_out_for_another_place_than_the_one_that_it_was_kept_for(
    file: Path,
) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    for elsewhere in (
        "http://other-host",
        "https://api.deepseek.com:8443",
        "http://api.deepseek.com",
    ):
        with pytest.raises(CredentialsError) as refused:
            key_for(file, "deepseek", elsewhere)

        assert str(refused.value) == (
            "the key that is stored for deepseek was stored for https://api.deepseek.com, and "
            f"the requests for deepseek now go to {elsewhere}. It is not sent there. Enter a "
            "key for this address with 'columbina setup', or put it into the variable of the "
            "service"
        )
        assert KEY not in str(refused.value)


def test_the_file_is_a_text_that_can_be_read_and_ends_with_its_line(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    assert file.read_bytes() == (
        b'{\n  "keys": {\n    "deepseek": {\n      "key": "sk-test-0123456789",\n'
        b'      "sent_to": "https://api.deepseek.com"\n    }\n  },\n  "version": 1\n}\n'
    )


def test_a_key_for_another_service_leaves_the_first_one_alone(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    keep(file, "openai", OTHER, OPENAI)

    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK), "openai": (OTHER, OPENAI)}
    assert stored(file) == {"deepseek": DEEPSEEK, "openai": OPENAI}


def test_a_new_key_takes_the_place_of_the_one_that_was_kept_and_brings_its_own_place(
    file: Path,
) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    keep(file, "deepseek", OTHER, "https://mirror.example.test")

    assert on_disk(file) == {"deepseek": (OTHER, "https://mirror.example.test")}
    assert key_for(file, "deepseek", "https://mirror.example.test") == OTHER


def test_what_is_around_a_key_is_not_kept(file: Path) -> None:
    keep(file, "deepseek", f"  {KEY}\r\n", DEEPSEEK)

    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


@pytest.mark.parametrize("nothing", ["", "  \n"], ids=["nothing", "blanks"])
def test_nothing_is_not_kept_as_a_key(file: Path, nothing: str) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", nothing, DEEPSEEK)

    assert str(refused.value) == "there is no key to keep: what was given is empty"
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


@pytest.mark.parametrize(
    "found",
    ["sk-other 9876543210", "sk-other\n9876543210", chr(0x201C) + OTHER + chr(0x201D)],
    ids=["a blank", "a line break", "the quotation marks of a word processor"],
)
def test_what_cannot_be_sent_is_not_kept_as_a_key_and_is_not_repeated(
    file: Path, found: str
) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", found, DEEPSEEK)

    assert str(refused.value) == (
        "this cannot be a key: there is a character in it that no key has, such as a blank, a "
        "line break or a quotation mark of a word processor"
    )
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


def test_a_key_that_is_forgotten_is_gone_and_the_others_stay(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)
    keep(file, "openai", OTHER, OPENAI)

    assert forget(file, "openai")

    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}
    assert OTHER.encode() not in file.read_bytes()


def test_a_key_that_was_never_kept_cannot_be_forgotten(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)
    nowhere = file.with_name("none.json")

    def never(self: Path, target: Path) -> Path:
        raise AssertionError(f"{self} was put in the place of {target}")

    # Nothing is written where nothing changes.
    monkeypatch.setattr(Path, "replace", never)

    assert not forget(file, "openai")
    assert not forget(nowhere, "deepseek")
    assert not nowhere.exists()


def test_nothing_is_left_beside_the_file(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)
    keep(file, "openai", OTHER, OPENAI)
    forget(file, "openai")

    assert [found.name for found in file.parent.iterdir()] == ["credentials.json"]


def test_the_file_is_made_anew_and_for_its_user_alone(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    made: list[tuple[str, int, int]] = []
    real = os.open

    def recorded(path: str | Path, flags: int, mode: int = 0o777) -> int:
        if flags & os.O_CREAT:
            made.append((Path(path).parent.name, flags & (os.O_CREAT | os.O_EXCL), mode))
        return real(path, flags, mode)

    monkeypatch.setattr(os, "open", recorded)

    keep(file, "deepseek", KEY, DEEPSEEK)

    # One file, in the directory of the keys, which must not be there yet, and with the mode
    # of a file that only its user reads and writes.
    assert made == [("home", os.O_CREAT | os.O_EXCL, 0o600)]


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_the_file_has_the_mode_of_a_file_of_its_user_alone(file: Path) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    assert stat.S_IMODE(file.stat().st_mode) == 0o600


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_a_file_that_others_can_read_is_not_used(file: Path) -> None:
    written(file, holding(deepseek=KEY))
    file.chmod(0o640)

    with pytest.raises(CredentialsError, match="can be read by other users of this machine"):
        stored(file)


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such modes")
def test_a_file_that_others_could_read_is_its_users_alone_once_it_is_written_anew(
    file: Path,
) -> None:
    written(file, holding(deepseek=KEY))
    file.chmod(0o644)

    keep(file, "openai", OTHER, OPENAI)

    assert stat.S_IMODE(file.stat().st_mode) == 0o600
    assert stored(file) == {"deepseek": DEEPSEEK, "openai": OPENAI}


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
    written(file, holding(deepseek=KEY))
    # As a system with modes sees a file, also where this is run on one without them.
    monkeypatch.setattr(credentials, "_HAS_MODES", True)
    monkeypatch.setattr(os, "fstat", seen_as(mode))

    if used:
        assert stored(file) == {"deepseek": DEEPSEEK}
        assert key_for(file, "deepseek", DEEPSEEK) == KEY
        return
    for read in (lambda: stored(file), lambda: key_for(file, "deepseek", DEEPSEEK)):
        with pytest.raises(CredentialsError) as refused:
            read()
        assert str(refused.value) == (
            f"the file of stored keys, {file}, can be read by other users of this machine, "
            f"so it is not used. Make it yours alone with: chmod 600 {file}"
        )


def test_where_files_have_no_such_modes_none_is_asked_about(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, holding(deepseek=KEY))
    monkeypatch.setattr(credentials, "_HAS_MODES", False)
    monkeypatch.setattr(os, "fstat", seen_as(0o666))

    assert stored(file) == {"deepseek": DEEPSEEK}


def test_one_who_writes_the_file_anew_reads_it_whoever_else_can(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, holding(deepseek=KEY, moonshot=KEY))
    monkeypatch.setattr(credentials, "_HAS_MODES", True)
    monkeypatch.setattr(os, "fstat", seen_as(0o644))

    keep(file, "openai", OTHER, OPENAI)
    assert forget(file, "moonshot")

    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK), "openai": (OTHER, OPENAI)}


@pytest.mark.parametrize(
    "kind",
    [stat.S_IFIFO, stat.S_IFCHR, stat.S_IFSOCK, stat.S_IFBLK, stat.S_IFDIR],
    ids=["a pipe", "a device", "a socket", "a disk", "a directory"],
)
def test_what_is_there_and_is_no_file_is_not_read(
    file: Path, monkeypatch: pytest.MonkeyPatch, kind: int
) -> None:
    written(file, holding(deepseek=KEY))
    # As the system would say it of what is no file. What lies there is read by nobody.
    monkeypatch.setattr(os, "fstat", seen_as(kind | 0o600))

    for read in (
        lambda: stored(file),
        lambda: key_for(file, "deepseek", DEEPSEEK),
        lambda: keep(file, "openai", OTHER, OPENAI),
        lambda: forget(file, "deepseek"),
    ):
        with pytest.raises(CredentialsError) as refused:
            read()

        assert str(refused.value) == (
            f"the file of stored keys, {file}, is no file, so it is not used"
        )


@pytest.mark.skipif(sys.platform == "win32", reason="Windows has no such pipes")
def test_a_pipe_in_the_place_of_the_file_keeps_nobody_waiting(file: Path) -> None:
    assert sys.platform != "win32"
    file.parent.mkdir(parents=True)
    os.mkfifo(file, 0o600)

    # Opening such a pipe to read it waits until somebody writes into it. Nobody does.
    with pytest.raises(CredentialsError, match="is no file, so it is not used"):
        stored(file)
    with pytest.raises(CredentialsError, match="is no file, so it is not used"):
        keep(file, "deepseek", KEY, DEEPSEEK)


def test_a_file_is_opened_without_waiting_for_whoever_might_write_into_it(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, holding(deepseek=KEY))
    asked: list[int] = []
    real = os.open

    def recorded(path: str | Path, flags: int, mode: int = 0o777) -> int:
        asked.append(flags)
        return real(path, flags, mode)

    monkeypatch.setattr(os, "open", recorded)

    assert stored(file) == {"deepseek": DEEPSEEK}

    # Where a system can open without waiting, that is what is asked for.
    without_waiting = getattr(os, "O_NONBLOCK", 0)
    assert asked == [os.O_RDONLY | without_waiting]


@pytest.mark.parametrize(
    "content",
    [
        "",
        f"deepseek = {KEY}",
        json.dumps([KEY]),
        json.dumps({"keys": {"deepseek": {"key": KEY, "sent_to": DEEPSEEK}}}),
        json.dumps({"version": 2, "keys": {"deepseek": {"key": KEY, "sent_to": DEEPSEEK}}}),
        json.dumps({"version": "1", "keys": {"deepseek": {"key": KEY, "sent_to": DEEPSEEK}}}),
        json.dumps({"version": 1}),
        json.dumps({"version": 1, "keys": [KEY]}),
        json.dumps({"version": 1, "keys": {"deepseek": KEY}}),
        json.dumps({"version": 1, "keys": {"deepseek": {"key": KEY}}}),
        json.dumps({"version": 1, "keys": {"deepseek": {"key": 5, "sent_to": DEEPSEEK}}}),
        json.dumps({"version": 1, "keys": {"deepseek": {"key": KEY, "sent_to": None}}}),
        json.dumps(
            {"version": 1, "keys": {"deepseek": {"key": KEY, "sent_to": DEEPSEEK, "more": KEY}}}
        ),
        json.dumps({"version": 1, "keys": {}, "more": KEY}),
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
        "a key without the place that it is for",
        "a key with no word of its place",
        "a number for a key",
        "nothing for a place",
        "something beside a key and its place",
        "something beside the keys",
    ],
)
def test_a_file_of_another_shape_is_not_used_and_nothing_of_it_is_repeated(
    file: Path, content: str
) -> None:
    written(file, content)
    reads = (
        lambda: stored(file),
        lambda: key_for(file, "deepseek", DEEPSEEK),
        lambda: keep(file, "openai", OTHER, OPENAI),
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


def test_a_directory_in_the_place_of_the_file_is_not_used(file: Path) -> None:
    file.mkdir(parents=True)
    # Windows does not open a directory in the first place. Elsewhere it can be opened, and
    # says of itself that it is no file.
    said = "cannot be read: " if sys.platform == "win32" else "is no file, so it is not used"

    for read in (lambda: stored(file), lambda: keep(file, "deepseek", KEY, DEEPSEEK)):
        with pytest.raises(CredentialsError) as refused:
            read()

        assert str(refused.value).startswith(f"the file of stored keys, {file}, {said}")
        assert not str(refused.value).endswith("None")
    assert list(file.iterdir()) == []


def test_what_was_opened_is_closed_again_whatever_comes_of_it(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    written(file, holding(deepseek=KEY))
    opened: list[int] = []
    closed: list[int] = []
    real_open, real_close = os.open, os.close

    def open_and_note(path: str | Path, flags: int, mode: int = 0o777) -> int:
        descriptor = real_open(path, flags, mode)
        opened.append(descriptor)
        return descriptor

    def close_and_note(descriptor: int) -> None:
        closed.append(descriptor)
        real_close(descriptor)

    monkeypatch.setattr(os, "open", open_and_note)
    monkeypatch.setattr(os, "close", close_and_note)

    assert stored(file) == {"deepseek": DEEPSEEK}
    monkeypatch.setattr(os, "fstat", seen_as(stat.S_IFIFO | 0o600))
    with pytest.raises(CredentialsError):
        stored(file)
    written(file, "no json")
    monkeypatch.setattr(os, "fstat", seen_as(0o600))
    with pytest.raises(CredentialsError):
        stored(file)

    assert len(opened) == 3
    assert [descriptor for descriptor in opened if descriptor not in closed] == []


def test_a_file_that_cannot_be_put_in_its_place_leaves_what_was_there(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    keep(file, "deepseek", KEY, DEEPSEEK)

    def refused_by_the_system(self: Path, target: Path) -> Path:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(Path, "replace", refused_by_the_system)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "openai", OTHER, OPENAI)

    assert str(refused.value) == (
        f"the file of stored keys, {file}, cannot be written: Permission denied"
    )
    monkeypatch.undo()
    assert stored(file) == {"deepseek": DEEPSEEK}
    # What was written beside the file held the keys, and is gone.
    assert [found.name for found in file.parent.iterdir()] == ["credentials.json"]


def test_a_file_whose_directory_cannot_be_made_cannot_be_written(tmp_path: Path) -> None:
    in_the_way = tmp_path / "home"
    in_the_way.write_text("a file where the directory would be", encoding="utf-8")
    file = in_the_way / "credentials.json"

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", KEY, DEEPSEEK)

    assert str(refused.value).startswith(f"the file of stored keys, {file}, cannot be ")
    assert in_the_way.read_text(encoding="utf-8") == "a file where the directory would be"


# A file of keys that is a link


def test_the_file_behind_a_link_is_the_one_that_is_written(file: Path) -> None:
    actual = file.parent / "kept elsewhere" / "actual.json"
    written(actual, holding(deepseek=KEY, openai=OTHER))
    link_file(file, actual)

    assert forget(file, "deepseek")
    keep(file, "moonshot", KEY, "https://api.moonshot.cn")

    # The link is a link still, and the file that it leads to no longer holds what was forgotten.
    assert file.is_symlink()
    assert on_disk(actual) == {
        "openai": (OTHER, DEEPSEEK),
        "moonshot": (KEY, "https://api.moonshot.cn"),
    }
    assert stored(file) == {"openai": DEEPSEEK, "moonshot": "https://api.moonshot.cn"}
    assert [found.name for found in actual.parent.iterdir()] == ["actual.json"]
    assert sorted(found.name for found in file.parent.iterdir()) == [
        "credentials.json",
        "kept elsewhere",
    ]


def test_the_file_is_written_where_its_name_leads(
    file: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # What a link does, for a system on which this test may not make one.
    elsewhere = tmp_path / "elsewhere" / "actual.json"
    resolve = Path.resolve

    def led(self: Path, strict: bool = False) -> Path:
        return elsewhere if self == file else resolve(self, strict)

    monkeypatch.setattr(Path, "resolve", led)

    keep(file, "deepseek", KEY, DEEPSEEK)

    assert on_disk(elsewhere) == {"deepseek": (KEY, DEEPSEEK)}
    assert not file.exists()
    assert [found.name for found in elsewhere.parent.iterdir()] == ["actual.json"]


def test_a_name_that_leads_round_in_a_circle_cannot_be_written(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def in_a_circle(self: Path, strict: bool = False) -> Path:
        raise RuntimeError(f"Symlink loop from {str(self)!r}")

    monkeypatch.setattr(Path, "resolve", in_a_circle)

    with pytest.raises(CredentialsError) as refused:
        keep(file, "deepseek", KEY, DEEPSEEK)

    assert str(refused.value) == (
        f"the file of stored keys, {file}, cannot be written: Symlink loop from {str(file)!r}"
    )


# Where an address leads


@pytest.mark.parametrize(
    ("address", "place"),
    [
        ("https://api.deepseek.com", "https://api.deepseek.com"),
        ("https://api.deepseek.com/", "https://api.deepseek.com"),
        ("https://api.deepseek.com/v1/chat/completions?x=1#y", "https://api.deepseek.com"),
        ("HTTPS://API.DeepSeek.COM/v1", "https://api.deepseek.com"),
        ("  https://api.deepseek.com/v1\n", "https://api.deepseek.com"),
        ("https://api.deepseek.com:443/v1", "https://api.deepseek.com"),
        ("http://localhost:80", "http://localhost"),
        ("https://api.deepseek.com:8443/v1", "https://api.deepseek.com:8443"),
        ("http://api.deepseek.com:443", "http://api.deepseek.com:443"),
        ("https://localhost:80/v1", "https://localhost:80"),
        ("http://localhost:11434/v1", "http://localhost:11434"),
        ("http://[::1]:8000/v1", "http://[::1]:8000"),
        ("http://[::1]/v1", "http://[::1]"),
        ("https://someone:secret@api.deepseek.com/v1", "https://api.deepseek.com"),
        ("ws://example.test:80/x", "ws://example.test:80"),
    ],
)
def test_where_an_address_leads_is_its_scheme_its_machine_and_its_port(
    address: str, place: str
) -> None:
    assert origin_of(address) == place


@pytest.mark.parametrize(
    "address",
    [
        "",
        "api.deepseek.com",
        "//api.deepseek.com/v1",
        "localhost:8000/v1",
        "https://",
        "http://[::1",
        "http://host:port/v1",
    ],
    ids=[
        "nothing",
        "no scheme",
        "two slashes and no scheme",
        "a port and no scheme",
        "no machine",
        "a bracket",
        "no port",
    ],
)
def test_an_address_that_cannot_be_taken_apart_is_its_own_place(address: str) -> None:
    assert origin_of(address) == address
    assert origin_of(f"  {address}\n") == address


def test_two_addresses_lead_to_one_place_only_if_all_three_are_the_same() -> None:
    places = {
        origin_of(address)
        for address in (
            "https://api.deepseek.com",
            "http://api.deepseek.com",
            "https://api.deepseek.com:8443",
            "https://api.deepseek.com.evil.test",
            "https://deepseek.com",
        )
    }

    assert len(places) == 5


# The keyring of a system

MEMORY = "the keyring of this system (vault Memory)"
ODD = 'sk-"odd"\\key'
"""A key that a text may spell in more than one way: with a quotation mark and a backslash."""

KEEPS_NOTHING = "the keyring that is set up here keeps nothing"
NOT_THERE = (
    "the key of deepseek is said to be in the keyring of this system, and it is not there. "
    "Enter it again with 'columbina setup'"
)


def payload(key: str, place: str) -> str:
    """What a keyring is given to keep for a key and the place that it is sent to."""
    return json.dumps({"sent_to": place, "key": key}, separators=(",", ":"))


def filed(path: Path) -> str:
    """The name under which the keys of a file are filed in a keyring."""
    return f"columbina {path.resolve()}"


def not_asked(said: str) -> str:
    """What whoever wants a key is told where the keyring could not be asked for it."""
    return (
        "the key of deepseek is kept in the keyring of this system, and the keyring could not "
        f"be asked for it: {said}. Put the key into the variable of the service, or enter it "
        "again with 'columbina setup'"
    )


@pytest.fixture
def unanswering() -> Iterator[Waiting]:
    """A keyring that answers nothing as long as the test lasts."""
    vault = Waiting()
    yield vault
    vault.released.set()
    settled()


def test_no_test_asks_the_keyring_of_the_system() -> None:
    # What a test stores there would lie among the passwords of whoever runs the tests.
    assert os.environ["PYTHON_KEYRING_BACKEND"] == "keyring.backends.null.Keyring"
    assert type(keyring.get_keyring()) is null.Keyring


def test_a_keyring_that_keeps_nothing_leaves_the_key_to_the_file(file: Path) -> None:
    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said=KEEPS_NOTHING)
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


def test_a_key_goes_into_the_keyring_and_the_file_says_only_that_there_is_one(file: Path) -> None:
    vault = Memory().install()

    kept = keep(file, "deepseek", f" {KEY}\n", DEEPSEEK)

    assert kept == Kept(keyring=MEMORY)
    assert vault.held == {(filed(file), "deepseek"): payload(KEY, DEEPSEEK)}
    # What was there is asked for, the key is given, and what is there now is asked for.
    assert [what for what, _, _ in vault.asked] == ["get", "set", "get"]
    assert file.read_bytes() == (
        b'{\n  "keys": {\n    "deepseek": {\n      "sent_to": "https://api.deepseek.com"\n'
        b'    }\n  },\n  "version": 1\n}\n'
    )
    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert key_for(file, "openai", OPENAI) is None
    assert stored(file) == {"deepseek": DEEPSEEK}


def test_a_keyring_is_asked_on_the_side_by_one_that_the_program_does_not_wait_for(
    file: Path,
) -> None:
    vault = Memory().install()

    keep(file, "deepseek", KEY, DEEPSEEK)
    key_for(file, "deepseek", DEEPSEEK)
    forget(file, "deepseek")

    assert vault.asked_from == {("keyring", False)}


def test_the_keys_of_two_files_lie_apart_in_one_keyring(file: Path, tmp_path: Path) -> None:
    vault = Memory().install()
    other = tmp_path / "another home" / "credentials.json"

    keep(file, "deepseek", KEY, DEEPSEEK)
    keep(other, "deepseek", OTHER, DEEPSEEK)

    assert vault.held == {
        (filed(file), "deepseek"): payload(KEY, DEEPSEEK),
        (filed(other), "deepseek"): payload(OTHER, DEEPSEEK),
    }
    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert key_for(other, "deepseek", DEEPSEEK) == OTHER
    assert forget(other, "deepseek")
    assert key_for(file, "deepseek", DEEPSEEK) == KEY


def test_in_a_keyring_the_keys_are_filed_under_the_file_that_the_name_leads_to(
    file: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    vault = Memory().install()
    # What a link does, for a system on which this test may not make one.
    elsewhere = tmp_path / "elsewhere" / "actual.json"
    resolve = Path.resolve

    def led(self: Path, strict: bool = False) -> Path:
        return elsewhere if self == file else resolve(self, strict)

    monkeypatch.setattr(Path, "resolve", led)

    keep(file, "deepseek", KEY, DEEPSEEK)

    assert list(vault.held) == [(f"columbina {elsewhere}", "deepseek")]


def test_the_keys_of_a_link_are_those_of_the_file_that_it_leads_to(file: Path) -> None:
    vault = Memory().install()
    actual = file.parent / "kept elsewhere" / "actual.json"
    written(actual, {"version": 1, "keys": {}})
    link_file(file, actual)

    keep(file, "deepseek", KEY, DEEPSEEK)

    assert list(vault.held) == [(filed(actual), "deepseek")]
    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert key_for(actual, "deepseek", DEEPSEEK) == KEY
    assert forget(actual, "deepseek")
    assert vault.held == {}
    assert stored(file) == {}


def test_a_key_in_the_keyring_takes_the_place_of_one_in_the_file(file: Path) -> None:
    written(file, holding(deepseek=KEY, openai=OTHER))
    Memory().install()

    keep(file, "deepseek", OTHER, "https://mirror.example.test")

    # The key that lay in the file as it is lies there no more.
    assert on_disk(file) == {
        "deepseek": (None, "https://mirror.example.test"),
        "openai": (OTHER, DEEPSEEK),
    }
    assert KEY.encode() not in file.read_bytes()
    assert key_for(file, "deepseek", "https://mirror.example.test") == OTHER
    assert key_for(file, "openai", DEEPSEEK) == OTHER


def test_where_a_key_in_the_keyring_goes_is_said_by_the_keyring_and_not_by_the_file(
    file: Path,
) -> None:
    Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    # Whoever can change the file names another place in it.
    written(file, {"version": 1, "keys": {"deepseek": {"sent_to": "http://other-host"}}})

    with pytest.raises(CredentialsError) as refused:
        key_for(file, "deepseek", "http://other-host")

    assert str(refused.value) == (
        "the key that is stored for deepseek was stored for https://api.deepseek.com, and "
        "the requests for deepseek now go to http://other-host. It is not sent there. Enter a "
        "key for this address with 'columbina setup', or put it into the variable of the "
        "service"
    )
    assert key_for(file, "deepseek", DEEPSEEK) == KEY


def test_a_key_that_lies_in_the_file_is_had_without_a_word_to_the_keyring(file: Path) -> None:
    written(file, holding(deepseek=KEY))
    vault = Locked().install()

    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert key_for(file, "openai", OPENAI) is None
    assert vault.asked == []


def test_whoever_lists_the_keys_asks_no_keyring(file: Path) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    vault.asked.clear()

    assert stored(file) == {"deepseek": DEEPSEEK}
    assert vault.asked == []


def test_what_reaches_a_keyring_is_not_loaded_before_a_keyring_is_asked(tmp_path: Path) -> None:
    # A start of the program that finds its key in a variable, or in the file, asks none.
    written(tmp_path / "keys.json", holding(deepseek=KEY))
    program = (
        "import sys; from pathlib import Path; from columbina.llm import credentials; "
        "file = Path('keys.json'); print(sorted(credentials.stored(file))); "
        "print(len(credentials.key_for(file, 'deepseek', 'https://api.deepseek.com') or '')); "
        "print('keyring' in sys.modules)"
    )

    done = subprocess.run(
        [sys.executable, "-c", program], capture_output=True, text=True, check=True, cwd=tmp_path
    )

    assert done.stdout.split() == ["['deepseek']", str(len(KEY)), "False"]


@pytest.mark.parametrize(
    ("module", "called"),
    [
        ("keyring.backends.Windows", "the Credential Manager of Windows"),
        ("keyring.backends.macOS", "the Keychain of macOS"),
        ("keyring.backends.SecretService", "the Secret Service of this desktop"),
        ("keyring.backends.libsecret", "the Secret Service of this desktop"),
        ("keyring.backends.kwallet", "the KWallet of this desktop"),
        ("keyring.backends.chainer", "the keyring of this system (chainer Kind)"),
    ],
)
def test_a_keyring_is_called_by_what_a_person_knows_it_by(
    file: Path, module: str, called: str
) -> None:
    # The package reaches a keyring of this kind through a module of this name.
    assert importlib.util.find_spec(module) is not None

    class Kind(Memory):
        pass

    Kind.__module__ = module
    Kind().install()

    assert keep(file, "deepseek", KEY, DEEPSEEK) == Kept(keyring=called)


# A keyring that does not take a key


def test_a_system_without_a_keyring_leaves_the_key_to_the_file(file: Path) -> None:
    keyring.set_keyring(fail.Keyring())

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said="there is no keyring here")
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}
    assert key_for(file, "deepseek", DEEPSEEK) == KEY


def test_a_keyring_that_does_not_open_leaves_the_key_to_the_file(file: Path) -> None:
    vault = Locked().install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said="Failed to unlock the collection")
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}
    assert vault.asked == [("get", filed(file), "deepseek")]


def test_a_program_that_cannot_reach_a_keyring_leaves_the_key_to_the_file(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # What an installation leaves that lost the package.
    monkeypatch.setitem(sys.modules, "keyring", None)

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said="import of keyring halted; None in sys.modules")
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


def as_it_is(given: str) -> str:
    return given


def as_python_shows_it(given: str) -> str:
    return repr(given)


def as_json_writes_it(given: str) -> str:
    return json.dumps(given)


def the_key_in(given: str) -> str:
    key = json.loads(given)["key"]
    assert isinstance(key, str)
    return key


def the_key_as_python_shows_it(given: str) -> str:
    return repr(the_key_in(given))


def the_key_as_json_writes_it(given: str) -> str:
    return json.dumps(the_key_in(given))


@pytest.mark.parametrize("key", [KEY, ODD], ids=["a key", "a key that is spelled in many ways"])
@pytest.mark.parametrize(
    ("spelled", "said"),
    [
        (as_it_is, "cannot keep [key]"),
        (as_python_shows_it, "cannot keep '[key]'"),
        (as_json_writes_it, 'cannot keep "[key]"'),
        (the_key_in, "cannot keep [key]"),
        (the_key_as_python_shows_it, "cannot keep '[key]'"),
        (the_key_as_json_writes_it, 'cannot keep "[key]"'),
    ],
    ids=[
        "what it was given",
        "what it was given, as Python shows it",
        "what it was given, as JSON writes it",
        "the key alone",
        "the key alone, as Python shows it",
        "the key alone, as JSON writes it",
    ],
)
def test_what_a_keyring_says_is_passed_on_without_the_key_however_it_spells_it(
    file: Path, key: str, spelled: Callable[[str], str], said: str
) -> None:
    Refusing(lambda given: PasswordSetError(f"cannot keep {spelled(given)}")).install()

    kept = keep(file, "deepseek", key, DEEPSEEK)

    assert kept == Kept(keyring=None, said=said)
    assert on_disk(file) == {"deepseek": (key, DEEPSEEK)}


@pytest.mark.parametrize(
    ("says", "said"),
    [
        ("the keyring\n   is  busy.\r\n", "the keyring is busy"),
        ("No!", "No"),
        ("", "PasswordSetError"),
        (" . ", "PasswordSetError"),
        ("y" * 160, "y" * 160),
        ("y" * 161, "y" * 160 + " [...]"),
        ("word " * 50, "word " * 31 + "word [...]"),
        ("a sentence that ends. " * 8, "a sentence that ends. " * 7 + "a sent [...]"),
    ],
    ids=[
        "on more than one line",
        "with a mark at its end",
        "nothing",
        "nothing but a mark",
        "as long as is passed on",
        "longer than is passed on",
        "cut after a blank",
        "cut in a word",
    ],
)
def test_what_a_keyring_says_is_passed_on_in_one_line_and_not_at_length(
    file: Path, says: str, said: str
) -> None:
    Refusing(lambda given: PasswordSetError(says)).install()

    assert keep(file, "deepseek", KEY, DEEPSEEK) == Kept(keyring=None, said=said)


def test_the_key_is_taken_out_of_what_a_keyring_says_before_that_is_cut(file: Path) -> None:
    Refusing(lambda given: PasswordSetError("z" * 140 + f" {given} is not kept")).install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    # Cut first, the text would end in the middle of what the keyring was given.
    assert kept == Kept(keyring=None, said="z" * 140 + " [key] is not kept")


@pytest.mark.parametrize(
    "error",
    [TimeoutError("the bus took too long"), SystemExit("the keyring left")],
    ids=["an error that is one of waiting too long", "an end of the program"],
)
def test_whatever_a_keyring_fails_with_is_what_it_said(
    file: Path, monkeypatch: pytest.MonkeyPatch, error: BaseException
) -> None:
    # Were this taken for a keyring that does not answer, it would be said so after this long.
    monkeypatch.setattr(credentials, "_WAIT", 2.0)
    Refusing(lambda given: error).install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said=str(error))


def test_a_keyring_that_does_not_answer_is_not_waited_for_for_good(
    file: Path, monkeypatch: pytest.MonkeyPatch, unanswering: Waiting
) -> None:
    monkeypatch.setattr(credentials, "_WAIT", 0.05)
    unanswering.install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert unanswering.reached.is_set()
    assert kept == Kept(keyring=None, said="the keyring did not answer within 0.05 seconds")
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


@pytest.mark.parametrize(
    ("hands_out", "said"),
    [
        (None, KEEPS_NOTHING),
        ("something else", "the keyring that is set up here does not hand out what it was given"),
    ],
    ids=["nothing", "something else"],
)
def test_a_keyring_that_does_not_hand_out_what_it_was_given_is_left_without_it(
    file: Path, hands_out: str | None, said: str
) -> None:
    vault = Rewriting(hands_out).install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept == Kept(keyring=None, said=said)
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}
    # What it made of the key does not stay in it.
    assert vault.held == {}


def test_a_keyring_that_has_nothing_to_give_back_is_one_that_keeps_nothing(file: Path) -> None:
    vault = Dropping().install()

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    # It was asked to give the key back, and what it said to that is nobody's concern.
    assert [what for what, _, _ in vault.asked] == ["get", "set", "get", "delete"]
    assert kept == Kept(keyring=None, said=KEEPS_NOTHING)
    assert on_disk(file) == {"deepseek": (KEY, DEEPSEEK)}


def test_a_keyring_that_does_not_hand_out_what_it_was_given_holds_again_what_it_held(
    file: Path,
) -> None:
    vault = Rewriting("something else").install()
    before = payload(OTHER, DEEPSEEK)
    vault.held[filed(file), "deepseek"] = before

    kept = keep(file, "deepseek", KEY, DEEPSEEK)

    assert kept.keyring is None
    assert vault.held == {(filed(file), "deepseek"): before}


def test_a_key_that_the_file_could_not_be_told_of_does_not_stay_in_the_keyring(
    file: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)

    def refused_by_the_system(self: Path, target: Path) -> Path:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(Path, "replace", refused_by_the_system)

    for provider, place in (("deepseek", "https://mirror.example.test"), ("openai", OPENAI)):
        with pytest.raises(CredentialsError) as refused:
            keep(file, provider, OTHER, place)

        assert str(refused.value) == (
            f"the file of stored keys, {file}, cannot be written: Permission denied"
        )
    monkeypatch.undo()
    # The keyring holds what it held: the first key, and nothing for the other service.
    assert vault.held == {(filed(file), "deepseek"): payload(KEY, DEEPSEEK)}
    assert key_for(file, "deepseek", DEEPSEEK) == KEY
    assert stored(file) == {"deepseek": DEEPSEEK}


# A key that is asked for and is not had from the keyring


def no_keyring() -> None:
    keyring.set_keyring(fail.Keyring())


def a_keyring_that_does_not_open() -> None:
    Locked().install()


@pytest.mark.parametrize(
    ("there_is", "said"),
    [
        (no_keyring, "there is no keyring here"),
        (a_keyring_that_does_not_open, "Failed to unlock the collection"),
    ],
    ids=["no keyring", "a keyring that does not open"],
)
def test_whoever_asks_for_a_key_that_the_keyring_does_not_give_is_told_what_to_do(
    file: Path, there_is: Callable[[], None], said: str
) -> None:
    Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    there_is()

    with pytest.raises(CredentialsError) as refused:
        key_for(file, "deepseek", DEEPSEEK)

    assert str(refused.value) == not_asked(said)
    assert refused.value.__cause__ is None
    assert refused.value.__context__ is None


def test_whoever_asks_for_a_key_does_not_wait_for_the_keyring_for_good(
    file: Path, monkeypatch: pytest.MonkeyPatch, unanswering: Waiting
) -> None:
    Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    monkeypatch.setattr(credentials, "_WAIT", 1.0)
    unanswering.install()

    with pytest.raises(CredentialsError) as refused:
        key_for(file, "deepseek", DEEPSEEK)

    assert str(refused.value) == not_asked("the keyring did not answer within 1 seconds")


@pytest.mark.parametrize(
    "left",
    [
        None,
        "",
        "no json",
        json.dumps([KEY]),
        json.dumps({"sent_to": DEEPSEEK}),
        json.dumps({"key": KEY}),
        json.dumps({"sent_to": DEEPSEEK, "key": 5}),
        json.dumps({"sent_to": DEEPSEEK, "key": KEY, "more": KEY}),
    ],
    ids=[
        "nothing",
        "an empty text",
        "no json",
        "a list",
        "a place without a key",
        "a key without a place",
        "a number for a key",
        "something beside a key and its place",
    ],
)
def test_a_key_that_the_keyring_does_not_hold_as_it_was_given_is_not_there(
    file: Path, left: str | None
) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    if left is None:
        vault.held.clear()
    else:
        vault.held[filed(file), "deepseek"] = left

    with pytest.raises(CredentialsError) as refused:
        key_for(file, "deepseek", DEEPSEEK)

    assert str(refused.value) == NOT_THERE
    # The error that said what is wrong with what the keyring holds repeats it.
    assert refused.value.__cause__ is None
    assert refused.value.__context__ is None


# A key that is forgotten, and the keyring


def test_a_key_that_is_forgotten_is_taken_out_of_the_keyring(file: Path) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    keep(file, "openai", OTHER, OPENAI)

    assert forget(file, "openai")

    assert vault.held == {(filed(file), "deepseek"): payload(KEY, DEEPSEEK)}
    assert stored(file) == {"deepseek": DEEPSEEK}
    assert not forget(file, "openai")


def test_a_key_that_the_file_lost_sight_of_is_taken_out_of_the_keyring_all_the_same(
    file: Path,
) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    file.unlink()

    assert forget(file, "deepseek")

    assert vault.held == {}
    # Nothing is written where the file knows of nothing.
    assert not file.exists()


def test_a_key_that_went_into_the_file_later_does_not_leave_the_first_one_in_the_keyring(
    file: Path,
) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    Locked().install()
    assert keep(file, "deepseek", OTHER, DEEPSEEK).keyring is None
    assert key_for(file, "deepseek", DEEPSEEK) == OTHER
    vault.install()

    assert forget(file, "deepseek")

    assert vault.held == {}
    assert stored(file) == {}


def test_a_key_in_a_keyring_that_does_not_open_is_not_forgotten_and_the_file_still_says_so(
    file: Path,
) -> None:
    Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    Locked().install()

    with pytest.raises(CredentialsError) as refused:
        forget(file, "deepseek")

    assert str(refused.value) == (
        "the key of deepseek is kept in the keyring of this system, and it could not be taken "
        "out of it: Failed to unlock the collection"
    )
    assert refused.value.__cause__ is None
    assert refused.value.__context__ is None
    assert stored(file) == {"deepseek": DEEPSEEK}


def test_a_key_in_the_file_is_forgotten_whatever_the_keyring_does(file: Path) -> None:
    written(file, holding(deepseek=KEY, openai=OTHER))
    Locked().install()

    assert forget(file, "deepseek")
    # And what is known nowhere is no error where the keyring cannot be asked.
    assert not forget(file, "moonshot")

    assert on_disk(file) == {"openai": (OTHER, DEEPSEEK)}


def test_a_key_that_the_keyring_lost_is_forgotten_where_the_file_says_it(file: Path) -> None:
    vault = Memory().install()
    keep(file, "deepseek", KEY, DEEPSEEK)
    vault.held.clear()

    assert forget(file, "deepseek")

    assert stored(file) == {}
