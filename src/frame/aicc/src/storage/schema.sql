CREATE TABLE IF NOT EXISTS aicc_schema_meta (
 schema_key TEXT PRIMARY KEY, schema_version BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS aicc_provider_inventory_lkgs (
 provider_instance_name TEXT PRIMARY KEY, schema_version INTEGER NOT NULL DEFAULT 1,
 provider_profile_id TEXT NOT NULL, protocol_adapter_id TEXT NOT NULL,
 provider_model_list_fingerprint TEXT NOT NULL, metadata_applied_seq BIGINT NOT NULL,
 inventory_revision TEXT, discovered_at_ms BIGINT NOT NULL, snapshot_json TEXT NOT NULL,
 snapshot_sha256 TEXT NOT NULL, created_at_ms BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_provider_inventory_lkgs_updated ON aicc_provider_inventory_lkgs(updated_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_provider_inventory_lkgs_metadata ON aicc_provider_inventory_lkgs(metadata_applied_seq);
CREATE TABLE IF NOT EXISTS aicc_usage_event (
 event_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL,
 caller_app_id TEXT, task_id TEXT NOT NULL, trace_id TEXT, idempotency_key TEXT, method TEXT NOT NULL,
 capability TEXT NOT NULL, request_model TEXT NOT NULL, provider_instance_name TEXT NOT NULL,
 provider_model TEXT NOT NULL, input_tokens BIGINT, output_tokens BIGINT, total_tokens BIGINT,
 request_units BIGINT, usage_json TEXT NOT NULL, finance_snapshot_json TEXT,
 finance_amount REAL, finance_currency TEXT, finance_valid INTEGER NOT NULL DEFAULT 0,
 created_at_ms BIGINT NOT NULL, operator_id TEXT, attribution_json TEXT,
 credit_micro BIGINT, credit_status TEXT, fallback_from TEXT);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_time ON aicc_usage_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_time ON aicc_usage_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_trace_time ON aicc_usage_event(trace_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_user_time ON aicc_usage_event(user_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_method_time ON aicc_usage_event(method, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_provider_instance_time ON aicc_usage_event(provider_instance_name, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_model_time ON aicc_usage_event(provider_model, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_usage_event_request_model_time ON aicc_usage_event(request_model, created_at_ms);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_task ON aicc_usage_event(tenant_id, task_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aicc_usage_event_tenant_idem ON aicc_usage_event(tenant_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS aicc_execution_record (
 tenant_id TEXT NOT NULL, method TEXT NOT NULL, idempotency_key TEXT NOT NULL,
 task_id TEXT NOT NULL UNIQUE, body_fingerprint TEXT NOT NULL, state TEXT NOT NULL,
 record_json TEXT NOT NULL, created_at_ms BIGINT NOT NULL, expires_at_ms BIGINT NOT NULL,
 PRIMARY KEY (tenant_id, method, idempotency_key));
CREATE INDEX IF NOT EXISTS idx_aicc_execution_record_state ON aicc_execution_record(state, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_execution_record_expiry ON aicc_execution_record(expires_at_ms);
CREATE TABLE IF NOT EXISTS aicc_route_trace_event (
 trace_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, caller_app_id TEXT, task_id TEXT NOT NULL,
 request_id TEXT, route_id TEXT, provider_trace_id TEXT, request_model TEXT NOT NULL,
 selected_exact_model TEXT, provider_instance_name TEXT, api_type TEXT NOT NULL,
 scheduler_profile TEXT, outcome TEXT, route_trace_json TEXT NOT NULL, created_at_ms BIGINT NOT NULL,
 fallback_applied INTEGER NOT NULL DEFAULT 0, warning_count BIGINT NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_time ON aicc_route_trace_event(created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_tenant_time ON aicc_route_trace_event(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_task_time ON aicc_route_trace_event(task_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_fallback_time ON aicc_route_trace_event(tenant_id, fallback_applied, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_route_trace_event_warning_time ON aicc_route_trace_event(tenant_id, warning_count, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_artifact_scope (
 obj_id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, user_id TEXT NOT NULL,
 caller_app_id TEXT NOT NULL, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_artifact_scope_tenant ON aicc_artifact_scope(tenant_id, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_artifact_url_source (
 url_hash TEXT PRIMARY KEY, url TEXT NOT NULL,
 provider_instance_name TEXT NOT NULL, protocol_adapter_id TEXT NOT NULL,
 origin_provider TEXT NOT NULL, artifact_id TEXT, content_digest TEXT, expires_at_ms BIGINT,
 tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, caller_app_id TEXT NOT NULL,
 request_id TEXT NOT NULL, created_at_ms BIGINT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_aicc_artifact_url_source_tenant ON aicc_artifact_url_source(tenant_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_aicc_artifact_url_source_provider ON aicc_artifact_url_source(provider_instance_name, created_at_ms);
CREATE TABLE IF NOT EXISTS aicc_provider_artifact (
 content_digest TEXT NOT NULL, obj_id TEXT NOT NULL,
 provider_instance_name TEXT NOT NULL, origin_provider TEXT NOT NULL, tenant_id TEXT NOT NULL,
 artifact_id TEXT NOT NULL, expires_at_ms BIGINT, created_at_ms BIGINT NOT NULL,
 CHECK ((content_digest='') <> (obj_id='')),
 PRIMARY KEY (content_digest, obj_id, provider_instance_name, origin_provider, tenant_id));
CREATE INDEX IF NOT EXISTS idx_aicc_provider_artifact_provider
 ON aicc_provider_artifact(provider_instance_name, origin_provider, created_at_ms);
