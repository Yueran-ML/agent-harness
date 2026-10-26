"""What was said in a source, word by word, and the sentences that the words make.

**The seam.** A transcriber is given the sound of a source as a transcriber reads it, 16 kHz and
one channel, and gives back the words with their times. Two come with this program: one that
reads words that were written down beforehand, from a file, and one that runs faster-whisper on
this machine, in a process of its own (:mod:`columbina.video.whisper_worker`). faster-whisper is
not a dependency of this program, and neither is its model: the user installs the one and
downloads the other, and the program says how where either is missing. It never downloads a
model by itself.

**Sentences.** A sentence ends with a word that ends with a stop, a question mark or an
exclamation mark, in any script; after a pause of more than :data:`SENTENCE_PAUSE_US`; or after
:data:`SENTENCE_WORDS` words.

**The times** that a transcriber gives are early as a whole, by an amount of its own. The first
word after a silence begins where the silence ends, so of every such word the difference is
taken, and the times of the transcript are moved by the middle one of them.

What is said in a source is something that a file says, and not something that the user asks:
a sentence that tells an agent to do something is a line of the film, and nothing more.
"""

import asyncio
import json
import os
import statistics
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Protocol, cast

from columbina import process
from columbina.video.media import Silence
from columbina.video.timeline import Transcript, Word

__all__ = [
    "SENTENCE_PAUSE_US",
    "SENTENCE_WORDS",
    "Scripted",
    "Transcriber",
    "TranscriptionError",
    "Whisper",
    "fitted",
    "sentences_of",
    "transcript_of",
]

SENTENCE_PAUSE_US = 700_000
"""A pause longer than this ends a sentence."""

SENTENCE_WORDS = 40
"""A sentence ends after this many words, whatever follows."""

# The stops of Latin, those of Chinese and Japanese, which are as wide as a character, and the
# ellipsis.
_STOPS = (".", "!", "?", "。", "！", "？", "…")  # noqa: RUF001

_FOLLOWING_US = 500_000
"""How soon after the end of a silence a word has to begin to be the first one after it."""

_WHISPER_SECONDS = 6 * 60 * 60.0
"""How long a transcription may take: hours, for a long source on a slow machine."""


class TranscriptionError(Exception):
    """A source could not be transcribed. The words say why, and what to do about it."""


class Transcriber(Protocol):
    """Something that writes down what was said."""

    async def words(self, sound: Path, language: str | None) -> list[Word]:
        """Return the words of a sound of 16 kHz and one channel, in order.

        Raises:
            TranscriptionError: It could not be done.
        """
        ...


class Scripted:
    """A transcriber that reads the words from a file of JSON, as they were written down before.

    The file is a list of words, each with ``start_us``, ``end_us`` and ``text``.
    """

    def __init__(self, path: Path) -> None:
        self.path = path

    async def words(self, sound: Path, language: str | None) -> list[Word]:
        """Return the words of the file, whatever the sound."""
        try:
            given: object = json.loads(
                await asyncio.to_thread(self.path.read_text, encoding="utf-8")
            )
            if not isinstance(given, list):
                raise TypeError("it is no list")
            return [Word.model_validate(one) for one in cast("list[object]", given)]
        except (OSError, ValueError, TypeError) as error:
            raise TranscriptionError(f"the words in {self.path} cannot be read: {error}") from None


class Whisper:
    """faster-whisper, run on this machine in a process of its own.

    Args:
        model: A directory that holds a model of faster-whisper, as it was downloaded.
        python: The interpreter that has faster-whisper.
        device: ``cuda``, ``cpu`` or ``auto``.
    """

    def __init__(self, model: Path, python: str = sys.executable, device: str = "auto") -> None:
        self.model = model
        self.python = python
        self.device = device

    async def words(self, sound: Path, language: str | None) -> list[Word]:
        """Have faster-whisper write down the words of a sound.

        Raises:
            TranscriptionError: There is no model, faster-whisper is not installed, or it failed.
        """
        if not await asyncio.to_thread((self.model / "model.bin").is_file):
            raise TranscriptionError(
                f"there is no model of faster-whisper in {self.model}. Download one, for instance "
                "with `huggingface-cli download Systran/faster-whisper-small --local-dir "
                f"{self.model}`"
            )
        argv = [
            self.python,
            "-m",
            "columbina.video.whisper_worker",
            str(sound),
            str(self.model),
            language or "",
            self.device,
        ]
        env = {**process.cleaned(os.environ), "PYTHONUTF8": "1"}
        try:
            done = await process.run(
                argv,
                cwd=sound.parent,
                env=env,
                timeout_seconds=_WHISPER_SECONDS,
                streams="apart",
                max_output_bytes=256 * 1024 * 1024,
            )
        except OSError as error:
            raise TranscriptionError(f"{self.python} could not be started: {error}") from None
        if done.exit_code != 0:
            said = (done.error_output or done.output).strip().splitlines()
            raise TranscriptionError(
                f"faster-whisper failed: {said[-1] if said else done.exit_code}"
            )
        try:
            return [
                Word.model_validate_json(line) for line in done.output.splitlines() if line.strip()
            ]
        except ValueError as error:
            raise TranscriptionError(f"faster-whisper wrote what is no word: {error}") from None


def sentences_of(words: Sequence[Word]) -> tuple[tuple[int, int], ...]:
    """Group words into sentences: where each begins and ends in the words, the end not included."""
    found: list[tuple[int, int]] = []
    begun = 0
    for index, word in enumerate(words):
        last = index + 1 == len(words)
        stops = word.text.rstrip().endswith(_STOPS)
        paused = not last and words[index + 1].start_us - word.end_us > SENTENCE_PAUSE_US
        if last or stops or paused or index + 1 - begun >= SENTENCE_WORDS:
            found.append((begun, index + 1))
            begun = index + 1
    return tuple(found)


def fitted(
    words: Sequence[Word], silences: Sequence[Silence], length_us: int
) -> tuple[list[Word], int]:
    """Move the times of the words so that the first word after each silence begins where it ends.

    Returns:
        The words, moved, and by how much they were moved.
    """
    gaps: list[int] = []
    for silence in silences:
        following = next((word for word in words if word.start_us >= silence.start_us), None)
        if following is not None and abs(following.start_us - silence.end_us) <= _FOLLOWING_US:
            gaps.append(silence.end_us - following.start_us)
    if not gaps:
        return list(words), 0
    shift = round(statistics.median(gaps))

    def moved(us: int) -> int:
        return min(max(0, us + shift), length_us)

    return [
        word.model_copy(update={"start_us": moved(word.start_us), "end_us": moved(word.end_us)})
        for word in words
    ], shift


def transcript_of(
    asset: str,
    digest: str,
    words: Sequence[Word],
    silences: Sequence[Silence],
    length_us: int,
    language: str | None = None,
) -> Transcript:
    """Make the transcript of a source out of the words that a transcriber gave.

    Words without a letter or a digit in them, and words that end before they begin, are kept:
    they are what was said. Their text is trimmed.
    """
    trimmed = [
        word.model_copy(update={"text": word.text.strip()}) for word in words if word.text.strip()
    ]
    ordered = sorted(trimmed, key=lambda word: word.start_us)
    moved, shift = fitted(ordered, silences, length_us)
    return Transcript(
        asset=asset,
        digest=digest,
        language=language,
        offset_us=shift,
        words=tuple(moved),
        sentences=sentences_of(moved),
    )
