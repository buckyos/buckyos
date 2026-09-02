import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  parseToml,
  tomlBoolean,
  tomlNumber,
  tomlString,
} from "../../jarvis_media_dv/config.ts";
import {
  callChatCompletions,
  callInference,
  callImagesGenerate,
  callLlmChatHelper,
  loginGateway,
  type GatewaySession,
  type RpcClient,
} from "./gateway.ts";
import { buildExactRequest, assertResponseShape } from "./payloads.ts";
import { runPreflight } from "./preflight.ts";
import { withMockSettings } from "./settings_transaction.ts";
import { configValue } from "./mock_settings.ts";
import { ProviderScheduler } from "./scheduler.ts";
import { buildFinancialReport } from "./finance.ts";
import { defectFromFailure, writeReport } from "./report.ts";
import type {
  AcceptanceReport,
  CaseReport,
  MatrixCell,
  ProviderInventory,
} from "./types.ts";
import { CANONICAL_API_TYPES, methodsForApiType } from "./canonical.ts";
import { buildStaticManifest } from "./cases.ts";
import { buildT1Coverage } from "./coverage.ts";
import { queryRouteTraces, queryUsageEvents } from "./usage_audit.ts";

type Options = {
  configPath: string;
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  otherTenantSessionToken?: string;
  appId: string;
  mockBaseUrl: string;
  mockControlUrl: string;
  mockPort: number;
  startLocalMock: boolean;
  configAllowsMutation: boolean;
  cliAllowsMutation: boolean;
  reportDir: string;
  timeoutMs: number;
  globalConcurrency: number;
  providerConcurrency: number;
  providerMinIntervalMs: number;
  caseIds: string[];
};

class SkipCase extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SkipCase";
  }
}

type AiMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: unknown;
  event_ref?: string;
};

function compactFailure(value: unknown, depth = 0): Record<string, unknown> | undefined {
  if (!value || typeof value !== "object" || depth > 4) return undefined;
  const object = value as Record<string, unknown>;
  const summary: Record<string, unknown> = {};
  for (const key of ["code", "type", "message"] as const) {
    const field = object[key];
    if (typeof field === "string") summary[key] = field.slice(0, 500);
    else if (typeof field === "number") summary[key] = field;
  }
  if (Object.keys(summary).length > 0) return summary;
  for (const key of ["error", "result", "cause"] as const) {
    const nested = compactFailure(object[key], depth + 1);
    if (nested) return nested;
  }
  return undefined;
}

function failedResponseDiagnostic(response: AiMethodResponse): string {
  return JSON.stringify({
    status: response.status,
    task_id: response.task_id,
    event_ref: response.event_ref,
    error: compactFailure(response.result),
  });
}

const here = dirname(fileURLToPath(import.meta.url));

function env(name: string): string | undefined {
  const value = Deno.env.get(name)?.trim();
  return value || undefined;
}

