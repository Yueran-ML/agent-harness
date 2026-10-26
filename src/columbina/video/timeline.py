"""The timeline of a film, and the stretches of the sources that it is cut from.

**Words.** A transcript is a list of words of one source, each with where it begins and ends
in the source, and the sentences that the words make. A word is known by its place in the list.

**The main line** is the talk: a list of sentences in the order in which they are heard, each
with the words of it that are kept. It holds no times. The stretches that are played are worked
out of the words every time, :func:`cuts`, so the story and the timeline never say different
things.

**Where a cut falls.** A transcriber puts the end of a word early, by up to half a second: a cut
there would take the end of the word with it. So a cut between two words falls into the silence
between them where there is one, and keeps a little of it on either side. Where the two are
spoken without a pause, the cut falls where the second begins, and is said to be tight: it may
be heard.

**Pauses.** Between two sentences that follow each other in the source, the pause is the silence
between them, no longer than the longest pause of the timeline. A sentence may name its own pause.
Between two sentences that do not follow each other, the pause is what is left of the silence
on both sides of the cuts.

**What lies over the main line** is in the layers, laid by :mod:`columbina.video.layers`.

Every time here is in microseconds. The stretches are put on frames at the end, by the rate of
the film.
"""

from dataclasses import dataclass
from fractions import Fraction
from typing import Literal

from pydantic import Field

from columbina.data import Data
from columbina.video.media import Silence

__all__ = [
    "LEAD_US",
    "PAUSE_LEAST_US",
    "PAUSE_MOST_US",
    "Cut",
    "Entry",
    "Layer",
    "Timeline",
    "Transcript",
    "Word",
    "cuts",
    "frames",
    "kept_words",
]

LEAD_US = 120_000
"""How much of a silence is kept before a word that a cut leads into, and after one it leaves."""

PAUSE_LEAST_US = 300_000
"""How long a pause is that was taken out of the story."""

PAUSE_MOST_US = 1_000_000
"""How long a pause is at most, unless the timeline says otherwise."""


class Word(Data):
    """A word that was said.

    Attributes:
        start_us: Where it begins in the source.
        end_us: Where it ends.
        text: The word, as it is shown.
        confidence: How sure the transcriber was of it, from 0 to 1.
    """

    start_us: int
    end_us: int
    text: str
    confidence: float = 1.0


class Transcript(Data):
    """What was said in a source.

    Attributes:
        asset: The source.
        digest: The digest of the source that it was made of.
        language: The language, as the transcriber found it.
        offset_us: How far the times of the transcriber were moved to fit the pauses.
        words: The words, in the order in which they were said.
        sentences: Where each sentence begins and ends in the words, the end not included.
        shown: Words whose text was corrected, by their place: what is shown instead.
    """

    asset: str
    digest: str
    language: str | None = None
    offset_us: int = 0
    words: tuple[Word, ...]
    sentences: tuple[tuple[int, int], ...]
    shown: dict[int, str] = Field(default_factory=dict[int, str])

    def text(self, index: int) -> str:
        """Return the text of a word as it is shown."""
        return self.shown.get(index, self.words[index].text)


class Entry(Data):
    """A sentence of the main line.

    Attributes:
        asset: The source that it was said in.
        sentence: Which sentence of its transcript it is.
        kept: The words of it that are kept, by their places in the transcript, in order.
        pause_us: How long the pause after it lasts, or ``None`` for the pause that the
            source has, no longer than the longest pause of the timeline.
    """

    asset: str
    sentence: int
    kept: tuple[int, ...]
    pause_us: int | None = None


