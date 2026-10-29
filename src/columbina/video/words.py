"""What was said in media, and where it is silent, kept beside a project of the engine.

**Where.** The analysis of media is kept in ``analysis/`` of the directory of the project of the
engine, by the digest of the media (the ``digest`` of its identity, as the engine writes it and
:func:`columbina.video.project.identity_of` makes it), so that it holds for the same file under
any id and in any timeline, and is found again once the file is taken in again:

- ``analysis/words-<digest16>.json``: the words, with the sentences that they make, and the words
  that are shown otherwise since they were heard wrong (:class:`Words`).
- ``analysis/silences-<digest16>-<db>-<ms>.json``: where it is silent, below ``db`` decibels for
  at least ``ms`` milliseconds, as pairs of microseconds.

``<digest16>`` is the first 16 hexadecimal digits of the digest. Neither file is a part of the
project: the engine does not read them, and an undo leaves them as they are. A correction of a
word (``video_correct``) is written into the words, and never into the project.

Every function here blocks.
"""

import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, Literal, cast

from pydantic import Field, ValidationError

from columbina.data import Data
from columbina.video.media import Silence
from columbina.video.project import written
from columbina.video.story import line_of, normal
from columbina.video.transcript import Transcript, Word

__all__ = [
    "ANALYSIS",
    "Words",
    "WordsError",
    "corrected",
    "keep_silences",
    "keep_words",
    "kept_silences",
    "kept_words",
    "silences_path",
    "transcripts_of",
    "words_path",
]

ANALYSIS = "analysis"
"""The directory of a project of the engine that the analysis of its media is kept in."""

_DIGITS = 16


class WordsError(Exception):
    """The words of media cannot be read or changed. The words say why, for a person."""


class Words(Data):
    """What was said in media, as it is kept.

    Attributes:
        version: The version of this file.
        digest: The digest of the media that the words were written down from.
        language: The language, as the transcriber found it.
        offset_us: How far the times of the transcriber were moved to fit the pauses.
        words: The words, in the order in which they were said, with their times in the media.
        sentences: Where each sentence begins and ends in the words, the end not included.
        shown: Words whose text was corrected, by their place: what is shown instead.
    """

    version: Literal[1] = 1
    digest: str
    language: str | None = None
    offset_us: int = 0
    words: tuple[Word, ...]
    sentences: tuple[tuple[int, int], ...]
    shown: dict[int, str] = Field(default_factory=dict[int, str])

    @classmethod
    def of(cls, transcript: Transcript) -> "Words":
        """The words of a transcript, as they are kept."""
        return cls(
            digest=transcript.digest,
            language=transcript.language,
            offset_us=transcript.offset_us,
            words=transcript.words,
            sentences=transcript.sentences,
            shown=transcript.shown,
        )

    def transcript(self, media: str) -> Transcript:
        """The words as the transcript of ``media``, the id that the media has in a project."""
        return Transcript(
            asset=media,
            digest=self.digest,
            language=self.language,
            offset_us=self.offset_us,
            words=self.words,
            sentences=self.sentences,
            shown=self.shown,
        )


def _is_digest(digest: str) -> bool:
    head = digest[:_DIGITS]
    return len(head) == _DIGITS and all(character in "0123456789abcdef" for character in head)


def _digits(digest: str) -> str:
    """The digits of a digest that the files are named by.

    Raises:
        WordsError: It is no digest: a project may come from anywhere.
    """
    if not _is_digest(digest):
        raise WordsError(f"{digest!r} is no digest of media")
    return digest[:_DIGITS]


def words_path(directory: Path, digest: str) -> Path:
    """Where the words of the media of a digest are kept, in the project of ``directory``.

    Raises:
        WordsError: It is no digest.
    """
    return directory / ANALYSIS / f"words-{_digits(digest)}.json"


def silences_path(directory: Path, digest: str, noise_db: float, shortest_us: int) -> Path:
    """Where the silences of the media of a digest are kept, found with a threshold.

    Raises:
        WordsError: It is no digest.
    """
    shortest_ms = shortest_us // 1000
    return directory / ANALYSIS / f"silences-{_digits(digest)}-{noise_db:g}-{shortest_ms}.json"


