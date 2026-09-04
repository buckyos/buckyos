import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type {
  AcceptanceReport,
  CaseReport,
  ProductDefect,
  ResultStatus,
  FinancialAggregate,
  FinancialReport,
} from "./types.ts";
import { FAILURE_CLASSES, RESULT_STATUSES } from "./types.ts";

export const ACCEPTANCE_REPORT_SCHEMA_VERSION = 1 as const;

const SECRET_KEY = /(?:api[_-]?key|authorization|password|private[_-]?key|session[_-]?token|refresh[_-]?token|access[_-]?token|cookie)/i;
const SECRET_VALUE = /(?:bearer\s+[a-z0-9._~+/=-]+|sk-[a-z0-9_-]{12,}|-----BEGIN [A-Z ]+PRIVATE KEY-----)/ig;

export function redact(value: unknown, key = ""): unknown {
  if (SECRET_KEY.test(key)) return "[REDACTED]";
  if (typeof value === "string") return value.replace(SECRET_VALUE, "[REDACTED]");
  if (Array.isArray(value)) return value.map((item) => redact(item));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([childKey, child]) => [
        childKey,
        redact(child, childKey),
      ]),
    );
  }
  return value;
}

export function assertNoSecrets(value: unknown): void {
  const serialized = JSON.stringify(value);
  const findings = [
    /sk-[a-z0-9_-]{12,}/i,
    /-----BEGIN [A-Z ]+PRIVATE KEY-----/,
    /"(?:password|session_token|api_key|authorization)"\s*:\s*"(?!\[REDACTED\])/i,
  ].filter((pattern) => pattern.test(serialized));
  if (findings.length > 0) throw new Error("report contains sensitive data");
}

export function isProviderRestricted(error: unknown): boolean {
  return String(error).toLowerCase().includes("request not allowed");
}

export function caseTotals(cases: CaseReport[]): Record<ResultStatus, number> {
  const totals: Record<ResultStatus, number> = {
    passed: 0,
    failed: 0,
    provider_restricted: 0,
    skipped: 0,
    not_applicable: 0,
    review: 0,
  };
  for (const result of cases) totals[result.status] += 1;
  return totals;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function requireString(value: unknown, field: string): void {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
}

function requireNonNegativeNumber(value: unknown, field: string): void {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new Error(`${field} must be a finite non-negative number`);
  }
}

export function validateAcceptanceReport(value: unknown): asserts value is AcceptanceReport {
  if (!isObject(value)) throw new Error("acceptance report must be an object");
  if (value.schema_version !== ACCEPTANCE_REPORT_SCHEMA_VERSION) {
    throw new Error("unsupported acceptance report schema_version");
  }
  for (const field of ["run_id", "started_at", "finished_at", "commit", "baseline_revision"] as const) {
    requireString(value[field], field);
  }
  if (typeof value.allow_real_model_calls !== "boolean") {
    throw new Error("allow_real_model_calls must be boolean");
  }
  for (const field of [
    "planned_real_calls", "actual_real_calls", "estimated_cost_usd", "actual_cost_usd",
    "raw_cost_usd", "credit_applied_usd",
  ] as const) {
    requireNonNegativeNumber(value[field], field);
  }
  if (!Number.isInteger(value.planned_real_calls) || !Number.isInteger(value.actual_real_calls)) {
    throw new Error("real call counts must be integers");
  }
  if (!Array.isArray(value.cases)) throw new Error("cases must be an array");
  const caseIds = new Set<string>();
  for (const [index, rawCase] of value.cases.entries()) {
    if (!isObject(rawCase)) throw new Error(`cases[${index}] must be an object`);
    requireString(rawCase.case_id, `cases[${index}].case_id`);
    const caseId = String(rawCase.case_id);
    if (caseIds.has(caseId)) throw new Error(`duplicate report case_id ${caseId}`);
    caseIds.add(caseId);
    if (rawCase.run_id !== value.run_id) throw new Error(`${caseId}.run_id differs from report`);
    if (!["T1", "T1.5", "T2", "T3"].includes(String(rawCase.layer))) {
      throw new Error(`${caseId}.layer is invalid`);
    }
    if (!RESULT_STATUSES.includes(rawCase.status as never)) throw new Error(`${caseId}.status is invalid`);
    requireString(rawCase.method, `${caseId}.method`);
    for (const field of ["outbound_message_ids", "artifact_ids", "attempts"] as const) {
      if (!Array.isArray(rawCase[field])) throw new Error(`${caseId}.${field} must be an array`);
    }
    for (const [attemptIndex, rawAttempt] of (rawCase.attempts as unknown[]).entries()) {
      if (!isObject(rawAttempt)) throw new Error(`${caseId}.attempts[${attemptIndex}] must be an object`);
      if (!Number.isInteger(rawAttempt.attempt) || Number(rawAttempt.attempt) < 1) {
        throw new Error(`${caseId}.attempts[${attemptIndex}].attempt must be a positive integer`);
      }
      if (!RESULT_STATUSES.includes(rawAttempt.status as never)) {
        throw new Error(`${caseId}.attempts[${attemptIndex}].status is invalid`);
      }
      if (rawAttempt.failure_class !== undefined &&
        !FAILURE_CLASSES.includes(rawAttempt.failure_class as never)) {
        throw new Error(`${caseId}.attempts[${attemptIndex}].failure_class is invalid`);
      }
    }
  }
  if (!Array.isArray(value.product_defects)) throw new Error("product_defects must be an array");
  if (!isObject(value.finance) || value.finance.currency !== "USD" || !Array.isArray(value.finance.entries)) {
    throw new Error("finance must use the fixed USD report schema");
  }
  if (!isObject(value.cleanup) || !["passed", "failed"].includes(String(value.cleanup.status)) ||
    !Array.isArray(value.cleanup.details) ||
    value.cleanup.details.some((detail) => typeof detail !== "string")) {
    throw new Error("cleanup must use the fixed report schema");
  }
}

