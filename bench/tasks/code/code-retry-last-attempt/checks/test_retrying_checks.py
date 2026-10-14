import unittest

from retrying import retry


class Waits:
    """Stands in for time.sleep and keeps how long it was asked to wait."""

    def __init__(self):
        self.seconds = []

    def __call__(self, seconds):
        self.seconds.append(seconds)


def failing(failures, error=ConnectionError):
    """Return a function that raises a new error in each of its first calls, its calls and its errors."""
    calls = []
    raised = []

    def function(*args, **kwargs):
        calls.append((args, kwargs))
        if len(calls) <= failures:
            exception = error(f"attempt {len(calls)}")
            raised.append(exception)
            raise exception
        return "done"

    return function, calls, raised


class AttemptsTest(unittest.TestCase):
    def test_called_as_many_times_as_attempts(self):
        for attempts in (1, 2, 3, 5):
            with self.subTest(attempts=attempts):
                function, calls, raised = failing(100)
                wrapped = retry(attempts=attempts, sleep=Waits())(function)
                with self.assertRaises(ConnectionError) as caught:
                    wrapped()
                self.assertEqual(len(calls), attempts)
                self.assertIs(caught.exception, raised[-1])

    def test_success_on_the_last_attempt(self):
        for attempts in (1, 2, 4):
            with self.subTest(attempts=attempts):
                function, calls, _ = failing(attempts - 1)
                self.assertEqual(retry(attempts=attempts, sleep=Waits())(function)(), "done")
                self.assertEqual(len(calls), attempts)

    def test_success_before_the_last_attempt(self):
        function, calls, _ = failing(1)
        self.assertEqual(retry(attempts=5, sleep=Waits())(function)(), "done")
        self.assertEqual(len(calls), 2)

    def test_arguments_reach_every_attempt(self):
        function, calls, _ = failing(2)
        self.assertEqual(retry(attempts=3, sleep=Waits())(function)(1, 2, key="x"), "done")
        self.assertEqual(calls, [((1, 2), {"key": "x"})] * 3)


class WaitsTest(unittest.TestCase):
    def test_waits_between_attempts_and_not_after_the_last(self):
        waits = Waits()
        function, _, _ = failing(100)
        with self.assertRaises(ConnectionError):
            retry(attempts=4, delay=0.5, backoff=2.0, sleep=waits)(function)()
        self.assertEqual(waits.seconds, [0.5, 1.0, 2.0])

    def test_no_wait_after_a_success(self):
        waits = Waits()
        function, _, _ = failing(2)
        retry(attempts=5, delay=0.25, backoff=4.0, sleep=waits)(function)()
        self.assertEqual(waits.seconds, [0.25, 1.0])

    def test_one_attempt_never_waits(self):
        waits = Waits()
        function, calls, _ = failing(100)
        with self.assertRaises(ConnectionError):
            retry(attempts=1, delay=1.0, sleep=waits)(function)()
        self.assertEqual(len(calls), 1)
        self.assertEqual(waits.seconds, [])


class ExceptionsTest(unittest.TestCase):
    def test_subclasses_are_retried(self):
        function, calls, _ = failing(2, TimeoutError)
        wrapped = retry(attempts=3, exceptions=(OSError,), sleep=Waits())(function)
        self.assertEqual(wrapped(), "done")
        self.assertEqual(len(calls), 3)

    def test_last_error_of_a_retried_kind_propagates(self):
        function, calls, raised = failing(100, TimeoutError)
        wrapped = retry(attempts=2, exceptions=(OSError,), sleep=Waits())(function)
        with self.assertRaises(TimeoutError) as caught:
            wrapped()
        self.assertEqual(len(calls), 2)
        self.assertIs(caught.exception, raised[1])

    def test_other_exceptions_are_raised_at_once(self):
        waits = Waits()
        function, calls, _ = failing(100, KeyError)
        with self.assertRaises(KeyError):
            retry(attempts=3, exceptions=(ConnectionError,), sleep=waits)(function)()
        self.assertEqual(len(calls), 1)
        self.assertEqual(waits.seconds, [])

    def test_at_least_one_attempt(self):
        with self.assertRaises(ValueError):
            retry(attempts=0)

    def test_wrapped_function_keeps_its_name(self):
        @retry(attempts=2)
        def fetch_price():
            """Return a price."""
            return 1

        self.assertEqual(fetch_price.__name__, "fetch_price")
        self.assertEqual(fetch_price.__doc__, "Return a price.")


if __name__ == "__main__":
    unittest.main()
