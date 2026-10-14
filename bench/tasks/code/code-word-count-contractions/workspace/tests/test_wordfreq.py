import unittest
from collections import Counter

from wordfreq import top_words, word_counts


class WordCountsTest(unittest.TestCase):
    def test_case_and_punctuation_are_ignored(self):
        self.assertEqual(
            word_counts("The cat; THE hat! snake_case, the end."),
            Counter({"the": 3, "cat": 1, "hat": 1, "snake": 1, "case": 1, "end": 1}),
        )

    def test_contractions(self):
        self.assertEqual(word_counts("Don't stop, don’t!"), Counter({"don't": 2, "stop": 1}))


class TopWordsTest(unittest.TestCase):
    def test_most_frequent_first(self):
        self.assertEqual(top_words("b a b c a b", 2), [("b", 3), ("a", 2)])

    def test_ignored_words(self):
        self.assertEqual(top_words("b a b c a b", 2, ignore=["B"]), [("a", 2), ("c", 1)])


if __name__ == "__main__":
    unittest.main()
