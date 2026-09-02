import { CANONICAL_API_TYPES, methodsForApiType } from "./canonical.ts";
import type { AcceptanceCase } from "./types.ts";

type CaseSeed = Pick<
  AcceptanceCase,
  | "case_id"
  | "layer"
  | "priority"
  | "tags"
  | "method"
  | "api_type"
  | "mock_scenario"
  | "expected_error_class"
> & Partial<AcceptanceCase>;

function makeCase(seed: CaseSeed): AcceptanceCase {
  return {
    case_id: seed.case_id,
    layer: seed.layer,
    priority: seed.priority,
    tags: seed.tags,
    input_entry: seed.input_entry ?? "zone_gateway",
    user: seed.user ?? "acceptance-user-a",
    session: seed.session ?? "isolated-per-case",
    provider_driver: seed.provider_driver ?? "mock",
    provider_instance: seed.provider_instance ?? null,
    model_selector: seed.model_selector ?? null,
    api_type: seed.api_type,
    method: seed.method,
    required_capabilities: seed.required_capabilities ?? [],
    disabled_capabilities: seed.disabled_capabilities ?? [],
    fixtures: seed.fixtures ?? [],
    mock_scenario: seed.mock_scenario,
    expected_exact_model: seed.expected_exact_model ?? null,
    expected_provider_instance: seed.expected_provider_instance ?? null,
    expected_task_status: seed.expected_task_status ??
      (seed.expected_error_class ? "failed" : "succeeded"),
    expected_error_class: seed.expected_error_class,
    expected_output: seed.expected_output ?? {
      kinds: seed.expected_error_class ? [] : ["structured"],
      attachment_count: { min: 0, max: 0 },
      mime_types: [],
    },
    semantic_rubric: seed.semantic_rubric ?? [],
    timeout_ms: seed.timeout_ms ?? 30_000,
    max_attempts: seed.max_attempts ?? 1,
    estimated_cost_usd: seed.estimated_cost_usd ?? 0,
    cleanup: seed.cleanup ?? ["reset_mock", "remove_case_overlay"],
  };
}

const ROUTING_CASES: AcceptanceCase[] = [
  ["exact_model_hits_instance", null],
  ["logical_model_selects_candidate", null],
  ["metadata_variant_expands_exact_model", null],
  ["version_exact_rule", null],
  ["version_pattern_rule", null],
  ["version_default_rule", null],
  ["legal_missing_model", "model_not_found"],
  ["invalid_exact_model", "invalid_request"],
  ["invalid_logical_path", "invalid_request"],
  ["missing_provider_instance", "provider_not_found"],
  ["disabled_model", "no_route"],
  ["offline_model", "no_route"],
  ["unmounted_model", "no_route"],
  ["corrupt_metadata", "baseline_mismatch"],
  ["strict_no_fallback", "provider_runtime_failed"],
  ["parent_fallback", null],
  ["target_logical_fallback", null],
  ["target_exact_fallback", null],
  ["exact_default_no_fallback", "provider_runtime_failed"],
  ["fallback_api_type_boundary", "no_route"],
  ["fallback_loop", "invalid_config"],
  ["fallback_max_depth", "invalid_config"],
  ["provider_allow", null],
  ["provider_deny", null],
  ["local_only", null],
  ["privacy_boundary", null],
  ["health_filter", null],
  ["quota_filter", null],
  ["budget_filter", null],
  ["context_limit_filter", null],
  ["output_limit_filter", null],
  ["locked_policy_cannot_override", "policy_denied"],
  ["missing_metadata_is_conservative", null],
  ["min_line_admission", "no_route"],
  ["disable_line_applied", null],
  ["auto_mount_admission", null],
  ["manual_mount_requires_mapping", "no_route"],
  ["global_exact_model_weight", null],
  ["logical_exact_model_weight", null],
  ["provider_instance_weight", null],
  ["system_config_then_request_overlay", null],
] .map(([name, error]) => makeCase({
  case_id: `t1.route.${name}`,
  layer: "T1",
  priority: "P0",
  tags: ["routing"],
  method: "route.resolve",
  api_type: "llm",
  mock_scenario: String(name),
  expected_error_class: error as string | null,
}));

const PROFILES = [
  "cost_first",
  "latency_first",
  "quality_first",
  "balanced",
  "local_first",
  "strict_local",
];

