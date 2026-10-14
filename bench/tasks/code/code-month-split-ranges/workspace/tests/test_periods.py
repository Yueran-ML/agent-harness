import unittest
from datetime import date

from periods import rent_by_month, split_by_month


class SplitByMonthTest(unittest.TestCase):
    def test_period_within_one_month(self):
        self.assertEqual(
            split_by_month(date(2024, 3, 5), date(2024, 3, 20)),
            [(date(2024, 3, 5), date(2024, 3, 20))],
        )

    def test_period_across_two_months(self):
        self.assertEqual(
            split_by_month(date(2024, 1, 30), date(2024, 2, 2)),
            [(date(2024, 1, 30), date(2024, 1, 31)), (date(2024, 2, 1), date(2024, 2, 2))],
        )

    def test_period_across_the_new_year(self):
        self.assertEqual(
            split_by_month(date(2024, 12, 30), date(2025, 1, 2)),
            [(date(2024, 12, 30), date(2024, 12, 31)), (date(2025, 1, 1), date(2025, 1, 2))],
        )

    def test_period_ending_on_the_first_of_a_month(self):
        self.assertEqual(
            split_by_month(date(2024, 1, 30), date(2024, 2, 1)),
            [(date(2024, 1, 30), date(2024, 1, 31)), (date(2024, 2, 1), date(2024, 2, 1))],
        )


class RentTest(unittest.TestCase):
    def test_rent_for_part_of_a_month(self):
        self.assertEqual(rent_by_month(date(2024, 4, 1), date(2024, 4, 15), 90000), {(2024, 4): 45000})


if __name__ == "__main__":
    unittest.main()
