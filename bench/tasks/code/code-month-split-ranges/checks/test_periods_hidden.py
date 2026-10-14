import unittest
from datetime import date, timedelta

from months import days_in_month, month_end, month_start, next_month_start
from periods import days_by_month, rent_by_month, split_by_month

DAYS = {
    2023: [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
    2024: [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
}


class MonthsTest(unittest.TestCase):
    def test_every_month_ends_on_its_last_day(self):
        for year, lengths in DAYS.items():
            for month, length in enumerate(lengths, start=1):
                with self.subTest(year=year, month=month):
                    self.assertEqual(month_end(date(year, month, 10)), date(year, month, length))
                    self.assertEqual(days_in_month(year, month), length)

    def test_next_month_start(self):
        self.assertEqual(next_month_start(date(2024, 12, 31)), date(2025, 1, 1))
        self.assertEqual(next_month_start(date(1999, 12, 1)), date(2000, 1, 1))
        self.assertEqual(next_month_start(date(2024, 1, 31)), date(2024, 2, 1))
        self.assertEqual(month_start(date(2024, 12, 31)), date(2024, 12, 1))

    def test_century_years(self):
        self.assertEqual(days_in_month(1900, 2), 28)
        self.assertEqual(days_in_month(2000, 2), 29)
        self.assertEqual(days_in_month(2000, 12), 31)


class SplitTest(unittest.TestCase):
    def test_single_day(self):
        for day in [date(2024, 2, 29), date(2024, 12, 31), date(2025, 1, 1), date(2023, 6, 15)]:
            with self.subTest(day=day):
                self.assertEqual(split_by_month(day, day), [(day, day)])

    def test_ending_on_the_first_of_a_month(self):
        self.assertEqual(
            split_by_month(date(2024, 2, 27), date(2024, 3, 1)),
            [(date(2024, 2, 27), date(2024, 2, 29)), (date(2024, 3, 1), date(2024, 3, 1))],
        )
        self.assertEqual(
            split_by_month(date(2023, 11, 1), date(2024, 1, 1)),
            [
                (date(2023, 11, 1), date(2023, 11, 30)),
                (date(2023, 12, 1), date(2023, 12, 31)),
                (date(2024, 1, 1), date(2024, 1, 1)),
            ],
        )

    def test_starting_on_the_last_day_of_a_month(self):
        self.assertEqual(
            split_by_month(date(2024, 4, 30), date(2024, 5, 3)),
            [(date(2024, 4, 30), date(2024, 4, 30)), (date(2024, 5, 1), date(2024, 5, 3))],
        )

    def test_pieces_cover_every_day_once(self):
        periods = [
            (date(2023, 11, 15), date(2024, 2, 10)),
            (date(2024, 1, 1), date(2024, 12, 31)),
            (date(2022, 12, 31), date(2025, 1, 1)),
        ]
        for start, end in periods:
            with self.subTest(start=start, end=end):
                pieces = split_by_month(start, end)
                self.assertEqual(pieces[0][0], start)
                self.assertEqual(pieces[-1][1], end)
                for first, last in pieces:
                    self.assertLessEqual(first, last)
                    self.assertEqual((first.year, first.month), (last.year, last.month))
                for (_, last), (first, _) in zip(pieces, pieces[1:]):
                    self.assertEqual(first, last + timedelta(days=1))
                    self.assertEqual(first.day, 1)
                days = sum((last - first).days + 1 for first, last in pieces)
                self.assertEqual(days, (end - start).days + 1)

    def test_end_before_start_is_refused(self):
        with self.assertRaises(ValueError):
            split_by_month(date(2024, 5, 2), date(2024, 5, 1))


class BillingTest(unittest.TestCase):
    def test_days_by_month(self):
        self.assertEqual(
            days_by_month(date(2024, 12, 20), date(2025, 1, 1)), {(2024, 12): 12, (2025, 1): 1}
        )

    def test_rent_by_month(self):
        self.assertEqual(
            rent_by_month(date(2024, 12, 1), date(2024, 12, 31), 31000), {(2024, 12): 31000}
        )
        self.assertEqual(
            rent_by_month(date(2025, 1, 16), date(2025, 2, 1), 62000),
            {(2025, 1): 32000, (2025, 2): 2214},
        )


if __name__ == "__main__":
    unittest.main()
