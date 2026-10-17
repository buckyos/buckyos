"""The public entry: reservations with idempotent retries."""

from idempotency import IdempotencyCache
from inventory import InsufficientStock, Inventory


class ReservationService:
    def __init__(self, stock=None):
        self.inventory = Inventory(stock)
        self.cache = IdempotencyCache()

    def reserve(self, tenant_id, request_id, sku, quantity):
        """Reserve `quantity` units of `sku` for `tenant_id`.

        Returns {"ok": True, "remaining": n} or {"ok": False, "error": text}.
        A retried (tenant_id, request_id) returns the first result.
        """
        done = self.cache.get(tenant_id, request_id)
        if done is not None:
            return done
        try:
            remaining = self.inventory.reserve(sku, quantity)
            result = {"ok": True, "remaining": remaining}
        except InsufficientStock as e:
            result = {"ok": False, "error": str(e)}
        return self.cache.put(tenant_id, request_id, result)

    def stock(self, sku):
        return self.inventory.available(sku)
