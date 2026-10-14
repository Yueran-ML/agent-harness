import unittest

from release import release_to_deploy
from versions import Version, compare, parse, sort_versions


class ParseTest(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(
            parse("2.0.0-rc.1+build.5"), Version(2, 0, 0, ("rc", "1"), ("build", "5"))
        )

    def test_not_a_version(self):
        with self.assertRaises(ValueError):
            parse("2.0")


class CompareTest(unittest.TestCase):
    def test_numbers_compare_as_numbers(self):
        self.assertEqual(compare("1.10.0", "1.9.0"), 1)

    def test_prerelease_is_lower_than_its_release(self):
        self.assertEqual(compare("2.0.0-rc.1", "2.0.0"), -1)

    def test_numbers_in_prereleases_compare_as_numbers(self):
        self.assertEqual(
            sort_versions(["1.0.0-beta.11", "1.0.0-beta.2"]), ["1.0.0-beta.2", "1.0.0-beta.11"]
        )


class ReleaseTest(unittest.TestCase):
    def test_highest_release_is_deployed(self):
        self.assertEqual(release_to_deploy(["v1.9.0", "v1.10.0", "nightly"]), "v1.10.0")


if __name__ == "__main__":
    unittest.main()
