"""Compatibility reader and worker/plugin edge cases."""

import json
import os
from fractions import Fraction
from pathlib import Path

import columbina.video
from columbina.tools import ToolRun
from columbina.video import Config, Videos, whisper_worker
from columbina.video.legacy import Cut, Entry, Plan, Timeline, captions
from columbina.video.transcribe import transcript_of
from columbina.video.transcript import Word


def test_captions_leave_out_what_has_no_time_in_the_film() -> None:
    said = transcript_of(
        "talk",
        "d",
        [
            Word(start_us=0, end_us=100_000, text="a"),
            Word(start_us=200_000, end_us=300_000, text="b"),
        ],
        [],
        400_000,
    )
    timeline = Timeline(
        main=(
            Entry(asset="talk", sentence=0, kept=()),
            Entry(asset="talk", sentence=0, kept=(0, 1)),
        )
    )
    plan = Plan(Fraction(25), (Cut("talk", 0, 100_000, 1),), (0,), (3,), (0,), 3)

    assert captions(plan, timeline, {"talk": said}) == []


def test_the_worker_imports_by_name() -> None:
    assert whisper_worker._module("json") is json  # pyright: ignore[reportPrivateUsage]


def test_without_the_file_seam_a_path_is_taken_from_the_working_directory(tmp_path: Path) -> None:
    videos = Videos(Config(enabled=True), None)
    run = ToolRun("c1", tmp_path, object())  # type: ignore[arg-type]

    assert videos.checked(run, "video_info", "film", "read") == (tmp_path / "film").resolve()
    assert columbina.video.name == "video"
    assert os.sep