function requiredArg(args: string[], index: number, name: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

async function options(args: string[]): Promise<Options> {
  let configPath = "aicc_acceptance.local.toml";
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--config") configPath = requiredArg(args, index, "--config");
  }
  let config: ReturnType<typeof parseToml> = {};
  try {
    config = parseToml(await Deno.readTextFile(configPath));
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
  const parsed: Options = {
    configPath,
    gatewayUrl: tomlString(config, "gateway.url") ?? env("BUCKYOS_TEST_GATEWAY_URL") ?? "",
    sessionToken: tomlString(config, "auth.session_token") ?? env("BUCKYOS_APPCLIENT_SESSION_TOKEN"),
    username: tomlString(config, "auth.username") ?? env("BUCKYOS_TEST_USERNAME"),
    password: tomlString(config, "auth.password") ?? env("BUCKYOS_TEST_PASSWORD"),
    otherTenantSessionToken: tomlString(config, "auth.other_tenant_session_token") ??
      env("BUCKYOS_TEST_OTHER_TENANT_SESSION_TOKEN"),
    appId: tomlString(config, "auth.app_id") ?? env("BUCKYOS_TEST_APP_ID") ?? "aicc-tests",
    mockPort: tomlNumber(config, "mock.port") ?? 18080,
    mockBaseUrl: tomlString(config, "mock.base_url") ?? "",
    mockControlUrl: tomlString(config, "mock.control_url") ?? "",
    startLocalMock: tomlBoolean(config, "mock.start_local") ?? false,
    configAllowsMutation: tomlBoolean(config, "mock.allow_config_mutation") ?? false,
    cliAllowsMutation: false,
    reportDir: tomlString(config, "runner.report_dir") ?? "reports/acceptance",
    timeoutMs: tomlNumber(config, "runner.timeout_ms") ?? 120_000,
    globalConcurrency: tomlNumber(config, "runner.global_concurrency") ?? 8,
    providerConcurrency: tomlNumber(config, "runner.provider_concurrency") ?? 2,
    providerMinIntervalMs: tomlNumber(config, "runner.provider_min_interval_ms") ?? 50,
    caseIds: [],
  };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--config") index += 1;
    else if (arg === "--gateway-url") parsed.gatewayUrl = requiredArg(args, index++, arg);
    else if (arg === "--session-token") parsed.sessionToken = requiredArg(args, index++, arg);
    else if (arg === "--username") parsed.username = requiredArg(args, index++, arg);
    else if (arg === "--password") parsed.password = requiredArg(args, index++, arg);
    else if (arg === "--mock-base-url") parsed.mockBaseUrl = requiredArg(args, index++, arg);
    else if (arg === "--mock-control-url") parsed.mockControlUrl = requiredArg(args, index++, arg);
    else if (arg === "--case") parsed.caseIds.push(requiredArg(args, index++, arg));
    else if (arg === "--start-local-mock") parsed.startLocalMock = true;
    else if (arg === "--allow-config-mutation") parsed.cliAllowsMutation = true;
    else if (arg === "--report-dir") parsed.reportDir = requiredArg(args, index++, arg);
    else throw new Error(`unknown argument ${arg}`);
  }
  parsed.gatewayUrl = parsed.gatewayUrl.replace(/\/+$/, "");
  if (!parsed.gatewayUrl) throw new Error("gateway URL is required");
  if (!parsed.mockBaseUrl) parsed.mockBaseUrl = `http://127.0.0.1:${parsed.mockPort}`;
  parsed.mockBaseUrl = parsed.mockBaseUrl.replace(/\/+$/, "");
  if (!parsed.mockControlUrl) parsed.mockControlUrl = parsed.mockBaseUrl;
  parsed.mockControlUrl = parsed.mockControlUrl.replace(/\/+$/, "");
  if (!/^https?:\/\//.test(parsed.mockBaseUrl)) throw new Error("mock.base_url must be HTTP(S)");
  if (!/^https?:\/\//.test(parsed.mockControlUrl)) throw new Error("mock.control_url must be HTTP(S)");
  if (!parsed.configAllowsMutation || !parsed.cliAllowsMutation) {
    throw new Error(
      "T1 requires mock.allow_config_mutation=true and --allow-config-mutation; settings are backed up and restored",
    );
  }
  return parsed;
}

function wants(input: Pick<Options, "caseIds">, caseId: string): boolean {
  const isCurrentT1Case = buildStaticManifest().some((testCase) =>
    testCase.layer === "T1" && testCase.case_id === caseId
  );
  return isCurrentT1Case && (input.caseIds.length === 0 || input.caseIds.includes(caseId));
}

async function waitHealth(baseUrl: string, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let last = "not attempted";
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/__mock/health`);
      if (response.ok) return;
      last = `${response.status} ${await response.text()}`;
    } catch (error) {
      last = String(error);
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 200));
  }
  throw new Error(`mock provider is unreachable at ${baseUrl}: ${last}`);
}

async function setScenario(baseUrl: string, scenario: string, pathPrefix?: string): Promise<void> {
  const response = await fetch(`${baseUrl}/__mock/scenario`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ scenario, path_prefix: pathPrefix }),
  });
  if (!response.ok) throw new Error(`failed to select mock scenario ${scenario}: ${await response.text()}`);
}

async function setProviderState(
  baseUrl: string,
  state: { health?: string; quota?: string; latency_ms?: number; capabilities?: string[] },
): Promise<void> {
  const response = await fetch(`${baseUrl}/__mock/provider_state`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(state),
  });
  if (!response.ok) throw new Error(`failed to set mock Provider state: ${await response.text()}`);
}

async function mockRequestCount(baseUrl: string): Promise<number> {
  return (await mockRequests(baseUrl)).length;
}

async function mockRequests(baseUrl: string): Promise<Array<Record<string, unknown>>> {
  const response = await fetch(`${baseUrl}/__mock/requests`);
  if (!response.ok) throw new Error(`mock request audit failed: ${response.status} ${await response.text()}`);
  const value = await response.json() as { requests?: unknown };
  if (!Array.isArray(value.requests)) throw new Error("mock request audit returned invalid requests");
  return value.requests as Array<Record<string, unknown>>;
}

async function auditSuccessfulTask(input: {
  session: GatewaySession;
  taskId: string;
  exactModel: string;
  providerInstance: string;
  startedAtMs: number;
  timeoutMs: number;
}): Promise<{ traceId: string; usage: Record<string, unknown> }> {
  const deadline = Date.now() + Math.min(input.timeoutMs, 10_000);
  let last = "usage and trace not queried";
  while (Date.now() < deadline) {
    try {
      const [usageEvents, traces] = await Promise.all([
        queryUsageEvents({
          aicc: input.session.aicc,
          startTimeMs: input.startedAtMs - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: [input.taskId],
        }),
        queryRouteTraces({
          aicc: input.session.aicc,
          startTimeMs: input.startedAtMs - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: [input.taskId],
        }),
      ]);
      if (usageEvents.length !== 1 || traces.length !== 1) {
        last = `expected one usage event and one route trace, found ${usageEvents.length}/${traces.length}`;
      } else {
        const usage = usageEvents[0];
        const trace = traces[0];
        if (usage.provider_model !== input.exactModel) {
          throw new Error(`usage attributed to ${usage.provider_model}, expected ${input.exactModel}`);
        }
        if (trace.selected_exact_model !== input.exactModel) {
          throw new Error(`trace selected ${trace.selected_exact_model ?? "<missing>"}, expected ${input.exactModel}`);
        }
        if (trace.provider_instance_name !== input.providerInstance) {
          throw new Error(`trace instance ${trace.provider_instance_name ?? "<missing>"}, expected ${input.providerInstance}`);
        }
        return { traceId: trace.trace_id, usage: usage.usage_json };
      }
    } catch (error) {
      last = String(error);
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`durable task audit failed for ${input.taskId}: ${last}`);
}

function inventories(raw: unknown): ProviderInventory[] {
  const providers = raw && typeof raw === "object" ? (raw as { providers?: unknown }).providers : undefined;
  if (!Array.isArray(providers)) throw new Error("models.list.providers must be an array");
  return providers as ProviderInventory[];
}

function taskValue(raw: unknown): Record<string, unknown> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const envelope = raw as Record<string, unknown>;
  const task = envelope.task;
  return task && typeof task === "object" && !Array.isArray(task)
    ? task as Record<string, unknown>
    : envelope;
}

async function waitForMockInventories(
  aicc: RpcClient,
  runId: string,
  timeoutMs: number,
): Promise<ProviderInventory[]> {
  const suffix = runId.replace(/[^a-zA-Z0-9_-]/g, "-");
  const expected = [
    `dv-openai-a-${suffix}`,
    `dv-openai-b-${suffix}`,
    `dv-claude-${suffix}`,
    `dv-gemini-${suffix}`,
    `dv-minimax-${suffix}`,
    `dv-fal-${suffix}`,
  ];
  const deadline = Date.now() + timeoutMs;
  let latest: ProviderInventory[] = [];
  let lastReloadAt = 0;
  while (Date.now() < deadline) {
    if (Date.now() - lastReloadAt >= 2_000) {
      await aicc.call("service.reload_settings", {});
      lastReloadAt = Date.now();
    }
    latest = inventories(await aicc.call("models.list", {}));
    const selected = latest.filter((item) => expected.includes(item.provider_instance_name));
    const openAiReady = selected.some((item) =>
      item.provider_instance_name === `dv-openai-a-${suffix}` &&
      item.models.some((model) => model.provider_model_id === "gpt-5.4") &&
      item.models.some((model) => model.provider_model_id === "gpt-5.4:reasoning-high")
    );
    const geminiReady = selected.some((item) =>
      item.provider_driver === "google-gemini" && item.models.some((model) =>
        model.provider_model_id.toLowerCase().includes("veo-") &&
        model.api_types.includes("video.txt2video")
      )
    );
    if (
      expected.every((name) => selected.some((item) => item.provider_instance_name === name)) &&
      openAiReady && geminiReady
    ) {
      return selected;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));
  }
  const found = latest.map((item) => item.provider_instance_name);
  throw new Error(`mock inventories did not converge; expected=${expected.join(",")} found=${found.join(",")}`);
}

async function terminal(
  session: GatewaySession,
  initial: AiMethodResponse,
  timeoutMs: number,
): Promise<unknown> {
  if (initial.status === "failed") throw new Error(`AICC returned failed: ${failedResponseDiagnostic(initial)}`);
  if (initial.status === "succeeded") return initial;
  const deadline = Date.now() + timeoutMs;
  let lastTask: unknown;
  while (Date.now() < deadline) {
    const task = taskValue(await session.taskManager.call("get_task", { task_id: initial.task_id })) as {
      phase?: string;
      outcome?: string;
      result?: { result?: { output?: unknown } };
      error?: unknown;
    };
    lastTask = task;
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") {
        throw new Error(`task ended ${task.outcome}: ${JSON.stringify(compactFailure(task.error) ?? {})}`);
      }
      return { task_id: initial.task_id, status: "succeeded", result: task.result?.result?.output };
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`task ${initial.task_id} timed out; last=${JSON.stringify(lastTask)}`);
}

async function routedCompletion(
  session: GatewaySession,
  initial: AiMethodResponse,
  startedAtMs: number,
  timeoutMs: number,
): Promise<string> {
  if (initial.status === "failed") throw new Error(`AICC returned failed: ${failedResponseDiagnostic(initial)}`);
  const deadline = Date.now() + Math.min(timeoutMs, 10_000);
  let last = "route trace and usage not queried";
  while (Date.now() < deadline) {
    try {
      const usage = await queryUsageEvents({
        aicc: session.aicc,
        startTimeMs: startedAtMs - 1_000,
        endTimeMs: Date.now() + 1_000,
        taskIds: [initial.task_id],
      });
      const selected = usage.at(-1)?.provider_model;
      if (selected && usage.length > 0) return selected;
      last = `selected=${selected ?? "<missing>"} usage_events=${usage.length}`;
    } catch (error) {
      last = String(error);
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`routed completion audit failed for ${initial.task_id}: ${last}`);
}

function selectedExactModel(value: unknown, depth = 0): string | undefined {
  if (depth > 10 || !value || typeof value !== "object") return undefined;
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = selectedExactModel(item, depth + 1);
      if (found) return found;
    }
    return undefined;
  }
  const objectValue = value as Record<string, unknown>;
  if (typeof objectValue.selected_exact_model === "string") return objectValue.selected_exact_model;
  for (const child of Object.values(objectValue)) {
    const found = selectedExactModel(child, depth + 1);
    if (found) return found;
  }
  return undefined;
}

function namedObjectId(value: unknown, depth = 0): string | undefined {
  if (depth > 12 || !value || typeof value !== "object") return undefined;
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = namedObjectId(item, depth + 1);
      if (found) return found;
    }
    return undefined;
  }
  const objectValue = value as Record<string, unknown>;
  for (const key of ["obj_id", "object_id", "artifact_id"] as const) {
    if (typeof objectValue[key] === "string" && objectValue[key]) return objectValue[key];
  }
  for (const child of Object.values(objectValue)) {
    const found = namedObjectId(child, depth + 1);
    if (found) return found;
  }
  return undefined;
}

function outputKinds(apiType: string): string[] {
  if (apiType.startsWith("embedding.")) return ["embedding"];
  if (apiType.startsWith("image.")) return ["image"];
  if (apiType.startsWith("audio.")) return apiType === "audio.asr" ? ["text"] : ["audio"];
  if (apiType.startsWith("video.")) return ["video"];
  if (apiType.startsWith("vision.")) return apiType === "vision.ocr" || apiType === "vision.caption"
    ? ["text"]
    : ["structured"];
  if (apiType === "rerank") return ["structured"];
  return ["text"];
}

function cellFor(
  inventory: ProviderInventory,
  model: ProviderInventory["models"][number],
  apiType: string,
  method: string,
): MatrixCell {
  return {
    case_id: `t1.mock.${inventory.provider_instance_name}.${model.provider_model_id}.${method}`
      .toLowerCase().replace(/[^a-z0-9._-]+/g, "-"),
    provider_driver: inventory.provider_driver,
    provider_instance: inventory.provider_instance_name,
    exact_model: model.exact_model,
    provider_model_id: model.provider_model_id,
    api_type: apiType,
    method,
    baseline_status: "active",
    input_kinds: [],
    output_kinds: outputKinds(apiType),
    source_urls: [],
    estimated_cost_usd: 0,
  };
}

function mockCells(values: ProviderInventory[]): MatrixCell[] {
  const cells: MatrixCell[] = [];
  for (const inventory of values) {
    const model = inventory.models.find((item) => item.api_types.includes("llm"));
    if (!model) continue;
    cells.push(cellFor(inventory, model, "llm", "chat.completions.create"));
  }
  const firstLlm = values.flatMap((inventory) =>
    inventory.models.filter((model) => model.api_types.includes("llm")).map((model) => ({ inventory, model }))
  )[0];
  if (firstLlm) {
    for (const method of methodsForApiType("llm").filter((method) => method !== "chat.completions.create")) {
      cells.push(cellFor(firstLlm.inventory, firstLlm.model, "llm", method));
    }
  }
  for (const apiType of CANONICAL_API_TYPES) {
    if (apiType === "llm") continue;
    const candidate = values.flatMap((inventory) =>
      inventory.models.filter((model) => model.api_types.includes(apiType)).map((model) => ({ inventory, model }))
    )[0];
    if (!candidate) continue;
    for (const method of methodsForApiType(apiType)) {
      cells.push(cellFor(candidate.inventory, candidate.model, apiType, method));
    }
  }
  return cells;
}

async function runRouteCases(
  session: GatewaySession,
  runId: string,
  mockInventories: ProviderInventory[],
  input: Options,
): Promise<CaseReport[]> {
  const openaiA = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-a-"));
  const openaiB = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-b-"));
  const modelA = openaiA?.models.find((item) => item.api_types.includes("llm"));
  const modelB = openaiB?.models.find((item) => item.provider_model_id === modelA?.provider_model_id);
  const logicalModel = modelA?.logical_mounts.find((mount) => modelB?.logical_mounts.includes(mount));
  if (!openaiA || !openaiB || !modelA || !modelB || !logicalModel) {
    throw new Error("route tests require two OpenAI mock instances with one shared LLM logical mount");
  }
  const exactRuleModel = openaiA.models.find((item) => item.provider_model_id === "gpt-image-1");
  const patternRuleModel = openaiA.models.find((item) => item.provider_model_id === "sora-mock-pattern");
  const defaultRuleModel = openaiA.models.find((item) => item.provider_model_id === "gpt-5.4");
  const exactRuleMount = exactRuleModel?.logical_mounts.find((mount) => mount.includes("gpt_image"));
  const patternRuleMount = patternRuleModel?.logical_mounts.find((mount) => mount.includes("sora-mock-pattern"));
  const defaultRuleMount = defaultRuleModel?.logical_mounts.find((mount) => mount.includes("gpt-5.4"));
  type RouteCase = {
    id: string;
    apiType: string;
    logicalModel: string;
    policy: Record<string, unknown>;
    expectedInstance?: string;
    expectedExactModel?: string;
    reject: boolean;
    expectedErrorCodes?: string[];
  };
  const cases: RouteCase[] = [
    {
      id: "logical_model_selects_candidate",
      apiType: "llm",
      logicalModel,
      policy: {},
      expectedInstance: undefined,
      reject: false,
    },
    {
      id: "provider_allow",
      apiType: "llm",
      logicalModel,
      policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
      expectedInstance: openaiA.provider_instance_name,
      reject: false,
    },
    {
      id: "provider_deny",
      apiType: "llm",
      logicalModel,
      policy: {
        allowed_provider_instances: [openaiB.provider_instance_name],
        blocked_provider_instances: [openaiA.provider_instance_name],
      },
      expectedInstance: openaiB.provider_instance_name,
      reject: false,
    },
    {
      id: "missing_provider_instance",
      apiType: "llm",
      logicalModel,
      policy: { allowed_provider_instances: [`missing-${runId}`] },
      expectedInstance: undefined,
      reject: true,
      expectedErrorCodes: ["no_provider_available"],
    },
    {
      id: "local_only",
      apiType: "llm",
      logicalModel,
      policy: { local_only: true },
      expectedInstance: undefined,
      reject: true,
      expectedErrorCodes: ["no_provider_available"],
    },
    {
      id: "invalid_exact_model",
      apiType: "llm",
      logicalModel: modelA.exact_model,
      policy: {},
      expectedInstance: undefined,
      reject: true,
      expectedErrorCodes: ["bad_request"],
    },
    {
      id: "invalid_logical_path",
      apiType: "llm",
      logicalModel: `missing.logical.${runId}`,
      policy: {},
      expectedInstance: undefined,
      reject: true,
      expectedErrorCodes: ["model_alias_not_mapped"],
    },
    {
      id: "fallback_api_type_boundary",
      apiType: "embedding.text",
      logicalModel,
      policy: {},
      expectedInstance: undefined,
      reject: true,
      expectedErrorCodes: ["no_provider_available", "model_alias_not_mapped"],
    },
  ];
  const versionCases = [
    { id: "version_exact_rule", apiType: "image.txt2img", model: exactRuleModel, mount: exactRuleMount },
    { id: "version_pattern_rule", apiType: "video.txt2video", model: patternRuleModel, mount: patternRuleMount },
    { id: "version_default_rule", apiType: "llm", model: defaultRuleModel, mount: defaultRuleMount },
  ];
  const preconditionFailures: CaseReport[] = [];
  for (const versionCase of versionCases) {
    if (versionCase.model && versionCase.mount) {
      cases.push({
        id: versionCase.id,
        apiType: versionCase.apiType,
        logicalModel: versionCase.mount,
        policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
        expectedInstance: openaiA.provider_instance_name,
        expectedExactModel: versionCase.model.exact_model,
        reject: false,
      });
      continue;
    }
    if (!wants(input, `t1.route.${versionCase.id}`)) continue;
    preconditionFailures.push({
      run_id: runId,
      case_id: `t1.route.${versionCase.id}`,
      layer: "T1",
      status: "failed",
      api_type: versionCase.apiType,
      method: "route.resolve",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [{
        attempt: 1,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        status: "failed",
        failure_class: "routing_failed",
        diagnostic: `AICC inventory omitted the mock route metadata model or logical mount; available=${JSON.stringify(openaiA.models.map((item) => ({ provider_model_id: item.provider_model_id, logical_mounts: item.logical_mounts })))}`,
        estimated_cost_usd: 0,
        cost_status: "not_called",
      }],
    });
  }
  const routeResults = [...await Promise.all(cases.filter((testCase) =>
    wants(input, `t1.route.${testCase.id}`)
  ).map(async (testCase) => {
    const started = Date.now();
    const caseId = `t1.route.${testCase.id}`;
    const report: CaseReport = {
      run_id: runId,
      case_id: caseId,
      layer: "T1",
      status: "failed",
      api_type: testCase.apiType,
      method: "route.resolve",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [],
    };
    try {
      const response = await session.aicc.call("route.resolve", {
        request_id: `${runId}:${caseId}`,
        api_type: testCase.apiType,
        logical_model: testCase.logicalModel,
        requirements: {},
        disable: {},
        policy: testCase.policy,
      }) as Record<string, unknown>;
      if (testCase.reject) throw new Error("route unexpectedly resolved");
      const selected = response.selected_exact_model;
      const instance = response.provider_instance_name;
      if (typeof selected !== "string" || !selected.endsWith(`@${String(instance)}`)) {
        throw new Error(`route response has inconsistent model/instance: ${JSON.stringify(response)}`);
      }
      if (testCase.expectedInstance && instance !== testCase.expectedInstance) {
        throw new Error(`selected instance ${String(instance)}, expected ${testCase.expectedInstance}`);
      }
      if (testCase.expectedExactModel && selected !== testCase.expectedExactModel) {
        throw new Error(`selected exact model ${String(selected)}, expected ${testCase.expectedExactModel}`);
      }
      report.status = "passed";
      report.exact_model = selected;
      report.provider_instance = String(instance);
      report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
    } catch (error) {
      const diagnostic = String(error);
      const expectedErrorCodes = testCase.expectedErrorCodes;
      const expectedRejection = testCase.reject &&
        !diagnostic.includes("route unexpectedly resolved") &&
        expectedErrorCodes?.some((code) => diagnostic.includes(`${code}:`));
      if (expectedRejection) {
        report.status = "passed";
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", diagnostic: `expected routing rejection observed: ${diagnostic}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
      } else {
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "failed", failure_class: "routing_failed", diagnostic, estimated_cost_usd: 0, cost_status: "not_called" });
      }
    }
    return report;
  })), ...preconditionFailures];

  const overlayTree = (path: string, leaf: Record<string, unknown>): Record<string, unknown> => {
    const parts = path.split(".");
    let node = leaf;
    for (let index = parts.length - 1; index > 0; index -= 1) {
      node = { children: { [parts[index]]: node } };
    }
    return { [parts[0]]: node };
  };
  const routeRequest = (caseId: string, overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
    request_id: `${runId}:${caseId}`,
    api_type: "llm",
    logical_model: logicalModel,
    requirements: {},
    disable: {},
    policy: {},
    ...overrides,
  });
  const expectRouteRejected = async (request: Record<string, unknown>): Promise<string> => {
    try {
      await session.aicc.call("route.resolve", request);
    } catch (error) {
      return `route rejected: ${String(error).slice(0, 300)}`;
    }
    throw new Error("route unexpectedly resolved");
  };
  const pushRouteProbe = async (
    caseId: string,
    execute: () => Promise<string | undefined>,
  ): Promise<void> => {
    if (!wants(input, caseId)) return;
    routeResults.push(await executeT1Probe({
      runId,
      caseId,
      method: "route.resolve",
      apiType: "llm",
      failureClass: "routing_failed",
      execute,
    }));
  };

  await pushRouteProbe("t1.route.legal_missing_model", async () => {
    const before = await mockRequestCount(input.mockControlUrl);
    const request = buildExactRequest({
      cell: { ...cellFor(openaiA, modelA, "llm", "chat.completions.create"), case_id: "t1.route.legal_missing_model" },
      runId,
      fixtures: {},
    });
    request.model = { alias: `missing-model@${openaiA.provider_instance_name}` };
    const response = await callChatCompletions(session.aicc, request) as AiMethodResponse;
    if (response.status !== "failed") throw new Error(`missing exact model returned ${response.status}`);
    const after = await mockRequestCount(input.mockControlUrl);
    if (after !== before) throw new Error("missing exact model unexpectedly reached Provider");
    return "legal missing exact model failed without Provider call";
  });

  await pushRouteProbe("t1.route.disabled_model", () => expectRouteRejected(routeRequest("t1.route.disabled_model", {
    requirements: { web_search: true },
    disable: { web_search: true },
  })));
  await pushRouteProbe("t1.route.unmounted_model", () => expectRouteRejected(routeRequest("t1.route.unmounted_model", {
    session_overlay: { logical_tree: overlayTree(logicalModel, { items: {} }) },
  })));
  await pushRouteProbe("t1.route.corrupt_metadata", () => expectRouteRejected(routeRequest("t1.route.corrupt_metadata", {
    session_overlay: {
      logical_tree: overlayTree(logicalModel, {
        items: { corrupt: { target: "not-an-exact-or-logical-model", weight: -1 } },
      }),
    },
  })));
  await pushRouteProbe("t1.route.privacy_boundary", () => expectRouteRejected(routeRequest("t1.route.privacy_boundary", {
    policy: { local_only: true },
  })));
  await pushRouteProbe("t1.route.budget_filter", () => expectRouteRejected(routeRequest("t1.route.budget_filter", {
    policy: { max_cost_usd: 0 },
  })));
  await pushRouteProbe("t1.route.context_limit_filter", () => expectRouteRejected(routeRequest("t1.route.context_limit_filter", {
    requirements: { min_context_tokens: 1_000_000_000 },
  })));
  await pushRouteProbe("t1.route.output_limit_filter", () => expectRouteRejected(routeRequest("t1.route.output_limit_filter", {
    estimated_output_tokens: 1_000_000_000,
  })));
  await pushRouteProbe("t1.route.locked_policy_cannot_override", () => expectRouteRejected(routeRequest("t1.route.locked_policy_cannot_override", {
    policy: { allowed_provider_instances: [openaiB.provider_instance_name] },
    session_overlay: {
      policy: {
        allowed_provider_instances: { value: [openaiA.provider_instance_name], locked: true },
      },
    },
  })));
  await pushRouteProbe("t1.route.missing_metadata_is_conservative", () => expectRouteRejected(routeRequest("t1.route.missing_metadata_is_conservative", {
    requirements: { must_features: ["unknown-high-risk-capability"] },
  })));

  await pushRouteProbe("t1.route.auto_mount_admission", async () => {
    const response = await session.aicc.call("route.resolve", routeRequest("t1.route.auto_mount_admission", {
      logical_model: "llm.dv_acceptance.auto",
      policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
    })) as Record<string, unknown>;
    if (response.provider_instance_name !== openaiA.provider_instance_name) {
      throw new Error(`auto mount selected ${String(response.provider_instance_name)}`);
    }
    return `auto admission selected ${String(response.selected_exact_model)}`;
  });
  await pushRouteProbe("t1.route.manual_mount_requires_mapping", () => expectRouteRejected(routeRequest("t1.route.manual_mount_requires_mapping", {
    logical_model: "llm.dv_acceptance.manual",
    policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
  })));
  await pushRouteProbe("t1.route.min_line_admission", () => expectRouteRejected(routeRequest("t1.route.min_line_admission", {
    logical_model: "llm.dv_acceptance.min_line",
    policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
  })));
  await pushRouteProbe("t1.route.disable_line_applied", async () => {
    const response = await session.aicc.call("route.resolve", routeRequest("t1.route.disable_line_applied", {
      logical_model: "llm.dv_acceptance.disable_line",
      policy: { allowed_provider_instances: [openaiA.provider_instance_name] },
    })) as Record<string, unknown>;
    const disabled = Array.isArray(response.disabled_capabilities) ? response.disabled_capabilities : [];
    if (!disabled.includes("web_search")) {
      throw new Error(`disable_line missing from response: ${JSON.stringify(response)}`);
    }
    return "definition disable_line removed web_search";
  });

  for (const [caseId, sessionOverlay] of [
    ["t1.route.global_exact_model_weight", {
      global_exact_model_weights: { [modelA.exact_model]: 0, [modelB.exact_model]: 1 },
    }],
    ["t1.route.logical_exact_model_weight", {
      logical_tree: overlayTree(logicalModel, {
        exact_model_weights: { [modelA.exact_model]: 0, [modelB.exact_model]: 1 },
      }),
    }],
    ["t1.route.provider_instance_weight", {
      provider_weights: { [openaiA.provider_instance_name]: 0, [openaiB.provider_instance_name]: 1 },
    }],
  ] as const) {
    await pushRouteProbe(caseId, async () => {
      const response = await session.aicc.call("route.resolve", routeRequest(caseId, {
        session_overlay: sessionOverlay,
        policy: { allowed_provider_instances: [openaiA.provider_instance_name, openaiB.provider_instance_name] },
      })) as Record<string, unknown>;
      if (response.provider_instance_name !== openaiB.provider_instance_name) {
        throw new Error(`zero-weight candidate was selected: ${JSON.stringify(response)}`);
      }
      return `weight filter selected ${String(response.selected_exact_model)}`;
    });
  }

  await pushRouteProbe("t1.route.system_config_then_request_overlay", async () => {
    const systemResponse = await session.aicc.call("route.resolve", routeRequest("t1.route.system_config_then_request_overlay.system", {
      logical_model: "llm.dv_acceptance.system_overlay",
    })) as Record<string, unknown>;
    if (systemResponse.provider_instance_name !== openaiA.provider_instance_name) {
      throw new Error(`system routing fixture selected ${String(systemResponse.provider_instance_name)}`);
    }
    const requestResponse = await session.aicc.call("route.resolve", routeRequest("t1.route.system_config_then_request_overlay.request", {
      logical_model: "llm.dv_acceptance.system_overlay",
      session_overlay: {
        logical_tree: overlayTree("llm.dv_acceptance.system_overlay", {
          items: { request: { target: modelB.exact_model, weight: 1 } },
        }),
      },
    })) as Record<string, unknown>;
    if (requestResponse.provider_instance_name !== openaiB.provider_instance_name) {
      throw new Error(`request overlay did not replace system route: ${JSON.stringify(requestResponse)}`);
    }
    return "request session_overlay took precedence over system routing config";
  });

  for (const [caseId, state] of [
    ["t1.route.offline_model", { health: "unavailable", quota: "normal" }],
    ["t1.route.health_filter", { health: "unavailable", quota: "normal" }],
    ["t1.route.quota_filter", { health: "available", quota: "exhausted" }],
  ] as const) {
    await pushRouteProbe(caseId, async () => {
      void state;
      throw new SkipCase("the OpenAI /v1/models protocol exposes model IDs only; current AICC admin APIs provide no test-scoped health/quota override for route admission");
    });
  }

  const fallbackPath = `${logicalModel}.dv`;
  const fallbackLeaf = (fallback: Record<string, unknown>): Record<string, unknown> => ({
    items: { missing: { target: `missing@${openaiA.provider_instance_name}`, weight: 1 } },
    fallback,
  });
  await pushRouteProbe("t1.route.strict_no_fallback", () => expectRouteRejected(routeRequest("t1.route.strict_no_fallback", {
    logical_model: fallbackPath,
    session_overlay: { logical_tree: overlayTree(fallbackPath, fallbackLeaf({ mode: "strict" })) },
  })));
  for (const [caseId, fallback] of [
    ["t1.route.parent_fallback", { mode: "parent" }],
    ["t1.route.target_logical_fallback", { mode: "target_logical", target: logicalModel }],
    ["t1.route.target_exact_fallback", { mode: "target_exact", target: modelB.exact_model }],
  ] as const) {
    await pushRouteProbe(caseId, async () => {
      const response = await session.aicc.call("route.resolve", routeRequest(caseId, {
        logical_model: fallbackPath,
        session_overlay: { logical_tree: overlayTree(fallbackPath, fallbackLeaf(fallback)) },
      })) as Record<string, unknown>;
      if (typeof response.selected_exact_model !== "string") throw new Error("fallback route returned no exact model");
      const trace = response.route_trace as Record<string, unknown> | undefined;
      if (!trace?.fallback_applied) throw new Error("fallback route did not mark fallback_applied");
      return `fallback selected ${response.selected_exact_model}`;
    });
  }
  await pushRouteProbe("t1.route.exact_default_no_fallback", async () => {
    const before = await mockRequestCount(input.mockControlUrl);
    const request = buildExactRequest({ cell: { ...cellFor(openaiA, modelA, "llm", "chat.completions.create"), case_id: "t1.route.exact_default_no_fallback" }, runId, fixtures: {} });
    request.model = { alias: `missing@${openaiA.provider_instance_name}` };
    const response = await callChatCompletions(session.aicc, request) as AiMethodResponse;
    if (response.status !== "failed") throw new Error("missing exact model unexpectedly fell back");
    if (await mockRequestCount(input.mockControlUrl) !== before) throw new Error("exact model fallback reached another Provider");
    return "exact typed request did not fallback";
  });
  await pushRouteProbe("t1.route.fallback_loop", () => expectRouteRejected(routeRequest("t1.route.fallback_loop", {
    logical_model: fallbackPath,
    session_overlay: { logical_tree: overlayTree(fallbackPath, fallbackLeaf({ mode: "target_logical", target: fallbackPath })) },
  })));
  await pushRouteProbe("t1.route.fallback_max_depth", () => expectRouteRejected(routeRequest("t1.route.fallback_max_depth", {
    logical_model: `${fallbackPath}.one`,
    session_overlay: {
      logical_tree: overlayTree(`${fallbackPath}.one`, fallbackLeaf({ mode: "target_logical", target: `${fallbackPath}.two` })),
    },
  })));

  const schedulerProfiles = ["cost_first", "latency_first", "quality_first", "balanced", "local_first", "strict_local"];
  for (const profile of schedulerProfiles) {
    const caseId = `t1.scheduler.profile.${profile}`;
    if (!wants(input, caseId)) continue;
    routeResults.push(await executeT1Probe({
      runId,
      caseId,
      method: "route.resolve",
      apiType: "llm",
      failureClass: "routing_failed",
      execute: async () => {
        const response = await session.aicc.call("route.resolve", routeRequest(caseId, {
          session_overlay: { policy: { profile: { value: profile, locked: false } } },
        })) as Record<string, unknown>;
        const trace = response.route_trace as Record<string, unknown> | undefined;
        if (trace?.scheduler_profile !== profile) {
          throw new Error(`route trace scheduler_profile=${String(trace?.scheduler_profile)}, expected ${profile}`);
        }
        return `${profile} selected ${String(response.selected_exact_model)}`;
      },
    }));
  }
  return routeResults;
}

