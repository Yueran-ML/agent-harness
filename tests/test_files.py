"""Files that are written whole or not at all: ``columbina.files``."""

import os
import sys
from pathlib import Path

import pytest

from columbina.files import replaced
from support.links import link_file

POSIX = sys.platform != "win32"


def test_a_text_is_written_as_utf_8_with_line_feeds(tmp_path: Path) -> None:
    file = tmp_path / "a" / "b.json"

    replaced(file, "月\n")

    assert file.read_bytes() == "月\n".encode()


def test_bytes_are_written_as_they_are(tmp_path: Path) -> None:
    file = tmp_path / "picture.png"

    replaced(file, b"\x89PNG\r\n")

    assert file.read_bytes() == b"\x89PNG\r\n"


def test_the_file_that_was_there_is_put_in_its_place_and_nothing_is_left_beside_it(
    tmp_path: Path,
) -> None:
    file = tmp_path / "said.json"
    file.write_text("before", encoding="utf-8")

    replaced(file, "after")

    assert file.read_text(encoding="utf-8") == "after"
    assert sorted(path.name for path in tmp_path.iterdir()) == ["said.json"]


def test_a_file_that_cannot_be_put_in_its_place_leaves_the_place_as_it_was(
    tmp_path: Path,
) -> None:
    place = tmp_path / "said.json"
    place.mkdir()
    (place / "inside").write_text("kept", encoding="utf-8")

    # Each system says it in other words: a folder that is not empty, or no access.
    with pytest.raises(OSError):  # noqa: PT011
        replaced(place, "after")

    assert (place / "inside").read_text(encoding="utf-8") == "kept"
    assert sorted(path.name for path in tmp_path.iterdir()) == ["said.json"]


@pytest.mark.skipif(not POSIX, reason="only a system with modes says who may read a file")
def test_a_private_file_is_its_users_alone_and_another_is_as_the_system_makes_it(
    tmp_path: Path,
) -> None:
    replaced(tmp_path / "private", "secret", private=True)
    replaced(tmp_path / "open", "open")

    assert (tmp_path / "private").stat().st_mode & 0o777 == 0o600
    assert (tmp_path / "open").stat().st_mode & 0o777 == 0o666 & ~_umask()


def _umask() -> int:
    mask = os.umask(0)
    os.umask(mask)
    return mask


def test_where_the_file_is_a_link_the_file_that_it_leads_to_is_written(tmp_path: Path) -> None:
    real = tmp_path / "real.json"
    real.write_text("before", encoding="utf-8")
    link = tmp_path / "link.json"
    link_file(link, real)

    replaced(link, "after")

    assert link.is_symlink()
    assert real.read_text(encoding="utf-8") == "after"
