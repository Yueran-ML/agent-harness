"""The story: the main line as text, written and read back."""

from pathlib import Path
from typing import Any

import pytest

from columbina.video.story import (
    StoryError,
    dialogue_track,
    line_of,
    normal,
    of_track,
    story_path,
    telling,
)
from columbina.video.transcribe import transcript_of
from columbina.video.transcript import Transcript, Word
from support.studio import clip_of, timeline_of, track_of


def word(start: float, end: float, text: str) -> Word:
    return Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)


def talk() -> Transcript:
    return transcript_of(
        "talk",
        "d" * 64,
        [
            word(0.2, 0.5, "Hello,"),
            word(0.6, 0.8, "uh"),
            word(0.9, 1.3, "today"),
            word(1.35, 2.0, "we cut."),
            word(3.2, 3.5, "This"),
            word(3.55, 3.8, "this"),
            word(3.85, 4.6, "works."),
            word(5.0, 5.2, "我"),
            word(5.25, 5.4, "我"),
            word(5.45, 5.9, "觉得"),
            word(6.0, 6.4, "好。"),
        ],
        [],
        70_000_000,
    )


def test_words_are_matched_by_their_letters_alone() -> None:
    assert normal("Ｈello, World！") == "helloworld"  # noqa: RUF001 - letters as wide as a character
    assert normal("你好。") == "你好"
    assert (
        line_of([("Hello,", False), ("uh", True), ("世界", False), ("好", True), ("ok", True)])
        == "Hello, ~~uh~~ 世界~~好 ok~~"
    )
    assert line_of([]) == ""


def media_talk() -> Transcript:
    return talk().model_copy(update={"asset": "m1"})


def test_the_story_of_a_track_has_a_line_for_each_sentence_of_each_clip() -> None:
    timeline = timeline_of(
        track_of("V1", [clip_of("c2", 0, 175, media="m9")], kind="video"),
        # Sentence 1, "This this works.", is cut in two by a blade at 3.72 s of the media.
        track_of("A1", [clip_of("c3", 0, 93), clip_of("c4", 93, 82, ticks=93 * 512)]),
        track_of("A2", [clip_of("c5", 0, 50, media="m9")]),
    )
    words = {"m1": media_talk()}

    story = of_track(timeline, "A1", words, 12)
    other = of_track(timeline, "V1", words, 12)
    told = telling(timeline, "A1", words)

    assert story.splitlines()[2:] == [
        "[c3 00:00] Hello, uh today we cut.",
        "[pause 1.2s]",
        "[c3 00:03] This this",
        "[c4 00:03] works.",
        "[c4 00:05] 我我觉得好。",
    ]
    assert [(part.clip.id, part.sentence) for part in told.parts] == [
        ("c3", 0), ("c3", 1), ("c4", 1), ("c4", 2),
    ]  # fmt: skip
    assert other.splitlines()[2:] == ["> c2: m9 has no transcript", ""]
    assert told.digest != telling(timeline, "A2", words).digest
    assert telling(timeline, "A2", words).parts == ()
    with pytest.raises(StoryError, match="has no track A9"):
        of_track(timeline, "A9", words, 12)
    subtitles: dict[str, Any] = {"id": "S1", "kind": "subtitle", "name": "S1", "enabled": True,
                 "locked": False, "language": "", "cues": []}  # fmt: skip
    with pytest.raises(StoryError, match="holds no clips that are said"):
        of_track(timeline_of(subtitles), "S1", words, 12)


def test_a_clip_played_backward_says_its_last_sentence_first() -> None:
    timeline = timeline_of(track_of("A1", [clip_of("c3", 0, 175, reverse=True)]))

    lines = of_track(timeline, "A1", {"m1": media_talk()}, 1).splitlines()[2:]

    assert [line.split("] ")[1] for line in lines if line.startswith("[c")] == [
        "我我觉得好。",
        "This this works.",
        "Hello, uh today we cut.",
    ]
    # 0.6 s of media are left after "好。", which ends at 6.4 s of the 7 s that the clip shows.
    assert lines[0] == "[c3 00:00] 我我觉得好。"


def test_the_track_that_is_said_is_a1_or_else_the_first_of_sound_or_of_picture() -> None:
    video, a2 = track_of("V1", [], kind="video"), track_of("A2", [])

    assert dialogue_track(timeline_of(video, a2, track_of("A1", []))) == "A1"
    assert dialogue_track(timeline_of(video, a2)) == "A2"
    assert dialogue_track(timeline_of(video)) == "V1"
    assert dialogue_track(timeline_of()) is None
    assert dialogue_track({"tracks": "x"}) is None


def test_the_story_of_a_timeline_is_kept_by_its_id(tmp_path: Path) -> None:
    assert story_path(tmp_path, "t12") == tmp_path / "story" / "t12.md"
    for wrong in ("../t1", "t1/x", "", "t 1"):
        with pytest.raises(StoryError, match="no id of a timeline"):
            story_path(tmp_path, wrong)
