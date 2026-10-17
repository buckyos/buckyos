"""Results of handled requests, by idempotency key."""


class IdempotencyCache:
    def __init__(self):
        self._results = {}

    def key(self, tenant_id, request_id):
        return request_id

    def get(self, tenant_id, request_id):
        return self._results.get(self.key(tenant_id, request_id))

    def put(self, tenant_id, request_id, result):
        self._results[self.key(tenant_id, request_id)] = result
        return result
