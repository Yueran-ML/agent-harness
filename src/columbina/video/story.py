"""The story: what is said in a film as text, which a person or a model edits, and what it means.

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
- A line is the words of a sentence that one clip plays. A blade through a word displays it
  once, on the clip containing its middle, and cutting it removes all its continuous pieces.
  A trimmed word with only a head or tail remaining stays visible. A sentence cut between
  words becomes two lines, one of each clip. Keep the time marker: it distinguishes repeated
  sentences. Lines within the same second use milliseconds. A clip that plays backward says
  its last sentence first.
- A word that is taken away or struck through with ``~~`` is cut, and so is every word of a line
  that is taken away; words and lines can be neither added, changed nor moved. A word without
  letters or digits, as ``—``, ``♪`` or ``...``, goes with the nearest word of its line that has
  them: the one before it, or the one after it where it begins the line. A line of such words
  alone is cut where it is taken away, emptied or struck through, and kept otherwise. A story
  that is not changed cuts nothing.
- ``[pause 1.8s]`` is the pause after a line, where it is longer than :data:`PAUSE_SHOWN_US`. A
  smaller number makes it shorter; taking the line away makes it
  :data:`~columbina.video.transcript.PAUSE_LEAST_US`.
- A line that begins with ``>`` is a note, as for a clip whose media has no transcript.

What a story cuts is worked out by :mod:`columbina.video.textcut`. Where a clip reads its media
is counted as the engine counts it, :mod:`columbina.video.span`.

"""

import hashlib
import json
import re
import unicodedata
from bisect import bisect_left
from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from fractions import Fraction
from pathlib import Path
from typing import Any, cast

from pydantic import ValidationError

from columbina.video.edits import Rate
from columbina.video.span import Placed, frames_showing, placed, source_span
from columbina.video.transcript import Transcript

__all__ = [
    "PAUSE_SHOWN_US",
    "STORIES",
    "Marked",
    "Part",
    "Spoken",
    "StaleStoryError",
    "StoryError",
    "Telling",
    "dialogue_track",
    "line_of",
    "marked",
    "normal",
    "of_track",
    "story_path",
    "telling",
]
PAUSE_SHOWN_US = 800_000
_PAUSE = re.compile(r"^\[pause\s+(\d+(?:\.\d+)?)\s*s\]$")
_DIGEST = 12


class StoryError(Exception):
    """A story cannot be committed. The words say where, and why."""


def normal(text: str) -> str:
    """The letters, digits and characters of a text, in small letters: what words are matched by."""
    folded = unicodedata.normalize("NFKC", text).casefold()
    return "".join(character for character in folded if character.isalnum())


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


def _struck_out(text: str) -> bool:
    """Whether nothing of a line is left: it is empty, or all that it says is struck through."""
    struck = False
    for piece in re.split(r"(~~)", text):
        if piece == "~~":
            struck = not struck
        elif not struck and piece.strip():
            return False
    return True


def _kept_of(shown: Sequence[str], text: str) -> set[int] | None:
    """Find which of some words, by their places, a line keeps; ``None`` if it cannot be them.

    Each word is cut, struck through, or kept as it is. A word without letters or digits, as
    ``—`` or ``♪``, follows the nearest word with them in its line: the one before it, or the
    one after it where it begins the line. A line of such words alone is kept unless it is
    emptied or struck through. Where there is more than one way, the words said earlier are cut.
    """
    words = [(place, normal(one)) for place, one in enumerate(shown)]
    counted = [(place, said) for place, said in words if said]
    line = _marked(text)
    if not counted:
        if line:
            return None
        return set() if _struck_out(text) else {place for place, _ in words}
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
    places = [place for place, _ in counted]
    for place, said in words:
        if not said:
            before = bisect_left(places, place)
            if places[max(0, before - 1)] in kept:
                kept.add(place)
    return kept


STORIES = "story"
"""The directory of a project of the engine that the stories of its timelines are kept in."""

