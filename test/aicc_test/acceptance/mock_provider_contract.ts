export const MOCK_PROVIDER_CONTRACT_VERSION = 1 as const;

export const MOCK_PROVIDER_SCENARIOS = [
  "success",
  "stream_success",
  "async_success",
  "async_failed",
  "async_pending",
  "bad_request",
  "unauthorized",
  "forbidden",
  "not_found",
  "idempotency_conflict",
  "rate_limit",
  "provider_5xx",
  "connection_failed",
  "timeout_short",
  "timeout_long",
  "malformed_response",
  "wrong_mime",
  "missing_usage",
  "safety_blocked",
  "quota_exhausted",
  "invalid_resource",
  "embedding_dimension_mismatch",
  "embedding_row_count_mismatch",
  "embedding_order_mismatch",
  "embedding_nonfinite",
  "rerank_missing_score",
  "rerank_document_id_mismatch",
  "rerank_result_count_mismatch",
] as const;

export type MockProviderScenario = (typeof MOCK_PROVIDER_SCENARIOS)[number];

export const MOCK_PROVIDER_MANAGEMENT_ROUTES = Object.freeze({
  health: { method: "GET", path: "/__mock/health" },
  reset: { method: "POST", path: "/__mock/reset" },
  scenario: { method: "POST", path: "/__mock/scenario" },
  providerState: { method: "POST", path: "/__mock/provider_state" },
  requests: { method: "GET", path: "/__mock/requests" },
  metrics: { method: "GET", path: "/__mock/metrics" },
} as const);

export function validateMockProviderContract(): void {
  if (new Set(MOCK_PROVIDER_SCENARIOS).size !== MOCK_PROVIDER_SCENARIOS.length) {
    throw new Error("Mock Provider contract contains duplicate scenarios");
  }
  for (const required of ["success", "stream_success", "async_success", "rate_limit", "provider_5xx"] as const) {
    if (!MOCK_PROVIDER_SCENARIOS.includes(required)) {
      throw new Error(`Mock Provider contract missing ${required}`);
    }
  }
  const routes = Object.values(MOCK_PROVIDER_MANAGEMENT_ROUTES);
  const routeKeys = routes.map((route) => `${route.method} ${route.path}`);
  if (new Set(routeKeys).size !== routeKeys.length ||
    routes.some((route) => !route.path.startsWith("/__mock/"))) {
    throw new Error("Mock Provider management route contract is invalid");
  }
}
