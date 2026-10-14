import unittest

from bill import describe, split_bill
from money import format_cents


class SplitBillTest(unittest.TestCase):
    def test_bill_that_divides_evenly(self):
        self.assertEqual(split_bill("30.00", 3), [1000, 1000, 1000])

    def test_shares_add_up_to_the_bill(self):
        self.assertEqual(split_bill("100.00", 3), [3334, 3333, 3333])

    def test_one_person_pays_the_whole_bill(self):
        self.assertEqual(split_bill("0.29", 1), [29])

    def test_describe(self):
        self.assertEqual(describe("$12.00", ["Ann", "Bo"]), ["Ann pays 6.00", "Bo pays 6.00"])


class FormatCentsTest(unittest.TestCase):
    def test_format_cents(self):
        self.assertEqual(format_cents(1234), "12.34")
        self.assertEqual(format_cents(5), "0.05")


if __name__ == "__main__":
    unittest.main()
