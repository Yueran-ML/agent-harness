"""Cutting by the words: what is struck out of the story of a track becomes ranges of frames.

The story of a track (:func:`columbina.video.story.of_track`) shows the words that its clips
play. What is struck out of it, and the pauses that it makes shorter, become ranges of frames of
the timeline, :func:`cuts`; each range is cut out of every track that is not locked by a
``ripple_delete_range`` edit, :func:`edits_of`, so that what lies over the words is cut with
them and stays in step. The edits of one story are one transaction, and one step to undo.

**Where a cut falls.** The words of a clip are where its media has them, and a range is turned
into the frames of the clip that show that stretch of its media, at the speed of the clip and in
its direction, as the engine counts them (:mod:`columbina.video.span`). A run of words that are
struck out is cut from the end of the word before it, and :data:`LEAD_US` of the silence after
that word, to the start of the word after it, less as much: a little of the silence is left on
either side. Where the words follow each other more closely, the cut keeps the words that are
left whole. A run at the start of a clip is cut from the start of the clip, and one at its end
to its end, unless a word that another clip plays lies there.

**Pauses.** Where the story names a pause shorter than the one between two lines, or a pause
that it named is taken away (:data:`~columbina.video.timeline.PAUSE_LEAST_US`), the middle of
the pause is cut, so that the pause lasts that long. ``pause_most_ms`` makes every other pause
between two words that are left no longer than it.

**In order.** The ranges are merged where they meet, and the edits are written from the last to
the first, so that a range that is cut does not move the frames of those before it.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from fractions import Fraction
from itertools import pairwise
from typing import Any

from columbina.video import edits
from columbina.video.edits import Rate
from columbina.video.span import Placed, frames_showing, source_span
from columbina.video.story import Marked, StoryError, Telling, marked
from columbina.video.timeline import LEAD_US, PAUSE_LEAST_US, Transcript

__all__ = ["LEAD_US", "Range", "cuts", "edits_of", "merged"]


@dataclass(frozen=True, order=True)
class Range:
    """Frames of a timeline, from ``start`` to ``end``, the end not included.

    Attributes:
        start: The first frame.
        end: The frame after the last.
    """

    start: int
    end: int

    @property
    def frames(self) -> int:
        """How many frames it is."""
        return self.end - self.start


def merged(ranges: Sequence[Range]) -> list[Range]:
    """The ranges in order, those that meet or overlap made one, and those of no frame left out."""
    joined: list[Range] = []
    for one in sorted(ranges):
        if one.end <= one.start:
            continue
        if joined and one.start <= joined[-1].end:
            joined[-1] = Range(joined[-1].start, max(joined[-1].end, one.end))
        else:
            joined.append(one)
    return joined


def edits_of(timeline: str, ranges: Sequence[Range]) -> list[dict[str, Any]]:
    """The edits that cut ranges out of every unlocked track, the last first, as one transaction."""
    return [
        edits.ripple_delete_range(timeline, one.start, one.end) for one in reversed(merged(ranges))
    ]


def cuts(
    story: str,
    timeline: Mapping[str, Any],
    transcripts: Mapping[str, Transcript],
    *,
    pause_most_ms: int | None = None,
) -> list[Range]:
    """Work out the frames that a story of a track cuts out of its timeline, in order.

    Args:
        story: The story, as a person or a model changed it.
        timeline: The timeline, as the engine writes it in the project now.
        transcripts: The transcript of each media that has one, by the id of the media.
        pause_most_ms: How long a pause between two words that are left lasts at most, where
            the story does not name its own.

    Raises:
        StaleStoryError: The timeline, or the words of its media, changed since the story was
            written.
        StoryError: The story is not understood, or its track is locked, so that nothing would
            be cut of it.
    """
    said = marked(story, timeline, transcripts)
    told = said.telling
    if told.locked:
        raise StoryError(
            f"track {told.track} is locked, and a range is cut out of the tracks that are not: "
            "unlock it first"
        )
    found: list[Range] = []
    for clip in told.clips:
        transcript = transcripts.get(clip.media)
        if transcript is not None:
            found.extend(_struck(said, clip, transcript, told.rate))
    found.extend(_paused(said, pause_most_ms))
    return merged(found)


def _struck(marks: Marked, clip: Placed, transcript: Transcript, rate: Rate) -> list[Range]:
    """The ranges of a clip that its words struck out of the story take."""
    owned = sorted(index for owner_clip, index in marks.telling.owners if owner_clip == clip.id)
    mine = set(owned)
    runs: list[list[int]] = []
    for index in owned:
        if (marks.telling.owners[clip.id, index], index) in marks.kept:
            continue
        if runs and runs[-1][-1] + 1 == index:
            runs[-1].append(index)
        else:
            runs.append([index])
    scale = clip.begins.scale
    begins = Fraction(clip.begins.ticks * 1_000_000, scale)
    ends = Fraction((clip.begins.ticks + source_span(clip, rate)) * 1_000_000, scale)
    words = transcript.words
    found: list[Range] = []
    for run in runs:
        first, last = words[run[0]], words[run[-1]]
        before = run[0] - 1
        if before in mine:
            said = words[before].end_us
            start = Fraction(max(said, min(said + LEAD_US, first.start_us)))
        else:
            said = words[before].end_us if before >= 0 else begins
            start = min(Fraction(first.start_us), max(begins, Fraction(said)))
        after = run[-1] + 1
        if after in mine:
            said = words[after].start_us
            end = Fraction(min(said, max(said - LEAD_US, last.end_us)))
        else:
            said = words[after].start_us if after < len(words) else ends
            end = max(Fraction(last.end_us), min(ends, Fraction(said)))
        shown = frames_showing(clip, start, end, rate)
        found.append(Range(clip.start + shown[0], clip.start + shown[1]))
    return found


def _paused(marks: Marked, pause_most_ms: int | None) -> list[Range]:
    """The middles of the pauses that are made shorter, as ranges."""
    told: Telling = marks.telling
    left = sorted(
        (one.first, one.end, place)
        for place, part in enumerate(told.parts)
        for one in part.words
        if (part.clip.id, one.index) in marks.kept and one.end > one.first
    )
    most = None if pause_most_ms is None else pause_most_ms * 1000
    found: list[Range] = []
    for (_, end, place), (first, _, following) in pairwise(left):
        allowed = most
        if place != following:
            if place in marks.paused:
                allowed = marks.paused[place]
            elif place in marks.unpaused:
                allowed = PAUSE_LEAST_US
        if allowed is None:
            continue
        frames = round(Fraction(allowed * told.rate.num, told.rate.den * 1_000_000))
        if first - end > frames:
            kept = frames // 2
            found.append(Range(end + kept, first - (frames - kept)))
    return found
