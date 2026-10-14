import unittest

from retrying import retry


def no_wait(seconds):
    pass


class RetryTest(unittest.TestCase):
    def test_first_attempt_succeeds(self):
        calls = []

        @retry(attempts=3, sleep=no_wait)
        def fetch_price():
            calls.append(1)
            return 42

        self.assertEqual(fetch_price(), 42)
        self.assertEqual(len(calls), 1)

    def test_third_attempt_succeeds(self):
        calls = []

        @retry(attempts=3, sleep=no_wait)
        def fetch_price():
            calls.append(1)
            if len(calls) < 3:
                raise ConnectionError("connection reset")
            return 42

        self.assertEqual(fetch_price(), 42)
        self.assertEqual(len(calls), 3)

    def test_attempts_must_be_positive(self):
        with self.assertRaises(ValueError):
            retry(attempts=0)


if __name__ == "__main__":
    unittest.main()
