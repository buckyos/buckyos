import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  parseToml,
  tomlBoolean,
  tomlNumber,
  tomlString,
  tomlStrings,
} from "../../jarvis_media_dv/config.ts";
import { loginGateway, type GatewaySession, type RpcClient } from "./gateway.ts";
import { analyzeProviderMatrix, validateProviderBaseline } from "./manifest.ts";
import {
  assertResponseShape,
  buildExactRequest,
  type FixtureRefs,
  type ResourceFixture,
  type ResourceRef,
} from "./payloads.ts";
import { runPreflight } from "./preflight.ts";
import { defectFromFailure, writeReport } from "./report.ts";
import type {
  AcceptanceReport,
  CaseReport,
  FailureClass,
  ProviderInventory,
  ProviderBaseline,
} from "./types.ts";
import { ProviderScheduler, type ProviderLimits } from "./scheduler.ts";
import {
  buildFinancialReport,
  CostBudget,
  extractFinance,
  type CostReservation,
} from "./finance.ts";
import type { FinancialEntry } from "./types.ts";
import {
  indexUsageByTask,
  queryRouteTraces,
  queryUsageEvents,
  usageEventFinance,
} from "./usage_audit.ts";
import { selectSingleProviderInstances } from "./inventory_selection.ts";
import {
  applyProviderTokens,
  configuredProviderTokens,
  providerTokenDrivers,
  selectProviderTokens,
  type ProviderTokens,
} from "./provider_credentials.ts";
import { SettingsCleanupError, withAiccSettingsOverride } from "./settings_transaction.ts";
import { validateArtifactBytes, validateNamedArtifact, type ArtifactAudit, type ReadableNamedData } from "./artifact_validation.ts";
import { JudgeError, runJudge } from "./judge.ts";

type Options = {
  configPath: string;
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  appId: string;
  providers: string[];
  providerInstances: Record<string, string>;
  allowRealModelCalls: boolean;
  maxRealCalls: number;
  maxCostUsd: number;
  timeoutMs: number;
  reportDir: string;
  fixtures: FixtureRefs;
  globalConcurrency: number;
  providerLimits: ProviderLimits;
  providerLimitOverrides: Record<string, Partial<ProviderLimits>>;
  maxAttempts: number;
  retryDelayMs: number;
  estimatedCostPerCallUsd: number;
  judgeEnabled: boolean;
  judgeModel: string;
  judgeRubricVersion: string;
  judgeMinScore: number;
  providerTokens: ProviderTokens;
  applyProviderCredentials: boolean;
  allowCredentialMutationCli: boolean;
  shardIndex: number;
  shardCount: number;
  caseIds: string[];
};

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

function boolEnv(name: string): boolean | undefined {
  const value = env(name)?.toLowerCase();
  if (value === undefined) return undefined;
  if (["1", "true", "yes", "on"].includes(value)) return true;
  if (["0", "false", "no", "off"].includes(value)) return false;
  throw new Error(`${name} must be a boolean`);
}

function requiredArg(args: string[], index: number, name: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function resource(kind: string, value?: string, mime?: string): ResourceRef | undefined {
  if (!value) return undefined;
  if (/^(chunk|cyfile|obj):/i.test(value)) return { kind: "named_object", obj_id: value };
  return { kind: "url", url: value, ...(mime ? { mime_hint: mime } : {}) };
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 32_768) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 32_768));
  }
  return btoa(binary);
}