export function defectFromFailure(args: {
  component: ProductDefect["component"];
  caseReport: CaseReport;
  expected: string;
  observed: string;
  evidencePaths: string[];
}): ProductDefect {
  const failedAttempt = [...args.caseReport.attempts].reverse().find((attempt) =>
    attempt.status === "failed"
  );
  return {
    defect_id: `defect.${args.component.toLowerCase()}.${args.caseReport.case_id}`,
    component: args.component,
    case_id: args.caseReport.case_id,
    expected: args.expected,
    observed: args.observed,
    evidence_paths: args.evidencePaths,
    failure_class: failedAttempt?.failure_class ?? "assertion_failed",
  };
}

function markdown(report: AcceptanceReport): string {
  const totals = caseTotals(report.cases);
  const lines = [
    "# AICC E2E Acceptance Report",
    "",
    `- Run: \`${report.run_id}\``,
    `- Commit: \`${report.commit}\``,
    `- Capability baseline: \`${report.baseline_revision}\``,
    `- Real model calls: ${report.actual_real_calls}/${report.planned_real_calls}`,
    `- Planned maximum cost: $${report.estimated_cost_usd.toFixed(6)}`,
    `- Known actual / unknown estimated exposure: $${report.finance.actual_cost_usd.toFixed(6)} / $${report.finance.estimated_exposure_usd.toFixed(6)}`,
    `- Total exposure / budget: $${report.finance.total_exposure_usd.toFixed(6)} / $${report.finance.budget_usd.toFixed(6)}`,
    `- Unknown-cost calls: ${report.finance.unknown_cost_calls}; budget exceeded: ${report.finance.budget_exceeded}`,
    `- Results: passed=${totals.passed}, failed=${totals.failed}, provider_restricted=${totals.provider_restricted}, skipped=${totals.skipped}, not_applicable=${totals.not_applicable}, review=${totals.review}`,
    ...(report.manifest_coverage
      ? [`- Manifest coverage: ${report.manifest_coverage.executed}/${report.manifest_coverage.total} (${(report.manifest_coverage.coverage_rate * 100).toFixed(2)}%); passed=${report.manifest_coverage.passed}, failed=${report.manifest_coverage.failed}`]
      : []),
    ...(report.t1_requirement_coverage
      ? [`- T1 requirement branches: ${report.t1_requirement_coverage.executed_branches}/${report.t1_requirement_coverage.total_branches} (${(report.t1_requirement_coverage.coverage_rate * 100).toFixed(2)}%); passed=${report.t1_requirement_coverage.passed_branches}, failed=${report.t1_requirement_coverage.failed_branches}, skipped=${report.t1_requirement_coverage.skipped_branches}`]
      : []),
    ...(report.targeted_retest_command
      ? ["", "## Targeted retest", "", "Run after fixing the reported defect; repeat `--case` to select additional cases:", "", "```bash", report.targeted_retest_command, "```"]
      : []),
    "",
    "## Cases",
    "",
    "| Case | Layer | Provider | Model | API | Status |",
    "|---|---|---|---|---|---|",
  ];
  for (const result of report.cases) {
    lines.push(
      `| ${result.case_id} | ${result.layer} | ${result.provider_driver ?? "-"}/${result.provider_instance ?? "-"} | ${result.exact_model ?? "-"} | ${result.api_type ?? result.method} | ${result.status} |`,
    );
  }
  if (report.manifest_coverage?.unexecuted_case_ids.length) {
    lines.push(
      "",
      "## Unexecuted manifest cases",
      "",
      ...report.manifest_coverage.unexecuted_case_ids.map((caseId) => `- ${caseId}`),
    );
  }
  if (report.t1_requirement_coverage) {
    lines.push(
      "",
      "## T1 requirement branch coverage",
      "",
      "| Branch | Planned cases | Executed cases | Status |",
      "|---|---:|---:|---|",
      ...report.t1_requirement_coverage.branches.map((branch) =>
        `| ${branch.branch_id} | ${branch.planned_case_ids.length} | ${branch.executed_case_ids.length} | ${branch.status} |`
      ),
      "",
      "## T1 combination coverage",
      "",
      "| Combination group | Planned cells | Executed cells | Passed cells | Coverage |",
      "|---|---:|---:|---:|---:|",
      ...report.t1_requirement_coverage.combination_groups.map((group) =>
        `| ${group.group_id} | ${group.planned_cells} | ${group.executed_cells} | ${group.passed_cells} | ${(group.coverage_rate * 100).toFixed(2)}% |`
      ),
    );
  }
  if (report.model_coverage?.length) {
    lines.push(
      "",
      "## Physical model coverage",
      "",
      "| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |",
      "|---|---|---|---|---|---|",
    );
    for (const model of report.model_coverage) {
      lines.push(
        `| ${model.provider_driver}/${model.provider_instance} | ${model.provider_model_id} | ${model.physical_model_id} | ${model.status} | ${model.reason ?? "physical model"} | ${model.retained_exact_model ?? "-"} |`,
      );
    }
  }
  if (report.document_format_coverage?.length) {
    lines.push(
      "",
      "## Document format coverage",
      "",
      "| Provider | Model | Format | Official status |",
      "|---|---|---|---|",
      ...report.document_format_coverage.map((item) =>
        `| ${item.provider_driver}/${item.provider_instance} | ${item.provider_model_id} | ${item.format} | ${item.status} |`
      ),
    );
  }
  lines.push("", "## Confirmed product defects", "");
  if (report.product_defects.length === 0) {
    lines.push("None.");
  } else {
    for (const defect of report.product_defects) {
      lines.push(
        `- **${defect.defect_id}** (${defect.component}, ${defect.failure_class}): expected ${defect.expected}; observed ${defect.observed}. Evidence: ${defect.evidence_paths.join(", ") || "report case"}`,
      );
    }
  }
  lines.push(
    "",
    "## Cleanup",
    "",
    `Status: ${report.cleanup.status}`,
    "",
    ...report.cleanup.details.map((detail) => `- ${detail}`),
    "",
  );
  return lines.join("\n");
}

