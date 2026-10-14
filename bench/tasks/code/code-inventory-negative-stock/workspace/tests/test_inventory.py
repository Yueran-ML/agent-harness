import unittest

from warehouse.errors import InsufficientStockError, UnknownItemError
from warehouse.inventory import Inventory


class InventoryTest(unittest.TestCase):
    def test_add_and_remove(self):
        inventory = Inventory({"MUG-01": 10})
        inventory.add("MUG-01", 5)
        inventory.remove("MUG-01", 4)
        self.assertEqual(inventory.quantity("MUG-01"), 11)

    def test_remove_unknown_item(self):
        with self.assertRaises(UnknownItemError):
            Inventory().remove("MUG-01", 1)

    def test_remove_more_than_in_stock(self):
        inventory = Inventory({"MUG-01": 3})
        with self.assertRaises(InsufficientStockError):
            inventory.remove("MUG-01", 5)
        self.assertEqual(inventory.quantity("MUG-01"), 3)

    def test_fulfil(self):
        inventory = Inventory({"MUG-01": 3, "TEE-02": 2})
        inventory.fulfil([("MUG-01", 2), ("TEE-02", 2)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 1, "TEE-02": 0})

    def test_fulfil_an_order_that_cannot_be_filled(self):
        inventory = Inventory({"MUG-01": 3, "TEE-02": 1})
        with self.assertRaises(InsufficientStockError):
            inventory.fulfil([("MUG-01", 2), ("TEE-02", 5)])
        self.assertEqual(inventory.snapshot(), {"MUG-01": 3, "TEE-02": 1})


if __name__ == "__main__":
    unittest.main()
