import unittest
from collections import Counter

from wordfreq import top_words, word_counts, words


class WordsTest(unittest.TestCase):
    def test_contractions_stay_whole(self):
        self.assertEqual(
            words("Don't stop, can't stop, WON'T stop."),
            ["don't", "stop", "can't", "stop", "won't", "stop"],
        )

    def test_curly_apostrophe_counts_as_an_apostrophe(self):
        self.assertEqual(words("Don’t, isn’t, o’clock"), ["don't", "isn't", "o'clock"])
        self.assertEqual(word_counts("It's its; it’s IT'S"), Counter({"it's": 3, "its": 1}))

    def test_apostrophes_inside_words(self):
        self.assertEqual(
            words("rock'n'roll at five o'clock, back in the '90s or the 90's"),
            [
                "rock'n'roll", "at", "five", "o'clock", "back",
                "in", "the", "90s", "or", "the", "90's",
            ],
        )

    def test_apostrophes_at_the_ends_of_words(self):
        self.assertEqual(
            words("'Tis the dogs' bowl, 'quoted' and ’curly’"),
            ["tis", "the", "dogs", "bowl", "quoted", "and", "curly"],
        )

    def test_other_punctuation_separates_words(self):
        self.assertEqual(
            words("“Wait…” — she said; well-known (really)! snake_case/path"),
            ["wait", "she", "said", "well", "known", "really", "snake", "case", "path"],
        )

    def test_letters_and_digits(self):
        self.assertEqual(
            words("Café crème, déjà vu: Route 66 and B2B"),
            ["café", "crème", "déjà", "vu", "route", "66", "and", "b2b"],
        )

    def test_no_words(self):
        self.assertEqual(words("... -- '' ’ !?"), [])
        self.assertEqual(words(""), [])


class CountsTest(unittest.TestCase):
    def test_counts_ignore_case(self):
        self.assertEqual(
            word_counts("Don't panic. DON'T Panic! don’t... PANIC?"),
            Counter({"don't": 3, "panic": 3}),
        )

    def test_top_words(self):
        text = "Don't panic. Don't! PANIC, don’t. Towel? towel; the the the the"
        self.assertEqual(
            top_words(text, 3, ignore=["The"]), [("don't", 3), ("panic", 2), ("towel", 2)]
        )
        self.assertEqual(top_words(text, 2), [("the", 4), ("don't", 3)])


if __name__ == "__main__":
    unittest.main()
