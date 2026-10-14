import unittest

from mdtable import render_table


class RenderTableTest(unittest.TestCase):
    def test_values_wider_than_headers(self):
        self.assertEqual(
            render_table(["Name", "Qty"], [["Tea", 2], ["Coffee", 10]]),
            "| Name   | Qty |\n"
            "| ------ | --- |\n"
            "| Tea    | 2   |\n"
            "| Coffee | 10  |\n",
        )

    def test_right_aligned_column(self):
        self.assertEqual(
            render_table(["Item", "Qty"], [["Teapot", 1], ["Cup", 12]], align=[None, "right"]),
            "| Item   | Qty |\n"
            "| ------ | --: |\n"
            "| Teapot |   1 |\n"
            "| Cup    |  12 |\n",
        )

    def test_headers_wider_than_values(self):
        self.assertEqual(
            render_table(["Product", "In stock"], [["Tea", 2], ["Mug", 15]]),
            "| Product | In stock |\n"
            "| ------- | -------- |\n"
            "| Tea     | 2        |\n"
            "| Mug     | 15       |\n",
        )


if __name__ == "__main__":
    unittest.main()