_TRACK_HEAD = re.compile(r"^<!--\s*columbina story:\s*(\S+)\s+(\d+)\s+([0-9a-f]{12})\s*-->$")
_TRACK_LINE = re.compile(r"^\[([^\s\]]+)(?:\s+([\d:.]+))?\]\s?(.*)$")
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
        owners: For each audible (clip, word), the clip whose line displays that word.
            A blade through a word retains one displayed word for the continuous fragments.
        silent: The clips whose media has no transcript, each with its media.
        digest: The digest of the timeline, the track and the words that the story tells of.
    """

    timeline: str
    track: str
    rate: Rate
    locked: bool
    clips: tuple[Placed, ...]
    parts: tuple[Part, ...]
    owners: Mapping[tuple[str, int], str]
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

    That is the timeline, the track, and the words, their times and sentence boundaries.
    The media digest alone cannot detect a new alignment of the same recording: accepting
    an old story then would cut different frames from those the user edited.
    """
    media = sorted({clip.media for clip in clips if clip.media in transcripts})
    words = [
        [
            one,
            transcripts[one].digest,
            [(word.start_us, word.end_us, word.text) for word in transcripts[one].words],
            transcripts[one].sentences,
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


def _continuous(left: Placed, right: Placed, rate: Rate) -> bool:
    """Whether the next clip continues the same source at the same speed and direction."""
    earlier, later = (right, left) if left.reverse else (left, right)
    return (
        left.end == right.start
        and left.media == right.media
        and left.speed == right.speed
        and left.reverse == right.reverse
        and Fraction(earlier.begins.ticks + source_span(earlier, rate), earlier.begins.scale)
        == Fraction(later.begins.ticks, later.begins.scale)
    )


def _speech(
    clips: Sequence[Placed],
    transcripts: Mapping[str, Transcript],
    rate: Rate,
) -> tuple[dict[tuple[str, int], str], dict[str, list[Spoken]]]:
    """Keep every audible fragment, displaying each word once per continuous source run.

    The midpoint chooses the displayed clip when it exists. If trimming left only a tail
    or head, display it on the clip containing the largest remaining piece. Repeated takes
    and nonadjacent excerpts are separate runs, so striking one never removes the other.
    """
    runs: list[list[Placed]] = []
    for clip in clips:
        if runs and _continuous(runs[-1][-1], clip, rate):
            runs[-1].append(clip)
        else:
            runs.append([clip])
    owners: dict[tuple[str, int], str] = {}
    spoken: dict[str, list[Spoken]] = {clip.id: [] for clip in clips}
    for run in runs:
        transcript = transcripts.get(run[0].media)
        if transcript is None:
            continue
        pieces: dict[int, list[tuple[Placed, Spoken]]] = {}
        middles: dict[int, str] = {}
        for clip in run:
            middle = set(_owned(transcript, clip, rate))
            for index, word in enumerate(transcript.words):
                first, end = frames_showing(
                    clip, word.start_us, max(word.start_us, word.end_us), rate
                )
                if end > first or index in middle:
                    pieces.setdefault(index, []).append(
                        (clip, Spoken(index, clip.start + first, clip.start + end))
                    )
                if index in middle:
                    middles[index] = clip.id
        for index, fragments in sorted(pieces.items()):
            owner = middles.get(index)
            if owner is None:
                owner = max(fragments, key=lambda item: item[1].end - item[1].first)[0].id
            for clip, _ in fragments:
                owners[clip.id, index] = owner
            spoken[owner].append(
                Spoken(
                    index,
                    min(word.first for _, word in fragments),
                    max(word.end for _, word in fragments),
                )
            )
    return owners, spoken


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
    owners, said = _speech(media, transcripts, rate)
    parts: list[Part] = []
    silent: list[tuple[str, str]] = []
    for clip in media:
        transcript = transcripts.get(clip.media)
        if transcript is None:
            silent.append((clip.id, clip.media))
            continue
        spoken = said[clip.id]
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
        owners=owners,
        silent=tuple(silent),
        digest=_digest(timeline, track, media, transcripts),
    )


def _timed(frame: int, rate: Rate) -> str:
    return _clock(frame * rate.den * 1_000_000 // rate.num)


def _line_times(told: Telling) -> list[str]:
    """Time markers precise enough to distinguish lines of one clip in the same second."""
    coarse = [_timed(part.first, told.rate) for part in told.parts]
    counts = Counter((part.clip.id, time) for part, time in zip(told.parts, coarse, strict=True))
    return [
        f"{time}.{part.first * told.rate.den * 1000 // told.rate.num % 1000:03d}"
        if counts[part.clip.id, time] > 1
        else time
        for part, time in zip(told.parts, coarse, strict=True)
    ]


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
    times = _line_times(told)
    for place, part in enumerate(told.parts):
        transcript = transcripts[part.clip.media]
        shown = line_of([(transcript.text(one.index), False) for one in part.words])
        lines.append(f"[{part.clip.id} {times[place]}] {shown}")
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
    clip are matched to its sentences in order, by their words and the time markers that they
    keep; where they could be matched in more than one way, as where a sentence said twice lost
    its time marker, the story is refused. Words can be neither added, changed nor moved, and
    neither can lines. ``[pause 1.2s]`` after a line is how long the pause after it is to be:
    shorter than it is, or it stays as it is. Taking away a pause that the story named when it
    was written makes it :data:`PAUSE_LEAST_US` long. A pause that is left where a line before
    it was taken away is the pause after the line before that; one before every line is passed
    over.

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
    said: list[tuple[int, str, str, str | None]] = []
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
        said.append((where, clip, found[3], found[2]))
    places = _placed_lines(told, transcripts, said)
    kept: set[tuple[str, int]] = set()
    for (_, clip, _, _), (place, keeps) in zip(said, places, strict=True):
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
    told: Telling,
    transcripts: Mapping[str, Transcript],
    said: list[tuple[int, str, str, str | None]],
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
    lines: list[tuple[int, str, str, str | None]],
    parts: list[int],
) -> list[tuple[int, set[int]]]:
    """Match lines in order by their words and retained time markers; refuse ambiguity."""
    known: dict[tuple[int, int], set[int] | None] = {}
    times = _line_times(told)

    def keeps(i: int, j: int) -> set[int] | None:
        if (i, j) not in known:
            part = told.parts[parts[j]]
            transcript = transcripts[part.clip.media]
            anchor = lines[i][3]
            known[i, j] = (
                _kept_of([transcript.text(one.index) for one in part.words], lines[i][2])
                if anchor is None or anchor == times[parts[j]]
                else None
            )
        return known[i, j]

    m, n = len(lines), len(parts)
    # Count ordered matches, capped at two: guessing between repeated sentences can remove
    # the wrong take. With no remaining lines, omitting the remaining parts is one match.
    fits = [[0] * (n + 1) for _ in range(m + 1)]
    for j in range(n + 1):
        fits[m][j] = 1
    for i in range(m - 1, -1, -1):
        for j in range(n - 1, -1, -1):
            fits[i][j] = min(
                2, fits[i][j + 1] + (fits[i + 1][j + 1] if keeps(i, j) is not None else 0)
            )
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
    if fits[0][0] > 1:
        raise StoryError(
            "the remaining lines are ambiguous: keep their original time markers, "
            "or write the story again before removing repeated sentences"
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