def kept_words(directory: Path, digest: str) -> Words | None:
    """The words of the media of a digest, if they were written down.

    Raises:
        WordsError: It is no digest, or the file cannot be read, or holds no words of it.
    """
    path = words_path(directory, digest)
    try:
        text = path.read_bytes()
    except FileNotFoundError:
        return None
    except OSError as error:
        raise WordsError(f"{path} cannot be read: {error}") from None
    try:
        words = Words.model_validate_json(text)
    except ValidationError as error:
        raise WordsError(f"{path} is not what this program writes: {error}") from None
    if words.digest[:_DIGITS] != _digits(digest):
        raise WordsError(f"{path} holds the words of other media, {words.digest[:12]}")
    return words


def keep_words(directory: Path, words: Words) -> Path:
    """Keep the words of media, in place of those that were kept, and return where.

    Raises:
        WordsError: Its digest is no digest.
        ProjectError: The file could not be written.
    """
    path = words_path(directory, words.digest)
    written(path, words.model_dump_json(indent=1) + "\n")
    return path


def kept_silences(
    directory: Path, digest: str, noise_db: float, shortest_us: int
) -> tuple[Silence, ...] | None:
    """Where the media of a digest is silent, if it was found with that threshold.

    Raises:
        WordsError: It is no digest, or the file cannot be read.
    """
    path = silences_path(directory, digest, noise_db, shortest_us)
    try:
        said: object = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError) as error:
        raise WordsError(f"{path} cannot be read: {error}") from None
    pairs = cast("list[object]", said) if isinstance(said, list) else [None]
    if not all(_pair(one) for one in pairs):
        raise WordsError(f"{path} is not what this program writes: no list of pairs of times")
    return tuple(Silence(*cast("list[int]", one)) for one in pairs)


def _pair(said: object) -> bool:
    return (
        isinstance(said, list)
        and len(cast("list[object]", said)) == 2
        and all(type(one) is int for one in cast("list[object]", said))
    )


def keep_silences(
    directory: Path,
    digest: str,
    noise_db: float,
    shortest_us: int,
    silences: Sequence[Silence],
) -> Path:
    """Keep where media is silent, found with a threshold, and return where.

    Raises:
        WordsError: It is no digest.
        ProjectError: The file could not be written.
    """
    path = silences_path(directory, digest, noise_db, shortest_us)
    written(path, json.dumps([[one.start_us, one.end_us] for one in silences]) + "\n")
    return path


def transcripts_of(directory: Path, project: Mapping[str, Any]) -> dict[str, Transcript]:
    """The transcripts of the media of a project of the engine that has words kept, by its id.

    Media whose identity names no digest has none.

    Raises:
        WordsError: Words that are kept cannot be read.
    """
    found: dict[str, Transcript] = {}
    for media in project.get("media", []):
        digest = media.get("identity", {}).get("digest")
        if not isinstance(digest, str) or not _is_digest(digest):
            continue
        words = kept_words(directory, digest)
        if words is not None:
            found[str(media["id"])] = words.transcript(str(media["id"]))
    return found


def corrected(directory: Path, digest: str, sentence: int, wrong: str, right: str) -> Words:
    """Show a word of a sentence that was heard wrong as it was said, and keep that.

    The word keeps its time. It is found by its letters, digits and characters, as the story
    matches words: neither stops, blanks nor cases count.

    Args:
        directory: The directory of the project of the engine.
        digest: The digest of the media.
        sentence: Which sentence of its words the word is in, counted from 0.
        wrong: The word as it is shown now.
        right: The word as it was said.

    Raises:
        WordsError: There are no words of the media, no such sentence, or the word is not in it
            once.
        ProjectError: The words could not be written.
    """
    words = kept_words(directory, digest)
    if words is None:
        raise WordsError(f"no words were written down of media {digest[:12]}: transcribe it first")
    if not 0 <= sentence < len(words.sentences):
        raise WordsError(f"there is no sentence {sentence} in the words of media {digest[:12]}")
    if not right.strip():
        raise WordsError("a word is not corrected to nothing: strike it through in the story")
    transcript = words.transcript("")
    first, last = words.sentences[sentence]
    found = [
        index for index in range(first, last) if normal(transcript.text(index)) == normal(wrong)
    ]
    if len(found) != 1:
        said = line_of([(transcript.text(index), False) for index in range(first, last)])
        raise WordsError(
            f'"{wrong}" is {"not" if not found else "more than once"} a word of sentence '
            f'{sentence}, which says "{said}"'
        )
    changed = words.model_copy(update={"shown": {**words.shown, found[0]: right.strip()}})
    keep_words(directory, changed)
    return changed
