import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { parseToml, tomlString } from "../../jarvis_media_dv/config.ts";
import { loginGateway, loginSudoSystemConfig } from "./gateway.ts";
import { decisionInput, decisionRouteRequirements, assertDecisionResult } from "./decision.ts";
import { withAiccSettingsOverride } from "./settings_transaction.ts";
import { queryUsageEvents, queryRouteTraces, usageEventFinance } from "./usage_audit.ts";
import { redact } from "./report.ts";

const args = process.argv.slice(2);
const option = (name: string, fallback: string) => {
  const index = args.indexOf(name);
  if (index < 0) return fallback;
  if (!args[index + 1] || args[index + 1].startsWith("--")) throw new Error(`${name} requires a value`);
  return args[index + 1];
};
const execute = args.includes("--execute");
const plan = {
  layer: "T2", case_id: "openrouter-jev-mixed-single", provider_instance: "openrouter-default",
  physical_model: "typesafe/jev-1.13", max_provider_attempts: 1, concurrency: 1,
  timeout_ms: 60_000, budget_usd: 0.01, sdk_retries: 0, runtime_failover: false,
  fallback: false, judge_calls: 0, temporary_settings: ["provider timeout", "locked routing policy"],
};
process.stdout.write(`${JSON.stringify({execute, plan}, null, 2)}\n`);
if (execute && !args.includes("--allow-config-mutation")) throw new Error("execution requires --allow-config-mutation to install and restore the bounded policy");

