import unittest

from durations import format_duration, parse_duration
from timeouts import read_timeouts


class ParseDurationTest(unittest.TestCase):
    def test_hours_and_minutes(self):
        self.assertEqual(parse_duration("1h30m"), 5400)

    def test_parts_separated_by_a_space(self):
        self.assertEqual(parse_duration("2d 4h"), 187200)

    def test_decimal_number_is_not_a_duration(self):
        with self.assertRaises(ValueError):
            parse_duration("1.5h")

    def test_number_without_a_unit_is_not_a_duration(self):
        with self.assertRaises(ValueError):
            parse_duration("1h30")


class FormatDurationTest(unittest.TestCase):
    def test_largest_unit_first(self):
        self.assertEqual(format_duration(5400), "1h30m")
        self.assertEqual(format_duration(0), "0s")


class TimeoutsTest(unittest.TestCase):
    def test_defaults_fill_in(self):
        self.assertEqual(
            read_timeouts({"connect": "30s"}), {"connect": 30, "job": 7200, "retention": 2592000}
        )


if __name__ == "__main__":
    unittest.main()
