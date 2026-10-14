import unittest

from slugs import slugify, unique_slug


class SlugifyTest(unittest.TestCase):
    def test_plain_title(self):
        self.assertEqual(slugify("Release Notes"), "release-notes")

    def test_accented_title(self):
        self.assertEqual(slugify("Crème Brûlée"), "creme-brulee")

    def test_punctuation_between_words(self):
        self.assertEqual(slugify("Hello, World!"), "hello-world")

    def test_cut_short(self):
        self.assertEqual(slugify("Release Notes", max_length=8), "release")


class UniqueSlugTest(unittest.TestCase):
    def test_taken_slug_gets_a_number(self):
        self.assertEqual(unique_slug("Release Notes", {"release-notes"}), "release-notes-2")


if __name__ == "__main__":
    unittest.main()
