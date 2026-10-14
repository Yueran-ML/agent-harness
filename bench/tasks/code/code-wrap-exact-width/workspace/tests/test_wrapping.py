import unittest

from wrapping import fill, wrap


class WrapTest(unittest.TestCase):
    def test_short_text_stays_on_one_line(self):
        self.assertEqual(wrap("thank you", 20), ["thank you"])

    def test_whitespace_counts_as_one_space(self):
        self.assertEqual(wrap("  thank\n you  ", 20), ["thank you"])

    def test_long_word_is_cut_into_pieces(self):
        self.assertEqual(
            wrap("a supercalifragilistic day", 8), ["a", "supercal", "ifragili", "stic day"]
        )

    def test_a_line_may_be_exactly_as_long_as_the_width(self):
        self.assertEqual(wrap("pay at the counter", 10), ["pay at the", "counter"])

    def test_fill_joins_the_lines(self):
        self.assertEqual(fill("a b c", 3), "a b\nc")


if __name__ == "__main__":
    unittest.main()
