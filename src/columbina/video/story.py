"""The story: what is said in a film as text, which a person or a model edits, and what it means.

There are two kinds of story. The story of a track (below, :func:`of_track`) is that of a
timeline of the engine; the story of the main line is that of a project of the earlier kind,
which is read still to carry such a project over into the engine.

**The story of a track** is ``story/<timeline>.md`` in the directory of a project of the engine,
:func:`story_path`. It says what the clips of a track of dialogue play, A1 unless another is
named, in the order in which they are heard, a sentence a line:

```
<!-- columbina story: t1 42 3f9a61c2e0d4 -->

[c12 00:14] Hello, today we cut a film.
[pause 1.8s]
[c12 00:20] This tool cuts by the words.
[c15 00:23] It cuts where they are struck out.
```

- The first line names the timeline, the revision of the project that the story was written at,
  and the digest of what it was written from: the timeline, the track, and the words of its
  media as they are shown. A story whose digest is not that of the timeline as it is now is
  refused, :class:`StaleStoryError`: it is written again, and changed again. The revision is
  there to be read: the engine counts a revision for every change of the project, as for a proxy
  that it made, and a story is still good where its timeline is as it was.
- A line is the words of a sentence that one clip plays: those whose middle is in the stretch of
  its media that the clip shows. A sentence that a blade cut in two is two lines, one of each
  clip. The time is where the line is heard on the timeline, and means nothing when it is
  changed. A clip that plays backward says its last sentence first.
- A word that is taken away or struck through with ``~~`` is cut, and so is every word of a line
  that is taken away; words and lines can be neither added, changed nor moved.
- ``[pause 1.8s]`` is the pause after a line, where it is longer than :data:`PAUSE_SHOWN_US`. A
  smaller number makes it shorter; taking the line away makes it
  :data:`~columbina.video.timeline.PAUSE_LEAST_US`.
- A line that begins with ``>`` is a note, as for a clip whose media has no transcript.

What a story cuts is worked out by :mod:`columbina.video.textcut`. Where a clip reads its media
is counted as the engine counts it, :mod:`columbina.video.span`.

**The story of the main line** is a file of a project of the earlier kind, ``story.md``. It is
written from the timeline, and read back when it is committed:

```
<!-- columbina story: 3f9a61c2e0d4 -->

[talk.0 00:01] Hello, ~~uh~~ today we cut a film.
[talk.1 00:06] ~~This this~~ this tool cuts by the words.
  > picture screen over "cuts by the words"
[pause 1.8s]
[talk.2 00:12] ~~Let us look, no, again.~~
```

- A line is a sentence: its source and its number in square brackets, and where it was said in
  the source, which is there to be read and means nothing when it is changed.
- A line that is taken away is a sentence that is cut. A word that is taken away, or struck
  through with ``~~``, is cut. A sentence or a word that was cut is written struck through, and
  comes back when the strokes are taken away.
- A line that is moved moves the sentence.
- ``[pause 1.8s]`` after a sentence is the pause after it. A smaller number makes it shorter;
  taking the line away makes it :data:`~columbina.video.timeline.PAUSE_LEAST_US`. A pause is
  written where it is longer than :data:`PAUSE_SHOWN_US`.
- A line that begins with ``>`` says what lies over the film there, and is not read.
- The words that are kept are the words that were said: another word is not taken, nor are the
  words in another order. A word that was heard wrong is corrected elsewhere, and only shown so.
- A line that is not understood is an error, and nothing of the story is taken then.

The digest in the first line is that of the timeline that the story was written from. A story of
another timeline is not committed.

**Matching words.** A line is matched against the words of its own sentence alone, by their
letters, digits and characters of Chinese and Japanese: neither stops, blanks nor cases count.
Where a word was said twice and one of the two was taken away, the later one is kept: of
"I I think", the "I" that was said first is cut.
"""

import hashlib
import json
import re
import unicodedata
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from fractions import Fraction
from pathlib import Path
from typing import Any, cast

from pydantic import ValidationError

