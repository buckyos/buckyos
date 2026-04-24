# AICC Usage Log DB Technical Requirements

## 1. Goal

`aicc-usage-log-db` provides a durable local usage log for AICC completed provider calls.

The goal is to support basic local usage statistics, such as:

- usage by provider in the last 1 day
- usage by provider in the last 7 days
- usage by model or capability in a time range

This database is not a financial ledger. Provider or server financial data may be recorded as a snapshot, but authoritative billing and reconciliation remain on the server side.

## 2. Non-Goals

- Do not implement payment, balance, invoice, or authoritative billing logic.
- Do not depend on TaskMgr completed tasks as the source of truth.
- Do not require a complex aggregation engine in the first version.
- Do not block provider execution because optional financial snapshot data is missing.

## 3. Data Classification

### Durable Data

`aicc_usage_event`

Append-only records for successful AICC provider completions. This data must survive service restart, completed task cleanup, overlay install, and upgrade.

### Disposable Data

In-memory route metrics, temporary task events, and TaskMgr completed task records are not usage log source data.

## 4. Storage Strategy

AICC must use the platform RDB instance for usage log structured data.

The first version only needs one table. Future summary tables can be added later if query performance requires them.

## 5. Event Write Rule

AICC must write one usage event after a provider call completes successfully.

The same write path must be used for:

- `ProviderStartResult::Immediate`
- long task final event

Successful final summary must include `usage`. If usage is missing, AICC should treat it as a provider protocol error rather than writing a successful usage event.

Optional financial data can be recorded if present. Missing financial data must not make a successful usage event invalid.

## 6. Minimal Schema

Table: `aicc_usage_event`

| Column | Type | Nullable | Description |
|---|---|---:|---|
| `event_id` | TEXT PK | NO | Stable usage event id. |
| `tenant_id` | TEXT | NO | User / tenant identity from RPC context. |
| `caller_app_id` | TEXT | YES | Caller app id if available. |
| `task_id` | TEXT | NO | AICC external task id. |
| `idempotency_key` | TEXT | YES | Request idempotency key if provided. |
| `capability` | TEXT | NO | AICC capability, such as `LlmRouter` or `Text2Image`. |
| `request_model` | TEXT | NO | Logical model requested by caller, such as `llm.plan.default`. |
| `provider_model` | TEXT | NO | Resolved provider model. This field should contain enough information to identify provider, instance, and real model. |
| `input_tokens` | INTEGER | YES | Input token count when available. |
| `output_tokens` | INTEGER | YES | Output token count when available. |
| `total_tokens` | INTEGER | YES | Total token count when available. |
| `request_units` | INTEGER | YES | Generic request/unit count for non-token providers. |
| `usage_json` | TEXT | NO | Serialized normalized usage object for full detail and future extension. |
| `finance_snapshot_json` | TEXT | YES | Optional provider/server reported finance snapshot. Non-authoritative. |
| `created_at_ms` | INTEGER | NO | Event creation time in Unix milliseconds. |

Indexes:

- `idx_aicc_usage_event_time` on `created_at_ms`
- `idx_aicc_usage_event_tenant_time` on `(tenant_id, created_at_ms)`
- `idx_aicc_usage_event_model_time` on `(provider_model, created_at_ms)`
- `idx_aicc_usage_event_request_model_time` on `(request_model, created_at_ms)`

Constraints:

- `event_id` must be unique.
- If `idempotency_key` is present, `(tenant_id, idempotency_key)` should be unique for successful usage events.
- `(tenant_id, task_id)` should be unique.

## 7. Query Requirements

The usage log query API should be flexible enough for both dashboards and debugging.

The first version should provide one general query interface instead of many fixed report APIs:

`query_usage`

Input:

- `time_range`
  - explicit: `start_time_ms` and `end_time_ms`
  - shortcut: `last_1d`, `last_7d`, `last_30d`
- optional filters
  - `tenant_id`
  - `caller_app_id`
  - `request_model`
  - `provider_model`
  - `capability`
  - `task_id`
  - `idempotency_key`
- optional grouping
  - no group: return a total summary
  - one or more dimensions: `provider_model`, `request_model`, `capability`, `caller_app_id`, `tenant_id`
- optional time bucket
  - `hour`
  - `day`
  - no bucket
- optional output mode
  - `summary`: aggregated rows only
  - `events`: raw usage events
  - `summary_and_events`: both, for debugging
- pagination for raw events
  - `limit`
  - `cursor`

Output:

- `total_requests`
- aggregated usage values
- grouped rows when grouping is set
- bucketed rows when a time bucket is set
- raw events when requested
- `next_cursor` when more raw events are available
- optional aggregated financial snapshot fields only when the data is numeric and comparable

Required common queries:

- total usage for the last 1 day
- total usage for the last 7 days
- usage by provider model for the last 1 day
- usage by provider model for the last 7 days
- usage by request model and provider model in a custom time range
- recent raw events for a provider model or task

The first version can aggregate in service code after selecting rows from RDB. SQL aggregation can be added later. The API shape should not assume only provider-based summaries.

## 8. Usage Semantics

`usage_json` is the local usage fact recorded by AICC.

For LLM calls it should include token usage when available:

- `input_tokens`
- `output_tokens`
- `total_tokens`

The token fields must also be copied to top-level columns so SQL can aggregate common statistics without parsing JSON.

For non-token providers, `usage_json` must still represent usage in a normalized way. `request_units` can be used as the first generic top-level metric. A future extension may add more top-level unit fields, such as image count, audio seconds, video seconds, or tool calls, when SQL aggregation needs them.

## 9. Finance Snapshot Semantics

`finance_snapshot_json` is optional and non-authoritative.

It can store provider/server reported values such as:

- `amount`
- `currency`
- `credits_used`
- `source`
- `provider_trace_id`

AICC must not use this field as the final billing ledger. Server-side billing remains the reconciliation authority.

## 10. Retention

Usage events must not follow TaskMgr completed task cleanup.

Initial retention can be simple:

- keep usage events indefinitely, or
- keep at least 90 days if a cleanup policy is required.

The exact cleanup policy should be configurable later.

## 11. Acceptance Criteria

- A completed AICC request with usage writes exactly one durable usage event.
- Repeating the same `tenant_id + idempotency_key` does not create duplicate usage events.
- Completed TaskMgr task deletion does not remove usage events.
- A query for `last_1d` can return usage grouped by provider.
- A query for `last_7d` can return usage grouped by provider.
- Provider financial snapshot is stored when present but is not required for success.
- Missing usage in a successful provider final result is treated as provider protocol error.