function financeTable(title: string, values: FinancialAggregate[]): string[] {
  const lines = [
    `## ${title}`,
    "",
    "| Key | Calls | Input tokens | Output tokens | Total tokens | Request units | Raw USD | Credit USD | Billed USD | Unknown estimated USD | Unknown calls |",
    "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
  ];
  for (const value of values) {
    lines.push(
      `| ${value.key} | ${value.calls} | ${value.input_tokens} | ${value.output_tokens} | ${value.total_tokens} | ${value.request_units} | ${value.raw_cost_usd.toFixed(6)} | ${value.credit_applied_usd.toFixed(6)} | ${value.actual_cost_usd.toFixed(6)} | ${value.estimated_exposure_usd.toFixed(6)} | ${value.unknown_cost_calls} |`,
    );
  }
  return [...lines, ""];
}

function financeMarkdown(finance: FinancialReport): string {
  return [
    "# AICC Test Financial Report",
    "",
    `- Currency: ${finance.currency}`,
    `- Budget: $${finance.budget_usd.toFixed(6)}`,
    `- Planned maximum: ${finance.planned_max_calls} calls / $${finance.planned_max_cost_usd.toFixed(6)}`,
    `- Executed calls: ${finance.actual_calls}`,
    `- Known actual cost: $${finance.actual_cost_usd.toFixed(6)}`,
    `- Raw cost before credit: $${finance.raw_cost_usd.toFixed(6)}`,
    `- Credit applied: $${finance.credit_applied_usd.toFixed(6)}`,
    `- Unknown-cost estimated exposure: $${finance.estimated_exposure_usd.toFixed(6)}`,
    `- Total exposure: $${finance.total_exposure_usd.toFixed(6)}`,
    `- Remaining budget: $${finance.remaining_budget_usd.toFixed(6)}`,
    `- Unknown-cost calls: ${finance.unknown_cost_calls}`,
    `- Budget exceeded: ${finance.budget_exceeded}`,
    "",
    ...financeTable("By provider", finance.by_provider),
    ...financeTable("By provider instance", finance.by_instance),
    ...financeTable("By exact model", finance.by_model),
    ...financeTable("By API type", finance.by_api_type),
    ...financeTable("By case", finance.by_case),
  ].join("\n");
}

