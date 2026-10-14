import unittest

from durations import format_duration, parse_duration
from timeouts import read_timeouts


class ValidDurationTest(unittest.TestCase):
    def test_valid_durations_are_read(self):
        cases = {
            "45s": 45,
            "90m": 5400,
            "2d": 172800,
            "1d2h3m4s": 93784,
            "1h 30m": 5400,
            "10m 5s": 605,
            "  15m  ": 900,
            "1H30M": 5400,
            "0s": 0,
            "100h": 360000,
            "3d 0h 1s": 259201,
        }
        for text, seconds in cases.items():
            with self.subTest(text=text):
                self.assertEqual(parse_duration(text), seconds)

    def test_formatted_durations_read_back(self):
        for seconds in [1, 59, 60, 61, 3599, 3600, 5400, 86399, 86400, 93784, 1000000]:
            with self.subTest(seconds=seconds):
                self.assertEqual(parse_duration(format_duration(seconds)), seconds)


class InvalidDurationTest(unittest.TestCase):
    def test_text_that_is_not_a_duration_raises(self):
        for text in [
            "",
            "   ",
            "2.5h",
            "0.5d",
            "1,5h",
            "1h30",
            "45",
            "1h30x",
            "1h30m!",
            "h",
            "1hm",
            "-5m",
            "+5m",
            "2h-15m",
            "1d.2h",
            "ten minutes",
            "1w",
            "m5",
            "1h/30m",
        ]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    parse_duration(text)


class TimeoutsTest(unittest.TestCase):
    def test_timeouts_are_read(self):
        self.assertEqual(
            read_timeouts({"job": "1h 15m"}),
            {"connect": 10, "job": 4500, "retention": 2592000},
        )

    def test_a_mistyped_timeout_is_refused(self):
        for text in ["1.5h", "90", "2h30"]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    read_timeouts({"job": text})


if __name__ == "__main__":
    unittest.main()