class Layer(Data):
    """Something that lies over the main line or under it: a picture, a sound or a title.

    Attributes:
        id: Its id, for a change.
        kind: ``picture`` over the film, ``sound`` under it, or ``title``.
        asset: The source that it shows or plays; none for a title.
        text: The words of a title.
        anchor: Where it begins: the source and the place of the word of the main line that
            it begins with. ``None`` for a layer that begins at a frame.
        until: The word of the main line that it lasts until, its end included.
        at_frame: The frame where it begins, if it has no anchor.
        frames: How many frames it lasts, if it does not last until a word.
        source_us: Where in its source it begins.
        position: Where it stands over the film: ``full``, or a corner.
        scale: How large it is beside the film, from 0 to 1.
        opacity: How much of it is seen, from 0 to 1.
        volume: How loud its sound is, 1 as it is.
        fade_us: How long it fades in and out.
        speed: How fast it plays, 1 as it is.
        floating: Whether its anchor was cut away, so that it lies nowhere.
    """

    id: str
    kind: Literal["picture", "sound", "title"]
    asset: str | None = None
    text: str | None = None
    anchor: tuple[str, int] | None = None
    until: tuple[str, int] | None = None
    at_frame: int | None = Field(default=None, ge=0)
    frames: int | None = Field(default=None, ge=1)
    source_us: int = Field(default=0, ge=0)
    position: Literal["full", "top-left", "top-right", "bottom-left", "bottom-right", "center"] = (
        "full"
    )
    scale: float = Field(default=1.0, gt=0, le=1)
    opacity: float = Field(default=1.0, ge=0, le=1)
    volume: float = Field(default=1.0, ge=0, le=4)
    fade_us: int = Field(default=0, ge=0)
    speed: float = Field(default=1.0, ge=0.5, le=2)
    floating: bool = False


class Timeline(Data):
    """The film, as it was committed last.

    Attributes:
        version: The version of this file.
        main: The main line, in the order in which it is heard.
        layers: What lies over it and under it.
        crossfades: How many frames two neighbouring stretches of the main line cross into each
            other, by the place of the first of the two.
        pause_most_us: How long a pause of the source lasts at most.
        captions: Whether the words of the main line are shown under the film.
        duck: Whether a sound under the film is made quieter while somebody speaks.
    """

    version: Literal[1] = 1
    main: tuple[Entry, ...] = ()
    layers: tuple[Layer, ...] = ()
    crossfades: dict[int, int] = Field(default_factory=dict[int, int])
    pause_most_us: int = PAUSE_MOST_US
    captions: bool = True
    duck: bool = True


@dataclass(frozen=True)
class Cut:
    """A stretch of a source that is played, on the timeline of the film.

    Attributes:
        asset: The source.
        start_us: Where it begins in the source.
        end_us: Where it ends in the source.
        entry: The place of the sentence of the main line that it belongs to.
        tight: Whether it begins where a word was spoken without a pause before it.
    """

    asset: str
    start_us: int
    end_us: int
    entry: int
    tight: bool = False


def kept_words(transcript: Transcript, sentence: int) -> tuple[int, ...]:
    """Every word of a sentence, which is what a sentence that was never cut keeps."""
    first, last = transcript.sentences[sentence]
    return tuple(range(first, last))


def _silence_between(
    silences: tuple[Silence, ...], after_us: int, before_us: int
) -> Silence | None:
    """The longest silence that lies between the end of one word and the beginning of the next.

    A transcriber puts ends early, so a silence counts that begins before the end of the first
    word, as long as it ends after it and no later than the next word begins, give or take.
    """
    found = [
        one
        for one in silences
        if one.end_us > after_us and one.start_us < before_us and one.end_us <= before_us + LEAD_US
    ]
    return max(found, key=lambda one: one.end_us - one.start_us, default=None)


def _runs(kept: tuple[int, ...]) -> list[tuple[int, int]]:
    """Group the places of the kept words into runs of neighbours, each as first and last."""
    runs: list[tuple[int, int]] = []
    for index in kept:
        if runs and runs[-1][1] == index - 1:
            runs[-1] = (runs[-1][0], index)
        else:
            runs.append((index, index))
    return runs