from columbina.video.edits import Rate
from columbina.video.span import Placed, frames_showing, placed, source_span
from columbina.video.timeline import (
    PAUSE_LEAST_US,
    Entry,
    Layer,
    Timeline,
    Transcript,
    kept_words,
)

__all__ = [
    "PAUSE_SHOWN_US",
    "STORIES",
    "Marked",
    "Part",
    "Spoken",
    "StaleStoryError",
    "Story",
    "StoryError",
    "Telling",
    "dialogue_track",
    "label",
    "line_of",
    "marked",
    "natural_pause",
    "normal",
    "of_track",
    "read",
    "story_path",
    "telling",
    "whole",
    "written",
]

PAUSE_SHOWN_US = 800_000
"""A pause longer than this has a line of its own in the story."""

_HEAD = re.compile(r"^<!--\s*columbina story:\s*([0-9a-f]+)\s*-->$")
_LINE = re.compile(r"^\[([^\s\]]+)\.(\d+)(?:\s+[\d:]+)?\]\s?(.*)$")
_PAUSE = re.compile(r"^\[pause\s+(\d+(?:\.\d+)?)\s*s\]$")
_DIGEST = 12


class StoryError(Exception):
    """A story cannot be committed. The words say where, and why."""


@dataclass(frozen=True)
class Story:
    """What a story says.

    Attributes:
        base: The digest that its first line names: of the timeline that it was written from.
        main: The main line that it makes.
    """

    base: str
    main: tuple[Entry, ...]


def normal(text: str) -> str:
    """The letters, digits and characters of a text, in small letters: what words are matched by."""
    folded = unicodedata.normalize("NFKC", text).casefold()
    return "".join(character for character in folded if character.isalnum())


def label(entry_or_asset: str, sentence: int) -> str:
    """The name of a sentence in the story."""
    return f"{entry_or_asset}.{sentence}"


def _latin(text: str) -> bool:
    return any(character.isascii() and character.isalnum() for character in text)


def line_of(parts: list[tuple[str, bool]]) -> str:
    """Write words in a line: with a blank between words of Latin letters, and none in Chinese.

    Words that were cut, one after the other, are struck through together.
    """
    line = ""
    struck = False
    previous = ""
    for text, cut in parts:
        gap = " " if previous and (_latin(previous[-1]) or _latin(text[:1])) else ""
        if cut and not struck:
            line += f"{gap}~~{text}"
        elif not cut and struck:
            line += f"~~{gap}{text}"
        else:
            line += f"{gap}{text}"
        struck = cut
        previous = text
    return f"{line}~~" if struck else line


def _clock(us: int) -> str:
    seconds = us // 1_000_000
    return f"{seconds // 60:02d}:{seconds % 60:02d}"


def natural_pause(transcript: Transcript, entry: Entry, following: Entry | None) -> int | None:
    """How long the pause after a sentence is in its source, where the next one follows it there."""
    if following is None or following.asset != entry.asset or not entry.kept or not following.kept:
        return None
    if following.kept[0] != entry.kept[-1] + 1:
        return None
    return transcript.words[following.kept[0]].start_us - transcript.words[entry.kept[-1]].end_us


def _layers_at(layers: tuple[Layer, ...], asset: str, words: range) -> list[str]:
    """What lies over the film from a word of a sentence on, for a person to read."""
    said: list[str] = []
    for layer in layers:
        if layer.anchor is None or layer.anchor[0] != asset or layer.anchor[1] not in words:
            continue
        what = layer.text if layer.kind == "title" else layer.asset
        said.append(f"  > {layer.kind} {what} {layer.id}")
    return said


