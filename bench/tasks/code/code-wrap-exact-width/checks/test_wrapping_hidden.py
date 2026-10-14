import unittest

from wrapping import fill, wrap, wrap_paragraphs

TEXTS = [
    "the quick brown fox jumps over the lazy dog",
    "Please keep this receipt. Items can be returned within 30 days of purchase.",
    "a bb ccc dddd eeeee ffffff ggggggg hhhhhhhh",
    "x y z",
    "one two three four five six seven eight nine ten",
    "pneumonoultramicroscopic words get cut when they are too long",
    "  spaces   and\ttabs\nand newlines  count as one  ",
]


def greedy(text, width):
    """Greedy wrapping, word by word, as the docstring of wrap describes it."""
    lines = []
    line = ""
    for word in text.split():
        if line and len(line) + 1 + len(word) <= width:
            line += " " + word
            continue
        if line:
            lines.append(line)
        while len(word) > width:
            lines.append(word[:width])
            word = word[width:]
        line = word
    if line:
        lines.append(line)
    return lines


class ExactFitTest(unittest.TestCase):
    def test_examples(self):
        cases = [
            (
                "the quick brown fox jumps over the lazy dog",
                9,
                ["the quick", "brown fox", "jumps", "over the", "lazy dog"],
            ),
            ("one two three four", 7, ["one two", "three", "four"]),
            ("ab cd ef gh", 5, ["ab cd", "ef gh"]),
            ("a b c d", 3, ["a b", "c d"]),
            ("ab cd ef gh", 11, ["ab cd ef gh"]),
            ("abcde fghij", 5, ["abcde", "fghij"]),
            ("x", 1, ["x"]),
        ]
        for text, width, expected in cases:
            with self.subTest(text=text, width=width):
                self.assertEqual(wrap(text, width), expected)

    def test_fill(self):
        self.assertEqual(fill("one two three four", 7), "one two\nthree\nfour")
        self.assertEqual(fill("", 7), "")

    def test_paragraphs(self):
        text = "first para here\n\n  \nsecond one is longer\n"
        self.assertEqual(
            wrap_paragraphs(text, 10), ["first para", "here", "", "second one", "is longer"]
        )


class EveryWidthTest(unittest.TestCase):
    def test_same_lines_as_greedy_wrapping(self):
        for text in TEXTS:
            for width in range(1, 41):
                with self.subTest(text=text, width=width):
                    self.assertEqual(wrap(text, width), greedy(text, width))

    def test_lines_fit_and_are_as_full_as_they_can_be(self):
        for text in TEXTS:
            for width in range(1, 41):
                with self.subTest(text=text, width=width):
                    lines = wrap(text, width)
                    for line in lines:
                        self.assertLessEqual(len(line), width)
                        self.assertEqual(line, line.strip())
                    for line, following in zip(lines, lines[1:]):
                        self.assertGreater(len(line) + 1 + len(following.split()[0]), width)
                    self.assertEqual("".join(lines).replace(" ", ""), "".join(text.split()))


class EdgeTest(unittest.TestCase):
    def test_no_words_gives_no_lines(self):
        self.assertEqual(wrap("", 5), [])
        self.assertEqual(wrap(" \n\t ", 5), [])

    def test_width_below_one_is_refused(self):
        with self.assertRaises(ValueError):
            wrap("abc", 0)


if __name__ == "__main__":
    unittest.main()
