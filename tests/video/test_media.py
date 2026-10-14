"""What the editing of video asks of ffmpeg: ``columbina.video.media``."""

import os
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from columbina import process, programs
from columbina.process import Completed
from columbina.video.media import MediaUnavailableError, Tools, checked, located, versions
from support import media as real_media
from support.links import link_file

WINDOWS = sys.platform == "win32"
EXE = ".exe" if WINDOWS else ""

EIGHT = """ffmpeg version 8.1.1-full_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg \
developers
built with gcc 15.2.0 (Rev13, Built by MSYS2 project)
configuration: --enable-gpl --enable-version3
libavutil      60. 26.101 / 60. 26.101
libavcodec     62. 28.101 / 62. 28.101
libavformat    62. 12.101 / 62. 12.101
libavdevice    62.  3.101 / 62.  3.101
libavfilter    11. 14.101 / 11. 14.101
libswscale      9.  5.101 /  9.  5.101
libswresample   6.  3.101 /  6.  3.101
"""

UBUNTU = """ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023 the FFmpeg developers
built with gcc 13 (Ubuntu 13.2.0-23ubuntu3)
libavutil      58. 29.100 / 58. 29.100
libavformat    60. 16.100 / 60. 16.100
libavfilter     9. 12.100 /  9. 12.100
"""

NIGHTLY = """ffmpeg version N-117000-g1234abcd Copyright (c) 2000-2026 the FFmpeg developers
libavformat    62. 13.100 / 62. 13.100
libavfilter    11. 15.100 / 11. 15.100
"""

OLD = """ffmpeg version 5.1.6-0+deb12u1 Copyright (c) 2000-2024 the FFmpeg developers
libavformat    59. 27.100 / 59. 27.100
libavfilter     8. 44.100 /  8. 44.100
"""


def programs_in(directory: Path, *names: str) -> Path:
    """Put files that pass for programs into a directory, by name."""
    directory.mkdir(parents=True, exist_ok=True)
    for name in names:
        program = directory / f"{name}{EXE}"
        program.write_bytes(b"")
        program.chmod(0o755)
    return directory


def answering(monkeypatch: pytest.MonkeyPatch, said: str, code: int | None = 0) -> list[list[str]]:
    """Have every program that is run say something and end so, and keep what was run."""
    ran: list[list[str]] = []

    async def run(argv: Sequence[str], **options: Any) -> Completed:
        ran.append(list(argv))
        return Completed(
            exit_code=code,
            timed_out=code is None,
            output=said,
            output_bytes=len(said),
            omitted_bytes=0,
            saved_bytes=0,
        )

    monkeypatch.setattr(process, "run", run)
    return ran


@pytest.fixture
def root(tmp_path: Path) -> Path:
    return Path(os.path.realpath(tmp_path))


# The versions


@pytest.mark.parametrize(
    ("said", "both"),
    [(EIGHT, (62, 11)), (UBUNTU, (60, 9)), (NIGHTLY, (62, 11)), (OLD, (59, 8))],
    ids=["8.1", "6.1 of Ubuntu", "a nightly build", "5.1"],
)
def test_the_versions_are_read_from_those_of_the_libraries(
    said: str, both: tuple[int, int]
) -> None:
    assert versions(said) == both


@pytest.mark.parametrize(
    "said",
    ["", "ffmpeg version 8.1.1\n", "libavformat    62. 12.101\n", "libavfilter    11. 14.101\n"],
)
def test_without_both_libraries_there_is_no_version(said: str) -> None:
    assert versions(said) is None


# Finding the programs


async def test_ffmpeg_on_the_search_path_and_ffprobe_beside_it_are_found(root: Path) -> None:
    ffmpeg = real_media.real()

    tools = await located(os.environ["PATH"], [root], cwd=root)

    assert tools.ffmpeg == ffmpeg
    assert tools.ffprobe == str(Path(ffmpeg).with_name(f"ffprobe{EXE}"))
    assert tools.avformat >= 60
    assert tools.avfilter >= 9


async def test_without_ffmpeg_it_says_how_to_install_it(root: Path) -> None:
    (root / "empty").mkdir()

    with pytest.raises(MediaUnavailableError, match="no ffmpeg program was found") as raised:
        await located(str(root / "empty"), [], cwd=root)

    assert "winget install Gyan.FFmpeg" in str(raised.value)
    assert "brew install ffmpeg" in str(raised.value)


