import unittest

from mdtable import render_table
from report import sales_table


def lines(*texts):
    return "".join(text + "\n" for text in texts)


class WidthsTest(unittest.TestCase):
    def test_values_wider_than_headers(self):
        self.assertEqual(
            render_table(["Name", "Qty"], [["Tea", 2], ["Coffee", 10]]),
            lines("| Name   | Qty |", "| ------ | --- |", "| Tea    | 2   |", "| Coffee | 10  |"),
        )

    def test_headers_wider_than_values(self):
        self.assertEqual(
            render_table(["Product", "In stock"], [["Tea", 2], ["Mug", 15]]),
            lines(
                "| Product | In stock |",
                "| ------- | -------- |",
                "| Tea     | 2        |",
                "| Mug     | 15       |",
            ),
        )

    def test_table_without_rows(self):
        self.assertEqual(
            render_table(["Name", "Email address"], []),
            lines("| Name | Email address |", "| ---- | ------------- |"),
        )

    def test_narrow_columns_are_three_wide(self):
        self.assertEqual(
            render_table(["a", "b"], [["x", ""]]),
            lines("| a   | b   |", "| --- | --- |", "| x   |     |"),
        )

    def test_escaped_pipes_in_values(self):
        self.assertEqual(
            render_table(["Expr", "Means"], [["a|b", "a or b"], ["a&b", "a and b"]]),
            lines(
                "| Expr | Means   |",
                "| ---- | ------- |",
                "| a\\|b | a or b  |",
                "| a&b  | a and b |",
            ),
        )

    def test_escaped_pipe_in_a_header(self):
        self.assertEqual(
            render_table(["A|B", "C"], [[1, 2]]),
            lines("| A\\|B | C   |", "| ---- | --- |", "| 1    | 2   |"),
        )

    def test_widths_are_counted_with_len(self):
        self.assertEqual(
            render_table(["Item", "説明文です"], [["pen", "ペン"], ["ink", "インク"]]),
            lines(
                "| Item | 説明文です |",
                "| ---- | ----- |",
                "| pen  | ペン    |",
                "| ink  | インク   |",
            ),
        )

    def test_aligned_columns_with_wide_headers(self):
        self.assertEqual(
            render_table(
                ["Region", "Revenue"], [["North", 1200], ["South", 75]], align=[None, "right"]
            ),
            lines(
                "| Region | Revenue |",
                "| ------ | ------: |",
                "| North  |    1200 |",
                "| South  |      75 |",
            ),
        )
        self.assertEqual(
            render_table(["Status"], [["ok"], ["fail"]], align=["center"]),
            lines("| Status |", "| :----: |", "|   ok   |", "|  fail  |"),
        )
        self.assertEqual(
            render_table(["Name"], [["Al"]], align=["left"]),
            lines("| Name |", "| :--- |", "| Al   |"),
        )

    def test_every_line_is_as_long_as_the_others(self):
        tables = [
            (["#", "Description"], [[1, "short"], [22, "a much longer description"]]),
            (["Long header here", "x"], [["a", "b|c|d"], ["", "line\nbreak"]]),
            (["Ratio", "Flag"], [[0.5, True], [12.25, False]]),
            (["Path | Name", "Size"], [["/tmp", 4096], ["/", 0]]),
        ]
        for headers, rows in tables:
            with self.subTest(headers=headers):
                text = render_table(headers, rows)
                self.assertEqual(len({len(line) for line in text.splitlines()}), 1, text)
                self.assertEqual(len(text.splitlines()), len(rows) + 2)


class ReportTest(unittest.TestCase):
    def test_sales_table(self):
        self.assertEqual(
            sales_table({"South": (3, 99.5), "North": (12, 1500.0)}),
            lines(
                "| Region | Units |  Revenue |",
                "| ------ | ----: | -------: |",
                "| North  |    12 | 1,500.00 |",
                "| South  |     3 |    99.50 |",
            ),
        )


if __name__ == "__main__":
    unittest.main()