def written(timeline: Timeline, transcripts: dict[str, Transcript], digest: str) -> str:
    """Write the story of a timeline.

    Every sentence of the transcripts is in it: those of the main line in their order, each
    sentence that was cut after the one that it followed in its source, and the sentences of a
    source of which nothing is kept at its end.
    """
    lines = [f"<!-- columbina story: {digest[:_DIGEST]} -->", ""]
    placed: set[tuple[str, int]] = {(entry.asset, entry.sentence) for entry in timeline.main}

    def cut_after(asset: str, sentence: int) -> list[str]:
        """The sentences that were cut and follow one in its source, up to the next that is kept."""
        transcript = transcripts[asset]
        found: list[str] = []
        number = sentence + 1
        while number < len(transcript.sentences) and (asset, number) not in placed:
            found.append(_sentence(transcript, number, ()))
            placed.add((asset, number))
            number += 1
        return found

    for asset, transcript in transcripts.items():
        if not any(entry.asset == asset for entry in timeline.main):
            continue
        first = min(entry.sentence for entry in timeline.main if entry.asset == asset)
        # No sentence before the first one of the main line is on it.
        for number in range(first):
            lines.append(_sentence(transcript, number, ()))
            placed.add((asset, number))
    for place, entry in enumerate(timeline.main):
        transcript = transcripts[entry.asset]
        lines.append(_sentence(transcript, entry.sentence, entry.kept))
        start, end = transcript.sentences[entry.sentence]
        lines.extend(_layers_at(timeline.layers, entry.asset, range(start, end)))
        following = timeline.main[place + 1] if place + 1 < len(timeline.main) else None
        pause = entry.pause_us
        natural = natural_pause(transcript, entry, following)
        if pause is None and natural is not None and natural > PAUSE_SHOWN_US:
            pause = min(natural, timeline.pause_most_us)
        if pause is not None:
            lines.append(f"[pause {pause / 1_000_000:.1f}s]")
        lines.extend(cut_after(entry.asset, entry.sentence))
    for asset, transcript in transcripts.items():
        lines.extend(
            _sentence(transcript, number, ())
            for number in range(len(transcript.sentences))
            if (asset, number) not in placed
        )
    return "\n".join(lines) + "\n"


def _sentence(transcript: Transcript, number: int, kept: tuple[int, ...]) -> str:
    first, last = transcript.sentences[number]
    parts = [(transcript.text(index), index not in kept) for index in range(first, last)]
    at = _clock(transcript.words[first].start_us) if first < last else "00:00"
    return f"[{label(transcript.asset, number)} {at}] {line_of(parts)}"


def _marked(text: str) -> list[tuple[str, bool]]:
    """The characters that count of a line, each with whether it is struck through."""
    marked: list[tuple[str, bool]] = []
    struck = False
    for piece in re.split(r"(~~)", text):
        if piece == "~~":
            struck = not struck
            continue
        marked.extend((character, struck) for character in normal(piece))
    return marked


def _matched(transcript: Transcript, sentence: int, text: str, where: int) -> tuple[int, ...]:
    """Find which words of a sentence a line keeps.

    Each word is cut, struck through, or kept as it is. A word without letters is kept where the
    word before it is. Where there is more than one way, the words said earlier are cut.

    Raises:
        StoryError: The line is no way of cutting the words of its sentence.
    """
    first, last = transcript.sentences[sentence]
    kept = _kept_of([transcript.text(index) for index in range(first, last)], text)
    if kept is None:
        raise StoryError(
            f"line {where}: its words are not those of {label(transcript.asset, sentence)}, "
            f'which says "{" ".join(transcript.text(index) for index in range(first, last))}". '
            "Words can be taken away or struck through, and none can be added, changed or moved"
        )
    return tuple(sorted(first + place for place in kept))