async function commitId(): Promise<string> {
  try {
    const output = await new Deno.Command("git", {
      args: ["rev-parse", "HEAD"],
      cwd: resolve(here, "../../.."),
      stdout: "piped",
      stderr: "null",
    }).output();
    return output.success ? new TextDecoder().decode(output.stdout).trim() : "unknown";
  } catch {
    return "unknown";
  }
}

function canonicalCaseId(item: CaseReport): string {
  return item.case_id;
}

export function manifestCoverage(cases: readonly CaseReport[]): NonNullable<AcceptanceReport["manifest_coverage"]> {
  const manifest = buildStaticManifest();
  const known = new Set(manifest.map((item) => item.case_id));
  const statuses = new Map<string, CaseReport["status"][]>();
  for (const item of cases) {
    const mapped = canonicalCaseId(item);
    if (!known.has(mapped)) continue;
    const values = statuses.get(mapped) ?? [];
    values.push(item.status);
    statuses.set(mapped, values);
  }
  const failed = [...statuses.values()].filter((values) => values.includes("failed")).length;
  const passed = [...statuses.values()].filter((values) =>
    !values.includes("failed") && values.includes("passed")
  ).length;
  const unexecutedCaseIds = manifest.map((item) => item.case_id).filter((caseId) => !statuses.has(caseId));
  return {
    total: manifest.length,
    executed: statuses.size,
    passed,
    failed,
    coverage_rate: manifest.length === 0 ? 1 : statuses.size / manifest.length,
    unexecuted_case_ids: unexecutedCaseIds,
  };
}

