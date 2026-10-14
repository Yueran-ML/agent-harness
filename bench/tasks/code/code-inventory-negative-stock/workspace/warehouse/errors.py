"""What can go wrong with the stock."""


class InventoryError(Exception):
    """Something that was asked of the inventory cannot be done."""


class UnknownItemError(InventoryError):
    """There is no item with this SKU."""

    def __init__(self, sku):
        super().__init__(f"there is no item with the SKU {sku!r}")
        self.sku = sku


class InvalidQuantityError(InventoryError):
    """A quantity is not a whole number above zero."""

    def __init__(self, quantity):
        super().__init__(f"a quantity must be a whole number above zero, not {quantity!r}")
        self.quantity = quantity