async function loadDefaultFixtures(
  fixtures: FixtureRefs,
  gatewayUrl: string,
  sessionToken: string,
  uploadNamedObjects: boolean,
  uploadedObjectIds: string[],
): Promise<FixtureRefs> {
  const defaults: Record<Exclude<keyof FixtureRefs, "documents">, { path: string; mime: string }> = {
    image: { path: join(here, "../fixtures/marker.jpg"), mime: "image/jpeg" },
    mask: { path: join(here, "../fixtures/mask.png"), mime: "image/png" },
    audio: { path: join(here, "../../jarvis_media_dv/assets/audio_speech.wav"), mime: "audio/wav" },
    video: { path: join(here, "../../jarvis_media_dv/assets/video_fresh.mp4"), mime: "video/mp4" },
    document: { path: join(here, "../fixtures/facts.pdf"), mime: "application/pdf" },
  };
  const loaded = { ...fixtures };
  const { ndm_proxy, ndn } = await import("buckyos");
  const ndmProxy = uploadNamedObjects
    ? ndm_proxy.createNdmProxyClient({
      endpoint: gatewayUrl,
      sessionToken,
      fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
        const target = typeof request === "string"
          ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
          : request instanceof URL
          ? new URL(request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"))
          : request;
        return fetch(target, init);
      },
    }) as { putChunk: (objId: string, bytes: Uint8Array) => Promise<unknown> }
    : undefined;
  const variants = async (
    bytes: Uint8Array,
    mime: string,
    configured?: ResourceFixture,
  ): Promise<ResourceFixture> => {
    const objId = ndn.ChunkId.fromMix256Result(
      bytes.byteLength,
      ndn.sha256Bytes(bytes),
    ).toString();
    if (ndmProxy) {
      await ndmProxy.putChunk(objId, bytes);
      uploadedObjectIds.push(objId);
    }
    const result: Partial<Record<ResourceRef["kind"], ResourceRef>> = {
      base64: { kind: "base64", mime, data_base64: base64(bytes) },
      named_object: { kind: "named_object", obj_id: objId },
    };
    if (configured) {
      if ("kind" in configured) result[configured.kind] = configured;
      else Object.assign(result, configured);
    }
    return result;
  };
  for (const [kind, fixture] of Object.entries(defaults) as Array<[
    Exclude<keyof FixtureRefs, "documents">,
    { path: string; mime: string },
  ]>) {
    const bytes = await Deno.readFile(fixture.path);
    loaded[kind] = await variants(bytes, fixture.mime, loaded[kind]);
  }
  const documentFixtures: Record<string, { path: string; mime: string }> = {
    txt: { path: join(here, "../fixtures/facts.txt"), mime: "text/plain" },
    md: { path: join(here, "../fixtures/facts.md"), mime: "text/markdown" },
    pdf: { path: join(here, "../fixtures/facts.pdf"), mime: "application/pdf" },
    docx: { path: join(here, "../fixtures/facts.docx"), mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document" },
    xlsx: { path: join(here, "../fixtures/facts.xlsx"), mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" },
    csv: { path: join(here, "../fixtures/facts.csv"), mime: "text/csv" },
    tsv: { path: join(here, "../fixtures/facts.tsv"), mime: "text/tab-separated-values" },
    pptx: { path: join(here, "../fixtures/facts.pptx"), mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation" },
    html: { path: join(here, "../fixtures/facts.html"), mime: "text/html" },
    xml: { path: join(here, "../fixtures/facts.xml"), mime: "application/xml" },
    json: { path: join(here, "../fixtures/facts.json"), mime: "application/json" },
    yaml: { path: join(here, "../fixtures/facts.yaml"), mime: "application/yaml" },
    rtf: { path: join(here, "../fixtures/facts.rtf"), mime: "application/rtf" },
    epub: { path: join(here, "../fixtures/facts.epub"), mime: "application/epub+zip" },
    source: { path: join(here, "../fixtures/facts.py"), mime: "text/x-python" },
  };
  loaded.documents = { ...loaded.documents };
  for (const [format, fixture] of Object.entries(documentFixtures)) {
    loaded.documents[format] = await variants(
      await Deno.readFile(fixture.path),
      fixture.mime,
      loaded.documents[format],
    );
  }
  return loaded;
}

async function parseOptions(args: string[]): Promise<Options> {
  let configPath = "aicc_acceptance.local.toml";
  let commandLineProviders = false;
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--config") configPath = requiredArg(args, index, "--config");
  }
  let config: ReturnType<typeof parseToml> = {};
  try {
    config = parseToml(await Deno.readTextFile(configPath));
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
  const providerLimitOverrides: Record<string, Partial<ProviderLimits>> = {};
  const providerInstances: Record<string, string> = {};
  for (const [key, value] of Object.entries(config)) {
    const match = /^limits\.([^.]+)\.(max_concurrency|min_interval_ms)$/.exec(key);
    if (!match || typeof value !== "number") continue;
    const limits = providerLimitOverrides[match[1]] ?? {};
    if (match[2] === "max_concurrency") limits.maxConcurrency = value;
    else limits.minIntervalMs = value;
    providerLimitOverrides[match[1]] = limits;
  }
  for (const [key, value] of Object.entries(config)) {
    const match = /^instances\.([^.]+)\.name$/.exec(key);
    if (match && typeof value === "string" && value.trim()) {
      providerInstances[match[1]] = value.trim();
    }
  }
  const options: Options = {
    configPath,
    gatewayUrl: tomlString(config, "gateway.url") ?? env("BUCKYOS_TEST_GATEWAY_URL") ?? "",
    sessionToken: tomlString(config, "auth.session_token") ?? env("BUCKYOS_APPCLIENT_SESSION_TOKEN"),
    username: tomlString(config, "auth.username") ?? env("BUCKYOS_TEST_USERNAME"),
    password: tomlString(config, "auth.password") ?? env("BUCKYOS_TEST_PASSWORD"),
    appId: tomlString(config, "auth.app_id") ?? env("BUCKYOS_TEST_APP_ID") ?? "aicc-tests",
    providers: tomlStrings(config, "runner.providers") ??
      env("AICC_ACCEPTANCE_PROVIDERS")?.split(",").map((item) => item.trim()).filter(Boolean) ?? [],
    providerInstances,
    allowRealModelCalls: tomlBoolean(config, "runner.allow_real_model_calls") ??
      boolEnv("AICC_ALLOW_REAL_MODEL_CALLS") ?? false,
    maxRealCalls: tomlNumber(config, "runner.max_real_calls") ?? 500,
    maxCostUsd: tomlNumber(config, "runner.max_cost_usd") ?? 100,
    timeoutMs: tomlNumber(config, "runner.timeout_ms") ?? 900_000,
    reportDir: tomlString(config, "runner.report_dir") ?? "reports/acceptance",
    globalConcurrency: tomlNumber(config, "runner.global_concurrency") ?? 8,
    providerLimits: {
      maxConcurrency: tomlNumber(config, "runner.provider_concurrency") ?? 2,
      minIntervalMs: tomlNumber(config, "runner.provider_min_interval_ms") ?? 250,
    },
    providerLimitOverrides,
    maxAttempts: tomlNumber(config, "runner.max_attempts") ?? 2,
    retryDelayMs: tomlNumber(config, "runner.retry_delay_ms") ?? 1_000,
    estimatedCostPerCallUsd: tomlNumber(config, "runner.estimated_cost_per_call_usd") ?? 0.01,
    judgeEnabled: tomlBoolean(config, "judge.enabled") ?? true,
    judgeModel: tomlString(config, "judge.model") ?? "llm.plan.default",
    judgeRubricVersion: tomlString(config, "judge.rubric_version") ?? "2026-08-27.1",
    judgeMinScore: tomlNumber(config, "judge.min_score") ?? 0.7,
    providerTokens: configuredProviderTokens(config, env),
    applyProviderCredentials: tomlBoolean(config, "provider_credentials.apply_to_aicc_settings") ?? false,
    allowCredentialMutationCli: false,
    shardIndex: tomlNumber(config, "runner.shard_index") ?? 0,
    shardCount: tomlNumber(config, "runner.shard_count") ?? 1,
    caseIds: [],
    fixtures: {
      image: resource("image", tomlString(config, "fixtures.image"), "image/png"),
      mask: resource("mask", tomlString(config, "fixtures.mask"), "image/png"),
      audio: resource("audio", tomlString(config, "fixtures.audio"), "audio/wav"),
      video: resource("video", tomlString(config, "fixtures.video"), "video/mp4"),
      document: resource("document", tomlString(config, "fixtures.document"), "application/pdf"),
      documents: Object.fromEntries(
        ["txt", "md", "pdf", "doc", "docx", "xls", "xlsx", "csv", "tsv", "ppt", "pptx", "html", "xml", "json", "yaml", "rtf", "epub", "source"]
          .map((format) => [format, resource("document", tomlString(config, `fixtures.document_${format}`))])
          .filter((entry): entry is [string, ResourceRef] => Boolean(entry[1])),
      ),
    },
  };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--config") index += 1;
    else if (arg === "--gateway-url") options.gatewayUrl = requiredArg(args, index++, arg);
    else if (arg === "--session-token") options.sessionToken = requiredArg(args, index++, arg);
    else if (arg === "--username") options.username = requiredArg(args, index++, arg);
    else if (arg === "--password") options.password = requiredArg(args, index++, arg);
    else if (arg === "--provider") {
      if (!commandLineProviders) {
        options.providers = [];
        commandLineProviders = true;
      }
      options.providers.push(requiredArg(args, index++, arg));
    }
    else if (arg === "--provider-instance") {
      const value = requiredArg(args, index++, arg);
      const separator = value.indexOf(":");
      if (separator <= 0 || separator === value.length - 1) {
        throw new Error("--provider-instance must be provider_driver:instance_name");
      }
      options.providerInstances[value.slice(0, separator)] = value.slice(separator + 1);
    }
    else if (arg === "--allow-real-model-calls") options.allowRealModelCalls = true;
    else if (arg === "--no-real-model-calls") options.allowRealModelCalls = false;
    else if (arg === "--allow-credential-mutation") options.allowCredentialMutationCli = true;
    else if (arg === "--max-real-calls") options.maxRealCalls = Number(requiredArg(args, index++, arg));
    else if (arg === "--max-cost-usd") options.maxCostUsd = Number(requiredArg(args, index++, arg));
    else if (arg === "--timeout-ms") options.timeoutMs = Number(requiredArg(args, index++, arg));
    else if (arg === "--global-concurrency") options.globalConcurrency = Number(requiredArg(args, index++, arg));
    else if (arg === "--provider-concurrency") options.providerLimits.maxConcurrency = Number(requiredArg(args, index++, arg));
    else if (arg === "--provider-min-interval-ms") options.providerLimits.minIntervalMs = Number(requiredArg(args, index++, arg));
    else if (arg === "--max-attempts") options.maxAttempts = Number(requiredArg(args, index++, arg));
    else if (arg === "--retry-delay-ms") options.retryDelayMs = Number(requiredArg(args, index++, arg));
    else if (arg === "--judge") options.judgeEnabled = true;
    else if (arg === "--no-judge") options.judgeEnabled = false;
    else if (arg === "--judge-model") options.judgeModel = requiredArg(args, index++, arg);
    else if (arg === "--shard-index") options.shardIndex = Number(requiredArg(args, index++, arg));
    else if (arg === "--shard-count") options.shardCount = Number(requiredArg(args, index++, arg));
    else if (arg === "--case") options.caseIds.push(requiredArg(args, index++, arg));
    else if (arg === "--provider-limit") {
      const [provider, concurrency, interval] = requiredArg(args, index++, arg).split(":");
      if (!provider || concurrency === undefined || interval === undefined) {
        throw new Error("--provider-limit must be provider:max_concurrency:min_interval_ms");
      }
      options.providerLimitOverrides[provider] = {
        maxConcurrency: Number(concurrency),
        minIntervalMs: Number(interval),
      };
    }
    else if (arg === "--report-dir") options.reportDir = requiredArg(args, index++, arg);
    else throw new Error(`unknown argument ${arg}`);
  }
  options.gatewayUrl = options.gatewayUrl.replace(/\/+$/, "");
  if (!options.gatewayUrl) throw new Error("gateway URL is required");
  if (!Number.isInteger(options.maxRealCalls) || options.maxRealCalls < 0) {
    throw new Error("max_real_calls must be a non-negative integer");
  }
  if (!Number.isFinite(options.maxCostUsd) || options.maxCostUsd < 0) {
    throw new Error("max_cost_usd must be non-negative");
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("timeout_ms must be positive");
  }
  if (!Number.isInteger(options.globalConcurrency) || options.globalConcurrency < 1) {
    throw new Error("global_concurrency must be a positive integer");
  }
  if (!Number.isInteger(options.maxAttempts) || options.maxAttempts < 1) {
    throw new Error("max_attempts must be a positive integer");
  }
  if (!Number.isFinite(options.retryDelayMs) || options.retryDelayMs < 0) {
    throw new Error("retry_delay_ms must be non-negative");
  }
  if (!Number.isFinite(options.estimatedCostPerCallUsd) || options.estimatedCostPerCallUsd < 0) {
    throw new Error("estimated_cost_per_call_usd must be non-negative");
  }
  if (!Number.isFinite(options.judgeMinScore) || options.judgeMinScore < 0 || options.judgeMinScore > 1) {
    throw new Error("judge.min_score must be between 0 and 1");
  }
  if (!Number.isInteger(options.shardCount) || options.shardCount < 1 ||
    !Number.isInteger(options.shardIndex) || options.shardIndex < 0 ||
    options.shardIndex >= options.shardCount) {
    throw new Error("shard_index must satisfy 0 <= shard_index < shard_count");
  }
  options.providers = [...new Set(options.providers)];
  const tokenDrivers = providerTokenDrivers(options.providerTokens);
  if (tokenDrivers.length > 0 && !options.applyProviderCredentials) {
    throw new Error("provider API tokens are configured but provider_credentials.apply_to_aicc_settings is false");
  }
  if (options.applyProviderCredentials && tokenDrivers.length > 0 && !options.allowCredentialMutationCli) {
    throw new Error("applying Provider credentials requires --allow-credential-mutation");
  }
  return options;
}

function normalizeInventories(raw: unknown): ProviderInventory[] {
  if (!raw || typeof raw !== "object") throw new Error("models.list returned non-object");
  const providers = (raw as { providers?: unknown }).providers;
  if (!Array.isArray(providers)) throw new Error("models.list.providers must be an array");
  return providers.map((value) => {
    const provider = value as ProviderInventory;
    if (!provider.provider_driver || !provider.provider_instance_name || !Array.isArray(provider.models)) {
      throw new Error("models.list contains invalid provider inventory");
    }
    return provider;
  });
}

async function waitForProviderInventories(input: {
  aicc: RpcClient;
  expectedDrivers: string[];
  expectedInstances: Record<string, string>;
  timeoutMs: number;
}): Promise<ProviderInventory[]> {
  const deadline = Date.now() + Math.min(input.timeoutMs, 60_000);
  let latest: ProviderInventory[] = [];
  let lastReloadAt = 0;
  do {
    if (Date.now() - lastReloadAt >= 2_000) {
      await input.aicc.call("service.reload_settings", {});
      lastReloadAt = Date.now();
    }
    latest = normalizeInventories(await input.aicc.call("models.list", {}));
    const ready = input.expectedDrivers.every((driver) => latest.some((inventory) =>
      inventory.provider_driver === driver &&
      (!input.expectedInstances[driver] || inventory.provider_instance_name === input.expectedInstances[driver])
    ));
    if (ready) return latest;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));
  } while (Date.now() < deadline);
  throw new Error(
    `Provider inventories did not converge: expected=${input.expectedDrivers.map((driver) => `${driver}:${input.expectedInstances[driver] ?? "*"}`).join(",")} found=${latest.map((inventory) => `${inventory.provider_driver}:${inventory.provider_instance_name}`).join(",")}`,
  );
}