const PROFILE_CASES = PROFILES.map((profile) => makeCase({
  case_id: `t1.scheduler.profile.${profile}`,
  layer: "T1",
  priority: "P0",
  tags: ["routing", "scheduler", "profile"],
  method: "route.resolve",
  api_type: "llm",
  mock_scenario: profile,
  expected_error_class: null,
}));

const CANONICAL_ROUTING_CASES = CANONICAL_API_TYPES.flatMap((apiType) =>
  methodsForApiType(apiType).map((method) => makeCase({
    case_id: `t1.route.api_type.${apiType}.${method}`,
    layer: "T1",
    priority: "P0",
    tags: ["routing", "api_type", apiType],
    method,
    api_type: apiType,
    mock_scenario: `route_api_type_${apiType}`,
    expected_error_class: null,
  }))
);

const PROVIDER_BOUNDARY_TRIGGER_CASES: AcceptanceCase[] = [
  ["rate_limit_fallback", "rate_limit", null],
  ["server_error_fallback", "provider_5xx", null],
  ["connection_failure_fallback", "connection_failed", null],
  ["timeout_fallback", "timeout_short", null],
  ["malformed_response_rejected", "malformed_response", "provider_protocol_failed"],
  ["wrong_mime_rejected", "wrong_mime", "resource_failed"],
  ["missing_usage_rejected", "missing_usage", "usage_failed"],
].map(([name, scenario, error]) => makeCase({
  case_id: `t1.runtime_boundary.${name}`,
  layer: "T1",
  priority: "P0",
  tags: ["routing", "runtime_boundary"],
  method: "chat.completions.create",
  api_type: "llm",
  mock_scenario: scenario as string,
  expected_error_class: error as string | null,
}));

const HISTORY_CASES: AcceptanceCase[] = [
  makeCase({
    case_id: "t1.history.same_session_reuses_exact_model",
    layer: "T1",
    priority: "P0",
    tags: ["routing", "history", "soft_preference"],
    method: "chat.completions.create",
    api_type: "llm",
    session: "history-session-a",
    mock_scenario: "history_exact_model_still_eligible",
    expected_exact_model: "gpt-4@openai-mock-a",
    expected_provider_instance: "openai-mock-a",
    expected_error_class: null,
  }),
  ...[
    "api_type_changed",
    "required_capability_changed",
    "disabled_capability_changed",
    "provider_denied",
    "instance_unhealthy",
    "quota_exhausted",
    "budget_exhausted",
    "local_only_changed",
    "context_limit_exceeded",
    "output_limit_exceeded",
    "locked_policy_changed",
  ].map((reason) => makeCase({
    case_id: `t1.history.hard_constraint_overrides.${reason}`,
    layer: "T1",
    priority: "P0",
    tags: ["routing", "history", "hard_constraint"],
    method: "chat.completions.create",
    api_type: "llm",
    session: "history-session-a",
    mock_scenario: `history_${reason}`,
    expected_exact_model: ["disabled_capability_changed", "budget_exhausted", "local_only_changed", "context_limit_exceeded", "output_limit_exceeded"].includes(reason)
      ? null
      : "gpt-5@openai-mock-b",
    expected_provider_instance: ["disabled_capability_changed", "budget_exhausted", "local_only_changed", "context_limit_exceeded", "output_limit_exceeded"].includes(reason)
      ? null
      : "openai-mock-b",
    expected_error_class: ["disabled_capability_changed", "budget_exhausted", "local_only_changed", "context_limit_exceeded", "output_limit_exceeded"].includes(reason)
      ? "no_route"
      : null,
  })),
  makeCase({
    case_id: "t1.history.sessions_do_not_leak",
    layer: "T1",
    priority: "P0",
    tags: ["routing", "history", "isolation"],
    method: "chat.completions.create",
    api_type: "llm",
    session: "history-session-b",
    mock_scenario: "history_cross_session_isolation",
    expected_exact_model: "gpt-5@openai-mock-b",
    expected_provider_instance: "openai-mock-b",
    expected_error_class: null,
  }),
];

