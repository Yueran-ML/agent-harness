import unittest

from slugs import slugify, unique_slug


class AccentsTest(unittest.TestCase):
    def test_accented_letters_keep_their_base_letter(self):
        cases = {
            "Crème Brûlée": "creme-brulee",
            "Ångström Units": "angstrom-units",
            "Señor Niño": "senor-nino",
            "naïve café": "naive-cafe",
            "ÉCOLE NORMALE": "ecole-normale",
            "Dvořák: Symphony No. 9": "dvorak-symphony-no-9",
            "Zoë's Über Guide": "zoe-s-uber-guide",
        }
        for text, slug in cases.items():
            with self.subTest(text=text):
                self.assertEqual(slugify(text), slug)


class SeparatorsTest(unittest.TestCase):
    def test_one_separator_between_words(self):
        cases = {
            "Hello,   World!!": "hello-world",
            "--Already--slugged--": "already-slugged",
            "a / b \\ c": "a-b-c",
            "  spaces   everywhere  ": "spaces-everywhere",
            "C++ & C# tips": "c-c-tips",
            "Top 10 Tips for 2024": "top-10-tips-for-2024",
            "What?! Really...": "what-really",
        }
        for text, slug in cases.items():
            with self.subTest(text=text):
                self.assertEqual(slugify(text), slug)

    def test_another_separator(self):
        self.assertEqual(
            slugify("Crème brûlée, à la carte", separator="_"), "creme_brulee_a_la_carte"
        )
        self.assertEqual(slugify("foo -- bar", separator="_"), "foo_bar")

    def test_nothing_left(self):
        self.assertEqual(slugify("!!! ???"), "")
        self.assertEqual(slugify(""), "")


class MaxLengthTest(unittest.TestCase):
    def test_cut_short(self):
        self.assertEqual(slugify("Crème Brûlée Recipes", max_length=13), "creme-brulee")
        self.assertEqual(slugify("Crème Brûlée Recipes", max_length=15), "creme-brulee-re")
        self.assertEqual(slugify("Hello,   World", max_length=7), "hello-w")
        self.assertEqual(slugify("Café", max_length=10), "cafe")


class UniqueSlugTest(unittest.TestCase):
    def test_taken_slugs_get_a_number(self):
        self.assertEqual(unique_slug("Café Menu", set()), "cafe-menu")
        self.assertEqual(unique_slug("Café Menu", {"cafe-menu"}), "cafe-menu-2")
        self.assertEqual(unique_slug("Café  Menu!", {"cafe-menu", "cafe-menu-2"}), "cafe-menu-3")

    def test_title_without_letters(self):
        self.assertEqual(unique_slug("???", set()), "post")
        self.assertEqual(unique_slug("???", {"post"}), "post-2")


if __name__ == "__main__":
    unittest.main()