def _kept_of(shown: Sequence[str], text: str) -> set[int] | None:
    """Find which of some words, by their places, a line keeps; ``None`` if it cannot be them.

    Each word is cut, struck through, or kept as it is. A word without letters is kept where the
    word before it is. Where there is more than one way, the words said earlier are cut.
    """
    words = [(place, normal(one)) for place, one in enumerate(shown)]
    counted = [(place, said) for place, said in words if said]
    line = _marked(text)
    size = len(line)
    # able[i][j]: the words from i on can make the line from j on.
    able = [[False] * (size + 1) for _ in range(len(counted) + 1)]
    able[len(counted)][size] = True
    for i in range(len(counted) - 1, -1, -1):
        said = counted[i][1]
        for j in range(size, -1, -1):
            if able[i + 1][j]:
                able[i][j] = True
                continue
            piece = line[j : j + len(said)]
            if (
                "".join(character for character, _ in piece) == said
                and len({struck for _, struck in piece}) == 1
                and able[i + 1][j + len(said)]
            ):
                able[i][j] = True
    if not able[0][0]:
        return None
    kept: set[int] = set()
    j = 0
    for i, (place, said) in enumerate(counted):
        if able[i + 1][j]:
            continue
        if not line[j][1]:
            kept.add(place)
        j += len(said)
    with_marks = [place for place, said in words if said == "" and place - 1 in kept]
    return kept | set(with_marks)


def read(story: str, transcripts: dict[str, Transcript]) -> Story:
    """Read what a story makes of the main line.

    Args:
        story: The story.
        transcripts: The transcripts of the sources of the story.

    Raises:
        StoryError: It is no story, or a line of it is not understood.
    """
    lines = story.splitlines()
    head = _HEAD.match(lines[0].strip().lstrip("﻿")) if lines else None
    if head is None:
        raise StoryError("line 1: the story does not begin with the line that names its timeline")
    main: list[Entry] = []
    paused: dict[int, int] = {}
    seen: set[tuple[str, int]] = set()
    for where, raw in enumerate(lines[1:], start=2):
        line = raw.strip()
        if not line or line.startswith(">") or (line.startswith("<!--") and line.endswith("-->")):
            continue
        pause = _PAUSE.match(line)
        if pause is not None:
            if not main:
                raise StoryError(
                    f"line {where}: a pause comes after a sentence, and none is before it"
                )
            paused[len(main) - 1] = int(Fraction(pause[1]) * 1_000_000)
            continue
        found = _LINE.match(line)
        if found is None:
            raise StoryError(f"line {where}: it is neither a sentence, a pause nor a note")
        asset, number, text = found[1], int(found[2]), found[3]
        transcript = transcripts.get(asset)
        if transcript is None or number >= len(transcript.sentences):
            raise StoryError(f"line {where}: there is no sentence {label(asset, number)}")
        if (asset, number) in seen:
            raise StoryError(
                f"line {where}: the sentence {label(asset, number)} is in the story twice"
            )
        seen.add((asset, number))
        kept = _matched(transcript, number, text, where)
        if kept:
            main.append(Entry(asset=asset, sentence=number, kept=kept))
    return Story(base=head[1], main=tuple(_with_pauses(main, paused, transcripts)))


def _with_pauses(
    main: list[Entry], paused: dict[int, int], transcripts: dict[str, Transcript]
) -> list[Entry]:
    """Give each sentence the pause that the story names after it.

    A sentence after which the story names no pause, and after which the source has one that the
    story would have named, had its line taken away: it is made short.
    """
    done: list[Entry] = []
    for place, entry in enumerate(main):
        following = main[place + 1] if place + 1 < len(main) else None
        natural = natural_pause(transcripts[entry.asset], entry, following)
        if place in paused:
            done.append(entry.model_copy(update={"pause_us": paused[place]}))
        elif natural is not None and natural > PAUSE_SHOWN_US:
            done.append(entry.model_copy(update={"pause_us": PAUSE_LEAST_US}))
        else:
            done.append(entry)
    return done


def whole(transcript: Transcript) -> tuple[Entry, ...]:
    """The main line of a source that was never cut: every sentence of it, every word."""
    return tuple(
        Entry(asset=transcript.asset, sentence=number, kept=kept_words(transcript, number))
        for number in range(len(transcript.sentences))
        if transcript.sentences[number][0] < transcript.sentences[number][1]
    )


# The story of a track


STORIES = "story"
"""The directory of a project of the engine that the stories of its timelines are kept in."""

