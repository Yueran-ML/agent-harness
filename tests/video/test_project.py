"""Source identity, atomic analysis writes, and explained media subprocess errors."""

from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from columbina import process
from columbina.process import Completed
from columbina.video import media
from columbina.video.media import (
    MediaUnavailableError,
    Tools,
    UnreadableMediaError,
    located,
)
from columbina.video.project import ProjectError, identity_of, written
from support.media import real


@pytest.fixture
async def tools(tmp_path: Path) -> Tools:
    """The ffmpeg of this machine. A test that asks for it is passed over where there is none."""
    found = real()
    return await located(str(Path(found).parent), [], cwd=tmp_path)


def test_a_file_is_written_whole_or_not_at_all(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "deep" / "file.json"
    written(target, "first")
    assert target.read_text(encoding="utf-8") == "first"

    def refused(self: Path, other: Path) -> Path:
        raise OSError("it is open elsewhere")

    monkeypatch.setattr(Path, "replace", refused)
    with pytest.raises(ProjectError, match="could not be written: it is open elsewhere"):
        written(target, "second")

    assert target.read_text(encoding="utf-8") == "first"
    assert [one.name for one in target.parent.iterdir()] == ["file.json"]


def test_the_identity_of_a_file_is_its_size_and_both_its_ends(tmp_path: Path) -> None:
    small, large = tmp_path / "small", tmp_path / "large"
    small.write_bytes(b"x" * 10)
    large.write_bytes(b"a" * 200_000)
    before = identity_of(large)

    large.write_bytes(b"a" * 100_000 + b"b" + b"a" * 99_999)
    middle = identity_of(large)
    large.write_bytes(b"a" * 199_999 + b"b")

    assert identity_of(small).size == 10
    # A change in the middle is found by the time, and not by the digest.
    assert middle.digest == before.digest
    assert identity_of(large).digest != before.digest


def ended(
    code: int | None, *, timed_out: bool = False, output: str = "", error: str = ""
) -> Completed:
    """How a run of a program ended, as a test says it."""
    return Completed(
        exit_code=code,
        timed_out=timed_out,
        output=output,
        output_bytes=len(output),
        omitted_bytes=0,
        saved_bytes=0,
        error_output=error,
    )


@pytest.mark.parametrize(
    ("done", "said"),
    [
        (ended(None, timed_out=True), "took more than"),
        (ended(1, error="Invalid data found\n"), "failed: Invalid data found"),
        (ended(1), "failed: 1"),
        (ended(0, output="not json"), "answered with no document"),
        (ended(0, output="[1]"), "answered with no document"),
    ],
)
async def test_what_ffprobe_cannot_answer_is_said(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch, done: Completed, said: str
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        return done

    monkeypatch.setattr(process, "run", run)

    with pytest.raises(UnreadableMediaError, match=said):
        await media.probed(tools, tmp_path / "x.mp4", place=tmp_path)


async def test_an_ffmpeg_that_cannot_be_started_any_more_is_said(
    tmp_path: Path, tools: Tools, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        raise OSError("gone")

    monkeypatch.setattr(process, "run", run)

    with pytest.raises(MediaUnavailableError, match="could not be started: gone"):
        await media.audio(tools, tmp_path / "x.mp4", tmp_path / "x.wav")
