"""How often each word occurs in a text."""

import re
from collections import Counter

_WORD = re.compile(r"[^\W_]+(?:'[^\W_]+)*")
"""A run of letters or digits, with an apostrophe inside it now and then."""


def words(text):
    """Return the words of a text in lower case, in the order in which they occur."""
    return _WORD.findall(text.lower().replace("’", "'"))


def word_counts(text):
    """Return how many times each word occurs in a text."""
    return Counter(words(text))


def top_words(text, count=10, ignore=()):
    """Return the ``count`` most frequent words of a text, each with how often it occurs.

    The most frequent come first, and words that occur equally often are in
    alphabetical order. The words in ``ignore`` are left out, whatever their case.
    """
    skipped = {word.lower() for word in ignore}
    ranked = sorted(
        ((word, times) for word, times in word_counts(text).items() if word not in skipped),
        key=lambda item: (-item[1], item[0]),
    )
    return ranked[:count]