_TRACK_HEAD = re.compile(r"^<!--\s*columbina story:\s*(\S+)\s+(\d+)\s+([0-9a-f]{12})\s*-->$")
_TRACK_LINE = re.compile(r"^\[([^\s\]]+)(?:\s+[\d:]+)?\]\s?(.*)$")
_TIMELINE_ID = re.compile(r"^[0-9A-Za-z_-]{1,64}$")


class StaleStoryError(StoryError):
    """The story was written from a timeline that has changed since, or from other words."""


@dataclass(frozen=True)
class Spoken:
    """A word as a clip plays it.

    Attributes:
        index: Its place in the transcript of the media of the clip.
        first: The first frame of the timeline that shows it.
        end: The frame after the last; ``first`` where no frame shows it, as a word that ends
            where it begins.
    """

    index: int
    first: int
    end: int


@dataclass(frozen=True)
class Part:
    """A line of the story of a track: the words of a sentence that one clip plays.

    Attributes:
        clip: The clip.
        sentence: The sentence of the transcript of its media.
        words: Its words, in the order in which they were said.
    """

    clip: Placed
    sentence: int
    words: tuple[Spoken, ...]

    @property
    def first(self) -> int:
        """The first frame of the timeline that shows a word of it."""
        return min(one.first for one in self.words)

    @property
    def end(self) -> int:
        """The frame after the last that shows a word of it."""
        return max(one.end for one in self.words)


@dataclass(frozen=True)
class Telling:
    """What the story of a track is written from.

    Attributes:
        timeline: The id of the timeline.
        track: The id of the track.
        rate: The rate of the timeline.
        locked: Whether the track is locked.
        clips: The clips of media of the track, in the order of their start.
        parts: The lines, in the order in which they are heard.
        silent: The clips whose media has no transcript, each with its media.
        digest: The digest of the timeline, the track and the words that the story tells of.
    """

    timeline: str
    track: str
    rate: Rate
    locked: bool
    clips: tuple[Placed, ...]
    parts: tuple[Part, ...]
    silent: tuple[tuple[str, str], ...]
    digest: str

    def pause_after(self, place: int) -> int | None:
        """How long the pause after a line is, in microseconds, up to the next line."""
        if place + 1 >= len(self.parts):
            return None
        frames = self.parts[place + 1].first - self.parts[place].end
        return frames * self.rate.den * 1_000_000 // self.rate.num


@dataclass(frozen=True)
class Marked:
    """What a story of a track says should be cut.

    Attributes:
        telling: What it was written from.
        kept: The words that it keeps, each as its clip and its place in the transcript.
        paused: How long the pause after a line is to be, by the place of the line in
            ``telling.parts``, where the story names one that it did not name when it was
            written.
        unpaused: The places of the lines after which the story named a pause when it was
            written, and names none now: those pauses are made :data:`PAUSE_LEAST_US` long.
    """

    telling: Telling
    kept: frozenset[tuple[str, int]]
    paused: dict[int, int]
    unpaused: frozenset[int]


def story_path(directory: Path, timeline: str) -> Path:
    """Where the story of a timeline is kept, in the project of the engine of ``directory``.

    Raises:
        StoryError: The id is not one that a timeline of the engine has.
    """
    if _TIMELINE_ID.fullmatch(timeline) is None:
        raise StoryError(f"{timeline!r} is no id of a timeline")
    return directory / STORIES / f"{timeline}.md"


def _tracks(timeline: Mapping[str, Any]) -> list[dict[str, Any]]:
    tracks = timeline.get("tracks")
    listed = cast("list[object]", tracks) if isinstance(tracks, list) else []
    return [cast("dict[str, Any]", one) for one in listed if isinstance(one, dict)]


def dialogue_track(timeline: Mapping[str, Any]) -> str | None:
    """The track that a story tells of where none is named.

    It is A1, or else the first track of sound, or else the first track of picture.
    """
    tracks = _tracks(timeline)
    if any(one.get("id") == "A1" for one in tracks):
        return "A1"
    for kind in ("audio", "video"):
        found = [str(one.get("id")) for one in tracks if one.get("kind") == kind]
        if found:
            return found[0]
    return None