async function run(): Promise<void> {
  const reportPath = resolve(option("--report", "reports/openrouter-jev-smoke.json"));
  const report: Record<string, any> = { plan, started_at: new Date().toISOString(), inference_requests: 0, real_provider_attempts: 0, cleanup: "not_needed", status: "inspecting" };
  const config = parseToml(await readFile(option("--config", "aicc_acceptance.local.toml"), "utf8"));
  const credentials = {
    gatewayUrl: tomlString(config, "gateway.url") ?? "https://test.buckyos.io",
    username: tomlString(config, "auth.username") ?? tomlString(config, "gateway.username"),
    password: tomlString(config, "auth.password") ?? tomlString(config, "gateway.password"),
    appId: "aicc-tests",
  };
  const secrets = [credentials.password ?? ""];
  for (const key of ["log", "info", "debug", "warn", "error"] as const) console[key] = () => {};
  const originalFetch = globalThis.fetch;
  let submissions = 0;
  globalThis.fetch = async (input, init) => {
    if (typeof init?.body === "string") {
      let body: any;
      try { body = JSON.parse(init.body); } catch {}
      if (body?.method === "decision.evaluate") {
        if (!execute || ++submissions > 1) throw new Error("single-inference guard rejected submission");
        report.inference_requests = submissions;
        return originalFetch(input, {...init, signal: AbortSignal.timeout(plan.timeout_ms + 5000)});
      }
    }
    return originalFetch(input, init);
  };
  const started = Date.now();
  try {
    const session = await loginGateway(credentials);
    secrets.push(session.sessionToken);
    report.gateway = credentials.gatewayUrl;
    const aicc = session.aicc;
    const inspect = async () => {
      report.refresh = await aicc.call("provider.refresh_models", {provider_instance_name: plan.provider_instance});
      const models: any = await aicc.call("models.list", {});
      const provider = (models.providers ?? []).find((p: any) => p.provider_instance_name === plan.provider_instance);
      const list = (models.models ?? provider?.models ?? []).filter((m: any) => !m.provider_instance_name || m.provider_instance_name === plan.provider_instance);
      report.inventory_revision = provider?.inventory_revision ?? list[0]?.inventory_revision;
      report.jev_models = list.filter((m: any) => String(m.provider_model_id ?? m.exact_model).includes("jev"));
      report.preview = await aicc.call("routing.preview", {paths:["decision"],requirements:decisionRouteRequirements(),explain:true});
      assert.equal(report.preview.entries?.[0]?.available, true, "preview must admit the same mixed requirements");
      report.route = await aicc.call("route.resolve", {api_type:"decision",logical_model:"decision",requirements:decisionRouteRequirements(),
        policy:{allowed_provider_instances:[plan.provider_instance],allow_fallback:false,runtime_failover:false,max_cost:{amount:plan.budget_usd,currency:"USD"},explain:true}});
      const exact = report.route.selected_exact_model;
      assert.ok([`${plan.physical_model}@${plan.provider_instance}`, `~typesafe/jev-latest@${plan.provider_instance}`].includes(exact), "route must select the verified physical Jev identity");
      return exact;
    };
    if (!execute) { await inspect(); report.status = "ready_without_inference"; return; }
    const systemConfig = await loginSudoSystemConfig(credentials);
    report.cleanup = "pending";
    await withAiccSettingsOverride({systemConfig,aicc,description:"single OpenRouter decision smoke",patch(settings: any) {
      const patched = structuredClone(settings);
      const provider = patched.providers.find((p: any) => p.provider_instance_name === plan.provider_instance);
      assert.ok(provider, "existing OpenRouter instance is required");
      provider.timeout_ms = plan.timeout_ms;
      patched.session_config ??= {};
      patched.session_config.policy ??= {};
      Object.assign(patched.session_config.policy, Object.fromEntries(Object.entries({
        allowed_provider_instances:[plan.provider_instance],allow_fallback:false,allow_exact_model_fallback:false,runtime_failover:false,
        max_estimated_cost:{amount:plan.budget_usd,currency:"USD"},
      }).map(([key,value]) => [key,{value,locked:true}])));
      return patched;
    }, async execute() {
      const exact = await inspect();
      const traceId = `openrouter-jev-smoke-${Date.now()}`;
      const began = Date.now();
      report.trace_id = traceId;
      report.response = await aicc.call("decision.evaluate", {...decisionInput(),exact_model:exact,trace_id:traceId,idempotency_key:traceId,execution_mode:"immediate"});
      report.inference_elapsed_ms = Date.now() - began;
      const result = report.response;
      assert.equal(result.status,"succeeded");
      assertDecisionResult(result);
      assert.equal(result.model,"jev-1.13.0");
      assert.ok(Number.isInteger(result.usage?.input_tokens) && Number.isInteger(result.usage?.output_tokens));
      const taskIds = [result.task_id];
      report.usage_events = await queryUsageEvents({aicc,startTimeMs:started-1000,endTimeMs:Date.now()+1000,taskIds});
      report.traces = await queryRouteTraces({aicc,startTimeMs:started-1000,endTimeMs:Date.now()+1000,taskIds});
      report.task = await session.taskManager.call("get_task", {task_id:result.task_id});
      report.upstream = report.task.task?.result?.result?.output?.value?.provider_metadata;
      assert.equal(report.upstream?.model,"typesafe/jev-1.13-20260917");
      assert.equal(report.upstream?.provider,"TypeSafe");
      assert.ok(typeof report.upstream?.id === "string" && report.upstream.id.length > 0);
      assert.ok(report.traces.every((trace: any) => trace.runtime_failover_count === 0));
      const events: any = await aicc.call("events.list",{limit:100});
      report.events = (events.events ?? []).filter((event: any) => JSON.stringify(event).includes(result.task_id) || JSON.stringify(event).includes(traceId));
      assert.equal(report.usage_events.length,1,"one durable charge is required");
      const finance = usageEventFinance(report.usage_events[0]);
      assert.equal(finance.usage.input_tokens,result.usage.input_tokens);
      assert.equal(finance.usage.output_tokens,result.usage.output_tokens);
      assert.ok(finance.actualCostUsd !== undefined && finance.actualCostUsd <= plan.budget_usd);
      assert.equal(finance.actualCostUsd,result.usage.reported_cost);
      assert.ok(report.traces.some((t: any) => t.provider_instance_name === plan.provider_instance && t.selected_exact_model === exact && t.api_type === "decision"));
      report.actual_cost_usd = finance.actualCostUsd;
      report.real_provider_attempts = 1;
      report.status = "passed";
    }});
    report.cleanup = "restored";
  } catch (error) {
    report.status = "failed";
    report.error = String(error);
    if (report.cleanup === "pending") report.cleanup = String(error).includes("cleanup failed") ? "failed" : "restored";
    if (submissions > 0 && report.real_provider_attempts === 0) report.real_provider_attempts = "unconfirmed; inspect trace, never retry automatically";
    process.exitCode = 1;
  } finally {
    globalThis.fetch = originalFetch;
    report.elapsed_ms = Date.now() - started;
    let safe = JSON.stringify(redact(report),null,2);
    for (const secret of secrets.filter(Boolean)) safe = safe.replaceAll(secret,"[REDACTED]");
    await mkdir(dirname(reportPath),{recursive:true});
    await writeFile(reportPath,safe+"\n");
    process.stdout.write(`${JSON.stringify({report:reportPath,status:report.status,inference_requests:submissions,actual_cost_usd:report.actual_cost_usd,cleanup:report.cleanup})}\n`);
  }
}
await run();
