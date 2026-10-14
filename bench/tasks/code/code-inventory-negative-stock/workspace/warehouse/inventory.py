"""How many of each item a warehouse holds."""

from warehouse.errors import InvalidQuantityError, UnknownItemError


class Inventory:
    """A count of each item, by SKU."""

    def __init__(self, stock=None):
        """Start with ``stock``, a dict of SKUs and how many of each there are."""
        self._stock = {}
        for sku, quantity in (stock or {}).items():
            self.add(sku, quantity)

    def quantity(self, sku):
        """Return how many of an item there are.

        Raises:
            UnknownItemError: There is no item with this SKU.
        """
        if sku not in self._stock:
            raise UnknownItemError(sku)
        return self._stock[sku]

    def add(self, sku, quantity):
        """Put ``quantity`` of an item in stock. An item not seen before is added."""
        _check(quantity)
        self._stock[sku] = self._stock.get(sku, 0) + quantity

    def remove(self, sku, quantity):
        """Take ``quantity`` of an item out of stock.

        Raises:
            UnknownItemError: There is no item with this SKU.
            InvalidQuantityError: The quantity is not a whole number above zero.
        """
        _check(quantity)
        if sku not in self._stock:
            raise UnknownItemError(sku)
        self._stock[sku] -= quantity

    def fulfil(self, order):
        """Take out of stock what an order asks for.

        Args:
            order: The lines of the order: pairs of a SKU and a quantity.
        """
        for sku, quantity in order:
            self.remove(sku, quantity)

    def snapshot(self):
        """Return how many of each item there are, as a dict by SKU."""
        return dict(self._stock)


def _check(quantity):
    """Raise ``InvalidQuantityError`` unless ``quantity`` is a whole number above zero."""
    if isinstance(quantity, bool) or not isinstance(quantity, int) or quantity < 1:
        raise InvalidQuantityError(quantity)