def _track(timeline: Mapping[str, Any], track: str) -> dict[str, Any]:
    for one in _tracks(timeline):
        if one.get("id") == track:
            if one.get("kind") not in ("audio", "video"):
                raise StoryError(f"track {track} holds no clips that are said")
            return one
    raise StoryError(f"timeline {timeline.get('id')} has no track {track}")


def _digest(
    timeline: Mapping[str, Any],
    track: str,
    clips: Sequence[Placed],
    transcripts: Mapping[str, Transcript],
) -> str:
    """The digest of what the story of a track is written from.

    That is the timeline, the track, and the words of its media, as they are shown.
    """
    media = sorted({clip.media for clip in clips if clip.media in transcripts})
    words = [
        [
            one,
            transcripts[one].digest,
            len(transcripts[one].words),
            sorted(transcripts[one].shown.items()),
        ]
        for one in media
    ]
    said = json.dumps(
        {"timeline": timeline, "track": track, "words": words},
        sort_keys=True,
        ensure_ascii=False,
        separators=(",", ":"),
    )
    return hashlib.sha256(said.encode("utf-8")).hexdigest()[:_DIGEST]


def _owned(transcript: Transcript, clip: Placed, rate: Rate) -> list[int]:
    """The words that a clip plays: those whose middle is in the stretch of media that it shows."""
    scale = clip.begins.scale
    begins = Fraction(clip.begins.ticks * 1_000_000, scale)
    ends = Fraction((clip.begins.ticks + source_span(clip, rate)) * 1_000_000, scale)
    return [
        index
        for index, word in enumerate(transcript.words)
        if begins <= Fraction(word.start_us + word.end_us, 2) < ends
    ]


def telling(
    timeline: Mapping[str, Any], track: str, transcripts: Mapping[str, Transcript]
) -> Telling:
    """Work out what the story of a track tells: the words that its clips play, by sentence.

    Args:
        timeline: The timeline, as the engine writes it in the project.
        track: The track.
        transcripts: The transcript of each media that has one, by the id of the media.

    Raises:
        StoryError: There is no such track, or it holds subtitles, or a clip of it cannot be read.
    """
    found = _track(timeline, track)
    try:
        rate = Rate.model_validate(timeline.get("rate"))
        clips = [placed(one) for one in found.get("clips", [])]
    except (ValueError, ValidationError) as error:
        raise StoryError(f"timeline {timeline.get('id')} cannot be read: {error}") from None
    media = sorted((one for one in clips if one is not None), key=lambda one: one.start)
    parts: list[Part] = []
    silent: list[tuple[str, str]] = []
    for clip in media:
        transcript = transcripts.get(clip.media)
        if transcript is None:
            silent.append((clip.id, clip.media))
            continue
        spoken: list[Spoken] = []
        for index in _owned(transcript, clip, rate):
            word = transcript.words[index]
            first, end = frames_showing(clip, word.start_us, max(word.start_us, word.end_us), rate)
            spoken.append(Spoken(index, clip.start + first, clip.start + end))
        of_clip = [
            Part(clip, number, tuple(one for one in spoken if start <= one.index < end))
            for number, (start, end) in enumerate(transcript.sentences)
        ]
        of_clip = [one for one in of_clip if one.words]
        # A clip that plays backward says its last sentence first.
        parts.extend(reversed(of_clip) if clip.reverse else of_clip)
    return Telling(
        timeline=str(timeline.get("id")),
        track=track,
        rate=rate,
        locked=bool(found.get("locked", False)),
        clips=tuple(media),
        parts=tuple(parts),
        silent=tuple(silent),
        digest=_digest(timeline, track, media, transcripts),
    )


