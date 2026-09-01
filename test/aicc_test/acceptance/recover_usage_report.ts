import { parseToml, tomlNumber, tomlString } from "../../jarvis_media_dv/config.ts";
import { loginGateway } from "./gateway.ts";
import { buildFinancialReport } from "./finance.ts";
import { writeReport } from "./report.ts";
import { providerInstanceFromExactModel, queryUsageEvents, usageEventFinance } from "./usage_audit.ts";
import type { AcceptanceReport, CaseReport, FinancialEntry } from "./types.ts";

const DRIVER_BY_INSTANCE: Record<string, string> = {
  "openai-main": "openai",
  "claude-main": "claude",
  "google-gemini-main": "google-gemini",
  "fal-main": "fal",
};

function arg(name: string): string | undefined {
  const index = Deno.args.indexOf(name);
  return index >= 0 ? Deno.args[index + 1] : undefined;
}

async function main(): Promise<void> {
  const configPath = arg("--config") ?? "aicc_acceptance.local.toml";
  const runId = arg("--run-id");
  const startedAt = arg("--started-at");
  if (!runId || !startedAt || !Number.isFinite(Date.parse(startedAt))) {
    throw new Error("--run-id and ISO --started-at are required");
  }
  const config = parseToml(await Deno.readTextFile(configPath));
  const session = await loginGateway({
    gatewayUrl: tomlString(config, "gateway.url") ?? "",
    sessionToken: tomlString(config, "auth.session_token"),
    username: tomlString(config, "auth.username"),
    password: tomlString(config, "auth.password"),
    appId: "control-panel",
  });
  const events = (await queryUsageEvents({
    aicc: session.aicc,
    startTimeMs: Date.parse(startedAt) - 1_000,
    endTimeMs: Date.now() + 1_000,
  })).filter((event) => {
    const instance = providerInstanceFromExactModel(event.provider_model);
    return Boolean(instance && DRIVER_BY_INSTANCE[instance]);
  });
  const entries: FinancialEntry[] = events.map((event) => {
    const instance = providerInstanceFromExactModel(event.provider_model)!;
    const finance = usageEventFinance(event);
    return {
      case_id: `recovered.${event.task_id}`,
      attempt: 1,
      provider_driver: DRIVER_BY_INSTANCE[instance],
      provider_instance: instance,
      exact_model: event.provider_model,
      api_type: event.capability,
      method: event.capability,
      started_at: new Date(event.created_at_ms).toISOString(),
      status: "review",
      usage: finance.usage,
      estimated_cost_usd: tomlNumber(config, "runner.estimated_cost_per_call_usd") ?? 0.01,
      actual_cost_usd: finance.actualCostUsd,
      raw_cost_usd: finance.rawCostUsd,
      credit_applied_usd: finance.creditAppliedUsd,
      cost_status: finance.actualCostUsd === undefined ? "unknown" : "actual",
    };
  });
  const finance = buildFinancialReport({
    entries,
    budgetUsd: tomlNumber(config, "runner.max_cost_usd") ?? 100,
    plannedMaxCalls: 398,
    plannedMaxCostUsd: 13.4244,
  });
  const cases: CaseReport[] = entries.map((entry) => ({
    run_id: runId,
    case_id: entry.case_id,
    layer: "T2",
    status: "review",
    provider_driver: entry.provider_driver,
    provider_instance: entry.provider_instance,
    exact_model: entry.exact_model,
    api_type: entry.api_type,
    method: entry.method,
    task_id: entry.case_id.slice("recovered.".length),
    outbound_message_ids: [],
    artifact_ids: [],
    usage: entry.usage,
    cost_usd: entry.actual_cost_usd,
    attempts: [{
      attempt: 1,
      started_at: entry.started_at,
      elapsed_ms: 0,
      status: "review",
      diagnostic: "Recovered from durable usage; original case assertion result was lost after cleanup failure",
      usage: entry.usage,
      estimated_cost_usd: entry.estimated_cost_usd,
      actual_cost_usd: entry.actual_cost_usd,
      raw_cost_usd: entry.raw_cost_usd,
      credit_applied_usd: entry.credit_applied_usd,
      cost_status: entry.cost_status,
    }],
  }));
  const report: AcceptanceReport = {
    schema_version: 1,
    run_id: `${runId}-recovered`,
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    commit: "72d42245855889dc16d85439deb5ac79e5e26d00",
    baseline_revision: "2026-08-26.2",
    allow_real_model_calls: true,
    planned_real_calls: 398,
    actual_real_calls: events.length,
    estimated_cost_usd: 13.4244,
    actual_cost_usd: finance.actual_cost_usd,
    raw_cost_usd: finance.raw_cost_usd,
    credit_applied_usd: finance.credit_applied_usd,
    finance,
    cases,
    product_defects: [],
    cleanup: {
      status: "passed",
      details: [
        "manual scoped cleanup removed only openai-main, claude-main, google-gemini-main, and fal-main sections",
        "post-cleanup runtime inspection confirmed the original SN-only inventory",
        "recovered report contains durable usage evidence only; protocol assertion outcomes require targeted rerun",
      ],
    },
  };
  const reportDir = tomlString(config, "runner.report_dir") ?? "reports/acceptance";
  await writeReport(`${reportDir}/${report.run_id}`, report);
  console.log(JSON.stringify({ report: `${reportDir}/${report.run_id}`, durable_usage_events: events.length }, null, 2));
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`AICC usage recovery failed: ${String(error)}`);
    Deno.exitCode = 1;
  });
}
