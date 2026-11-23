CREATE TABLE IF NOT EXISTS notifies (notify_id TEXT PRIMARY KEY, owner_did TEXT NOT NULL, requester TEXT NOT NULL, idempotency_key TEXT NOT NULL, state TEXT NOT NULL, created_at_ms BIGINT NOT NULL, row_json TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS idx_notifies_requester_key ON notifies(requester, idempotency_key);
CREATE INDEX IF NOT EXISTS idx_notifies_owner_created ON notifies(owner_did, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_notifies_state ON notifies(state);