async def test_an_ffmpeg_in_a_working_directory_is_not_taken(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ran = answering(monkeypatch, EIGHT)
    programs_in(root / "work", "ffmpeg", "ffprobe")

    with pytest.raises(MediaUnavailableError, match="lies in the working directory is not taken"):
        await located(str(root / "work"), [root / "work"], cwd=root)

    assert ran == []


async def test_without_ffprobe_beside_ffmpeg_neither_is_used(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ran = answering(monkeypatch, EIGHT)
    programs_in(root / "tools", "ffmpeg")
    programs_in(root / "other", "ffprobe")
    search = os.pathsep.join([str(root / "tools"), str(root / "other")])

    with pytest.raises(MediaUnavailableError, match="no ffprobe beside the ffmpeg program"):
        await located(search, [], cwd=root)

    assert ran == []


async def test_an_ffprobe_that_a_link_leads_into_a_working_directory_for_is_not_used(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, EIGHT)
    programs_in(root / "tools", "ffmpeg")
    programs_in(root / "work", "ffprobe")
    link_file(root / "tools" / f"ffprobe{EXE}", root / "work" / f"ffprobe{EXE}")

    with pytest.raises(MediaUnavailableError, match="no ffprobe beside the ffmpeg program"):
        await located(str(root / "tools"), [root / "work"], cwd=root)


async def test_the_way_to_ffprobe_is_walked_with_the_directories_that_it_must_not_touch(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, EIGHT)
    programs_in(root / "tools", "ffmpeg", "ffprobe")
    really = programs.trusted

    def trusted(program: Path, outside: Sequence[Path]) -> bool:
        # The way to ffprobe leads through the working directory, as a link on it would make it.
        if program.name == f"ffprobe{EXE}" and root / "work" in outside:
            return False
        return really(program, outside)

    monkeypatch.setattr(programs, "trusted", trusted)

    with pytest.raises(MediaUnavailableError, match="no ffprobe beside the ffmpeg program"):
        await located(str(root / "tools"), [root / "work"], cwd=root)
    tools = await located(str(root / "tools"), [root / "elsewhere"], cwd=root)
    assert tools.ffprobe == str(root / "tools" / f"ffprobe{EXE}")


async def test_what_ffmpeg_says_of_itself_is_asked_in_the_directory_given(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    asked: list[dict[str, Any]] = []

    async def run(argv: Sequence[str], **options: Any) -> Completed:
        asked.append({"argv": list(argv), **options})
        return Completed(
            exit_code=0,
            timed_out=False,
            output=EIGHT,
            output_bytes=len(EIGHT),
            omitted_bytes=0,
            saved_bytes=0,
        )

    monkeypatch.setattr(process, "run", run)
    programs_in(root / "tools", "ffmpeg", "ffprobe")

    tools = await located(str(root / "tools"), [], cwd=root / "tools")

    assert tools == Tools(
        ffmpeg=str(root / "tools" / f"ffmpeg{EXE}"),
        ffprobe=str(root / "tools" / f"ffprobe{EXE}"),
        avformat=62,
        avfilter=11,
    )
    (only,) = asked
    assert only["argv"] == [tools.ffmpeg, "-hide_banner", "-version"]
    assert only["cwd"] == root / "tools"
    assert only["timeout_seconds"] > 0


async def test_an_ffmpeg_older_than_six_is_not_used(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, OLD)
    programs_in(root / "tools", "ffmpeg", "ffprobe")

    with pytest.raises(MediaUnavailableError, match=r"too old.*libavformat 59.*6\.0 or newer"):
        await located(str(root / "tools"), [], cwd=root)


async def test_the_oldest_ffmpeg_that_will_do_is_used(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, UBUNTU)
    programs_in(root / "tools", "ffmpeg", "ffprobe")

    tools = await located(str(root / "tools"), [], cwd=root)

    assert (tools.avformat, tools.avfilter) == (60, 9)


@pytest.mark.parametrize(
    ("said", "code"),
    [("", 1), ("", None), ("ffmpeg version 8\n", 0), (EIGHT, 1)],
    ids=["failed", "took too long", "no libraries", "failed after saying"],
)
async def test_an_ffmpeg_that_does_not_say_what_it_is_is_not_used(
    root: Path, monkeypatch: pytest.MonkeyPatch, said: str, code: int | None
) -> None:
    answering(monkeypatch, said, code)
    programs_in(root / "tools", "ffmpeg", "ffprobe")

    with pytest.raises(MediaUnavailableError, match="did not say which version it is"):
        await located(str(root / "tools"), [], cwd=root)


async def test_an_ffmpeg_that_cannot_be_started_is_not_used(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        raise OSError(8, "Exec format error")

    monkeypatch.setattr(process, "run", run)
    programs_in(root / "tools", "ffmpeg", "ffprobe")

    with pytest.raises(MediaUnavailableError, match=r"could not be started: .*Exec format error"):
        await located(str(root / "tools"), [], cwd=root)


async def test_the_way_to_the_programs_is_walked_again_before_they_are_run(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, EIGHT)
    programs_in(root / "tools", "ffmpeg", "ffprobe")
    tools = await located(str(root / "tools"), [root / "work"], cwd=root)
    checked(tools)
    walked: list[tuple[Path, tuple[Path, ...]]] = []

    def trusted(program: Path, outside: Sequence[Path]) -> bool:
        walked.append((program, tuple(outside)))
        return program.name != f"ffprobe{EXE}"

    monkeypatch.setattr(programs, "trusted", trusted)

    with pytest.raises(MediaUnavailableError, match="no longer there, or the way to it"):
        checked(tools)
    assert walked == [
        (Path(tools.ffmpeg), (root / "work",)),
        (Path(tools.ffprobe), (root / "work",)),
    ]


async def test_a_program_that_is_gone_is_not_run(
    root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    answering(monkeypatch, EIGHT)
    programs_in(root / "tools", "ffmpeg", "ffprobe")
    tools = await located(str(root / "tools"), [], cwd=root)
    Path(tools.ffmpeg).unlink()

    with pytest.raises(MediaUnavailableError, match="no longer there"):
        checked(tools)