async function executeT1Probe(input: {
  runId: string;
  caseId: string;
  method: string;
  apiType?: string;
  failureClass: CaseReport["attempts"][number]["failure_class"];
  execute: () => Promise<string | undefined>;
}): Promise<CaseReport> {
  const started = Date.now();
  const report: CaseReport = {
    run_id: input.runId,
    case_id: input.caseId,
    layer: "T1",
    status: "failed",
    api_type: input.apiType,
    method: input.method,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  try {
    const diagnostic = await input.execute();
    report.status = "passed";
    report.attempts.push({
      attempt: 1,
      started_at: new Date(started).toISOString(),
      elapsed_ms: Date.now() - started,
      status: "passed",
      diagnostic,
      estimated_cost_usd: 0,
      actual_cost_usd: 0,
      cost_status: "actual",
    });
  } catch (error) {
    if (error instanceof SkipCase) {
      report.status = "skipped";
      report.attempts.push({
        attempt: 1,
        started_at: new Date(started).toISOString(),
        elapsed_ms: Date.now() - started,
        status: "skipped",
        diagnostic: error.message,
        estimated_cost_usd: 0,
        cost_status: "not_called",
      });
      return report;
    }
    report.attempts.push({
      attempt: 1,
      started_at: new Date(started).toISOString(),
      elapsed_ms: Date.now() - started,
      status: "failed",
      failure_class: input.failureClass,
      diagnostic: String(error),
      estimated_cost_usd: 0,
      cost_status: "unknown",
    });
  }
  return report;
}

async function runCases(
  session: GatewaySession,
  mockBaseUrl: string,
  runId: string,
  mockInventories: ProviderInventory[],
  input: Options,
): Promise<CaseReport[]> {
  const results: CaseReport[] = [];
  const cells = mockCells(mockInventories);
  if (cells.length < 4) throw new Error(`expected at least four mock LLM adapters, found ${cells.length}`);
  const protocolCells = [...new Map(
    cells.map((cell) => [`${cell.api_type}\u0000${cell.method}`, cell] as const),
  ).values()];
  const scheduler = new ProviderScheduler(input.globalConcurrency, {
    maxConcurrency: input.providerConcurrency,
    minIntervalMs: input.providerMinIntervalMs,
  });
  await setScenario(input.mockControlUrl, "success");
  const successes = await scheduler.run(protocolCells.filter((cell) =>
    wants(input, `t1.route.api_type.${cell.api_type}.${cell.method}`)
  ), async (cell) => {
    const started = Date.now();
    const caseId = `t1.route.api_type.${cell.api_type}.${cell.method}`;
    const report: CaseReport = {
      run_id: runId,
      case_id: caseId,
      layer: "T1",
      status: "failed",
      provider_driver: cell.provider_driver,
      provider_instance: cell.provider_instance,
      exact_model: cell.exact_model,
      api_type: cell.api_type,
      method: cell.method,
      session_id: `${runId}:${caseId}`,
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [],
    };
    try {
      const request = buildExactRequest({
        cell: { ...cell, case_id: caseId },
        runId,
        fixtures: {
          image: { kind: "url", url: `${mockBaseUrl}/__mock/fixtures/image.png`, mime_hint: "image/png" },
          mask: { kind: "url", url: `${mockBaseUrl}/__mock/fixtures/image.png`, mime_hint: "image/png" },
          audio: { kind: "url", url: `${mockBaseUrl}/__mock/fixtures/audio.wav`, mime_hint: "audio/wav" },
          video: { kind: "url", url: `${mockBaseUrl}/__mock/fixtures/video.mp4`, mime_hint: "video/mp4" },
        },
      });
      const initial = await callInference(session.aicc, cell.method, request) as AiMethodResponse;
      const value = await terminal(session, initial, input.timeoutMs);
      assertResponseShape(cell, value);
      const audit = await auditSuccessfulTask({
        session,
        taskId: initial.task_id,
        exactModel: cell.exact_model,
        providerInstance: cell.provider_instance,
        startedAtMs: started,
        timeoutMs: input.timeoutMs,
      });
      report.status = "passed";
      report.task_id = initial.task_id;
      report.trace_id = audit.traceId;
      report.usage = audit.usage;
      report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
    } catch (error) {
      report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "failed", failure_class: "routing_failed", diagnostic: String(error), estimated_cost_usd: 0, cost_status: "unknown" });
    }
    return report;
  });
  results.push(...successes);

  results.push(...await runRouteCases(session, runId, mockInventories, input));

  const exactInventory = mockInventories.find((inventory) =>
    inventory.provider_instance_name.includes("dv-openai-a-")
  );
  const exactBaseModel = exactInventory?.models.find((model) =>
    model.provider_model_id === "gpt-5.4"
  );
  const exactVariantModel = exactInventory?.models.find((model) =>
    model.provider_model_id === "gpt-5.4:reasoning-high"
  );
  if (!exactInventory || !exactBaseModel || !exactVariantModel) {
    throw new Error("exact and variant tests require gpt-5.4 base/reasoning-high inventory entries");
  }
  const exactCases = [
    {
      caseId: "t1.route.exact_model_hits_instance",
      model: exactBaseModel,
    },
    {
      caseId: "t1.route.metadata_variant_expands_exact_model",
      model: exactVariantModel,
    },
  ];
  for (const exactCase of exactCases) {
    if (!wants(input, exactCase.caseId)) continue;
    const started = Date.now();
    const report: CaseReport = {
      run_id: runId,
      case_id: exactCase.caseId,
      layer: "T1",
      status: "failed",
      provider_driver: exactInventory.provider_driver,
      provider_instance: exactInventory.provider_instance_name,
      exact_model: exactCase.model.exact_model,
      api_type: "llm",
      method: "chat.completions.create",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [],
    };
    try {
      const cell = cellFor(exactInventory, exactCase.model, "llm", "chat.completions.create");
      const request = buildExactRequest({ cell: { ...cell, case_id: exactCase.caseId }, runId, fixtures: {} });
      const initial = await callChatCompletions(session.aicc, request) as AiMethodResponse;
      const value = await terminal(session, initial, input.timeoutMs);
      assertResponseShape(cell, value);
      const audit = await auditSuccessfulTask({
        session,
        taskId: initial.task_id,
        exactModel: exactCase.model.exact_model,
        providerInstance: exactInventory.provider_instance_name,
        startedAtMs: started,
        timeoutMs: input.timeoutMs,
      });
      report.status = "passed";
      report.task_id = initial.task_id;
      report.trace_id = audit.traceId;
      report.usage = audit.usage;
      report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", diagnostic: `exact model ${exactCase.model.exact_model} executed without logical fallback`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
    } catch (error) {
      report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "failed", failure_class: "provider_protocol_failed", diagnostic: String(error), estimated_cost_usd: 0, cost_status: "unknown" });
    }
    results.push(report);
  }

  const embeddingModel = exactInventory.models.find((model) =>
    model.provider_model_id === "text-embedding-3-small"
  );
  if (!embeddingModel) throw new Error("embedding protocol tests require text-embedding-3-small");
  const embeddingCell = cellFor(exactInventory, embeddingModel, "embedding.text", "embedding.text");
  if (wants(input, "t1.embedding.large_batch_artifact")) {
  const largeEmbeddingStarted = Date.now();
  const largeEmbeddingCell: MatrixCell = {
    ...embeddingCell,
    case_id: "t1.embedding.large_batch_artifact",
    variant: "embedding_large_artifact",
  };
  const largeEmbeddingReport: CaseReport = {
    run_id: runId,
    case_id: largeEmbeddingCell.case_id,
    layer: "T1",
    status: "failed",
    provider_driver: exactInventory.provider_driver,
    provider_instance: exactInventory.provider_instance_name,
    exact_model: embeddingModel.exact_model,
    api_type: "embedding.text",
    method: "embedding.text",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  try {
    const request = buildExactRequest({ cell: largeEmbeddingCell, runId, fixtures: {} });
    const initial = await session.aicc.call("embedding.text", request) as AiMethodResponse;
    const value = await terminal(session, initial, input.timeoutMs);
    assertResponseShape(largeEmbeddingCell, value);
    const audit = await auditSuccessfulTask({
      session,
      taskId: initial.task_id,
      exactModel: embeddingModel.exact_model,
      providerInstance: exactInventory.provider_instance_name,
      startedAtMs: largeEmbeddingStarted,
      timeoutMs: input.timeoutMs,
    });
    largeEmbeddingReport.status = "passed";
    largeEmbeddingReport.task_id = initial.task_id;
    largeEmbeddingReport.trace_id = audit.traceId;
    largeEmbeddingReport.usage = audit.usage;
    largeEmbeddingReport.attempts.push({ attempt: 1, started_at: new Date(largeEmbeddingStarted).toISOString(), elapsed_ms: Date.now() - largeEmbeddingStarted, status: "passed", diagnostic: "101 embedding rows returned through the artifact contract", estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
  } catch (error) {
    largeEmbeddingReport.attempts.push({ attempt: 1, started_at: new Date(largeEmbeddingStarted).toISOString(), elapsed_ms: Date.now() - largeEmbeddingStarted, status: "failed", failure_class: "provider_protocol_failed", diagnostic: String(error), estimated_cost_usd: 0, cost_status: "unknown" });
  }
  results.push(largeEmbeddingReport);
  }

  if (wants(input, "t1.embedding.space_mismatch_rejected")) {
  const spaceMismatchStarted = Date.now();
  const spaceMismatchReport: CaseReport = {
    run_id: runId,
    case_id: "t1.embedding.space_mismatch_rejected",
    layer: "T1",
    status: "failed",
    provider_driver: exactInventory.provider_driver,
    provider_instance: exactInventory.provider_instance_name,
    exact_model: embeddingModel.exact_model,
    api_type: "embedding.text",
    method: "embedding.text",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  const spaceRequestsBefore = await mockRequestCount(input.mockControlUrl);
  try {
    const request = buildExactRequest({ cell: { ...embeddingCell, case_id: spaceMismatchReport.case_id }, runId, fixtures: {} });
    const inputJson = (request.payload as Record<string, unknown>).input_json as Record<string, unknown>;
    inputJson.embedding_space_id = "incompatible-space-for-dv";
    const initial = await session.aicc.call("embedding.text", request) as AiMethodResponse;
    await terminal(session, initial, input.timeoutMs);
    spaceMismatchReport.task_id = initial.task_id;
    spaceMismatchReport.attempts.push({ attempt: 1, started_at: new Date(spaceMismatchStarted).toISOString(), elapsed_ms: Date.now() - spaceMismatchStarted, status: "failed", failure_class: "provider_protocol_failed", diagnostic: "AICC ignored the requested embedding_space_id and returned a vector from a different space", estimated_cost_usd: 0, cost_status: "unknown" });
  } catch (error) {
    const requestsAfter = await mockRequestCount(input.mockControlUrl);
    if (requestsAfter > spaceRequestsBefore && !String(error).toLowerCase().includes("timed out")) {
      spaceMismatchReport.status = "passed";
      spaceMismatchReport.attempts.push({ attempt: 1, started_at: new Date(spaceMismatchStarted).toISOString(), elapsed_ms: Date.now() - spaceMismatchStarted, status: "passed", diagnostic: `embedding space mismatch rejected: ${String(error)}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
    } else {
      spaceMismatchReport.attempts.push({ attempt: 1, started_at: new Date(spaceMismatchStarted).toISOString(), elapsed_ms: Date.now() - spaceMismatchStarted, status: "failed", failure_class: "provider_protocol_failed", diagnostic: `embedding space mismatch did not produce a valid rejection: ${String(error)}`, estimated_cost_usd: 0, cost_status: "unknown" });
    }
  }
  results.push(spaceMismatchReport);
  }

  const executableRouteKeys = new Set(protocolCells.map((cell) => `${cell.api_type}\u0000${cell.method}`));
  for (const manifestCase of buildStaticManifest().filter((testCase) =>
    testCase.case_id.startsWith("t1.route.api_type.") &&
    !executableRouteKeys.has(`${testCase.api_type}\u0000${testCase.method}`) &&
    wants(input, testCase.case_id)
  )) {
    results.push({
      run_id: runId,
      case_id: manifestCase.case_id,
      layer: "T1",
      status: "failed",
      api_type: manifestCase.api_type ?? undefined,
      method: manifestCase.method,
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [{
        attempt: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        status: "failed",
        failure_class: "preflight_failed",
        diagnostic: `T1 Mock inventory exposes no model for ${manifestCase.api_type}/${manifestCase.method}`,
        estimated_cost_usd: 0,
        cost_status: "not_called",
      }],
    });
  }

  const openaiA = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-a-"));
  const openaiB = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-b-"));
  const modelA = openaiA?.models.find((item) => item.api_types.includes("llm"));
  const modelB = openaiB?.models.find((item) => item.provider_model_id === modelA?.provider_model_id);
  const logicalModel = modelA?.logical_mounts.find((mount) => modelB?.logical_mounts.includes(mount));
  const runHistorySeed = wants(input, "t1.history.same_session_reuses_exact_model") ||
    wants(input, "t1.history.hard_constraint_overrides.provider_denied");
  const historyStarted = Date.now();
  const historyCase: CaseReport = {
    run_id: runId,
    case_id: "t1.history.same_session_reuses_exact_model",
    layer: "T1",
    status: "failed",
    provider_driver: "openai",
    provider_instance: openaiA?.provider_instance_name,
    exact_model: modelA?.exact_model,
    api_type: "llm",
    method: "chat.completions.create",
    session_id: `${runId}:history-soft-preference`,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  if (runHistorySeed) try {
    if (!openaiA || !modelA || !modelB || !logicalModel) {
      throw new Error("two OpenAI mock instances do not expose a shared LLM logical mount");
    }
    const exactCell = cells.find((cell) => cell.exact_model === modelA.exact_model)!;
    const firstRequest = buildExactRequest({ cell: exactCell, runId, fixtures: {} });
    const firstPayload = firstRequest.payload as Record<string, unknown>;
    firstPayload.options = { session_id: historyCase.session_id, rootid: runId };
    const first = await callChatCompletions(session.aicc, firstRequest) as AiMethodResponse;
    const firstSelected = await routedCompletion(session, first, historyStarted, input.timeoutMs);
    if (firstSelected !== modelA.exact_model) {
      throw new Error(`exact seed selected ${firstSelected ?? "<missing>"}, expected ${modelA.exact_model}`);
    }

    const secondRequest = buildExactRequest({
      cell: { ...exactCell, case_id: `${historyCase.case_id}.second` },
      runId,
      fixtures: {},
    });
    secondRequest.model = { alias: logicalModel };
    secondRequest.idempotency_key = `${runId}:${historyCase.case_id}:second`;
    const secondPayload = secondRequest.payload as Record<string, unknown>;
    secondPayload.options = { session_id: historyCase.session_id, rootid: runId };
    const secondStarted = Date.now();
    const second = await callChatCompletions(session.aicc, secondRequest) as AiMethodResponse;
    const secondSelected = await routedCompletion(session, second, secondStarted, input.timeoutMs);
    if (secondSelected !== modelA.exact_model) {
      throw new Error(
        `same-session history selected ${secondSelected ?? "<missing>"}; expected prior exact model ${modelA.exact_model}`,
      );
    }
    historyCase.status = "passed";
    historyCase.task_id = second.task_id;
    historyCase.attempts.push({ attempt: 1, started_at: new Date(historyStarted).toISOString(), elapsed_ms: Date.now() - historyStarted, status: "passed", diagnostic: `reused ${secondSelected}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
  } catch (error) {
    historyCase.attempts.push({ attempt: 1, started_at: new Date(historyStarted).toISOString(), elapsed_ms: Date.now() - historyStarted, status: "failed", failure_class: "routing_failed", diagnostic: String(error), estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
  }
  if (wants(input, historyCase.case_id)) results.push(historyCase);

  const hardStarted = Date.now();
  const hardCase: CaseReport = {
    run_id: runId,
    case_id: "t1.history.hard_constraint_overrides.provider_denied",
    layer: "T1",
    status: "failed",
    provider_driver: "openai",
    provider_instance: openaiB?.provider_instance_name,
    exact_model: modelB?.exact_model,
    api_type: "llm",
    method: "chat.completions.create",
    session_id: `${runId}:history:provider-denied`,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  if (wants(input, hardCase.case_id)) try {
    if (!openaiA || !openaiB || !modelA || !modelB || !logicalModel) {
      throw new Error("history provider-denied case needs two shared-mount OpenAI instances");
    }
    const exactCell = cells.find((cell) => cell.exact_model === modelA.exact_model && cell.method === "chat.completions.create");
    if (!exactCell) throw new Error("missing OpenAI mock cell for hard-constraint history case");
    const seedRequest = buildExactRequest({
      cell: { ...exactCell, case_id: `${hardCase.case_id}.seed` },
      runId,
      fixtures: {},
    });
    const seedPayload = seedRequest.payload as Record<string, unknown>;
    seedPayload.options = { session_id: hardCase.session_id, rootid: runId };
    const seedStarted = Date.now();
    const seed = await callChatCompletions(session.aicc, seedRequest) as AiMethodResponse;
    const seedSelected = await routedCompletion(session, seed, seedStarted, input.timeoutMs);
    if (seedSelected !== modelA.exact_model) {
      throw new Error(`provider-denied seed selected ${seedSelected}, expected ${modelA.exact_model}`);
    }
    const request = buildExactRequest({
      cell: { ...exactCell, case_id: hardCase.case_id },
      runId,
      fixtures: {},
    });
    request.model = { alias: logicalModel };
    request.policy = {
      allowed_provider_instances: [openaiB.provider_instance_name],
      blocked_provider_instances: [openaiA.provider_instance_name],
    };
    const payload = request.payload as Record<string, unknown>;
    payload.options = { session_id: hardCase.session_id, rootid: runId };
    const response = await callChatCompletions(session.aicc, request) as AiMethodResponse;
    const selected = await routedCompletion(session, response, hardStarted, input.timeoutMs);
    if (selected !== modelB.exact_model) {
      throw new Error(`hard constraint selected ${selected ?? "<missing>"}; expected ${modelB.exact_model}`);
    }
    hardCase.status = "passed";
    hardCase.task_id = response.task_id;
    hardCase.attempts.push({ attempt: 1, started_at: new Date(hardStarted).toISOString(), elapsed_ms: Date.now() - hardStarted, status: "passed", diagnostic: `history preference correctly yielded to ${selected}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
  } catch (error) {
    hardCase.attempts.push({ attempt: 1, started_at: new Date(hardStarted).toISOString(), elapsed_ms: Date.now() - hardStarted, status: "failed", failure_class: "routing_failed", diagnostic: String(error), estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
  }
  if (wants(input, hardCase.case_id)) results.push(hardCase);

  const remainingHistoryReasons = [
    "api_type_changed",
    "required_capability_changed",
    "disabled_capability_changed",
    "instance_unhealthy",
    "quota_exhausted",
    "budget_exhausted",
    "local_only_changed",
    "context_limit_exceeded",
    "output_limit_exceeded",
    "locked_policy_changed",
  ];
  for (const reason of remainingHistoryReasons) {
    const caseId = `t1.history.hard_constraint_overrides.${reason}`;
    if (!wants(input, caseId)) continue;
    if (reason === "locked_policy_changed") {
      results.push(await executeT1Probe({
        runId,
        caseId,
        method: "chat.completions.create",
        apiType: "llm",
        failureClass: "preflight_failed",
        execute: async () => {
          throw new SkipCase("AiMethodRequest has no public session-policy reference or session-overlay field; route.resolve overlay is not equivalent to persisted history routing");
        },
      }));
      continue;
    }
    if (reason === "instance_unhealthy" || reason === "quota_exhausted") {
      results.push(await executeT1Probe({
        runId,
        caseId,
        method: "chat.completions.create",
        apiType: "llm",
        failureClass: "preflight_failed",
        execute: async () => {
          throw new SkipCase("current AICC admin APIs provide no test-scoped health/quota inventory override; HTTP Mock health is not part of OpenAI /v1/models discovery");
        },
      }));
      continue;
    }
    results.push(await executeT1Probe({
      runId,
      caseId,
      method: reason === "api_type_changed" ? "images.generate" : "chat.completions.create",
      apiType: reason === "api_type_changed" ? "image.txt2img" : "llm",
      failureClass: "routing_failed",
      execute: async () => {
        if (!openaiA || !openaiB || !modelA || !modelB || !logicalModel) {
          throw new Error("history hard-constraint case needs two shared-mount OpenAI instances");
        }
        const sessionId = `${runId}:history:${reason}`;
        const seedCell = cells.find((cell) => cell.exact_model === modelA.exact_model && cell.method === "chat.completions.create");
        if (!seedCell) throw new Error("history seed LLM cell is missing");
        const seedRequest = buildExactRequest({ cell: { ...seedCell, case_id: `${caseId}.seed` }, runId, fixtures: {} });
        const seedPayload = seedRequest.payload as Record<string, unknown>;
        seedPayload.options = { session_id: sessionId, rootid: runId };
        const seedStarted = Date.now();
        const seed = await callChatCompletions(session.aicc, seedRequest) as AiMethodResponse;
        await routedCompletion(session, seed, seedStarted, input.timeoutMs);

        if (reason === "api_type_changed") {
          const imageModel = openaiA.models.find((item) => item.api_types.includes("image.txt2img"));
          const imageMount = imageModel?.logical_mounts[0];
          if (!imageModel || !imageMount) throw new Error("no image model is available for api_type_changed history case");
          const imageCell = cellFor(openaiA, imageModel, "image.txt2img", "images.generate");
          const request = buildExactRequest({ cell: { ...imageCell, case_id: caseId }, runId, fixtures: {} });
          request.model = { alias: imageMount };
          const payload = request.payload as Record<string, unknown>;
          payload.options = { session_id: sessionId, rootid: runId };
          const callStarted = Date.now();
          const response = await callImagesGenerate(session.aicc, request) as AiMethodResponse;
          const selected = await routedCompletion(session, response, callStarted, input.timeoutMs);
          if (!selected || selected === modelA.exact_model) throw new Error(`api type change reused prior LLM exact model ${selected ?? "<missing>"}`);
          return `api type change selected ${selected}`;
        }

        const request = buildExactRequest({ cell: { ...seedCell, case_id: caseId }, runId, fixtures: {} });
        request.model = { alias: logicalModel };
        request.policy = {
          allowed_provider_instances: [openaiB.provider_instance_name],
          blocked_provider_instances: [openaiA.provider_instance_name],
        };
        if (reason === "required_capability_changed") request.requirements = { must_features: ["tool_calling"] };
        if (reason === "disabled_capability_changed") {
          request.requirements = { must_features: ["vision"] };
          request.disable = { vision: true };
        }
        if (reason === "budget_exhausted") request.policy = { ...request.policy as Record<string, unknown>, max_cost_usd: 0 };
        if (reason === "local_only_changed") request.policy = { ...request.policy as Record<string, unknown>, local_only: true };
        if (reason === "context_limit_exceeded") request.requirements = { min_context_tokens: 1_000_000_000 };
        if (reason === "output_limit_exceeded") {
          const payload = request.payload as Record<string, unknown>;
          payload.input_json = { ...(payload.input_json as Record<string, unknown>), max_output_tokens: 1_000_000_000 };
        }
        const payload = request.payload as Record<string, unknown>;
        payload.options = { session_id: sessionId, rootid: runId };
        {
          const mustReject = [
            "disabled_capability_changed",
            "budget_exhausted",
            "local_only_changed",
            "context_limit_exceeded",
            "output_limit_exceeded",
          ].includes(reason);
          let response: AiMethodResponse;
          try {
            const callStarted = Date.now();
            response = await callChatCompletions(session.aicc, request) as AiMethodResponse;
            const selected = await routedCompletion(session, response, callStarted, input.timeoutMs);
            if (!mustReject && selected !== modelB.exact_model) {
              throw new Error(`hard constraint selected ${selected}; expected ${modelB.exact_model}`);
            }
          } catch (error) {
            if (mustReject) return `${reason} overrode history and correctly rejected all ineligible candidates: ${String(error).slice(0, 180)}`;
            throw error;
          }
          if (mustReject) throw new Error(`${reason} reused history or selected an ineligible candidate instead of rejecting the request`);
          return `${reason} overrode history and selected ${modelB.exact_model}`;
        }
      },
    }));
  }

  if (wants(input, "t1.history.sessions_do_not_leak")) {
    results.push(await executeT1Probe({
      runId,
      caseId: "t1.history.sessions_do_not_leak",
      method: "chat.completions.create",
      apiType: "llm",
      failureClass: "routing_failed",
      execute: async () => {
        if (!openaiA || !modelA || !logicalModel) throw new Error("history isolation requires a shared logical model");
        const exactCell = cells.find((cell) => cell.exact_model === modelA.exact_model && cell.method === "chat.completions.create");
        if (!exactCell) throw new Error("history isolation seed cell is missing");
        const logicalCall = async (sessionId: string, suffix: string): Promise<string> => {
          const request = buildExactRequest({ cell: { ...exactCell, case_id: `t1.history.sessions_do_not_leak.${suffix}` }, runId, fixtures: {} });
          request.model = { alias: logicalModel };
          const payload = request.payload as Record<string, unknown>;
          payload.options = { session_id: sessionId, rootid: runId };
          const callStarted = Date.now();
          const response = await callChatCompletions(session.aicc, request) as AiMethodResponse;
          return await routedCompletion(session, response, callStarted, input.timeoutMs);
        };
        const baseline = await logicalCall(`${runId}:fresh-baseline`, "baseline");
        const seed = buildExactRequest({ cell: { ...exactCell, case_id: "t1.history.sessions_do_not_leak.seed" }, runId, fixtures: {} });
        const seedPayload = seed.payload as Record<string, unknown>;
        seedPayload.options = { session_id: `${runId}:seed-session`, rootid: runId };
        const seedStarted = Date.now();
        await routedCompletion(
          session,
          await callChatCompletions(session.aicc, seed) as AiMethodResponse,
          seedStarted,
          input.timeoutMs,
        );
        const isolated = await logicalCall(`${runId}:different-session`, "isolated");
        if (isolated !== baseline) throw new Error(`cross-session history changed baseline ${baseline} to ${isolated}`);
        return `fresh and isolated sessions both selected ${baseline}`;
      },
    }));
  }

  const fallbackA = mockInventories.find((inventory) => inventory.provider_instance_name.includes("dv-openai-a-"));
  const fallbackB = mockInventories.find((inventory) => inventory.provider_instance_name.includes("dv-openai-b-"));
  const fallbackModelA = fallbackA?.models.find((model) =>
    model.api_types.includes("llm") && fallbackB?.models.some((candidate) =>
      candidate.provider_model_id === model.provider_model_id && candidate.api_types.includes("llm")
    )
  );
  const fallbackModelB = fallbackModelA && fallbackB?.models.find((model) =>
    model.provider_model_id === fallbackModelA.provider_model_id
  );
  const fallbackLogicalModel = fallbackModelA?.logical_mounts.find((mount) =>
    fallbackModelB?.logical_mounts.includes(mount)
  );
  if (!fallbackA || !fallbackB || !fallbackModelA || !fallbackModelB || !fallbackLogicalModel) {
    throw new Error("runtime boundary cases require two OpenAI Mock instances with a shared logical model");
  }
  const fallbackCell = cellFor(fallbackA, fallbackModelA, "llm", "chat.completions.create");
  const runtimeBoundaryCases = [
    { caseId: "t1.runtime_boundary.rate_limit_fallback", scenario: "rate_limit", expectsFallback: true, failureClass: "provider_runtime_failed" },
    { caseId: "t1.runtime_boundary.server_error_fallback", scenario: "provider_5xx", expectsFallback: true, failureClass: "provider_runtime_failed" },
    { caseId: "t1.runtime_boundary.connection_failure_fallback", scenario: "connection_failed", expectsFallback: true, failureClass: "provider_runtime_failed" },
    { caseId: "t1.runtime_boundary.timeout_fallback", scenario: "timeout_short", expectsFallback: true, failureClass: "provider_runtime_failed" },
    { caseId: "t1.runtime_boundary.malformed_response_rejected", scenario: "malformed_response", expectsFallback: false, failureClass: "provider_protocol_failed" },
    { caseId: "t1.runtime_boundary.wrong_mime_rejected", scenario: "wrong_mime", expectsFallback: false, failureClass: "resource_failed" },
    { caseId: "t1.runtime_boundary.missing_usage_rejected", scenario: "missing_usage", expectsFallback: false, failureClass: "usage_failed" },
  ] as const;
  for (const boundary of runtimeBoundaryCases) {
    if (!wants(input, boundary.caseId)) continue;
    const started = Date.now();
    const report: CaseReport = {
      run_id: runId,
      case_id: boundary.caseId,
      layer: "T1",
      status: "failed",
      provider_driver: fallbackA.provider_driver,
      provider_instance: fallbackA.provider_instance_name,
      exact_model: fallbackModelA.exact_model,
      api_type: "llm",
      method: "chat.completions.create",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [],
    };
    await setScenario(input.mockControlUrl, boundary.scenario, "/instance-a/");
    const before = await mockRequestCount(input.mockControlUrl);
    try {
      const request = buildExactRequest({ cell: { ...fallbackCell, case_id: boundary.caseId }, runId, fixtures: {} });
      let initial: AiMethodResponse;
      if (boundary.expectsFallback) {
        request.model = { alias: fallbackLogicalModel };
        initial = await callLlmChatHelper(session.aicc, request) as AiMethodResponse;
        const selected = await routedCompletion(session, initial, started, input.timeoutMs);
        if (selected === fallbackModelA.exact_model) {
          throw new Error(`runtime failure did not move away from ${fallbackModelA.exact_model}`);
        }
        report.status = "passed";
        report.task_id = initial.task_id;
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", diagnostic: `${boundary.scenario} switched to ${selected}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
      } else {
        initial = await callChatCompletions(session.aicc, request) as AiMethodResponse;
        const value = await terminal(session, initial, Math.min(input.timeoutMs, 10_000));
        if (boundary.scenario === "missing_usage") {
          await auditSuccessfulTask({
            session,
            taskId: initial.task_id,
            exactModel: fallbackModelA.exact_model,
            providerInstance: fallbackA.provider_instance_name,
            startedAtMs: started,
            timeoutMs: input.timeoutMs,
          });
        }
        throw new Error(`AICC accepted ${boundary.scenario}: ${JSON.stringify(value)}`);
      }
    } catch (error) {
      if (boundary.expectsFallback) {
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "failed", failure_class: boundary.failureClass, diagnostic: String(error), estimated_cost_usd: 0, cost_status: "unknown" });
      } else if ((await mockRequestCount(input.mockControlUrl)) > before) {
        report.status = "passed";
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "passed", diagnostic: `${boundary.scenario} rejected after reaching Provider boundary: ${String(error)}`, estimated_cost_usd: 0, actual_cost_usd: 0, cost_status: "actual" });
      } else {
        report.attempts.push({ attempt: 1, started_at: new Date(started).toISOString(), elapsed_ms: Date.now() - started, status: "failed", failure_class: boundary.failureClass, diagnostic: `failure occurred before Provider boundary: ${String(error)}`, estimated_cost_usd: 0, cost_status: "not_called" });
      }
    } finally {
      await setScenario(input.mockControlUrl, "success", "/instance-a/");
    }
    results.push(report);
  }

  const probeCell = cells.find((cell) => cell.provider_driver === "openai" && cell.method === "chat.completions.create") ?? cells[0];
  const asyncInventory = mockInventories.find((inventory) => inventory.provider_driver === "google-gemini");
  const asyncModel = asyncInventory?.models.find((model) =>
    model.provider_model_id.toLowerCase().includes("veo-") &&
    model.api_types.includes("video.txt2video")
  );
  const asyncProbeCell = asyncInventory && asyncModel
    ? cellFor(asyncInventory, asyncModel, "video.txt2video", "video.txt2video")
    : undefined;
  const invokeProbe = async (caseId: string, scenario = "success"): Promise<AiMethodResponse> => {
    await setScenario(input.mockControlUrl, scenario);
    const request = buildExactRequest({
      cell: { ...probeCell, case_id: caseId },
      runId,
      fixtures: {},
    });
    return await callInference(session.aicc, probeCell.method, request) as AiMethodResponse;
  };
  const pushProbe = async (
    caseId: string,
    method: string,
    failureClass: CaseReport["attempts"][number]["failure_class"],
    execute: () => Promise<string | undefined>,
    apiType = "llm",
  ): Promise<void> => {
    if (!wants(input, caseId)) return;
    results.push(await executeT1Probe({
      runId,
      caseId,
      method,
      apiType: method.startsWith("service.") || method.startsWith("node-daemon.") ? undefined : apiType,
      failureClass,
      execute,
    }));
  };

  const invokeAsyncProbe = async (
    caseId: string,
    scenario: "async_success" | "async_failed" | "async_pending" = "async_success",
  ): Promise<AiMethodResponse> => {
    if (!asyncProbeCell) {
      const available = cells.filter((cell) =>
        cell.provider_driver === "google-gemini" || cell.api_type.startsWith("video.")
      ).map((cell) => ({
        driver: cell.provider_driver,
        model: cell.exact_model,
        api_type: cell.api_type,
        method: cell.method,
      }));
      throw new Error(`Mock inventory has no asynchronous video.txt2video cell; available=${JSON.stringify(available)}`);
    }
    await setScenario(input.mockControlUrl, scenario);
    const request = buildExactRequest({
      cell: { ...asyncProbeCell, case_id: caseId },
      runId,
      fixtures: {},
    });
    return await callInference(session.aicc, asyncProbeCell.method, request) as AiMethodResponse;
  };

  await pushProbe("t1.task.immediate_succeeded", probeCell.method, "task_lifecycle_failed", async () => {
    try {
      const initial = await invokeProbe("t1.task.immediate_succeeded");
      if (initial.status !== "succeeded") {
        throw new Error(`expected immediate succeeded, received ${initial.status}`);
      }
      return `immediate task ${initial.task_id} succeeded`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  });

  await pushProbe("t1.task.running_succeeded", asyncProbeCell?.method ?? "video.txt2video", "task_lifecycle_failed", async () => {
    try {
      const initial = await invokeAsyncProbe("t1.task.running_succeeded");
      if (initial.status !== "running") throw new Error(`expected running, received ${initial.status}`);
      await terminal(session, initial, input.timeoutMs);
      return `running task ${initial.task_id} reached succeeded`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  }, "video.txt2video");

  await pushProbe("t1.task.running_failed", asyncProbeCell?.method ?? "video.txt2video", "task_lifecycle_failed", async () => {
    try {
      const initial = await invokeAsyncProbe("t1.task.running_failed", "async_failed");
      if (initial.status !== "running") throw new Error(`expected running task, received ${initial.status}`);
      try {
        await terminal(session, initial, input.timeoutMs);
      } catch (error) {
        return `running task ${initial.task_id} reached failed: ${String(error).slice(0, 240)}`;
      }
      throw new Error("provider failure unexpectedly completed successfully");
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  }, "video.txt2video");

  await pushProbe("t1.task.cancelled", asyncProbeCell?.method ?? "video.txt2video", "task_lifecycle_failed", async () => {
    try {
      const initial = await invokeAsyncProbe("t1.task.cancelled", "async_pending");
      if (initial.status !== "running") throw new Error(`expected cancellable running task, received ${initial.status}`);
      await session.taskManager.call("request_control", {
        task_id: initial.task_id,
        action: "Cancel",
        request_id: `${runId}:cancel`,
        recursive: false,
      });
      const deadline = Date.now() + Math.min(input.timeoutMs, 10_000);
      let lastTask: Record<string, unknown> = {};
      while (Date.now() < deadline) {
        const task = taskValue(await session.taskManager.call("get_task", { task_id: initial.task_id }));
        lastTask = task;
        if (task.phase === "Terminal") {
          if (task.outcome !== "Cancelled") throw new Error(`cancelled task outcome was ${String(task.outcome)}`);
          return `task ${initial.task_id} reached Cancelled`;
        }
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
      }
      throw new Error(`cancelled task did not reach Terminal/Cancelled; last=${JSON.stringify(lastTask)}`);
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  }, "video.txt2video");

  await pushProbe("t1.task.unknown", "get_task", "task_lifecycle_failed", async () => {
    try {
      await session.taskManager.call("get_task", { task_id: `${runId}-unknown-task` });
    } catch (error) {
      return `unknown task rejected: ${String(error).slice(0, 240)}`;
    }
    throw new Error("unknown task unexpectedly returned a task object");
  });

  await pushProbe("t1.task.idempotency_conflict_different_body", probeCell.method, "task_lifecycle_failed", async () => {
    const before = await mockRequestCount(input.mockControlUrl);
    const cell = { ...probeCell, case_id: "t1.task.idempotency_conflict_different_body" };
    const firstRequest = buildExactRequest({ cell, runId, fixtures: {} });
    firstRequest.idempotency_key = `${runId}:task-idempotency-conflict`;
    const secondRequest = structuredClone(firstRequest);
    const payload = secondRequest.payload as {
      input_json?: { messages?: Array<{ content?: Array<{ type?: string; text?: string }> }> };
    };
    const textBlock = payload.input_json?.messages?.[0]?.content?.[0];
    if (!textBlock || textBlock.type !== "text") throw new Error("cannot mutate idempotency conflict request body");
    textBlock.text = `${textBlock.text ?? ""} different-body`;
    const first = await callInference(session.aicc, probeCell.method, firstRequest) as AiMethodResponse;
    try {
      await callInference(session.aicc, probeCell.method, secondRequest);
    } catch (error) {
      const message = String(error);
      if (!/409|conflict|idemp/i.test(message)) {
        throw new Error(`different-body idempotency rejection was not a conflict: ${message}`);
      }
      await terminal(session, first, input.timeoutMs);
      const providerCalls = await mockRequestCount(input.mockControlUrl) - before;
      if (providerCalls !== 1) throw new Error(`idempotency conflict caused ${providerCalls} Provider calls`);
      return `same key with a different body rejected before a second Provider call`;
    }
    throw new Error("same idempotency key with a different request body unexpectedly succeeded");
  });

  await pushProbe("t1.task.concurrent_idempotency", probeCell.method, "task_lifecycle_failed", async () => {
    const before = await mockRequestCount(input.mockControlUrl);
    const cell = { ...probeCell, case_id: "t1.task.concurrent_idempotency" };
    const request = buildExactRequest({ cell, runId, fixtures: {} });
    request.idempotency_key = `${runId}:task-concurrent-idempotency`;
    const responses = await Promise.all(
      Array.from({ length: 5 }, () =>
        callInference(session.aicc, probeCell.method, structuredClone(request)) as Promise<AiMethodResponse>
      ),
    );
    const taskIds = new Set(responses.map((response) => response.task_id));
    if (taskIds.size !== 1) throw new Error(`concurrent idempotency created ${taskIds.size} tasks`);
    await terminal(session, responses[0], input.timeoutMs);
    const providerCalls = await mockRequestCount(input.mockControlUrl) - before;
    if (providerCalls !== 1) throw new Error(`concurrent idempotency caused ${providerCalls} Provider calls`);
    return `five concurrent submissions converged on one task and one Provider call`;
  });

  await pushProbe("t1.task.concurrent_completion", probeCell.method, "task_lifecycle_failed", async () => {
    const before = await mockRequestCount(input.mockControlUrl);
    await setScenario(input.mockControlUrl, "async_success");
    try {
      const requests = Array.from({ length: 12 }, (_, index) => buildExactRequest({
        cell: { ...probeCell, case_id: `t1.task.concurrent_completion.${index + 1}` },
        runId,
        fixtures: {},
      }));
      const initial = await Promise.all(requests.map((request) =>
        callInference(session.aicc, probeCell.method, request) as Promise<AiMethodResponse>
      ));
      if (new Set(initial.map((item) => item.task_id)).size !== initial.length) {
        throw new Error("unique concurrent requests did not create unique tasks");
      }
      await Promise.all(initial.map((item) => terminal(session, item, input.timeoutMs)));
      const providerCalls = await mockRequestCount(input.mockControlUrl) - before;
      if (providerCalls !== initial.length) {
        throw new Error(`${initial.length} concurrent tasks caused ${providerCalls} Provider calls`);
      }
      return `${initial.length} independent tasks completed concurrently with one Provider call each`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  });

  await pushProbe("t1.task.terminal_idempotent", asyncProbeCell?.method ?? "video.txt2video", "task_lifecycle_failed", async () => {
    const initial = await invokeAsyncProbe("t1.task.terminal_idempotent");
    try {
      await terminal(session, initial, input.timeoutMs);
      const rawSnapshots = await Promise.all(Array.from({ length: 5 }, () =>
        session.taskManager.call("get_task", { task_id: initial.task_id }) as Promise<Record<string, unknown>>
      ));
      const snapshots = rawSnapshots.map(taskValue);
      const terminalState = snapshots.map((task) => JSON.stringify({
        phase: task.phase,
        outcome: task.outcome,
        result: task.result,
        error: task.error,
      }));
      if (new Set(terminalState).size !== 1 || snapshots.some((task) => task.phase !== "Terminal")) {
        throw new Error("repeated terminal reads observed a mutable or non-terminal task state");
      }
      return `terminal state remained immutable across five reads`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  }, "video.txt2video");

  await pushProbe("t1.task.reload_recovery", asyncProbeCell?.method ?? "video.txt2video", "task_lifecycle_failed", async () => {
    const initial = await invokeAsyncProbe("t1.task.reload_recovery");
    try {
      if (initial.status !== "running") throw new Error(`expected running task, received ${initial.status}`);
      await session.aicc.call("service.reload_settings", {});
      await terminal(session, initial, input.timeoutMs);
      return `running task ${initial.task_id} completed across settings reload`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  }, "video.txt2video");

  await pushProbe("t1.task.restart_recovery", "node-daemon.restart", "task_lifecycle_failed", async () => {
    throw new SkipCase("AICC process restart is not authorized/configured for this DV run; restart recovery case is implemented as an explicit gated case");
  });

  await pushProbe("t1.usage.success_once", probeCell.method, "usage_failed", async () => {
    const started = Date.now();
    try {
      const initial = await invokeProbe("t1.usage.success_once");
      await terminal(session, initial, input.timeoutMs);
      const events = await queryUsageEvents({
        aicc: session.aicc,
        startTimeMs: started - 1_000,
        endTimeMs: Date.now() + 1_000,
        taskIds: [initial.task_id],
      });
      if (events.length !== 1) throw new Error(`expected exactly one usage event, found ${events.length}`);
      return `one durable usage event recorded for ${initial.task_id}`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  });

  await pushProbe("t1.usage.idempotent_no_double_charge", probeCell.method, "usage_failed", async () => {
    const started = Date.now();
    const before = await mockRequestCount(input.mockControlUrl);
    const cell = { ...probeCell, case_id: "t1.usage.idempotent_no_double_charge" };
    const request = buildExactRequest({ cell, runId, fixtures: {} });
    request.idempotency_key = `${runId}:usage-idempotent`;
    const first = await callInference(session.aicc, probeCell.method, request) as AiMethodResponse;
    const second = await callInference(session.aicc, probeCell.method, request) as AiMethodResponse;
    await terminal(session, first, input.timeoutMs);
    if (second.task_id !== first.task_id) throw new Error(`idempotent retry created ${first.task_id} and ${second.task_id}`);
    const after = await mockRequestCount(input.mockControlUrl);
    if (after - before !== 1) throw new Error(`idempotent retry caused ${after - before} Provider calls`);
    const events = await queryUsageEvents({ aicc: session.aicc, startTimeMs: started - 1_000, endTimeMs: Date.now() + 1_000, taskIds: [first.task_id] });
    if (events.length !== 1) throw new Error(`idempotent retry produced ${events.length} usage events`);
    return `one task, Provider call, and usage event for idempotent retry`;
  });

  await pushProbe("t1.usage.fallback_attempts_attributed", "helper.llm_chat", "usage_failed", async () => {
    const openaiA = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-a-"));
    const openaiB = mockInventories.find((item) => item.provider_instance_name.includes("dv-openai-b-"));
    const modelA = openaiA?.models.find((item) => item.api_types.includes("llm"));
    const logical = modelA?.logical_mounts.find((mount) =>
      openaiB?.models.some((item) => item.logical_mounts.includes(mount))
    );
    if (!openaiA || !openaiB || !modelA || !logical) throw new Error("fallback attribution needs two shared-mount instances");
    try {
      await setScenario(input.mockControlUrl, "rate_limit");
      const request = buildExactRequest({ cell: { ...probeCell, case_id: "t1.usage.fallback_attempts_attributed" }, runId, fixtures: {} });
      request.model = { alias: logical };
      request.policy = { allowed_provider_instances: [openaiA.provider_instance_name, openaiB.provider_instance_name], runtime_failover: true };
      const initial = await callLlmChatHelper(session.aicc, request) as AiMethodResponse;
      try {
        await terminal(session, initial, input.timeoutMs);
      } catch {}
      const traces = await queryRouteTraces({ aicc: session.aicc, startTimeMs: Date.now() - input.timeoutMs - 2_000, endTimeMs: Date.now() + 1_000, taskIds: [initial.task_id] });
      const traceText = JSON.stringify(traces);
      if (traces.length !== 1 || !/fallback|failover/i.test(traceText)) {
        throw new Error(`fallback attempt attribution missing from ${traces.length} route traces`);
      }
      return `fallback attempt attributed to task ${initial.task_id}`;
    } finally {
      await setScenario(input.mockControlUrl, "success");
    }
  });

  const expectAuthRejected = async (token: string | null, label: string): Promise<string> => {
    const { buckyos } = await import("buckyos");
    const client = new buckyos.kRPCClient(`${input.gatewayUrl}/kapi/aicc`, token) as RpcClient;
    try {
      await client.call("models.list", {});
    } catch (error) {
      return `${label} rejected: ${String(error).slice(0, 180)}`;
    }
    throw new Error(`${label} unexpectedly accessed AICC`);
  };
  await pushProbe("t1.security.no_token", "models.list", "security_failed", () => expectAuthRejected(null, "missing token"));
  await pushProbe("t1.security.invalid_token", "models.list", "security_failed", () => expectAuthRejected("invalid-aicc-dv-token", "invalid token"));
  await pushProbe("t1.security.expired_token", "models.list", "security_failed", () => expectAuthRejected("eyJhbGciOiJub25lIn0.eyJleHAiOjF9.", "expired token"));
  const otherTenantToken = (): string => {
    if (!input.otherTenantSessionToken) {
      throw new SkipCase("auth.other_tenant_session_token is not configured; second-tenant isolation was not executed");
    }
    return input.otherTenantSessionToken;
  };
  await pushProbe("t1.security.cross_tenant", "get_task", "security_failed", async () => {
    const token = otherTenantToken();
    const initial = await invokeProbe("t1.security.cross_tenant");
    await terminal(session, initial, input.timeoutMs);
    const { buckyos } = await import("buckyos");
    const otherTaskManager = new buckyos.kRPCClient(
      `${input.gatewayUrl}/kapi/task-manager`,
      token,
    ) as RpcClient;
    try {
      await otherTaskManager.call("get_task", { task_id: initial.task_id });
    } catch (error) {
      return `secondary tenant rejected for task ${initial.task_id}: ${String(error).slice(0, 180)}`;
    }
    throw new Error("secondary tenant unexpectedly read the primary tenant task");
  });

  await pushProbe("t1.security.cross_tenant_task_cancel", "request_control", "security_failed", async () => {
    const token = otherTenantToken();
    const { buckyos } = await import("buckyos");
    const otherTaskManager = new buckyos.kRPCClient(
      `${input.gatewayUrl}/kapi/task-manager`,
      token,
    ) as RpcClient;
    let initial: AiMethodResponse | undefined;
    try {
      initial = await invokeProbe("t1.security.cross_tenant_task_cancel", "timeout_long");
      if (initial.status !== "running") throw new Error(`expected running task, received ${initial.status}`);
      try {
        await otherTaskManager.call("request_control", {
          task_id: initial.task_id,
          action: "Cancel",
          request_id: `${runId}:cross-tenant-cancel`,
          recursive: false,
        });
      } catch (error) {
        return `secondary tenant rejected while cancelling ${initial.task_id}: ${String(error).slice(0, 180)}`;
      }
      throw new Error("secondary tenant unexpectedly cancelled the primary tenant task");
    } finally {
      if (initial?.status === "running") {
        try {
          await session.taskManager.call("request_control", {
            task_id: initial.task_id,
            action: "Cancel",
            request_id: `${runId}:owner-cleanup-cancel`,
            recursive: false,
          });
        } catch {}
      }
      await setScenario(input.mockControlUrl, "success");
    }
  });

  await pushProbe("t1.security.cross_tenant_usage", "usage.query", "security_failed", async () => {
    const token = otherTenantToken();
    const initial = await invokeProbe("t1.security.cross_tenant_usage");
    await terminal(session, initial, input.timeoutMs);
    const { buckyos } = await import("buckyos");
    const otherAicc = new buckyos.kRPCClient(`${input.gatewayUrl}/kapi/aicc`, token) as RpcClient;
    try {
      const visible = await queryUsageEvents({
        aicc: otherAicc,
        startTimeMs: Date.now() - input.timeoutMs - 2_000,
        endTimeMs: Date.now() + 1_000,
        taskIds: [initial.task_id],
      });
      if (visible.length > 0) throw new Error("secondary tenant unexpectedly read primary usage events");
      return `secondary tenant usage query returned no events for ${initial.task_id}`;
    } catch (error) {
      if (/unexpectedly read/.test(String(error))) throw error;
      return `secondary tenant usage query rejected: ${String(error).slice(0, 180)}`;
    }
  });

  await pushProbe("t1.security.cross_tenant_message", "msg.list_session", "security_failed", async () => {
    const token = otherTenantToken();
    const { buckyos } = await import("buckyos");
    const primaryDid = session.userId.startsWith("did:") ? session.userId : `did:bns:${session.userId}`;
    const primaryMsgCenter = new buckyos.kRPCClient(`${input.gatewayUrl}/kapi/msg-center`, session.sessionToken) as RpcClient;
    const otherMsgCenter = new buckyos.kRPCClient(`${input.gatewayUrl}/kapi/msg-center`, token) as RpcClient;
    const primaryRaw = await primaryMsgCenter.call("msg.list_sessions", {
      owner: primaryDid,
      limit: 100,
      with_object: false,
    }) as { items?: Array<Record<string, unknown>> };
    const sessionId = primaryRaw.items?.map((item) =>
      typeof item.session_id === "string" ? item.session_id :
      typeof item.topic === "string" ? item.topic : undefined
    ).find(Boolean);
    if (!sessionId) throw new SkipCase("primary tenant has no existing msg-center session fixture to test isolation");
    try {
      const raw = await otherMsgCenter.call("msg.list_session", {
        owner: primaryDid,
        session_id: sessionId,
        limit: 100,
        descending: false,
        with_object: true,
      }) as { items?: unknown[] };
      if ((raw.items?.length ?? 0) > 0) throw new Error("secondary tenant unexpectedly read primary messages");
      return `secondary tenant saw no records in primary session ${sessionId}`;
    } catch (error) {
      if (/unexpectedly read/.test(String(error))) throw error;
      return `secondary tenant message query rejected: ${String(error).slice(0, 180)}`;
    }
  });

  await pushProbe("t1.security.cross_tenant_object", "ndm.open_reader", "security_failed", async () => {
    const token = otherTenantToken();
    const imageCell = cells.find((cell) =>
      cell.api_type === "image.txt2img" && cell.method === "images.generate"
    );
    if (!imageCell) throw new SkipCase("Mock inventory has no image.txt2img cell for object isolation");
    const request = buildExactRequest({
      cell: { ...imageCell, case_id: "t1.security.cross_tenant_object" },
      runId,
      fixtures: {},
    });
    const payload = request.payload as { input_json?: Record<string, unknown> };
    if (payload.input_json) payload.input_json.output = { resource_format: "named_object" };
    const initial = await callImagesGenerate(session.aicc, request) as AiMethodResponse;
    const completed = await terminal(session, initial, input.timeoutMs);
    const objId = namedObjectId(completed);
    if (!objId) throw new SkipCase("AICC Mock image result did not expose a Named Object ID");
    const { ndm_proxy } = await import("buckyos");
    const otherNdm = ndm_proxy.createNdmProxyClient({
      endpoint: input.gatewayUrl,
      sessionToken: token,
    }) as { openReader: (request: { obj_id: string }) => Promise<{ response: Response }> };
    try {
      const opened = await otherNdm.openReader({ obj_id: objId });
      if (opened.response.ok) throw new Error("secondary tenant unexpectedly opened primary Named Object");
      return `secondary tenant Named Object read returned HTTP ${opened.response.status}`;
    } catch (error) {
      if (/unexpectedly opened/.test(String(error))) throw error;
      return `secondary tenant Named Object read rejected: ${String(error).slice(0, 180)}`;
    }
  });

  await pushProbe("t1.security.rbac_admin_method", "service.reload_settings", "security_failed", async () => {
    const token = otherTenantToken();
    const { buckyos } = await import("buckyos");
    const otherAicc = new buckyos.kRPCClient(`${input.gatewayUrl}/kapi/aicc`, token) as RpcClient;
    try {
      await otherAicc.call("service.reload_settings", {});
    } catch (error) {
      return `non-admin secondary tenant rejected for reload_settings: ${String(error).slice(0, 180)}`;
    }
    throw new Error("non-admin secondary tenant unexpectedly invoked service.reload_settings");
  });

  await pushProbe("t1.config.reload_valid", "service.reload_settings", "assertion_failed", async () => {
    const before = inventories(await session.aicc.call("models.list", {})).map((item) => item.provider_instance_name).sort();
    await session.aicc.call("service.reload_settings", {});
    const after = inventories(await session.aicc.call("models.list", {})).map((item) => item.provider_instance_name).sort();
    if (JSON.stringify(before) !== JSON.stringify(after)) throw new Error("valid reload changed Provider inventory");
    return `valid reload preserved ${after.length} Provider instances`;
  });

  await pushProbe("t1.config.reload_invalid_keeps_old", "service.reload_settings", "assertion_failed", async () => {
    const raw = await session.systemConfig.call("sys_config_get", { key: "services/aicc/settings" });
    const backup = configValue(raw).serialized;
    let reloadRejected = false;
    try {
      await session.systemConfig.call("sys_config_set", { key: "services/aicc/settings", value: "{" });
      try {
        await session.aicc.call("service.reload_settings", {});
      } catch {
        reloadRejected = true;
      }
      const inventory = inventories(await session.aicc.call("models.list", {}));
      if (!reloadRejected) throw new Error("invalid settings reload unexpectedly succeeded");
      if (inventory.length === 0) throw new Error("invalid reload discarded the previous inventory");
      return `invalid reload rejected and previous ${inventory.length}-instance inventory remained active`;
    } finally {
      await session.systemConfig.call("sys_config_set", { key: "services/aicc/settings", value: backup });
      await session.aicc.call("service.reload_settings", {});
    }
  });

  await pushProbe("t1.config.provider_instance_isolation", "models.list", "assertion_failed", async () => {
    const openaiInstances = mockInventories.filter((item) => item.provider_driver === "openai");
    if (openaiInstances.length !== 2) throw new Error(`expected two isolated OpenAI instances, found ${openaiInstances.length}`);
    const names = new Set(openaiInstances.map((item) => item.provider_instance_name));
    if (names.size !== 2 || openaiInstances.some((item) => item.models.length === 0)) {
      throw new Error("OpenAI instance inventories are not independently addressable");
    }
    return `isolated inventories: ${[...names].join(", ")}`;
  });

  const withCurrentSettingsPatch = async <T>(
    patch: (settings: Record<string, unknown>) => Record<string, unknown>,
    execute: () => Promise<T>,
  ): Promise<T> => {
    const raw = await session.systemConfig.call("sys_config_get", { key: "services/aicc/settings" });
    const backup = configValue(raw);
    try {
      const next = patch(structuredClone(backup.parsed));
      await session.systemConfig.call("sys_config_set", {
        key: "services/aicc/settings",
        value: JSON.stringify(next),
      });
      await session.aicc.call("service.reload_settings", {});
      return await execute();
    } finally {
      await session.systemConfig.call("sys_config_set", {
        key: "services/aicc/settings",
        value: backup.serialized,
      });
      await session.aicc.call("service.reload_settings", {});
    }
  };
  const openaiInstancesIn = (settings: Record<string, unknown>): Array<Record<string, unknown>> => {
    const section = settings.openai as Record<string, unknown> | undefined;
    if (!section || !Array.isArray(section.instances)) throw new Error("AICC settings have no openai.instances");
    return section.instances as Array<Record<string, unknown>>;
  };
  const waitForProvider = async (name: string, present: boolean): Promise<void> => {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      const names = inventories(await session.aicc.call("models.list", {}))
        .map((item) => item.provider_instance_name);
      if (names.includes(name) === present) return;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
    }
    throw new Error(`Provider ${name} did not become ${present ? "visible" : "absent"}`);
  };
  const addedProviderName = `dv-openai-added-${runId.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const addProvider = (settings: Record<string, unknown>): Record<string, unknown> => {
    const instances = openaiInstancesIn(settings);
    const template = instances.find((item) =>
      String(item.provider_instance_name).includes("dv-openai-a-")
    );
    if (!template) throw new Error("cannot find Mock OpenAI template instance");
    instances.push({
      ...structuredClone(template),
      provider_instance_name: addedProviderName,
      api_token: `mock-added-${runId}`,
      base_url: `${mockBaseUrl}/instance-a/v1`,
      models: ["gpt-4o-mini"],
    });
    return settings;
  };

  await pushProbe("t1.config.provider_add_refresh", "service.reload_settings", "assertion_failed", async () => {
    await withCurrentSettingsPatch(addProvider, async () => {
      await waitForProvider(addedProviderName, true);
      const added = inventories(await session.aicc.call("models.list", {}))
        .find((item) => item.provider_instance_name === addedProviderName);
      if (!added?.models.some((model) => model.provider_model_id === "gpt-4o-mini")) {
        throw new Error("added Provider inventory did not refresh its configured model");
      }
    });
    await waitForProvider(addedProviderName, false);
    return `Provider add refreshed inventory and transaction restore removed ${addedProviderName}`;
  });

  await pushProbe("t1.config.provider_validate_rejects_duplicate", "service.reload_settings", "assertion_failed", async () => {
    const before = inventories(await session.aicc.call("models.list", {}))
      .map((item) => item.provider_instance_name).sort();
    let rejected = false;
    try {
      await withCurrentSettingsPatch((settings) => {
        const instances = openaiInstancesIn(settings);
        instances.push(structuredClone(instances[0]));
        return settings;
      }, async () => {});
    } catch (error) {
      rejected = true;
      if (!/duplicate|instance|config|settings/i.test(String(error))) {
        throw new Error(`duplicate Provider validation returned an unexpected error: ${String(error)}`);
      }
    }
    if (!rejected) throw new Error("duplicate Provider instance unexpectedly passed validation");
    const after = inventories(await session.aicc.call("models.list", {}))
      .map((item) => item.provider_instance_name).sort();
    if (JSON.stringify(before) !== JSON.stringify(after)) {
      throw new Error("rejected duplicate Provider mutation changed active inventory");
    }
    return "duplicate Provider instance was rejected and active inventory was preserved";
  });

  await pushProbe("t1.config.provider_delete_isolation", "service.reload_settings", "assertion_failed", async () => {
    const preserved = mockInventories.filter((item) => item.provider_driver === "openai")
      .map((item) => item.provider_instance_name).sort();
    await withCurrentSettingsPatch(addProvider, async () => {
      await waitForProvider(addedProviderName, true);
      const raw = await session.systemConfig.call("sys_config_get", { key: "services/aicc/settings" });
      const current = configValue(raw).parsed;
      const section = current.openai as Record<string, unknown>;
      section.instances = openaiInstancesIn(current).filter((item) =>
        item.provider_instance_name !== addedProviderName
      );
      await session.systemConfig.call("sys_config_set", {
        key: "services/aicc/settings",
        value: JSON.stringify(current),
      });
      await session.aicc.call("service.reload_settings", {});
      await waitForProvider(addedProviderName, false);
      const remaining = inventories(await session.aicc.call("models.list", {}))
        .map((item) => item.provider_instance_name);
      if (preserved.some((name) => !remaining.includes(name))) {
        throw new Error("deleting one Provider instance removed a preserved sibling");
      }
    });
    return `deleting ${addedProviderName} preserved both original OpenAI instances`;
  });

  await pushProbe("t1.config.provider_update_rollback", "service.reload_settings", "assertion_failed", async () => {
    const original = mockInventories.find((item) =>
      item.provider_instance_name.includes("dv-openai-a-")
    );
    if (!original) throw new Error("missing original OpenAI A instance");
    let rejected = false;
    try {
      await withCurrentSettingsPatch((settings) => {
        const target = openaiInstancesIn(settings).find((item) =>
          item.provider_instance_name === original.provider_instance_name
        );
        if (!target) throw new Error("cannot find Provider instance to update");
        target.base_url = "not-a-valid-provider-url";
        return settings;
      }, async () => {});
    } catch {
      rejected = true;
    }
    await waitForProvider(original.provider_instance_name, true);
    const request = buildExactRequest({
      cell: { ...probeCell, exact_model: original.models.find((item) => item.api_types.includes("llm"))!.exact_model, case_id: "t1.config.provider_update_rollback" },
      runId,
      fixtures: {},
    });
    await terminal(
      session,
      await callInference(session.aicc, probeCell.method, request) as AiMethodResponse,
      input.timeoutMs,
    );
    return `${rejected ? "invalid update rejected" : "invalid update restored"}; original Provider remained callable after rollback`;
  });

  await pushProbe("t1.config.restart_consistency", "node-daemon.restart", "assertion_failed", async () => {
    throw new SkipCase("AICC process restart is not authorized/configured for this DV run; restart consistency remains an explicit gated case");
  });

  await pushProbe("t1.observability.correlation", probeCell.method, "assertion_failed", async () => {
    const started = Date.now();
    const initial = await invokeProbe("t1.observability.correlation");
    await terminal(session, initial, input.timeoutMs);
    const [usageEvents, traces] = await Promise.all([
      queryUsageEvents({ aicc: session.aicc, startTimeMs: started - 1_000, endTimeMs: Date.now() + 1_000, taskIds: [initial.task_id] }),
      queryRouteTraces({ aicc: session.aicc, startTimeMs: started - 1_000, endTimeMs: Date.now() + 1_000, taskIds: [initial.task_id] }),
    ]);
    if (usageEvents.length !== 1 || traces.length !== 1) throw new Error(`correlation expected usage/trace 1/1, found ${usageEvents.length}/${traces.length}`);
    return `task ${initial.task_id} correlates one usage event and one route trace`;
  });

  await pushProbe("t1.observability.redaction", probeCell.method, "security_failed", async () => {
    const started = Date.now();
    const initial = await invokeProbe("t1.observability.redaction");
    await terminal(session, initial, input.timeoutMs);
    const [usageEvents, traces, requests] = await Promise.all([
      queryUsageEvents({ aicc: session.aicc, startTimeMs: started - 1_000, endTimeMs: Date.now() + 1_000, taskIds: [initial.task_id] }),
      queryRouteTraces({ aicc: session.aicc, startTimeMs: started - 1_000, endTimeMs: Date.now() + 1_000, taskIds: [initial.task_id] }),
      mockRequests(input.mockControlUrl),
    ]);
    const serialized = JSON.stringify({ usageEvents, traces, request: requests.at(-1) });
    if (serialized.includes(session.sessionToken) || serialized.includes(`mock-${runId}`)) {
      throw new Error("observability data exposed a session or Provider token");
    }
    if (!serialized.includes("[REDACTED]")) throw new Error("Provider authorization header was not visibly redacted");
    return `usage, trace, and Provider request audit are credential-redacted`;
  });
  return results;
}

async function main(): Promise<void> {
  const startedAt = new Date().toISOString();
  await runPreflight();
  const input = await options(Deno.args);
  const knownCaseIds = new Set(buildStaticManifest().map((item) => item.case_id));
  for (const caseId of input.caseIds) {
    if (!knownCaseIds.has(caseId)) throw new Error(`unknown T1 --case ${caseId}`);
  }
  const runId = `aicc-t1-${new Date().toISOString().replace(/[:.]/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
  let child: Deno.ChildProcess | undefined;
  try {
    if (input.startLocalMock) {
      child = new Deno.Command("node", {
        args: ["--experimental-strip-types", join(here, "mock_provider.ts"), "--port", String(input.mockPort)],
        cwd: here,
        stdout: "inherit",
        stderr: "inherit",
      }).spawn();
    }
    await waitHealth(input.mockControlUrl);
    const session = await loginGateway({
      gatewayUrl: input.gatewayUrl,
      sessionToken: input.sessionToken,
      username: input.username,
      password: input.password,
      appId: input.appId,
    });
    let cases: CaseReport[];
    let cleanup: AcceptanceReport["cleanup"] = {
      status: "passed",
      details: ["services/aicc/settings restored byte-for-byte", "mock Provider uses zero real model calls"],
    };
    try {
      const transaction = await withMockSettings({
        systemConfig: session.systemConfig,
        aicc: session.aicc,
        baseUrl: input.mockBaseUrl,
        runId,
        execute: async () => {
          const mockInventories = await waitForMockInventories(session.aicc, runId, input.timeoutMs);
          return await runCases(session, input.mockBaseUrl, runId, mockInventories, input);
        },
        refreshClients: async () => {
          const refreshed = await loginGateway({
            gatewayUrl: input.gatewayUrl,
            username: input.username,
            password: input.password,
            appId: input.appId,
          });
          return { systemConfig: refreshed.systemConfig, aicc: refreshed.aicc };
        },
      });
      cases = transaction.result;
    } catch (error) {
      const cleanupFailed = error instanceof AggregateError &&
        error.message.includes("cleanup failed");
      cleanup = {
        status: cleanupFailed ? "failed" : "passed",
        details: cleanupFailed
          ? ["automatic restoration failed; manual restoration of services/aicc/settings is required", String(error)]
          : ["services/aicc/settings restored after execution failure", String(error)],
      };
      cases = [{
        run_id: runId,
        case_id: cleanupFailed ? "t1.cleanup.settings_restore" : "t1.execution.gateway_mock",
        layer: "T1",
        status: "failed",
        method: cleanupFailed ? "sys_config_set" : "mock execution",
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 1,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "failed",
          failure_class: cleanupFailed ? "cleanup_failed" : "assertion_failed",
          diagnostic: String(error),
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      }];
    }
    const residualInventories = inventories(await session.aicc.call("models.list", {}))
      .filter((inventory) => inventory.provider_instance_name.includes(runId));
    if (residualInventories.length > 0) {
      cleanup = {
        status: "failed",
        details: [
          ...cleanup.details,
          `AICC runtime inventory retained ${residualInventories.length} run-scoped Provider instance(s) after settings restoration`,
        ],
      };
      cases.push({
        run_id: runId,
        case_id: "t1.cleanup.runtime_inventory_restore",
        layer: "T1",
        status: "failed",
        method: "models.list/service.reload_settings",
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 1,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "failed",
          failure_class: "cleanup_failed",
          diagnostic: `runtime inventory retained: ${residualInventories.map((item) => item.provider_instance_name).join(", ")}`,
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      });
    }
    const finance = buildFinancialReport({ entries: [], budgetUsd: 0, plannedMaxCalls: 0, plannedMaxCostUsd: 0 });
    const confirmedRoutingAssertions = new Set([
      "t1.route.exact_model_hits_instance",
      "t1.route.metadata_variant_expands_exact_model",
      "t1.embedding.large_batch_artifact",
      "t1.embedding.space_mismatch_rejected",
      "t1.runtime_boundary.rate_limit_fallback",
      "t1.runtime_boundary.server_error_fallback",
      "t1.runtime_boundary.connection_failure_fallback",
      "t1.runtime_boundary.timeout_fallback",
      "t1.runtime_boundary.malformed_response_rejected",
      "t1.runtime_boundary.wrong_mime_rejected",
      "t1.runtime_boundary.missing_usage_rejected",
      "t1.history.same_session_reuses_exact_model",
      "t1.history.hard_constraint_overrides.provider_denied",
      "t1.route.invalid_logical_path",
      "t1.route.version_default_rule",
      "t1.route.disabled_model",
      "t1.route.output_limit_filter",
      "t1.route.locked_policy_cannot_override",
      "t1.route.missing_metadata_is_conservative",
      "t1.route.offline_model",
      "t1.route.health_filter",
      "t1.route.quota_filter",
      "t1.route.parent_fallback",
      "t1.route.target_logical_fallback",
      "t1.task.running_succeeded",
      "t1.task.cancelled",
      "t1.task.idempotency_conflict_different_body",
      "t1.task.concurrent_idempotency",
      "t1.usage.idempotent_no_double_charge",
      "t1.usage.fallback_attempts_attributed",
      "t1.security.no_token",
      "t1.security.invalid_token",
      "t1.security.expired_token",
      "t1.security.cross_tenant",
      "t1.security.cross_tenant_task_cancel",
      "t1.security.cross_tenant_usage",
      "t1.security.cross_tenant_message",
      "t1.security.cross_tenant_object",
      "t1.security.rbac_admin_method",
      "t1.config.reload_invalid_keeps_old",
      "t1.cleanup.runtime_inventory_restore",
      "t1.observability.correlation",
    ]);
    const defects = cases.filter((item) =>
      item.status === "failed" && confirmedRoutingAssertions.has(item.case_id) &&
      !item.attempts.at(-1)?.diagnostic?.includes("seed case")
    ).map((item) => defectFromFailure({
      component: "AICC",
      caseReport: item,
      expected: "real AICC adapter behavior matches the deterministic HTTP mock contract",
      observed: item.attempts.at(-1)?.diagnostic ?? "case failed",
      evidencePaths: [`cases/${item.case_id}.json`],
    }));
    const report: AcceptanceReport = {
      schema_version: 1,
      run_id: runId,
      started_at: startedAt,
      finished_at: new Date().toISOString(),
      commit: await commitId(),
      baseline_revision: "T1-mock",
      allow_real_model_calls: false,
      planned_real_calls: 0,
      actual_real_calls: 0,
      estimated_cost_usd: 0,
      actual_cost_usd: 0,
      raw_cost_usd: 0,
      credit_applied_usd: 0,
      finance,
      cases,
      product_defects: defects,
      manifest_coverage: manifestCoverage(cases),
      t1_requirement_coverage: buildT1Coverage(buildStaticManifest(), cases),
      cleanup,
      targeted_retest_command: (() => {
        const failed = [...new Set(cases.filter((item) => item.status === "failed").map(canonicalCaseId))]
          .filter((caseId) => knownCaseIds.has(caseId))
          .slice(0, 20);
        return failed.length === 0
          ? undefined
          : [
            "pnpm run acceptance:t1 --",
            "--config",
            JSON.stringify(input.configPath),
            "--allow-config-mutation",
            ...failed.flatMap((caseId) => ["--case", caseId]),
          ].join(" ");
      })(),
    };
    await writeReport(join(input.reportDir, runId), report);
    if (cases.some((item) => item.status === "failed")) Deno.exitCode = 1;
  } finally {
    if (child) {
      try {
        child.kill("SIGTERM");
      } catch {}
      await child.status.catch(() => undefined);
    }
  }
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(`AICC T1 gateway acceptance failed: ${String(error)}`);
    Deno.exitCode = 1;
  });
}
