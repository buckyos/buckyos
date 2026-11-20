CREATE TABLE IF NOT EXISTS aicc_quota_rule (
 rule_id TEXT PRIMARY KEY, source TEXT NOT NULL, object_type TEXT NOT NULL,
 user_id TEXT NOT NULL, app_id TEXT NOT NULL, resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
 scope_key TEXT NOT NULL, kind TEXT NOT NULL, period TEXT, limit_tokens BIGINT, account_id TEXT,
 enabled INTEGER NOT NULL, deleted INTEGER NOT NULL, version BIGINT NOT NULL,
 pending_period TEXT, pending_effective_at_ms BIGINT, pending_conflict INTEGER NOT NULL DEFAULT 0,
 created_by TEXT NOT NULL, created_at_ms BIGINT NOT NULL, updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_user ON aicc_quota_rule(user_id, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_app ON aicc_quota_rule(app_id, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_scope ON aicc_quota_rule(scope_key, deleted);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_rule_pending ON aicc_quota_rule(pending_effective_at_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_counter (
 counter_key TEXT PRIMARY KEY, scope_key TEXT NOT NULL, period TEXT NOT NULL,
 window_start_ms BIGINT NOT NULL, window_end_ms BIGINT NOT NULL,
 settled_tokens BIGINT NOT NULL, reserved_tokens BIGINT NOT NULL, pending_tokens BIGINT NOT NULL,
 updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_counter_end ON aicc_quota_counter(window_end_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_account (
 account_id TEXT PRIMARY KEY, scope_key TEXT NOT NULL UNIQUE, object_type TEXT NOT NULL,
 user_id TEXT NOT NULL, app_id TEXT NOT NULL, resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
 granted_tokens BIGINT NOT NULL, settled_tokens BIGINT NOT NULL, reserved_tokens BIGINT NOT NULL,
 pending_tokens BIGINT NOT NULL, start_at_ms BIGINT NOT NULL, version BIGINT NOT NULL,
 updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_quota_grant (
 grant_id TEXT PRIMARY KEY, account_id TEXT NOT NULL, kind TEXT NOT NULL, amount_tokens BIGINT NOT NULL,
 operator TEXT NOT NULL, note TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_grant_account ON aicc_quota_grant(account_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_quota_attempt (
 attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, user_id TEXT NOT NULL, app_id TEXT NOT NULL,
 operator_id TEXT, provider_instance_name TEXT NOT NULL, exact_model TEXT NOT NULL, method TEXT NOT NULL,
 admitted_at_ms BIGINT NOT NULL, budget_tokens BIGINT NOT NULL, hard_limit INTEGER NOT NULL,
 state TEXT NOT NULL, charged_tokens BIGINT, reconcile_deadline_ms BIGINT NOT NULL,
 settled_at_ms BIGINT, applied_json TEXT NOT NULL, attribution_json TEXT);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_state ON aicc_quota_attempt(state, reconcile_deadline_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_user_time ON aicc_quota_attempt(user_id, admitted_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_app_time ON aicc_quota_attempt(app_id, admitted_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_attempt_task ON aicc_quota_attempt(task_id);
CREATE TABLE IF NOT EXISTS aicc_quota_audit (
 audit_id TEXT PRIMARY KEY, target_type TEXT NOT NULL, target_id TEXT NOT NULL, action TEXT NOT NULL,
 operator TEXT NOT NULL, before_json TEXT, after_json TEXT, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_quota_audit_target ON aicc_quota_audit(target_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_visibility_policy (
 subject TEXT PRIMARY KEY, entries_json TEXT NOT NULL, updated_by TEXT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_state (
 state_key TEXT PRIMARY KEY, int_value BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
