"""Stock per SKU and the reservation arithmetic."""


class InsufficientStock(Exception):
    pass


class Inventory:
    def __init__(self, stock=None):
        self._stock = dict(stock or {})

    def available(self, sku):
        return self._stock.get(sku, 0)

    def reserve(self, sku, quantity):
        """Take `quantity` units of `sku`; fail when there are not enough."""
        if quantity <= 0:
            raise ValueError("quantity must be positive")
        self._stock[sku] = self._stock.get(sku, 0) - quantity
        if self._stock[sku] < 0:
            raise InsufficientStock(
                f"{sku}: requested {quantity}, only {self._stock[sku] + quantity} available"
            )
        return self._stock[sku]