def _timed(frame: int, rate: Rate) -> str:
    return _clock(frame * rate.den * 1_000_000 // rate.num)


def of_track(
    timeline: Mapping[str, Any],
    track: str,
    transcripts: Mapping[str, Transcript],
    revision: int,
) -> str:
    """Write the story of a track of a timeline.

    Args:
        timeline: The timeline, as the engine writes it in the project.
        track: The track that is said, as :func:`dialogue_track` chooses it where none is named.
        transcripts: The transcript of each media that has one, by the id of the media.
        revision: The revision of the project that the timeline is of.

    Raises:
        StoryError: There is no such track, or it holds subtitles.
    """
    told = telling(timeline, track, transcripts)
    lines = [f"<!-- columbina story: {told.timeline} {revision} {told.digest} -->", ""]
    lines.extend(f"> {clip}: {media} has no transcript" for clip, media in told.silent)
    if told.silent:
        lines.append("")
    for place, part in enumerate(told.parts):
        transcript = transcripts[part.clip.media]
        shown = line_of([(transcript.text(one.index), False) for one in part.words])
        lines.append(f"[{part.clip.id} {_timed(part.first, told.rate)}] {shown}")
        pause = told.pause_after(place)
        if pause is not None and pause > PAUSE_SHOWN_US:
            lines.append(f"[pause {pause / 1_000_000:.1f}s]")
    return "\n".join(lines) + "\n"


def _shown_pause(told: Telling, place: int) -> int | None:
    """The pause that the story names after a line when it is written, if it names one."""
    pause = told.pause_after(place)
    if pause is None or pause <= PAUSE_SHOWN_US:
        return None
    return int(Fraction(f"{pause / 1_000_000:.1f}") * 1_000_000)


def _told_by_digest(
    timeline: Mapping[str, Any], digest: str, transcripts: Mapping[str, Transcript]
) -> Telling | None:
    """What the track whose story has a digest tells, if a track of the timeline has it."""
    for one in _tracks(timeline):
        if one.get("kind") in ("audio", "video"):
            told = telling(timeline, str(one.get("id")), transcripts)
            if told.digest == digest:
                return told
    return None


def marked(
    story: str, timeline: Mapping[str, Any], transcripts: Mapping[str, Transcript]
) -> Marked:
    """Read what the story of a track says should be cut.

    A line is the words of a sentence that a clip plays. A word that is taken away from it, or
    struck through, is cut, and so is every word of a line that is taken away. The lines of a
    clip are matched to its sentences in order; where a line could be either of two sentences,
    the one said later is kept. Words can be neither added, changed nor moved, and neither can
    lines. ``[pause 1.2s]`` after a line is how long the pause after it is to be: shorter than
    it is, or it stays as it is. Taking away a pause that the story named when it was written
    makes it :data:`PAUSE_LEAST_US` long. A pause that is left where a line before it was taken
    away is the pause after the line before that; one before every line is passed over.

    Args:
        story: The story.
        timeline: The timeline, as the engine writes it in the project now.
        transcripts: The transcript of each media that has one, by the id of the media.

    Raises:
        StaleStoryError: The timeline, or the words of its media, changed since the story was
            written: it is written again and changed again then.
        StoryError: It is no story of the timeline, or a line of it is not understood.
    """
    lines = story.splitlines()
    head = _TRACK_HEAD.match(lines[0].strip().lstrip("﻿")) if lines else None
    if head is None:
        raise StoryError(
            "line 1: the story does not begin with the line that names its timeline, its "
            "revision and its digest"
        )
    if head[1] != timeline.get("id"):
        raise StoryError(f"line 1: the story is of timeline {head[1]}, not {timeline.get('id')}")
    told = _told_by_digest(timeline, head[3], transcripts)
    if told is None:
        raise StaleStoryError(
            f"the story was written from revision {head[2]} of timeline {head[1]}, which has "
            "changed since, or so have the words of its media. Write the story again, and cut "
            "from that"
        )
    order = {clip.id: place for place, clip in enumerate(told.clips)}
    said: list[tuple[int, str, str]] = []
    paused: dict[int, int] = {}
    for where, raw in enumerate(lines[1:], start=2):
        line = raw.strip()
        if not line or line.startswith(">") or (line.startswith("<!--") and line.endswith("-->")):
            continue
        pause = _PAUSE.match(line)
        if pause is not None:
            # One before every line was left there when the line before it was taken away.
            if said:
                paused[len(said) - 1] = int(Fraction(pause[1]) * 1_000_000)
            continue
        found = _TRACK_LINE.match(line)
        if found is None:
            raise StoryError(f"line {where}: it is neither a line of a clip, a pause nor a note")
        clip = found[1]
        if clip not in order:
            raise StoryError(f"line {where}: there is no clip {clip} on track {told.track}")
        if said and order[clip] < order[said[-1][1]]:
            raise StoryError(
                f"line {where}: clip {clip} comes before clip {said[-1][1]} on the track. Lines "
                "cannot be moved: move the clips themselves"
            )
        said.append((where, clip, found[2]))
    places = _placed_lines(told, transcripts, said)
    kept: set[tuple[str, int]] = set()
    for (_, clip, _), (place, keeps) in zip(said, places, strict=True):
        part = told.parts[place]
        kept.update((clip, part.words[one].index) for one in keeps)
    named = {places[line][0]: us for line, us in paused.items()}
    shown = {place: _shown_pause(told, place) for place in range(len(told.parts))}
    lined = {place for place, _ in places}
    return Marked(
        telling=told,
        kept=frozenset(kept),
        paused={place: us for place, us in named.items() if us != shown[place]},
        unpaused=frozenset(
            place for place in lined if shown[place] is not None and place not in named
        ),
    )


def _placed_lines(
    told: Telling, transcripts: Mapping[str, Transcript], said: list[tuple[int, str, str]]
) -> list[tuple[int, set[int]]]:
    """Match each line to a line of the story as it was written, and find the words it keeps.

    Returns:
        For each line, the place of its part in ``told.parts``, and the places of the words of
        the part that it keeps.

    Raises:
        StoryError: The lines of a clip are no way of cutting its sentences.
    """
    placed_lines: list[tuple[int, set[int]]] = []
    at = 0
    while at < len(said):
        clip = said[at][1]
        ends = at
        while ends < len(said) and said[ends][1] == clip:
            ends += 1
        parts = [place for place, part in enumerate(told.parts) if part.clip.id == clip]
        placed_lines.extend(_matched_lines(told, transcripts, said[at:ends], parts))
        at = ends
    return placed_lines


def _matched_lines(
    told: Telling,
    transcripts: Mapping[str, Transcript],
    lines: list[tuple[int, str, str]],
    parts: list[int],
) -> list[tuple[int, set[int]]]:
    """Match the lines of a clip to its parts, in order; the parts said earlier go first."""
    known: dict[tuple[int, int], set[int] | None] = {}

    def keeps(i: int, j: int) -> set[int] | None:
        if (i, j) not in known:
            part = told.parts[parts[j]]
            transcript = transcripts[part.clip.media]
            known[i, j] = _kept_of([transcript.text(one.index) for one in part.words], lines[i][2])
        return known[i, j]

    m, n = len(lines), len(parts)
    # fits[i][j]: the lines from i on are some of the parts from j on, in order.
    fits = [[False] * (n + 1) for _ in range(m + 1)]
    for j in range(n + 1):
        fits[m][j] = True
    for i in range(m - 1, -1, -1):
        for j in range(n - 1, -1, -1):
            fits[i][j] = fits[i][j + 1] or (fits[i + 1][j + 1] and keeps(i, j) is not None)
    if not fits[0][0]:
        wrong = next(
            (line for i, line in enumerate(lines) if all(keeps(i, j) is None for j in range(n))),
            lines[0],
        )
        raise StoryError(
            f"line {wrong[0]}: its words are not those of a sentence of clip {wrong[1]} in its "
            "place. Words can be taken away or struck through, and none can be added, changed "
            "or moved"
        )
    matched: list[tuple[int, set[int]]] = []
    i = j = 0
    while i < m:
        if fits[i][j + 1]:
            j += 1
            continue
        matched.append((parts[j], cast("set[int]", keeps(i, j))))
        i += 1
        j += 1
    return matched
