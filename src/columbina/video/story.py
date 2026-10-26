"""The story: the main line as text, which a person or a model edits, and what an edit means.

The story is a file of the project, ``story.md``. It is written from the timeline, and read back
when it is committed:

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

import re
import unicodedata
from dataclasses import dataclass
from fractions import Fraction

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
    "Story",
    "StoryError",
    "label",
    "line_of",
    "natural_pause",
    "normal",
    "read",
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
    words = [(index, normal(transcript.text(index))) for index in range(first, last)]
    counted = [(index, said) for index, said in words if said]
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
        raise StoryError(
            f"line {where}: its words are not those of {label(transcript.asset, sentence)}, "
            f'which says "{" ".join(transcript.text(index) for index in range(first, last))}". '
            "Words can be taken away or struck through, and none can be added, changed or moved"
        )
    kept: set[int] = set()
    j = 0
    for i, (index, said) in enumerate(counted):
        if able[i + 1][j]:
            continue
        if not line[j][1]:
            kept.add(index)
        j += len(said)
    with_marks = [index for index, said in words if said == "" and index - 1 in kept]
    return tuple(sorted(kept | set(with_marks)))


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
