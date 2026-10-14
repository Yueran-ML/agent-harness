import unittest

from bill import describe, split_bill, split_evenly
from money import format_cents, to_cents


class ToCentsTest(unittest.TestCase):
    def test_amounts_are_read_exactly(self):
        cases = {
            "19.99": 1999,
            "1.15": 115,
            "4.35": 435,
            "8.20": 820,
            "9.95": 995,
            "0.57": 57,
            "2.01": 201,
            "64.99": 6499,
            "17.40": 1740,
            "1234567.89": 123456789,
            "0.07": 7,
            "12": 1200,
            "12.3": 1230,
            "$45.10": 4510,
            " 2.55 ": 255,
            "0": 0,
        }
        for text, cents in cases.items():
            with self.subTest(text=text):
                self.assertEqual(to_cents(text), cents)

    def test_negative_amount_is_refused(self):
        with self.assertRaises(ValueError):
            to_cents("-1.00")


class SplitEvenlyTest(unittest.TestCase):
    def test_examples(self):
        self.assertEqual(split_evenly(10000, 3), [3334, 3333, 3333])
        self.assertEqual(split_evenly(101, 4), [26, 25, 25, 25])
        self.assertEqual(split_evenly(2, 5), [1, 1, 0, 0, 0])
        self.assertEqual(split_evenly(7, 7), [1] * 7)
        self.assertEqual(split_evenly(0, 2), [0, 0])

    def test_shares_always_add_up(self):
        for total in [1, 2, 5, 99, 100, 101, 1999, 4510, 10001, 123457]:
            for people in range(1, 8):
                with self.subTest(total=total, people=people):
                    shares = split_evenly(total, people)
                    self.assertEqual(len(shares), people)
                    self.assertEqual(sum(shares), total)
                    self.assertLessEqual(max(shares) - min(shares), 1)
                    self.assertEqual(shares, sorted(shares, reverse=True))

    def test_nobody_to_pay_is_refused(self):
        with self.assertRaises(ValueError):
            split_evenly(100, 0)


class SplitBillTest(unittest.TestCase):
    def test_bills(self):
        self.assertEqual(split_bill("19.99", 2), [1000, 999])
        self.assertEqual(split_bill("45.10", 4), [1128, 1128, 1127, 1127])
        self.assertEqual(split_bill("8.20", 3), [274, 273, 273])
        self.assertEqual(split_bill("0.57", 2), [29, 28])
        self.assertEqual(split_bill("$64.99", 1), [6499])

    def test_describe(self):
        self.assertEqual(
            describe("100.00", ["Ann", "Bo", "Cy"]),
            ["Ann pays 33.34", "Bo pays 33.33", "Cy pays 33.33"],
        )
        self.assertEqual(describe("9.95", ["Ann", "Bo"]), ["Ann pays 4.98", "Bo pays 4.97"])
        self.assertEqual(format_cents(sum(split_bill("1234567.89", 7))), "1234567.89")


if __name__ == "__main__":
    unittest.main()