function csvCell(value: unknown): string {
  const text = String(value ?? "");
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function financeCsv(finance: FinancialReport): string {
  const header = [
    "case_id", "attempt", "provider_driver", "provider_instance", "exact_model",
    "api_type", "method", "started_at", "status", "input_tokens", "output_tokens",
    "total_tokens", "request_units", "estimated_cost_usd", "raw_cost_usd", "credit_applied_usd", "actual_cost_usd", "cost_status",
  ];
  const rows = finance.entries.map((entry) => [
    entry.case_id,
    entry.attempt,
    entry.provider_driver,
    entry.provider_instance,
    entry.exact_model,
    entry.api_type,
    entry.method,
    entry.started_at,
    entry.status,
    entry.usage?.input_tokens ?? "",
    entry.usage?.output_tokens ?? "",
    entry.usage?.total_tokens ?? "",
    entry.usage?.request_units ?? "",
    entry.estimated_cost_usd.toFixed(8),
    entry.raw_cost_usd?.toFixed(8) ?? "",
    entry.credit_applied_usd?.toFixed(8) ?? "",
    entry.actual_cost_usd?.toFixed(8) ?? "",
    entry.cost_status,
  ]);
  return [header, ...rows].map((row) => row.map(csvCell).join(",")).join("\n") + "\n";
}

export async function writeReport(
  outputDir: string,
  report: AcceptanceReport,
): Promise<void> {
  const safe = redact(report) as AcceptanceReport;
  validateAcceptanceReport(safe);
  assertNoSecrets(safe);
  await mkdir(outputDir, { recursive: true });
  await writeFile(
    join(outputDir, "summary.json"),
    `${JSON.stringify(safe, null, 2)}\n`,
    "utf8",
  );
  await writeFile(join(outputDir, "summary.md"), markdown(safe), "utf8");
  await mkdir(join(outputDir, "cases"), { recursive: true });
  for (const item of safe.cases) {
    const fileName = item.case_id.replace(/[^a-zA-Z0-9._-]+/g, "-");
    await writeFile(
      join(outputDir, "cases", `${fileName}.json`),
      `${JSON.stringify(item, null, 2)}\n`,
      "utf8",
    );
  }
  await writeFile(
    join(outputDir, "finance.json"),
    `${JSON.stringify(safe.finance, null, 2)}\n`,
    "utf8",
  );
  await writeFile(join(outputDir, "finance.md"), financeMarkdown(safe.finance), "utf8");
  await writeFile(join(outputDir, "finance.csv"), financeCsv(safe.finance), "utf8");
}
