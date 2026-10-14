import unittest

from warehouse.errors import (
    InsufficientStockError,
    InvalidQuantityError,
    InventoryError,
    UnknownItemError,
)
from warehouse.inventory import Inventory


class RemoveTest(unittest.TestCase):
    def test_more_than_in_stock_raises_and_takes_nothing(self):
        for stock, asked in ((3, 5), (1, 2), (10, 11), (4, 400)):
            with self.subTest(stock=stock, asked=asked):
                inventory = Inventory({"MUG-01": stock, "TEE-02": 7})
                with self.assertRaises(InsufficientStockError):
                    inventory.remove("MUG-01", asked)
                self.assertEqual(inventory.snapshot(), {"MUG-01": stock, "TEE-02": 7})

    def test_error_is_an_inventory_error(self):
        self.assertTrue(issubclass(InsufficientStockError, InventoryError))
        inventory = Inventory({"MUG-01": 1})
        with self.assertRaises(InventoryError):
            inventory.remove("MUG-01", 2)

    def test_error_says_what_was_asked_for_and_what_there_is(self):
        inventory = Inventory({"MUG-01": 3})
        with self.assertRaises(InsufficientStockError) as caught:
            inventory.remove("MUG-01", 5)
        self.assertEqual(caught.exception.sku, "MUG-01")
        self.assertEqual(caught.exception.requested, 5)
        self.assertEqual(caught.exception.available, 3)

    def test_all_of_the_stock_can_be_taken(self):
        inventory = Inventory({"MUG-01": 3})
        inventory.remove("MUG-01", 3)
        self.assertEqual(inventory.quantity("MUG-01"), 0)
        with self.assertRaises(InsufficientStockError) as caught:
            inventory.remove("MUG-01", 1)
        self.assertEqual(caught.exception.available, 0)
        self.assertEqual(inventory.quantity("MUG-01"), 0)

    def test_successive_removals(self):
        inventory = Inventory({"MUG-01": 5})
        inventory.remove("MUG-01", 2)
        inventory.remove("MUG-01", 2)
        with self.assertRaises(InsufficientStockError) as caught:
            inventory.remove("MUG-01", 2)
        self.assertEqual(caught.exception.requested, 2)
        self.assertEqual(caught.exception.available, 1)
        self.assertEqual(inventory.quantity("MUG-01"), 1)

    def test_other_errors_are_as_before(self):
        inventory = Inventory({"MUG-01": 3})
        with self.assertRaises(UnknownItemError):
            inventory.remove("CAP-03", 1)
        for quantity in (0, -1, 1.5):
            with self.subTest(quantity=quantity):
                with self.assertRaises(InvalidQuantityError):
                    inventory.remove("MUG-01", quantity)
        self.assertEqual(inventory.quantity("MUG-01"), 3)


class FulfilTest(unittest.TestCase):
    def test_order_that_can_be_filled(self):
        inventory = Inventory({"MUG-01": 3, "TEE-02": 2, "CAP-03": 9})
        inventory.fulfil([("MUG-01", 3), ("CAP-03", 4)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 0, "TEE-02": 2, "CAP-03": 5})

    def test_short_last_line_takes_nothing(self):
        inventory = Inventory({"MUG-01": 3, "TEE-02": 2, "CAP-03": 9})
        with self.assertRaises(InsufficientStockError):
            inventory.fulfil([("MUG-01", 1), ("CAP-03", 4), ("TEE-02", 5)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 3, "TEE-02": 2, "CAP-03": 9})

    def test_same_item_on_two_lines(self):
        inventory = Inventory({"MUG-01": 5, "TEE-02": 2})
        with self.assertRaises(InsufficientStockError):
            inventory.fulfil([("MUG-01", 3), ("TEE-02", 1), ("MUG-01", 3)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 5, "TEE-02": 2})
        inventory.fulfil([("MUG-01", 3), ("MUG-01", 2)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 0, "TEE-02": 2})

    def test_unknown_item_takes_nothing(self):
        inventory = Inventory({"MUG-01": 3})
        with self.assertRaises(InventoryError):
            inventory.fulfil([("MUG-01", 1), ("CAP-03", 1)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 3})


if __name__ == "__main__":
    unittest.main()
