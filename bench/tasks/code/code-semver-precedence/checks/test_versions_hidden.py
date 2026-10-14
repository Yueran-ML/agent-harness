import itertools
import unittest

from release import release_to_deploy
from versions import compare, latest, sort_versions

SPEC_ORDER = [
    "1.0.0-alpha",
    "1.0.0-alpha.1",
    "1.0.0-alpha.beta",
    "1.0.0-beta",
    "1.0.0-beta.2",
    "1.0.0-beta.11",
    "1.0.0-rc.1",
    "1.0.0",
]


class PrecedenceTest(unittest.TestCase):
    def assertOrdered(self, lower, higher):
        self.assertEqual(compare(lower, higher), -1, f"{lower} < {higher}")
        self.assertEqual(compare(higher, lower), 1, f"{higher} > {lower}")

    def test_example_of_the_specification(self):
        for lower, higher in itertools.combinations(SPEC_ORDER, 2):
            with self.subTest(lower=lower, higher=higher):
                self.assertOrdered(lower, higher)

    def test_release_is_higher_than_its_prereleases(self):
        for prerelease in ["3.1.4-0", "3.1.4-alpha", "3.1.4-rc.99", "3.1.4-zzz.9.9"]:
            with self.subTest(prerelease=prerelease):
                self.assertOrdered(prerelease, "3.1.4")
        self.assertOrdered("3.1.4", "3.1.5-alpha")
        self.assertOrdered("3.1.4-rc.1", "3.1.5-alpha")

    def test_identifiers_of_digits_compare_as_numbers(self):
        self.assertOrdered("1.0.0-rc.9", "1.0.0-rc.10")
        self.assertOrdered("1.0.0-rc.10", "1.0.0-rc.100")
        self.assertOrdered("1.0.0-2", "1.0.0-10")
        self.assertOrdered("1.0.0-alpha.3.9", "1.0.0-alpha.3.10")

    def test_identifiers_of_digits_are_lower_than_others(self):
        self.assertOrdered("1.0.0-1", "1.0.0-alpha")
        self.assertOrdered("1.0.0-9", "1.0.0-10a")
        self.assertOrdered("1.0.0-alpha.99", "1.0.0-alpha.x")

    def test_other_identifiers_compare_in_ascii_order(self):
        self.assertOrdered("1.0.0-a10", "1.0.0-a2")
        self.assertOrdered("1.0.0-Beta", "1.0.0-alpha")
        self.assertOrdered("1.0.0-alpha", "1.0.0-alpha.0")

    def test_core_numbers_compare_as_numbers(self):
        self.assertOrdered("1.9.0", "1.10.0")
        self.assertOrdered("1.99.99", "2.0.0")
        self.assertOrdered("0.9.12", "0.10.1")
        self.assertOrdered("1.0.9", "1.0.10")
        self.assertEqual(compare("1.2.3", "1.2.3"), 0)

    def test_build_metadata_does_not_count(self):
        self.assertEqual(compare("1.0.0+build.1", "1.0.0+build.2"), 0)
        self.assertEqual(compare("1.0.0-rc.1+abc", "1.0.0-rc.1"), 0)
        self.assertOrdered("1.0.0-rc.1+zzz", "1.0.0+aaa")


class ChoosingTest(unittest.TestCase):
    def test_sort_versions(self):
        shuffled = [
            "1.0.0",
            "1.0.0-beta.11",
            "0.9.0",
            "1.0.0-alpha",
            "1.0.0-rc.1",
            "1.0.1-alpha",
            "1.0.0-beta.2",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
        ]
        self.assertEqual(sort_versions(shuffled), ["0.9.0", *SPEC_ORDER, "1.0.1-alpha"])

    def test_latest(self):
        self.assertEqual(
            latest(["2.0.0-rc.1", "2.0.0", "2.0.0-rc.2", "1.10.0"], include_prereleases=True),
            "2.0.0",
        )
        self.assertEqual(latest(["1.9.0", "1.10.0", "1.10.1-beta"]), "1.10.0")
        self.assertEqual(
            latest(["1.10.1-beta.2", "1.10.1-beta.10"], include_prereleases=True),
            "1.10.1-beta.10",
        )

    def test_release_to_deploy(self):
        tags = ["v2.1.0-rc.1", "v2.1.0", "v2.1.0-rc.2", "nightly", "v2.0.9"]
        self.assertEqual(release_to_deploy(tags, allow_prereleases=True), "v2.1.0")
        self.assertEqual(release_to_deploy(tags), "v2.1.0")


if __name__ == "__main__":
    unittest.main()