function taskValue(raw: unknown): Record<string, unknown> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const envelope = raw as Record<string, unknown>;
  const task = envelope.task;
  return task && typeof task === "object" && !Array.isArray(task)
    ? task as Record<string, unknown>
    : envelope;
}

async function waitForTask(
  taskManager: RpcClient,
  response: AiMethodResponse,
  timeoutMs: number,
): Promise<unknown> {
  if (response.status === "failed") {
    throw new Error(`AICC returned failed: ${failedResponseDiagnostic(response)}`);
  }
  if (response.status !== "running") return response;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const raw = await taskManager.call("get_task", { task_id: response.task_id });
    const task = taskValue(raw) as {
      phase?: string;
      outcome?: string;
      result?: { result?: { output?: unknown } };
      error?: unknown;
    };
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") {
        throw new Error(
          `task ${response.task_id} ended ${task.outcome}: ${JSON.stringify(compactFailure(task.error) ?? {})}`,
        );
      }
      return {
        task_id: response.task_id,
        status: "succeeded",
        result: task.result?.result?.output,
        event_ref: response.event_ref,
      };
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 1_000));
  }
  throw new Error(`task ${response.task_id} timed out after ${timeoutMs} ms`);
}

function failureClass(error: unknown): FailureClass {
  const message = String(error).toLowerCase();
  if (error instanceof JudgeError || message.includes("judge")) return "judge_failed";
  if (message.includes("baseline")) return "baseline_mismatch";
  if (message.includes("artifact") || message.includes("mime")) return "resource_failed";
  if (message.includes("task") || message.includes("timeout") || message.includes("aicc returned failed")) {
    return "task_lifecycle_failed";
  }
  if (message.includes("provider")) return "provider_protocol_failed";
  return "assertion_failed";
}

function retryable(error: unknown): boolean {
  const message = String(error).toLowerCase();
  return [
    "429",
    "rate limit",
    "too many requests",
    "timeout",
    "timed out",
    "connection",
    "network",
    "temporarily unavailable",
    "service unavailable",
    "bad gateway",
    "gateway timeout",
    "provider_start_failed",
  ].some((marker) => message.includes(marker)) || /\b5\d\d\b/.test(message);
}

function estimatedCellCost(cell: { estimated_cost_usd?: number }, fallback: number): number {
  return typeof cell.estimated_cost_usd === "number" &&
      Number.isFinite(cell.estimated_cost_usd) && cell.estimated_cost_usd >= 0
    ? cell.estimated_cost_usd
    : fallback;
}

function semanticRubric(cell: { api_type: string; method: string }): string[] {
  const apiType = cell.api_type;
  if (apiType === "image.txt2img") return ["The image visibly contains a blue square and the number 4827."];
  if (apiType === "image.img2img") return ["The output preserves the source composition and applies warm evening light."];
  if (apiType === "image.inpaint") return ["The masked region is filled with plausible green leaves without corrupting the rest of the image."];
  if (apiType === "image.upscale") return ["The output preserves the source content and has visibly improved or increased resolution."];
  if (apiType === "image.bg_remove") return ["The foreground is preserved and the background is removed or transparent."];
  if (apiType === "vision.ocr") return ["The recognized text contains the visible acceptance marker 4827."];
  if (apiType === "vision.caption") return ["The caption accurately describes the supplied marker image."];
  if (apiType === "vision.detect") return ["The structured detection result identifies visible objects in the supplied image."];
  if (apiType === "vision.segment") return ["The segmentation result corresponds to visible regions in the supplied image."];
  if (apiType === "audio.tts") return ["The speech clearly says BuckyOS test number four eight two seven."];
  if (apiType === "audio.asr") return ["The transcript faithfully represents the supplied speech audio."];
  if (apiType === "audio.music") return ["The output is a short calm instrumental passage rather than speech or silence."];
  if (apiType === "audio.enhance") return ["The output preserves the source audio while reducing noise or improving clarity."];
  if (apiType === "video.txt2video") return ["The video shows a paper plane moving across a desk."];
  if (apiType === "video.img2video") return ["The video preserves the supplied image content and adds subtle motion with a slow camera push."];
  if (apiType === "video.video2video") return ["The output is a coherent transformation of the supplied video."];
  if (apiType === "video.extend") return ["The output coherently extends the supplied video rather than replacing it with unrelated content."];
  if (apiType === "video.upscale") return ["The output preserves the supplied video content at improved resolution or visual quality."];
  if (apiType === "agent.computer_use") return ["The result reports the title visible in the supplied test environment and does not invent an action result."];
  return [];
}