def _begins(transcript: Transcript, silences: tuple[Silence, ...], first: int) -> tuple[int, bool]:
    """Where a stretch that begins with a word begins in its source, and whether that is tight."""
    word = transcript.words[first]
    if first == 0:
        silence = _silence_between(silences, 0, word.start_us)
        if silence is not None:
            return max(silence.start_us, silence.end_us - LEAD_US), False
        return max(0, word.start_us - LEAD_US), False
    before = transcript.words[first - 1]
    silence = _silence_between(silences, before.end_us, word.start_us)
    if silence is None:
        return word.start_us, True
    return max(silence.start_us, silence.end_us - LEAD_US), False


def _ends(transcript: Transcript, silences: tuple[Silence, ...], last: int, length_us: int) -> int:
    """Where a stretch that ends with a word ends in its source."""
    word = transcript.words[last]
    if last + 1 == len(transcript.words):
        silence = _silence_between(silences, word.end_us, length_us)
        if silence is not None:
            return min(silence.end_us, silence.start_us + LEAD_US)
        return min(length_us, word.end_us + LEAD_US)
    after = transcript.words[last + 1]
    silence = _silence_between(silences, word.end_us, after.start_us)
    if silence is None:
        return after.start_us
    return min(silence.end_us, silence.start_us + LEAD_US)


def cuts(
    timeline: Timeline,
    transcripts: dict[str, Transcript],
    silences: dict[str, tuple[Silence, ...]],
    lengths: dict[str, int],
) -> list[Cut]:
    """Work out the stretches of the sources that the main line plays, in order.

    Args:
        timeline: The timeline.
        transcripts: The transcript of each source of the main line.
        silences: Where each of those sources is silent.
        lengths: How long each of them lasts.
    """
    played: list[Cut] = []
    for place, entry in enumerate(timeline.main):
        transcript = transcripts[entry.asset]
        quiet = silences.get(entry.asset, ())
        for first, last in _runs(entry.kept):
            start, tight = _begins(transcript, quiet, first)
            end = _ends(transcript, quiet, last, lengths[entry.asset])
            played.append(Cut(entry.asset, start, max(end, start), place, tight))
    return _paused(timeline, played, transcripts, silences)


def _paused(
    timeline: Timeline,
    played: list[Cut],
    transcripts: dict[str, Transcript],
    silences: dict[str, tuple[Silence, ...]],
) -> list[Cut]:
    """Join the stretches of sentences that follow each other in a source, with their pause.

    The pause that the source has between them is kept, no longer than the pause of the first
    sentence; a longer one is shortened in its middle.
    """
    joined: list[Cut] = []
    for cut in played:
        before = joined[-1] if joined else None
        if before is None or before.asset != cut.asset or before.entry == cut.entry:
            joined.append(cut)
            continue
        entry = timeline.main[before.entry]
        nearby = timeline.main[cut.entry]
        last, first = entry.kept[-1] if entry.kept else -1, nearby.kept[0] if nearby.kept else -1
        if first != last + 1:
            joined.append(cut)
            continue
        transcript = transcripts[cut.asset]
        silence = _silence_between(
            silences.get(cut.asset, ()),
            transcript.words[last].end_us,
            transcript.words[first].start_us,
        )
        allowed = timeline.pause_most_us if entry.pause_us is None else entry.pause_us
        if silence is None or silence.end_us - silence.start_us <= allowed:
            joined[-1] = Cut(before.asset, before.start_us, cut.end_us, before.entry, before.tight)
            continue
        half = allowed // 2
        joined[-1] = Cut(
            before.asset, before.start_us, silence.start_us + half, before.entry, before.tight
        )
        joined.append(
            Cut(cut.asset, silence.end_us - (allowed - half), cut.end_us, cut.entry, cut.tight)
        )
    return joined


def frames(us: int, rate: str) -> int:
    """Turn a time into the number of the frame it falls in, at a rate such as ``30000/1001``."""
    top, _, bottom = rate.partition("/")
    return round(Fraction(us, 1_000_000) * Fraction(int(top), int(bottom)))
