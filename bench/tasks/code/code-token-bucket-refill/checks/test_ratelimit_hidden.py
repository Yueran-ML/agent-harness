import unittest

from ratelimit import TokenBucket


class FakeClock:
    def __init__(self, now=0.0):
        self.now = now

    def __call__(self):
        return self.now


class RefillTimingTest(unittest.TestCase):
    def test_one_event_per_interval_at_the_exact_times(self):
        clock = FakeClock(0.0)
        bucket = TokenBucket(1, 0.5, clock)
        allowed = []
        for step in range(20):
            clock.now = step * 0.25
            allowed.append(bucket.try_acquire())
        self.assertEqual(allowed, [step % 2 == 0 for step in range(20)])

    def test_tokens_come_back_one_per_interval(self):
        clock = FakeClock(50.0)
        bucket = TokenBucket(4, 3.0, clock)
        for _ in range(4):
            self.assertTrue(bucket.try_acquire())
        self.assertEqual(bucket.available(), 0)
        clock.now = 52.75
        self.assertEqual(bucket.available(), 0)
        clock.now = 53.0
        self.assertEqual(bucket.available(), 1)
        clock.now = 59.0
        self.assertEqual(bucket.available(), 3)
        clock.now = 62.0
        self.assertEqual(bucket.available(), 4)
        clock.now = 500.0
        self.assertEqual(bucket.available(), 4)

    def test_just_before_the_interval_is_too_early(self):
        clock = FakeClock(10.0)
        bucket = TokenBucket(2, 1.0, clock)
        self.assertTrue(bucket.try_acquire())
        self.assertTrue(bucket.try_acquire())
        clock.now = 10.75
        self.assertFalse(bucket.try_acquire())
        self.assertEqual(bucket.wait_time(), 0.25)
        clock.now = 11.0
        self.assertTrue(bucket.try_acquire())
        clock.now = 11.5
        self.assertFalse(bucket.try_acquire())
        clock.now = 12.0
        self.assertTrue(bucket.try_acquire())
        self.assertFalse(bucket.try_acquire())

    def test_refill_starts_when_a_token_is_taken_from_the_full_bucket(self):
        clock = FakeClock(0.0)
        bucket = TokenBucket(2, 2.0, clock)
        clock.now = 5.0
        self.assertTrue(bucket.try_acquire())
        clock.now = 6.5
        self.assertEqual(bucket.available(), 1)
        clock.now = 7.0
        self.assertEqual(bucket.available(), 2)


class WaitTimeTest(unittest.TestCase):
    def test_waiting_as_long_as_told_is_enough(self):
        clock = FakeClock(0.0)
        bucket = TokenBucket(2, 1.25, clock)
        times = []
        for _ in range(100):
            if len(times) == 12:
                break
            if bucket.try_acquire():
                times.append(clock.now)
            else:
                wait = bucket.wait_time()
                self.assertGreater(wait, 0.0)
                clock.now += wait
        self.assertEqual(
            times, [0.0, 0.0, 1.25, 2.5, 3.75, 5.0, 6.25, 7.5, 8.75, 10.0, 11.25, 12.5]
        )

    def test_wait_time_and_available_agree(self):
        clock = FakeClock(0.0)
        bucket = TokenBucket(3, 0.5, clock)
        for _ in range(3):
            self.assertTrue(bucket.try_acquire())
        for step in range(1, 13):
            clock.now = step * 0.125
            with self.subTest(now=clock.now):
                if bucket.wait_time() == 0.0:
                    self.assertGreater(bucket.available(), 0)
                else:
                    self.assertEqual(bucket.available(), 0)


class SettingsTest(unittest.TestCase):
    def test_bad_settings_are_refused(self):
        with self.assertRaises(ValueError):
            TokenBucket(0, 1.0, FakeClock())
        with self.assertRaises(ValueError):
            TokenBucket(1, 0.0, FakeClock())


if __name__ == "__main__":
    unittest.main()
