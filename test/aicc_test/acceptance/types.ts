export const RESULT_STATUSES = [
  "passed",
  "failed",
  "provider_restricted",
  "skipped",
  "not_applicable",
  "review",
] as const;

export type ResultStatus = (typeof RESULT_STATUSES)[number];

export const FAILURE_CLASSES = [
  "preflight_failed",
  "baseline_mismatch",
  "routing_failed",
  "provider_protocol_failed",
  "provider_runtime_failed",
  "task_lifecycle_failed",
  "resource_failed",
  "message_transport_failed",
  "attachment_failed",
  "usage_failed",
  "security_failed",
  "judge_failed",
  "assertion_failed",
  "cleanup_failed",
  "platform_limitation",
] as const;

export type FailureClass = (typeof FAILURE_CLASSES)[number];

export type TestLayer = "T1" | "T2" | "T3";
export type Priority = "P0" | "P1" | "P2";

export type ExpectedOutput = {
  kinds: string[];
  attachment_count: { min: number; max: number };
  mime_types: string[];
};

export type AcceptanceCase = {
  case_id: string;
  layer: TestLayer;
  priority: Priority;
  tags: string[];
  input_entry: string;
  user: string;
  session: string;
  provider_driver: string | null;
  provider_instance: string | null;
  model_selector: { kind: "exact" | "logical"; value: string } | null;
  api_type: string | null;
  method: string;
  required_capabilities: string[];
  disabled_capabilities: string[];
  fixtures: string[];
  mock_scenario: string | null;
  expected_exact_model: string | null;
  expected_provider_instance: string | null;
  expected_task_status: string | null;
  expected_error_class: string | null;
  expected_output: ExpectedOutput;
  semantic_rubric: string[];
  timeout_ms: number;
  max_attempts: number;
  estimated_cost_usd: number;
  cleanup: string[];
};

export type ProviderModel = {
  exact_model: string;
  provider_model_id: string;
  provider_actual_model_id?: string;
  provider_options?: Record<string, unknown>;
  api_types: string[];
  logical_mounts: string[];
  health?: string;
  quota?: string;
  pricing?: {
    currency?: string;
    input_token?: number;
    output_token?: number;
    cache_input_token?: number;
    estimated_cost?: number;
  };
};

export type ProviderInventory = {
  provider_instance_name: string;
  provider_driver: string;
  provider_type?: string;
  inventory_revision?: string | null;
  models: ProviderModel[];
};

export type CapabilityRule = {
  model_pattern: string;
  status: "active" | "preview" | "deprecated" | "removed";
  source_status?: string;
  api_types: string[];
  methods: string[];
  input_kinds: string[];
  output_kinds: string[];
  document_formats?: string[];
  api_io?: Record<string, {
    input_combinations: string[][];
    output_combinations: string[][];
  }>;
  source_urls: string[];
  evidence_summary: string;
};

export type ModelCoverageRule = {
  model_pattern: string;
  action: "exclude" | "alias";
  physical_model_id?: string;
  reason: "deprecated_or_retiring" | "logical_alias" | "not_physical_model" | "unsupported_canonical_protocol";
  source_urls: string[];
  evidence_summary: string;
};

export type ModelCoverageRecord = {
  source?: "official_catalog" | "aicc_inventory";
  provider_driver: string;
  provider_instance: string;
  exact_model: string;
  provider_model_id: string;
  provider_actual_model_id?: string;
  physical_model_id: string;
  status: "included" | "filtered";
  reason?: "deprecated_or_retiring" | "logical_alias" | "not_physical_model" | "unsupported_canonical_protocol" | "duplicate_physical_model";
  retained_exact_model?: string;
  source_urls: string[];
  evidence_summary: string;
};

export type OfficialCatalogConfig = {
  endpoint: string;
  format: "openai" | "anthropic" | "gemini" | "fal" | "sn";
  authentication: "bearer" | "x-api-key" | "query-key" | "fal-key" | "none";
  page_size?: number;
};

export type ProviderBaseline = {
  schema_version: number;
  baseline_revision: string;
  checked_at: string;
  canonical_api_types: string[];
  providers: Array<{
    provider_driver: string;
    discovery: "official_catalog" | "runtime_catalog" | "internal_inventory";
    official_catalog: OfficialCatalogConfig;
    capability_source_provider?: string;
    source_urls: string[];
    protocol_evidence: {
      documentation_version: string;
      streaming_semantics: string;
      async_operation_semantics: string;
      usage_semantics: string;
      input_output_formats: string;
      item_request_batch_limits: string;
      image_audio_video_limits: string;
      context_output_limits: string;
      region_restrictions: string;
      account_tier_restrictions: string;
      preview_allowlist: string;
    };
    coverage_rules?: ModelCoverageRule[];
    rules: CapabilityRule[];
  }>;
};