function artifactSources(value: unknown, depth = 0): Array<Record<string, unknown>> {
  if (depth > 8 || value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap((item) => artifactSources(item, depth + 1));
  if (typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  const source = record.source && typeof record.source === "object" && !Array.isArray(record.source)
    ? record.source as Record<string, unknown>
    : undefined;
  const found = source && (typeof source.obj_id === "string" || typeof source.url === "string" || typeof source.data_base64 === "string")
    ? [{ ...source, _content_type: record.type }]
    : [];
  return [...found, ...Object.values(record).flatMap((child) => artifactSources(child, depth + 1))];
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

async function validateTerminalArtifacts(input: {
  terminal: unknown;
  ndm: ReadableNamedData;
  gatewayUrl: string;
  sessionToken: string;
}): Promise<{ audits: ArtifactAudit[]; ids: string[] }> {
  const sources = artifactSources(input.terminal);
  const seen = new Set<string>();
  const audits: ArtifactAudit[] = [];
  const ids: string[] = [];
  for (const source of sources) {
    const mime = typeof source.mime_hint === "string" ? source.mime_hint
      : typeof source.mime === "string" ? source.mime : undefined;
    const label = [source.filename, mime].filter((value): value is string => typeof value === "string").join(" ");
    if (typeof source.obj_id === "string") {
      if (seen.has(`obj:${source.obj_id}`)) continue;
      seen.add(`obj:${source.obj_id}`);
      audits.push(await validateNamedArtifact(input.ndm, { obj_id: source.obj_id, label }));
      ids.push(source.obj_id);
    } else if (typeof source.data_base64 === "string") {
      const key = `base64:${source.data_base64.length}:${source.data_base64.slice(0, 32)}`;
      if (seen.has(key)) continue;
      seen.add(key);
      audits.push(await validateArtifactBytes(decodeBase64(source.data_base64), { id: "[inline-base64]", label }));
    } else if (typeof source.url === "string") {
      if (seen.has(`url:${source.url}`)) continue;
      seen.add(`url:${source.url}`);
      const target = new URL(source.url);
      const gateway = new URL(input.gatewayUrl);
      const response = await fetch(target, target.origin === gateway.origin
        ? { headers: { authorization: `Bearer ${input.sessionToken}` } }
        : undefined);
      if (!response.ok) throw new Error(`artifact URL download failed with HTTP ${response.status}`);
      const declared = Number(response.headers.get("content-length"));
      if (Number.isFinite(declared) && declared > 256 * 1024 * 1024) throw new Error("artifact URL exceeds 256 MiB safety limit");
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.length > 256 * 1024 * 1024) throw new Error("artifact URL exceeds 256 MiB safety limit");
      audits.push(await validateArtifactBytes(bytes, { id: "[downloaded-url]", label: label || response.headers.get("content-type") || undefined }));
    }
  }
  return { audits, ids };
}

function modelFamily(exactModel: string): string {
  return exactModel.split("@")[0].split(":")[0].replace(/[-_]20\d{2}.*$/, "").replace(/[-_]v?\d+(?:\.\d+)*$/, "");
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

async function main(): Promise<void> {
  const runStartedAt = new Date().toISOString();
  await runPreflight();
  const options = await parseOptions(Deno.args);
  const runId = `aicc-${new Date().toISOString().replace(/[:.]/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
  const outputDir = join(options.reportDir, runId);
  const baseline = validateProviderBaseline(JSON.parse(
    await readFile(join(here, "provider_capability_baseline.json"), "utf8"),
  ));
  const session = await loginGateway({
    gatewayUrl: options.gatewayUrl,
    sessionToken: options.sessionToken,
    username: options.username,
    password: options.password,
    appId: options.appId,
  });
  const uploadedFixtureIds: string[] = [];
  options.fixtures = await loadDefaultFixtures(
    options.fixtures,
    options.gatewayUrl,
    session.sessionToken,
    options.allowRealModelCalls,
    uploadedFixtureIds,
  );
  const initialRuntimeInstances = new Set(
    normalizeInventories(await session.aicc.call("models.list", {}))
      .map((inventory) => inventory.provider_instance_name),
  );
  const execute = () => executeAcceptance({
    options,
    session,
    runStartedAt,
    runId,
    outputDir,
    baseline,
    uploadedFixtureIds,
  });
  options.providerTokens = selectProviderTokens(options.providerTokens, options.providers);
  const tokenDrivers = providerTokenDrivers(options.providerTokens);
  let outcome: { report: AcceptanceReport; effectiveBaseline: Record<string, unknown> };
  let cleanupFailure: SettingsCleanupError<typeof outcome> | undefined;
  if (tokenDrivers.length === 0) {
    outcome = await execute();
  } else {
    try {
      outcome = (await withAiccSettingsOverride({
        systemConfig: session.systemConfig,
        aicc: session.aicc,
        description: "AICC Provider credential override",
        patch: (settings) => applyProviderTokens(settings, options.providerTokens, options.providerInstances),
        execute,
        refreshClients: async () => {
          const refreshed = await loginGateway({
            gatewayUrl: options.gatewayUrl,
            username: options.username,
            password: options.password,
            appId: options.appId,
          });
          return { systemConfig: refreshed.systemConfig, aicc: refreshed.aicc };
        },
      })).result;
      outcome.report.cleanup.details = [
        ...outcome.report.cleanup.details,
        `temporary Provider credentials restored after testing (${tokenDrivers.join(", ")})`,
      ];
    } catch (error) {
      if (!(error instanceof SettingsCleanupError) || !error.executionResult) throw error;
      outcome = error.executionResult;
      cleanupFailure = error as SettingsCleanupError<typeof outcome>;
      outcome.report.cleanup = {
        status: "failed",
        details: ["automatic Provider credential restoration failed; manual restoration is required"],
      };
      outcome.report.cases.push({
        run_id: outcome.report.run_id,
        case_id: "t2.cleanup.provider_settings_restore",
        layer: "T2",
        status: "failed",
        method: "sys_config_set/service.reload_settings",
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 1,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "failed",
          failure_class: "cleanup_failed",
          diagnostic: error.message,
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      });
    }
  }
  if (tokenDrivers.length > 0) {
    const extraRuntimeInstances = normalizeInventories(await session.aicc.call("models.list", {}))
      .map((inventory) => inventory.provider_instance_name)
      .filter((name) => !initialRuntimeInstances.has(name));
    if (extraRuntimeInstances.length > 0) {
      outcome.report.cleanup = {
        status: "failed",
        details: [
          ...outcome.report.cleanup.details,
          `AICC runtime inventory retained temporary Provider instance(s): ${extraRuntimeInstances.join(", ")}`,
        ],
      };
      outcome.report.cases.push({
        run_id: outcome.report.run_id,
        case_id: "t2.cleanup.runtime_inventory_restore",
        layer: "T2",
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
          diagnostic: `runtime inventory retained: ${extraRuntimeInstances.join(", ")}`,
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      });
    }
  }
  await writeReport(outputDir, outcome.report);
  await writeFile(
    join(outputDir, "effective_baseline.json"),
    `${JSON.stringify(outcome.effectiveBaseline, null, 2)}\n`,
    "utf8",
  );
  if (cleanupFailure) Deno.exitCode = 1;
  if (outcome.report.cases.some((item) => item.status === "failed")) Deno.exitCode = 1;
  else if (outcome.report.cases.some((item) => item.status === "skipped" || item.status === "review")) Deno.exitCode = 2;
}

async function executeAcceptance(input: {
  options: Options;
  session: GatewaySession;
  runStartedAt: string;
  runId: string;
  outputDir: string;
  baseline: ProviderBaseline;
  uploadedFixtureIds: string[];
}): Promise<{ report: AcceptanceReport; effectiveBaseline: Record<string, unknown> }> {
  const { options, session, runStartedAt, runId, outputDir, baseline, uploadedFixtureIds } = input;
  const { ndm_proxy } = await import("buckyos");
  const ndm = ndm_proxy.createNdmProxyClient({
    endpoint: options.gatewayUrl,
    sessionToken: session.sessionToken,
    fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
      const target = typeof request === "string"
        ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
        : request instanceof URL
        ? new URL(request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"))
        : request;
      return fetch(target, init);
    },
  }) as ReadableNamedData & {
    removeChunk: (request: { chunk_id: string }) => Promise<unknown>;
    removeObject: (request: { obj_id: string }) => Promise<unknown>;
  };
  const selectedDrivers = options.providers.length > 0
    ? options.providers
    : baseline.providers.map((provider) => provider.provider_driver);
  const credentialDrivers = providerTokenDrivers(options.providerTokens);
  const inventories = credentialDrivers.length > 0
    ? await waitForProviderInventories({
      aicc: session.aicc,
      expectedDrivers: credentialDrivers,
      expectedInstances: options.providerInstances,
      timeoutMs: options.timeoutMs,
    })
    : normalizeInventories(await session.aicc.call("models.list", {}));
  const selectedInventories = selectSingleProviderInstances({
    inventories,
    drivers: selectedDrivers,
    configured: options.providerInstances,
  });
  const matrix = analyzeProviderMatrix({
    baseline,
    inventories: selectedInventories,
    selectedDrivers,
  });
  const sortedCells = [...matrix.cells].sort((left, right) =>
    left.case_id.localeCompare(right.case_id)
  );
  const requestedCases = new Set(options.caseIds);
  const selectedCells = requestedCases.size > 0
    ? sortedCells.filter((cell) => requestedCases.has(cell.case_id))
    : sortedCells.filter((_, index) => index % options.shardCount === options.shardIndex);
  const filteredRequestedCases = new Map<string, typeof matrix.coverage>();
  for (const caseId of requestedCases) {
    const matches = matrix.coverage.filter((record) => {
      const segment = record.provider_model_id.toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
      return record.status === "filtered" && caseId.toLowerCase().includes(`.${segment}.`);
    });
    if (matches.length > 0) filteredRequestedCases.set(caseId, matches);
  }
  const missingCases = [...requestedCases].filter((caseId) =>
    !sortedCells.some((cell) => cell.case_id === caseId) && !filteredRequestedCases.has(caseId)
  );
  if (missingCases.length > 0) {
    throw new Error(`requested cases are not in the selected Provider matrix: ${missingCases.join(", ")}`);
  }
  const cases: CaseReport[] = matrix.mismatches.map((mismatch, index) => ({
    run_id: runId,
    case_id: `t2.preflight.baseline_mismatch.${index + 1}`,
    layer: "T2",
    status: "failed",
    method: "models.list",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [{
      attempt: 1,
      started_at: new Date().toISOString(),
      elapsed_ms: 0,
      status: "failed",
      failure_class: "baseline_mismatch",
      diagnostic: mismatch,
    }],
  }));
  for (const [caseId, records] of filteredRequestedCases) {
    const record = records[0];
    cases.push({
      run_id: runId,
      case_id: caseId,
      layer: "T2",
      status: "skipped",
      provider_driver: record.provider_driver,
      provider_instance: record.provider_instance,
      exact_model: record.exact_model,
      method: "model_coverage.filter",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [{
        attempt: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        status: "skipped",
        diagnostic: records.map((item) =>
          `${item.provider_model_id}: ${item.reason}; ${item.evidence_summary}`
        ).join(" | "),
        estimated_cost_usd: 0,
        cost_status: "not_called",
      }],
    });
  }
  for (const driver of selectedDrivers) {
    if (!inventories.some((inventory) => inventory.provider_driver === driver)) {
      cases.push({
        run_id: runId,
        case_id: `t2.preflight.provider_missing.${driver}`,
        layer: "T2",
        status: "skipped",
        provider_driver: driver,
        method: "models.list",
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 1,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "skipped",
          diagnostic: "selected provider has no runtime inventory",
        }],
      });
    } else if (!matrix.coverage.some((record) =>
      record.provider_driver === driver && record.status === "included"
    )) {
      cases.push({
        run_id: runId,
        case_id: `t2.preflight.provider_no_eligible_models.${driver}`,
        layer: "T2",
        status: "skipped",
        provider_driver: driver,
        method: "model_coverage.filter",
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 0,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "skipped",
          diagnostic: "runtime inventory contains no active physical model after lifecycle, alias, and duplicate filtering",
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      });
    }
  }
  const preparedRequests = new Map<string, Record<string, unknown>>();
  for (const cell of selectedCells) {
    try {
      preparedRequests.set(cell.case_id, buildExactRequest({ cell, runId, fixtures: options.fixtures }));
    } catch (error) {
      cases.push({
        run_id: runId,
        case_id: cell.case_id,
        layer: "T2",
        status: "skipped",
        provider_driver: cell.provider_driver,
        provider_instance: cell.provider_instance,
        exact_model: cell.exact_model,
        api_type: cell.api_type,
        method: cell.method,
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 0,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "skipped",
          diagnostic: `fixture prerequisite missing: ${String(error)}`,
          estimated_cost_usd: 0,
          cost_status: "not_called",
        }],
      });
    }
  }
  const executableCells = selectedCells.filter((cell) => preparedRequests.has(cell.case_id));
  const relevantDocumentCoverage = requestedCases.size === 0
    ? matrix.documentCoverage
    : matrix.documentCoverage.filter((record) => [...requestedCases].some((caseId) =>
      caseId.includes(`.${record.provider_model_id.toLowerCase().replace(/[^a-z0-9._-]+/g, "-")}.`)
    ));
  for (const record of relevantDocumentCoverage.filter((item) => item.status === "not_applicable")) {
    const caseId = `t2.${record.provider_driver}.${record.provider_instance}.${record.provider_model_id}.document_format.${record.format}`
      .toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
    cases.push({
      run_id: runId,
      case_id: caseId,
      layer: "T2",
      status: "not_applicable",
      provider_driver: record.provider_driver,
      provider_instance: record.provider_instance,
      exact_model: record.exact_model,
      api_type: "llm",
      method: "official_capability_baseline",
      outbound_message_ids: [],
      artifact_ids: [],
      attempts: [{
        attempt: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        status: "not_applicable",
        diagnostic: `official documentation does not list ${record.format}; ${record.source_urls.join(", ")}`,
        estimated_cost_usd: 0,
        cost_status: "not_called",
      }],
    });
  }
  const plannedCases = executableCells.length;
  const judgedCells = options.judgeEnabled
    ? executableCells.filter((cell) => semanticRubric(cell).length > 0)
    : [];
  const plannedCalls = plannedCases * options.maxAttempts + judgedCells.length;
  const estimatedCost = executableCells.reduce(
    (sum, cell) => sum + estimatedCellCost(cell, options.estimatedCostPerCallUsd) * options.maxAttempts,
    0,
  ) + judgedCells.length * options.estimatedCostPerCallUsd;
  console.log(`[plan] run_id=${runId}`);
  console.log(`[plan] providers=${selectedDrivers.join(",")}`);
  console.log(`[plan] shard=${options.shardIndex + 1}/${options.shardCount} full_matrix_cases=${matrix.cells.length}`);
  if (requestedCases.size > 0) console.log(`[plan] targeted_cases=${selectedCells.length}`);
  console.log(`[plan] cases=${plannedCases} max_attempts=${options.maxAttempts} max_possible_calls=${plannedCalls} max_calls=${options.maxRealCalls}`);
  console.log(`[plan] concurrency global=${options.globalConcurrency} provider_default=${options.providerLimits.maxConcurrency} interval_default_ms=${options.providerLimits.minIntervalMs}`);
  for (const [provider, limits] of Object.entries(options.providerLimitOverrides)) {
    console.log(`[plan] provider_limit ${provider}: concurrency=${limits.maxConcurrency ?? options.providerLimits.maxConcurrency} interval_ms=${limits.minIntervalMs ?? options.providerLimits.minIntervalMs}`);
  }
  console.log(`[plan] estimated_cost_usd=${estimatedCost.toFixed(4)} max_cost_usd=${options.maxCostUsd.toFixed(4)}`);
  console.log(`[plan] judge enabled=${options.judgeEnabled} model=${options.judgeModel} semantic_cases=${options.judgeEnabled ? judgedCells.length : executableCells.filter((cell) => semanticRubric(cell).length > 0).length}`);
  if (options.allowRealModelCalls && plannedCalls > options.maxRealCalls) {
    throw new Error(`planned calls ${plannedCalls} exceed max_real_calls ${options.maxRealCalls}`);
  }
  if (options.allowRealModelCalls && estimatedCost > options.maxCostUsd) {
    throw new Error(`estimated cost ${estimatedCost} exceeds max_cost_usd ${options.maxCostUsd}`);
  }

  let actualCalls = 0;
  const financialEntries: FinancialEntry[] = [];
  const costBudget = new CostBudget(options.maxCostUsd);
  if (options.allowRealModelCalls) {
    const scheduler = new ProviderScheduler(
      options.globalConcurrency,
      options.providerLimits,
      options.providerLimitOverrides,
    );
    const executed = await Promise.all(executableCells.map(async (cell) => {
      const caseReport: CaseReport = {
        run_id: runId,
        case_id: cell.case_id,
        layer: "T2",
        status: "failed",
        provider_driver: cell.provider_driver,
        provider_instance: cell.provider_instance,
        exact_model: cell.exact_model,
        api_type: cell.api_type,
        method: cell.method,
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [],
      };
      const request = preparedRequests.get(cell.case_id)!;
      for (let attempt = 1; attempt <= options.maxAttempts; attempt += 1) {
        const started = Date.now();
        const attemptEstimate = estimatedCellCost(cell, options.estimatedCostPerCallUsd);
        let reservation: CostReservation | undefined;
        let reservationSettled = false;
        try {
          const initial = await scheduler.execute(cell.provider_driver, async () => {
            if (actualCalls >= options.maxRealCalls) {
              throw new Error(`max_real_calls ${options.maxRealCalls} exhausted`);
            }
            reservation = costBudget.reserve(attemptEstimate);
            actualCalls += 1;
            return await session.aicc.call(cell.method, request) as AiMethodResponse;
          });
          const terminal = await waitForTask(session.taskManager, initial, options.timeoutMs);
          assertResponseShape(cell, terminal);
          const artifacts = await validateTerminalArtifacts({
            terminal,
            ndm,
            gatewayUrl: options.gatewayUrl,
            sessionToken: session.sessionToken,
          });
          caseReport.artifact_ids = artifacts.ids;
          caseReport.artifact_audits = artifacts.audits;
          const finance = extractFinance(terminal);
          costBudget.settle(reservation!, finance.actualCostUsd);
          reservationSettled = true;
          financialEntries.push({
            case_id: cell.case_id,
            attempt,
            provider_driver: cell.provider_driver,
            provider_instance: cell.provider_instance,
            exact_model: cell.exact_model,
            api_type: cell.api_type,
            method: cell.method,
            started_at: new Date(started).toISOString(),
            status: "passed",
            usage: finance.usage,
            estimated_cost_usd: attemptEstimate,
            actual_cost_usd: finance.actualCostUsd,
            raw_cost_usd: finance.rawCostUsd,
            credit_applied_usd: finance.creditAppliedUsd,
            cost_status: finance.actualCostUsd === undefined ? "unknown" : "actual",
          });
          caseReport.status = "passed";
          caseReport.task_id = initial.task_id;
          caseReport.usage = finance.usage;
          caseReport.cost_usd = finance.actualCostUsd;
          const rubric = semanticRubric(cell);
          let finalStatus: "passed" | "failed" | "review" = "passed";
          let diagnostic = artifacts.audits.length > 0
            ? `validated ${artifacts.audits.length} output artifact(s)`
            : undefined;
          if (rubric.length > 0 && options.judgeEnabled) {
            const judgeStarted = Date.now();
            const judgeEstimate = options.estimatedCostPerCallUsd;
            let judgeReservation: CostReservation | undefined;
            let judgeSettled = false;
            try {
              const preferDifferentProvider = selectedInventories.some((inventory) =>
                inventory.provider_instance_name !== cell.provider_instance &&
                inventory.models.some((model) => model.api_types.includes("llm"))
              );
              const verdict = await runJudge({
                aicc: session.aicc,
                taskManager: session.taskManager,
                model: options.judgeModel,
                runId,
                caseId: cell.case_id,
                rubricVersion: options.judgeRubricVersion,
                rubric,
                testedModel: cell.exact_model,
                testedProviderInstance: cell.provider_instance,
                preferDifferentProvider,
                threshold: options.judgeMinScore,
                terminalResponse: terminal,
                timeoutMs: Math.min(options.timeoutMs, 180_000),
                invoke: async (request) => await scheduler.execute("judge", async () => {
                  if (actualCalls >= options.maxRealCalls) throw new Error(`max_real_calls ${options.maxRealCalls} exhausted before Judge`);
                  judgeReservation = costBudget.reserve(judgeEstimate);
                  actualCalls += 1;
                  return await session.aicc.call("llm.chat", request) as AiMethodResponse;
                }),
              });
              const judgeFinance = extractFinance(verdict.terminalResponse);
              costBudget.settle(judgeReservation!, judgeFinance.actualCostUsd);
              judgeSettled = true;
              financialEntries.push({
                case_id: `${cell.case_id}.judge`,
                attempt: 1,
                provider_driver: "judge",
                provider_instance: options.judgeModel,
                exact_model: options.judgeModel,
                api_type: "llm",
                method: "llm.chat",
                started_at: new Date(judgeStarted).toISOString(),
                status: verdict.passed ? "passed" : "failed",
                usage: judgeFinance.usage,
                estimated_cost_usd: judgeEstimate,
                actual_cost_usd: judgeFinance.actualCostUsd,
                raw_cost_usd: judgeFinance.rawCostUsd,
                credit_applied_usd: judgeFinance.creditAppliedUsd,
                cost_status: judgeFinance.actualCostUsd === undefined ? "unknown" : "actual",
              });
              caseReport.judge = {
                rubric_version: options.judgeRubricVersion,
                configured_model: options.judgeModel,
                task_id: verdict.taskId,
                input_summary: verdict.inputSummary,
                score: verdict.score,
                passed: verdict.passed,
                reason: verdict.reason,
              };
              if (!verdict.passed) throw new JudgeError(`Judge score ${verdict.score}: ${verdict.reason}`);
              diagnostic = [diagnostic, `Judge score ${verdict.score}: ${verdict.reason}`].filter(Boolean).join("; ");
            } catch (error) {
              if (judgeReservation && !judgeSettled) {
                costBudget.settle(judgeReservation);
                financialEntries.push({
                  case_id: `${cell.case_id}.judge`,
                  attempt: 1,
                  provider_driver: "judge",
                  provider_instance: options.judgeModel,
                  exact_model: options.judgeModel,
                  api_type: "llm",
                  method: "llm.chat",
                  started_at: new Date(judgeStarted).toISOString(),
                  status: "failed",
                  estimated_cost_usd: judgeEstimate,
                  cost_status: "unknown",
                });
              }
              throw error;
            }
          } else if (rubric.length > 0) {
            finalStatus = "review";
            diagnostic = [diagnostic, "semantic rubric requires Judge or manual review"].filter(Boolean).join("; ");
          }
          caseReport.status = finalStatus;
          caseReport.attempts.push({
            attempt,
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            status: finalStatus,
            diagnostic,
            usage: finance.usage,
            estimated_cost_usd: attemptEstimate,
            actual_cost_usd: finance.actualCostUsd,
            raw_cost_usd: finance.rawCostUsd,
            credit_applied_usd: finance.creditAppliedUsd,
            cost_status: finance.actualCostUsd === undefined ? "unknown" : "actual",
          });
          break;
        } catch (error) {
          if (reservation && !reservationSettled) {
            costBudget.settle(reservation);
            financialEntries.push({
              case_id: cell.case_id,
              attempt,
              provider_driver: cell.provider_driver,
              provider_instance: cell.provider_instance,
              exact_model: cell.exact_model,
              api_type: cell.api_type,
              method: cell.method,
              started_at: new Date(started).toISOString(),
              status: "failed",
              estimated_cost_usd: attemptEstimate,
              cost_status: "unknown",
            });
          }
          caseReport.attempts.push({
            attempt,
            started_at: new Date(started).toISOString(),
            elapsed_ms: Date.now() - started,
            status: "failed",
            failure_class: failureClass(error),
            diagnostic: String(error),
            estimated_cost_usd: reservation ? attemptEstimate : 0,
            cost_status: reservation ? "unknown" : "not_called",
          });
          if (attempt >= options.maxAttempts || !retryable(error)) break;
          if (options.retryDelayMs > 0) {
            await new Promise((resolvePromise) => setTimeout(resolvePromise, options.retryDelayMs));
          }
        }
      }
      return caseReport;
    }));
    cases.push(...executed);
  } else if (!options.allowRealModelCalls) {
    for (const cell of executableCells) {
      cases.push({
        run_id: runId,
        case_id: cell.case_id,
        layer: "T2",
        status: "skipped",
        provider_driver: cell.provider_driver,
        provider_instance: cell.provider_instance,
        exact_model: cell.exact_model,
        api_type: cell.api_type,
        method: cell.method,
        outbound_message_ids: [],
        artifact_ids: [],
        attempts: [{
          attempt: 0,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
          status: "skipped",
          diagnostic: "real model calls require --allow-real-model-calls",
        }],
      });
    }
  }
  if (options.allowRealModelCalls) {
    const successful = cases.filter((item) => item.task_id && financialEntries.some((entry) =>
      entry.case_id === item.case_id && entry.status === "passed"
    ));
    if (successful.length > 0) {
      try {
        let usageEvents = await queryUsageEvents({
          aicc: session.aicc,
          startTimeMs: new Date(runStartedAt).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: successful.map((item) => item.task_id!),
        });
        const deadline = Date.now() + 10_000;
        while (usageEvents.length < successful.length && Date.now() < deadline) {
          await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));
          usageEvents = await queryUsageEvents({
            aicc: session.aicc,
            startTimeMs: new Date(runStartedAt).getTime() - 1_000,
            endTimeMs: Date.now() + 1_000,
            taskIds: successful.map((item) => item.task_id!),
          });
        }
        const byTask = indexUsageByTask(usageEvents);
        const traces = await queryRouteTraces({
          aicc: session.aicc,
          startTimeMs: new Date(runStartedAt).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds: successful.map((item) => item.task_id!),
        });
        const tracesByTask = new Map<string, typeof traces>();
        for (const trace of traces) {
          const values = tracesByTask.get(trace.task_id) ?? [];
          values.push(trace);
          tracesByTask.set(trace.task_id, values);
        }
        for (const item of successful) {
          const durable = byTask.get(item.task_id!) ?? [];
          const routeTraces = tracesByTask.get(item.task_id!) ?? [];
          const trace = routeTraces.find((candidate) =>
            candidate.selected_exact_model === item.exact_model &&
            candidate.provider_instance_name === item.provider_instance
          );
          if (durable.length !== 1 || !trace) {
            item.status = "failed";
            item.attempts.push({
              attempt: item.attempts.length + 1,
              started_at: new Date().toISOString(),
              elapsed_ms: 0,
              status: "failed",
              failure_class: "usage_failed",
              diagnostic: `durable attribution failed for task ${item.task_id}: usage_events=${durable.length}, matching_route_trace=${Boolean(trace)}, observed_routes=${routeTraces.map((candidate) => `${candidate.selected_exact_model ?? "[missing]"}/${candidate.provider_instance_name ?? "[missing]"}`).join("|") || "[none]"}`,
              estimated_cost_usd: 0,
              cost_status: "not_called",
            });
            continue;
          }
          const audited = usageEventFinance(durable[0]);
          item.trace_id = trace.trace_id;
          item.usage = audited.usage;
          item.cost_usd = audited.actualCostUsd;
          const entry = [...financialEntries].reverse().find((candidate) =>
            candidate.case_id === item.case_id && candidate.status === "passed"
          );
          if (entry) {
            entry.usage = audited.usage;
            entry.actual_cost_usd = audited.actualCostUsd;
            entry.raw_cost_usd = audited.rawCostUsd;
            entry.credit_applied_usd = audited.creditAppliedUsd;
            entry.cost_status = audited.actualCostUsd === undefined ? "unknown" : "actual";
          }
          const attempt = [...item.attempts].reverse().find((candidate) => candidate.status === "passed");
          if (attempt) {
            attempt.usage = audited.usage;
            attempt.actual_cost_usd = audited.actualCostUsd;
            attempt.raw_cost_usd = audited.rawCostUsd;
            attempt.credit_applied_usd = audited.creditAppliedUsd;
            attempt.cost_status = audited.actualCostUsd === undefined ? "unknown" : "actual";
          }
        }
      } catch (error) {
        cases.push({
          run_id: runId,
          case_id: "t2.usage.durable_audit",
          layer: "T2",
          status: "failed",
          method: "usage.query",
          outbound_message_ids: [],
          artifact_ids: [],
          attempts: [{
            attempt: 1,
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            status: "failed",
            failure_class: "usage_failed",
            diagnostic: String(error),
            estimated_cost_usd: 0,
            cost_status: "not_called",
          }],
        });
      }
    }
  }
  if (options.allowRealModelCalls) {
    const judged = cases.filter((item) => item.judge?.task_id);
    if (judged.length > 0) {
      try {
        const taskIds = judged.map((item) => item.judge!.task_id);
        let events = await queryUsageEvents({
          aicc: session.aicc,
          startTimeMs: new Date(runStartedAt).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds,
        });
        const deadline = Date.now() + 10_000;
        while (events.length < judged.length && Date.now() < deadline) {
          await new Promise((resolvePromise) => setTimeout(resolvePromise, 500));
          events = await queryUsageEvents({
            aicc: session.aicc,
            startTimeMs: new Date(runStartedAt).getTime() - 1_000,
            endTimeMs: Date.now() + 1_000,
            taskIds,
          });
        }
        const byTask = indexUsageByTask(events);
        const traces = await queryRouteTraces({
          aicc: session.aicc,
          startTimeMs: new Date(runStartedAt).getTime() - 1_000,
          endTimeMs: Date.now() + 1_000,
          taskIds,
        });
        const traceByTask = new Map(traces.map((trace) => [trace.task_id, trace]));
        const driverByInstance = new Map(inventories.map((inventory) => [
          inventory.provider_instance_name,
          inventory.provider_driver,
        ]));
        for (const item of judged) {
          const judge = item.judge!;
          const event = byTask.get(judge.task_id) ?? [];
          const trace = traceByTask.get(judge.task_id);
          if (event.length !== 1 || !trace?.selected_exact_model || !trace.provider_instance_name) {
            item.status = "failed";
            item.attempts.push({
              attempt: item.attempts.length + 1,
              started_at: new Date().toISOString(),
              elapsed_ms: 0,
              status: "failed",
              failure_class: "usage_failed",
              diagnostic: `Judge attribution failed for task ${judge.task_id}: usage_events=${event.length}, route_trace=${Boolean(trace)}`,
              estimated_cost_usd: 0,
              cost_status: "not_called",
            });
            continue;
          }
          judge.exact_model = trace.selected_exact_model;
          judge.provider_instance = trace.provider_instance_name;
          judge.provider_driver = driverByInstance.get(trace.provider_instance_name);
          judge.distinct_provider_or_family = trace.provider_instance_name !== item.provider_instance ||
            modelFamily(trace.selected_exact_model) !== modelFamily(item.exact_model ?? "");
          const audited = usageEventFinance(event[0]);
          const entry = financialEntries.find((candidate) => candidate.case_id === `${item.case_id}.judge`);
          if (entry) {
            entry.provider_driver = judge.provider_driver ?? "unknown";
            entry.provider_instance = trace.provider_instance_name;
            entry.exact_model = trace.selected_exact_model;
            entry.usage = audited.usage;
            entry.actual_cost_usd = audited.actualCostUsd;
            entry.raw_cost_usd = audited.rawCostUsd;
            entry.credit_applied_usd = audited.creditAppliedUsd;
            entry.cost_status = audited.actualCostUsd === undefined ? "unknown" : "actual";
          }
        }
      } catch (error) {
        cases.push({
          run_id: runId,
          case_id: "t2.judge.durable_attribution",
          layer: "T2",
          status: "failed",
          method: "usage.query/trace.query",
          outbound_message_ids: [],
          artifact_ids: [],
          attempts: [{
            attempt: 1,
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
            status: "failed",
            failure_class: "usage_failed",
            diagnostic: String(error),
            estimated_cost_usd: 0,
            cost_status: "not_called",
          }],
        });
      }
    }
  }
  const cleanupDetails: string[] = [];
  const cleanupResidual: string[] = [];
  const generatedArtifactIds = [...new Set(cases.flatMap((item) => item.artifact_ids))]
    .filter((objId) => !uploadedFixtureIds.includes(objId));
  const removeNamed = async (objId: string): Promise<void> => {
    if (/^(?:mix256|chunk):/i.test(objId)) await ndm.removeChunk({ chunk_id: objId });
    else await ndm.removeObject({ obj_id: objId });
  };
  for (const objId of [...new Set(uploadedFixtureIds)]) {
    try {
      await removeNamed(objId);
    } catch {
      cleanupResidual.push(objId);
    }
  }
  for (const objId of generatedArtifactIds) {
    try {
      await removeNamed(objId);
    } catch {
      cleanupResidual.push(objId);
    }
  }
  cleanupDetails.push(`removed ${new Set(uploadedFixtureIds).size - cleanupResidual.filter((id) => uploadedFixtureIds.includes(id)).length} uploaded fixture object(s)`);
  cleanupDetails.push(`removed ${generatedArtifactIds.length - cleanupResidual.filter((id) => generatedArtifactIds.includes(id)).length} generated output object(s)`);
  if (cleanupResidual.length > 0) {
    cases.push({
      run_id: runId,
      case_id: "t2.cleanup.named_data",
      layer: "T2",
      status: "failed",
      method: "ndm.remove",
      outbound_message_ids: [],
      artifact_ids: cleanupResidual,
      attempts: [{
        attempt: 1,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
        status: "failed",
        failure_class: "cleanup_failed",
        diagnostic: `failed to remove ${cleanupResidual.length} test-scoped Named Data object(s)`,
        estimated_cost_usd: 0,
        cost_status: "not_called",
      }],
    });
  }
  const now = new Date().toISOString();
  const finance = buildFinancialReport({
    entries: financialEntries,
    budgetUsd: options.maxCostUsd,
    plannedMaxCalls: plannedCalls,
    plannedMaxCostUsd: estimatedCost,
    budgetExceeded: costBudget.budgetExceeded(),
  });
  const report: AcceptanceReport = {
    schema_version: 1,
    run_id: runId,
    started_at: runStartedAt,
    finished_at: now,
    commit: await commitId(),
    baseline_revision: baseline.baseline_revision,
    allow_real_model_calls: options.allowRealModelCalls,
    planned_real_calls: plannedCalls,
    actual_real_calls: actualCalls,
    estimated_cost_usd: estimatedCost,
    actual_cost_usd: finance.actual_cost_usd,
    raw_cost_usd: finance.raw_cost_usd,
    credit_applied_usd: finance.credit_applied_usd,
    finance,
    cases,
    model_coverage: matrix.coverage,
    document_format_coverage: matrix.documentCoverage,
    product_defects: [
      ...cases.filter((item) =>
        item.status === "failed" && item.case_id.startsWith("t2.preflight.baseline_mismatch.")
      ).map((item) => defectFromFailure({
        component: "AICC",
        caseReport: item,
        expected: "official Provider capability, AICC inventory declaration, and adapter support are bidirectionally consistent",
        observed: item.attempts.at(-1)?.diagnostic ?? "capability baseline mismatch",
        evidencePaths: [`cases/${item.case_id}.json`, "effective_baseline.json"],
      })),
      ...cases.filter((item) =>
        item.status === "failed" &&
        item.attempts.some((attempt) => attempt.status === "passed") &&
        item.attempts.at(-1)?.failure_class === "usage_failed"
      ).map((item) => defectFromFailure({
        component: "AICC",
        caseReport: item,
        expected: "a successful Provider call has one durable usage event and a matching exact-model/provider route trace",
        observed: item.attempts.at(-1)?.diagnostic ?? "durable usage/route attribution mismatch",
        evidencePaths: [`cases/${item.case_id}.json`, "finance.json"],
      })),
    ],
    targeted_retest_command: targetedRetestCommand(cases, options.configPath, options.timeoutMs),
    cleanup: {
      status: cleanupResidual.length === 0 ? "passed" : "failed",
      details: cleanupDetails,
    },
  };
  const effectiveBaseline = {
    schema_version: 1,
    generated_at: now,
    source_baseline_revision: baseline.baseline_revision,
    selected_provider_instances: selectedInventories.map((inventory) => ({
      provider_driver: inventory.provider_driver,
      provider_instance_name: inventory.provider_instance_name,
      inventory_revision: inventory.inventory_revision ?? null,
      models: inventory.models.map((model) => ({
        exact_model: model.exact_model,
        provider_model_id: model.provider_model_id,
        provider_actual_model_id: model.provider_actual_model_id ?? null,
        aicc_api_types: model.api_types,
        logical_mounts: model.logical_mounts,
      })),
    })),
    model_coverage: matrix.coverage,
    document_format_coverage: matrix.documentCoverage,
    shard: { index: options.shardIndex, count: options.shardCount },
    executed_matrix: selectedCells.map((cell) => ({
      case_id: cell.case_id,
      provider_driver: cell.provider_driver,
      provider_instance: cell.provider_instance,
      exact_model: cell.exact_model,
      api_type: cell.api_type,
      method: cell.method,
      variant: cell.variant ?? "default",
      input_kinds: cell.input_kinds,
      output_kinds: cell.output_kinds,
      resource_representation: cell.resource_representation ?? null,
      document_format: cell.document_format ?? null,
      normalized_status: cell.baseline_status,
      source_urls: cell.source_urls,
    })),
  };
  return { report, effectiveBaseline };
}

function targetedRetestCommand(cases: CaseReport[], configPath: string, timeoutMs: number): string {
  const selected = cases.filter((item) =>
    item.layer === "T2" && item.status === "failed" &&
    !item.case_id.startsWith("t2.preflight.") && item.provider_driver
  ).slice(0, 20);
  if (selected.length === 0) {
    return `pnpm run acceptance:gateway -- --config ${configPath} --allow-credential-mutation --timeout-ms ${timeoutMs} --no-real-model-calls`;
  }
  const providers = [...new Set(selected.flatMap((item) =>
    item.provider_driver ? [item.provider_driver] : []
  ))];
  return [
    "pnpm run acceptance:gateway --",
    `--config ${configPath}`,
    "--allow-credential-mutation",
    `--timeout-ms ${timeoutMs}`,
    ...providers.map((provider) => `--provider ${provider}`),
    ...selected.map((item) => `--case ${item.case_id}`),
  ].join(" ");
}

main().catch((error) => {
  console.error(`AICC gateway acceptance failed: ${String(error)}`);
  Deno.exitCode = 1;
});
