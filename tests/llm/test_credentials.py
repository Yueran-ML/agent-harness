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
    origin_of,
    sendable,
    stored,
    trimmed,
)
from support.links import link_file

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


def on_disk(path: Path) -> dict[str, tuple[str, str]]:
    """The keys of a file as they lie in it: each with the place that it is sent to."""
    held = json.loads(path.read_text(encoding="utf-8"))
    assert held["version"] == 1
    return {name: (entry["key"], entry["sent_to"]) for name, entry in held["keys"].items()}


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
        json.dumps({"version": 1, "keys": {"deepseek": {"sent_to": DEEPSEEK}}}),
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
        "a place without a key",
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