export type MatrixCell = {
  case_id: string;
  provider_driver: string;
  provider_instance: string;
  exact_model: string;
  provider_model_id: string;
  api_type: string;
  method: string;
  variant?: "default" | "embedding_large_artifact";
  baseline_status: CapabilityRule["status"];
  input_kinds: string[];
  output_kinds: string[];
  resource_representation?: "url" | "base64" | "named_object";
  document_format?: string;
  source_urls: string[];
  estimated_cost_usd?: number;
};

export type DocumentFormatCoverageRecord = {
  provider_driver: string;
  provider_instance: string;
  exact_model: string;
  provider_model_id: string;
  format: string;
  status: "supported" | "not_applicable";
  source_urls: string[];
};

export type AttemptReport = {
  attempt: number;
  started_at: string;
  elapsed_ms: number;
  status: ResultStatus;
  error_code?: string;
  failure_class?: FailureClass;
  diagnostic?: string;
  usage?: {
    input_tokens?: number;
    output_tokens?: number;
    total_tokens?: number;
    request_units?: number;
  };
  estimated_cost_usd?: number;
  actual_cost_usd?: number;
  raw_cost_usd?: number;
  credit_applied_usd?: number;
  cost_status?: "actual" | "estimated" | "unknown" | "not_called";
};

export type JudgeReport = {
  rubric_version: string;
  configured_model: string;
  task_id: string;
  exact_model?: string;
  provider_driver?: string;
  provider_instance?: string;
  input_summary: string;
  score: number;
  passed: boolean;
  reason: string;
  distinct_provider_or_family?: boolean;
};

export type CaseReport = {
  run_id: string;
  case_id: string;
  layer: TestLayer;
  status: ResultStatus;
  provider_driver?: string;
  provider_instance?: string;
  exact_model?: string;
  api_type?: string;
  method: string;
  trace_id?: string;
  task_id?: string;
  provider_operation_id?: string;
  inbound_message_id?: string;
  session_id?: string;
  outbound_message_ids: string[];
  artifact_ids: string[];
  artifact_audits?: import("./artifact_validation.ts").ArtifactAudit[];
  judge?: JudgeReport;
  usage?: Record<string, unknown>;
  cost_usd?: number;
  attempts: AttemptReport[];
};

export type ProductDefect = {
  defect_id: string;
  component: "AICC" | "Jarvis" | "msg-center" | "message-tunnel";
  case_id: string;
  expected: string;
  observed: string;
  evidence_paths: string[];
  failure_class: FailureClass;
};

export type AcceptanceReport = {
  schema_version: 1;
  run_id: string;
  started_at: string;
  finished_at: string;
  commit: string;
  baseline_revision: string;
  allow_real_model_calls: boolean;
  planned_real_calls: number;
  actual_real_calls: number;
  estimated_cost_usd: number;
  actual_cost_usd: number;
  raw_cost_usd: number;
  credit_applied_usd: number;
  finance: FinancialReport;
  cases: CaseReport[];
  product_defects: ProductDefect[];
  model_coverage?: ModelCoverageRecord[];
  document_format_coverage?: DocumentFormatCoverageRecord[];
  targeted_retest_command?: string;
  manifest_coverage?: {
    total: number;
    executed: number;
    passed: number;
    failed: number;
    coverage_rate: number;
    unexecuted_case_ids: string[];
  };
  t1_requirement_coverage?: import("./coverage.ts").T1Coverage;
  cleanup: { status: "passed" | "failed"; details: string[] };
};

export type FinancialEntry = {
  case_id: string;
  attempt: number;
  provider_driver: string;
  provider_instance: string;
  exact_model: string;
  api_type: string;
  method: string;
  started_at: string;
  status: ResultStatus;
  usage?: AttemptReport["usage"];
  estimated_cost_usd: number;
  actual_cost_usd?: number;
  raw_cost_usd?: number;
  credit_applied_usd?: number;
  cost_status: "actual" | "estimated" | "unknown";
};

export type FinancialAggregate = {
  key: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  request_units: number;
  actual_cost_usd: number;
  raw_cost_usd: number;
  credit_applied_usd: number;
  estimated_exposure_usd: number;
  unknown_cost_calls: number;
};

export type FinancialReport = {
  currency: "USD";
  budget_usd: number;
  planned_max_calls: number;
  planned_max_cost_usd: number;
  actual_calls: number;
  actual_cost_usd: number;
  raw_cost_usd: number;
  credit_applied_usd: number;
  estimated_exposure_usd: number;
  total_exposure_usd: number;
  remaining_budget_usd: number;
  unknown_cost_calls: number;
  budget_exceeded: boolean;
  entries: FinancialEntry[];
  by_provider: FinancialAggregate[];
  by_instance: FinancialAggregate[];
  by_model: FinancialAggregate[];
  by_api_type: FinancialAggregate[];
  by_case: FinancialAggregate[];
};