const CROSS_CUTTING_CASES: AcceptanceCase[] = [
  ["task.immediate_succeeded", "success", null],
  ["task.running_succeeded", "async_success", null],
  ["task.running_failed", "async_failed", "task_lifecycle_failed"],
  ["task.cancelled", "async_cancel", null],
  ["task.unknown", "unknown_task", "task_lifecycle_failed"],
  ["task.idempotency_conflict_different_body", "idempotency_conflict", null],
  ["task.concurrent_idempotency", "concurrent_idempotency", null],
  ["task.concurrent_completion", "concurrent_completion", null],
  ["task.terminal_idempotent", "duplicate_terminal", null],
  ["task.reload_recovery", "reload_recovery", null],
  ["task.restart_recovery", "restart_recovery", null],
  ["usage.success_once", "usage_success", null],
  ["usage.idempotent_no_double_charge", "usage_idempotent", null],
  ["usage.fallback_attempts_attributed", "usage_fallback", null],
  ["security.no_token", "no_token", "security_failed"],
  ["security.invalid_token", "invalid_token", "security_failed"],
  ["security.expired_token", "expired_token", "security_failed"],
  ["security.cross_tenant", "cross_tenant", "security_failed"],
  ["security.cross_tenant_task_cancel", "cross_tenant_task_cancel", "security_failed"],
  ["security.cross_tenant_usage", "cross_tenant_usage", "security_failed"],
  ["security.cross_tenant_message", "cross_tenant_message", "security_failed"],
  ["security.cross_tenant_object", "cross_tenant_object", "security_failed"],
  ["security.rbac_admin_method", "rbac_admin_method", "security_failed"],
  ["config.reload_valid", "reload_valid", null],
  ["config.reload_invalid_keeps_old", "reload_invalid", null],
  ["config.provider_instance_isolation", "instance_isolation", null],
  ["config.provider_add_refresh", "provider_add_refresh", null],
  ["config.provider_validate_rejects_duplicate", "provider_validate_rejects_duplicate", null],
  ["config.provider_delete_isolation", "provider_delete_isolation", null],
  ["config.provider_update_rollback", "provider_update_rollback", null],
  ["config.restart_consistency", "restart_consistency", null],
  ["observability.correlation", "correlation", null],
  ["observability.redaction", "redaction", null],
].map(([suffix, scenario, error]) => makeCase({
  case_id: `t1.${suffix}`,
  layer: "T1",
  priority: "P0",
  tags: [String(suffix).split(".")[0]],
  method: String(suffix).startsWith("config.")
    ? "service.reload_settings"
    : ["task.running_succeeded", "task.running_failed", "task.cancelled", "task.terminal_idempotent", "task.reload_recovery", "task.restart_recovery"].includes(String(suffix))
    ? "video.txt2video"
    : "chat.completions.create",
  api_type: String(suffix).startsWith("config.")
    ? null
    : ["task.running_succeeded", "task.running_failed", "task.cancelled", "task.terminal_idempotent", "task.reload_recovery", "task.restart_recovery"].includes(String(suffix))
    ? "video.txt2video"
    : "llm",
  mock_scenario: scenario as string,
  expected_error_class: error as string | null,
}));

const EMBEDDING_BOUNDARY_CASES: AcceptanceCase[] = [
  makeCase({
    case_id: "t1.embedding.large_batch_artifact",
    layer: "T1",
    priority: "P0",
    tags: ["typed_boundary", "embedding", "artifact"],
    method: "embedding.text",
    api_type: "embedding.text",
    mock_scenario: "success",
    expected_error_class: null,
    fixtures: ["unique_fact_text"],
    expected_output: {
      kinds: ["artifact"],
      attachment_count: { min: 1, max: 1 },
      mime_types: ["application/vnd.buckyos.embedding+json"],
    },
  }),
  makeCase({
    case_id: "t1.embedding.space_mismatch_rejected",
    layer: "T1",
    priority: "P0",
    tags: ["typed_boundary", "embedding", "routing"],
    method: "embedding.text",
    api_type: "embedding.text",
    mock_scenario: "success",
    expected_error_class: "provider_protocol_failed",
    fixtures: ["unique_fact_text"],
  }),
];

export function buildStaticManifest(): AcceptanceCase[] {
  return [
    ...ROUTING_CASES,
    ...PROFILE_CASES,
    ...CANONICAL_ROUTING_CASES,
    ...PROVIDER_BOUNDARY_TRIGGER_CASES,
    ...HISTORY_CASES,
    ...CROSS_CUTTING_CASES,
    ...EMBEDDING_BOUNDARY_CASES,
  ];
}
