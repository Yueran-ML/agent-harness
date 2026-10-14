import unittest
from decimal import Decimal

from report import format_report, totals_by_category


class ColumnsTest(unittest.TestCase):
    def test_extra_columns_in_any_order(self):
        text = (
            "id,note,amount,category,currency\n"
            "1,lunch,8.40,food,EUR\n"
            "2,bus,2.10,transport,EUR\n"
            "3,dinner,21.00,food,EUR\n"
            "4,train,15.35,transport,EUR\n"
        )
        self.assertEqual(
            totals_by_category(text), {"food": Decimal("29.40"), "transport": Decimal("17.45")}
        )

    def test_header_names_in_any_case_and_with_spaces(self):
        text = (
            " Amount , CATEGORY ,Date\n"
            "4.00,books,2024-01-02\n"
            "6.50,books,2024-01-09\n"
            "1.25,games,2024-01-10\n"
        )
        self.assertEqual(
            totals_by_category(text), {"books": Decimal("10.50"), "games": Decimal("1.25")}
        )

    def test_quoted_fields_with_commas(self):
        text = (
            "category,note,amount\n"
            '"food, drinks","pizza, cola",9.90\n'
            'rent,"May, June",1000.00\n'
            '"food, drinks",water,0.60\n'
        )
        self.assertEqual(
            totals_by_category(text),
            {"food, drinks": Decimal("10.50"), "rent": Decimal("1000.00")},
        )


class BlankLinesTest(unittest.TestCase):
    def test_blank_lines_before_between_and_after(self):
        text = (
            "\n\n  \ncategory,amount\n\nfuel,40.00\n \n\nfuel,35.50\n\ntolls,7.80\n\n\n"
        )
        self.assertEqual(
            totals_by_category(text), {"fuel": Decimal("75.50"), "tolls": Decimal("7.80")}
        )

    def test_windows_line_ends(self):
        text = (
            "date,category,amount\r\n\r\n"
            "2024-02-01,gifts,20.00\r\n"
            "2024-02-02,gifts,5.00\r\n\r\n"
        )
        self.assertEqual(totals_by_category(text), {"gifts": Decimal("25.00")})

    def test_header_alone_gives_no_totals(self):
        self.assertEqual(totals_by_category("category,amount\n"), {})
        self.assertEqual(totals_by_category("\n\ncategory,amount\n\n"), {})


class ReportTest(unittest.TestCase):
    def test_report_of_a_reordered_export(self):
        text = "amount,category\n\n3.00,b\n4.50,a\n"
        self.assertEqual(
            format_report(totals_by_category(text)),
            "a            4.50\nb            3.00\nTotal        7.50",
        )


if __name__ == "__main__":
    unittest.main()
