import unittest

from ratelimit import TokenBucket


class FakeClock:
    """A clock that moves only when a test moves it."""

    def __init__(self, now=0.0):
        self.now = now

    def __call__(self):
        return self.now


class TokenBucketTest(unittest.TestCase):
    def test_bucket_starts_full(self):
        bucket = TokenBucket(3, 1.0, FakeClock())
        self.assertEqual([bucket.try_acquire() for _ in range(4)], [True, True, True, False])

    def test_token_is_back_after_exactly_one_interval(self):
        clock = FakeClock(100.0)
        bucket = TokenBucket(1, 2.0, clock)
        self.assertTrue(bucket.try_acquire())
        clock.now = 101.0
        self.assertFalse(bucket.try_acquire())
        self.assertEqual(bucket.wait_time(), 1.0)
        clock.now = 102.0
        self.assertTrue(bucket.try_acquire())


if __name__ == "__main__":
    unittest.main()
