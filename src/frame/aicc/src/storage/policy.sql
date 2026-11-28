CREATE TABLE IF NOT EXISTS aicc_policy (
 policy_id TEXT PRIMARY KEY, source TEXT NOT NULL, subject TEXT NOT NULL, body_json TEXT NOT NULL,
 enabled INTEGER NOT NULL, version BIGINT NOT NULL,
 created_by TEXT NOT NULL, created_at_ms BIGINT NOT NULL, updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL,
 UNIQUE (source, subject));
CREATE TABLE IF NOT EXISTS aicc_price_entry (
 entry_key TEXT PRIMARY KEY, pricing_json TEXT NOT NULL, note TEXT,
 updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_price_rate (
 currency TEXT PRIMARY KEY, credit_per_unit REAL NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_credit_account (
 user_id TEXT PRIMARY KEY, balance_micro BIGINT NOT NULL, version BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_credit_entry (
 entry_id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, user_id TEXT NOT NULL, kind TEXT NOT NULL,
 amount_micro BIGINT NOT NULL, balance_after_micro BIGINT NOT NULL, external_ref_json TEXT,
 operator TEXT NOT NULL, note TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_credit_entry_user_time ON aicc_credit_entry(user_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_credit_charge (
 attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, usage_event_id TEXT, user_id TEXT NOT NULL, app_id TEXT,
 operator_id TEXT, provider_instance_name TEXT NOT NULL, exact_model TEXT NOT NULL,
 admitted_at_ms BIGINT NOT NULL, tokens BIGINT NOT NULL, credit_micro BIGINT NOT NULL, status TEXT NOT NULL,
 pricing_json TEXT NOT NULL, settled_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_credit_charge_user_time ON aicc_credit_charge(user_id, settled_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_credit_charge_task ON aicc_credit_charge(task_id);
CREATE TABLE IF NOT EXISTS aicc_quota_counter (
 scope_key TEXT PRIMARY KEY, scope_json TEXT NOT NULL, window_start_ms BIGINT NOT NULL,
 window_end_ms BIGINT NOT NULL, used_credit_micro BIGINT NOT NULL, used_tokens BIGINT NOT NULL,
 updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_quota_audit (
 audit_id TEXT PRIMARY KEY, target_type TEXT NOT NULL, target_id TEXT NOT NULL, action TEXT NOT NULL,
 operator TEXT NOT NULL, before_json TEXT, after_json TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_audit_target ON aicc_quota_audit(target_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_usage_event_subject (
 event_id TEXT NOT NULL, subject TEXT NOT NULL, created_at_ms BIGINT NOT NULL, PRIMARY KEY (event_id, subject));
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_subject_time ON aicc_usage_event_subject(subject, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_state (
 state_key TEXT PRIMARY KEY, int_value BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_schema_meta (
 schema_key TEXT PRIMARY KEY, schema_version BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL)
