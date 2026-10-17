# Inventory reservation service

A small reservation service: tenants reserve units of a SKU from a shared
stock. `service.ReservationService.reserve` is the public entry.

## Contract

1. **Insufficient stock never changes stock.** When a reservation asks for more
   units than are available, it fails and the stock stays exactly as it was.
2. **Idempotency keys are scoped to a tenant.** A `request_id` identifies a
   request *within its tenant*: two tenants may use the same `request_id` for
   different requests, and each gets its own result.
3. **A retried request is not applied twice.** The same tenant sending the same
   `request_id` again gets the first result back; the stock is reduced once.

## Layout

- `inventory.py` — the stock per SKU and the reservation arithmetic.
- `idempotency.py` — the cache of results by idempotency key.
- `service.py` — the public entry wiring both.
- `tests/test_happy_path.py` — the normal path (`python3 -m unittest discover -s tests`).
