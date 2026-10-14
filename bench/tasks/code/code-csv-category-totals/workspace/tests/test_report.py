import unittest
from decimal import Decimal

from report import format_report, totals_by_category


class TotalsTest(unittest.TestCase):
    def test_simple_export(self):
        text = "category,amount\nfood,10.00\nrent,500\nfood,2.50\n"
        self.assertEqual(
            totals_by_category(text), {"food": Decimal("12.50"), "rent": Decimal("500")}
        )

    def test_columns_are_found_by_their_names(self):
        text = (
            "Date,Amount,Category\n"
            "2024-05-01,12.50,groceries\n"
            "2024-05-02,3.20,transport\n"
            "2024-05-03,7.25,groceries\n"
        )
        self.assertEqual(
            totals_by_category(text),
            {"groceries": Decimal("19.75"), "transport": Decimal("3.20")},
        )

    def test_blank_lines_are_skipped(self):
        text = "\ncategory,amount\nfood,10.00\n\n   \nrent,500\n\n"
        self.assertEqual(
            totals_by_category(text), {"food": Decimal("10.00"), "rent": Decimal("500")}
        )


class FormatReportTest(unittest.TestCase):
    def test_one_line_per_category_and_the_sum(self):
        report = format_report({"rent": Decimal("500"), "food": Decimal("12.5")})
        self.assertEqual(report, "food        12.50\nrent       500.00\nTotal      512.50")


if __name__ == "__main__":
    unittest.main()
