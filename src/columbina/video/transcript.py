"""Timed words shared by transcription, Studio stories and the legacy reader."""

from pydantic import Field

from columbina.data import Data

LEAD_US = 120_000
"""Silence retained beside a word when a text cut joins its neighbours."""
PAUSE_LEAST_US = 300_000
"""Length left when a pause is taken out of a Studio story."""


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
