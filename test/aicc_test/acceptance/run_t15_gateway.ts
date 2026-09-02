import { spawn, type ChildProcess } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loginGateway, type GatewaySession } from "./gateway.ts";
import { validateCaseManifest } from "./manifest.ts";
import {
  buildT15Manifest,
  loadProviderProtocolCatalog,
  protocolContract,
  type ProviderProtocolCatalog,
} from "./provider_protocol_contracts.ts";
import { runPreflight } from "./preflight.ts";
import type { AcceptanceCase, ProviderInventory, ProviderModel } from "./types.ts";

type Options = {
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  appId: string;
  mockBaseUrl: string;
  mockControlUrl: string;
  mockPort: number;
  startLocalMock: boolean;
  configAllowsMutation: boolean;
  cliAllowsMutation: boolean;
  providers: string[];
  caseIds: string[];
  reportDir: string;
  timeoutMs: number;
  providerMinIntervalMs: number;
};

type CaseResult = {
  case_id: string;
  provider_driver: string | null;
  protocol_contract_id?: string;
  scenario: string | null;
  status: "passed" | "failed";
  diagnostic?: string;
  captured_requests: number;
};

const here = dirname(fileURLToPath(import.meta.url));

function required(args: string[], index: number, name: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function options(args: string[]): Options {
  const parsed: Options = {
    gatewayUrl: process.env.BUCKYOS_TEST_GATEWAY_URL ?? "",
    sessionToken: process.env.BUCKYOS_APPCLIENT_SESSION_TOKEN,
    username: process.env.BUCKYOS_TEST_USERNAME,
    password: process.env.BUCKYOS_TEST_PASSWORD,
    appId: process.env.BUCKYOS_TEST_APP_ID ?? "aicc-tests",
    mockBaseUrl: "",
    mockControlUrl: "",
    mockPort: 18081,
    startLocalMock: false,
    configAllowsMutation: process.env.AICC_T15_ALLOW_CONFIG_MUTATION === "true",
    cliAllowsMutation: false,
    providers: [],
    caseIds: [],
    reportDir: "reports/acceptance",
    timeoutMs: 120_000,
    providerMinIntervalMs: 50,
  };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--gateway-url") parsed.gatewayUrl = required(args, index++, arg);
    else if (arg === "--session-token") parsed.sessionToken = required(args, index++, arg);
    else if (arg === "--username") parsed.username = required(args, index++, arg);
    else if (arg === "--password") parsed.password = required(args, index++, arg);
    else if (arg === "--provider") parsed.providers.push(required(args, index++, arg));
    else if (arg === "--case") parsed.caseIds.push(required(args, index++, arg));
    else if (arg === "--mock-base-url") parsed.mockBaseUrl = required(args, index++, arg);
    else if (arg === "--mock-control-url") parsed.mockControlUrl = required(args, index++, arg);
    else if (arg === "--mock-port") parsed.mockPort = Number(required(args, index++, arg));
    else if (arg === "--timeout-ms") parsed.timeoutMs = Number(required(args, index++, arg));
    else if (arg === "--provider-min-interval-ms") parsed.providerMinIntervalMs = Number(required(args, index++, arg));
    else if (arg === "--report-dir") parsed.reportDir = required(args, index++, arg);
    else if (arg === "--start-local-mock") parsed.startLocalMock = true;
    else if (arg === "--allow-config-mutation") parsed.cliAllowsMutation = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  parsed.gatewayUrl = parsed.gatewayUrl.replace(/\/+$/, "");
  if (!parsed.gatewayUrl) throw new Error("--gateway-url or BUCKYOS_TEST_GATEWAY_URL is required");
  if (!Number.isInteger(parsed.mockPort) || parsed.mockPort < 1 || parsed.mockPort > 65535) {
    throw new Error("--mock-port must be 1..65535");
  }
  if (!Number.isFinite(parsed.timeoutMs) || parsed.timeoutMs < 1_000) throw new Error("--timeout-ms is invalid");
  if (!Number.isFinite(parsed.providerMinIntervalMs) || parsed.providerMinIntervalMs < 0) {
    throw new Error("--provider-min-interval-ms is invalid");
  }
  if (!parsed.mockBaseUrl) parsed.mockBaseUrl = `http://127.0.0.1:${parsed.mockPort}`;
  if (!parsed.mockControlUrl) parsed.mockControlUrl = parsed.mockBaseUrl;
  parsed.mockBaseUrl = parsed.mockBaseUrl.replace(/\/+$/, "");
  parsed.mockControlUrl = parsed.mockControlUrl.replace(/\/+$/, "");
  if (!parsed.configAllowsMutation || !parsed.cliAllowsMutation) {
    throw new Error(
      "T1.5 requires AICC_T15_ALLOW_CONFIG_MUTATION=true and --allow-config-mutation; temporary Provider instances are deleted in cleanup",
    );
  }
  return parsed;
}

async function waitMock(baseUrl: string, timeoutMs = 15_000): Promise<void> {
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
  throw new Error(`T1.5 mock is unreachable: ${last}`);
}

async function selectMock(baseUrl: string, testCase: AcceptanceCase): Promise<void> {
  const response = await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider_driver: testCase.provider_driver,
      contract_id: testCase.protocol_contract_id,
      scenario: testCase.mock_scenario,
    }),
  });
  if (!response.ok) throw new Error(`mock selection failed: ${response.status} ${await response.text()}`);
}

async function capturedRequests(baseUrl: string): Promise<Array<{ validation_errors?: unknown[]; pathname?: string; body?: unknown }>> {
  const response = await fetch(`${baseUrl}/__mock/requests`);
  if (!response.ok) throw new Error(`mock request audit failed: ${response.status} ${await response.text()}`);
  const value = await response.json() as { requests?: unknown };
  if (!Array.isArray(value.requests)) throw new Error("mock request audit is malformed");
  return value.requests as Array<{ validation_errors?: unknown[]; pathname?: string; body?: unknown }>;
}

const PROFILE_IDS: Record<string, string> = {
  openai: "openai",
  claude: "claude",
  "google-gemini": "gemini",
  fal: "fal",
  minimax: "minimax",
  openrouter: "openrouter",
  "sn-ai-provider": "sn",
};

function providerEndpoint(driver: string, mockBaseUrl: string): string {
  const suffix: Record<string, string> = {
    openai: "/v1",
    claude: "/v1",
    "google-gemini": "/v1beta",
    openrouter: "/api/v1",
    "sn-ai-provider": "/v1",
  };
  return `${mockBaseUrl}${suffix[driver] ?? ""}`;
}

function credentialType(driver: string): string {
  return ["claude", "google-gemini", "fal"].includes(driver) ? "api_key" : "bearer";
}

async function addProvider(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  driver: string,
  instance: string,
  mockBaseUrl: string,
): Promise<void> {
  const provider = catalog.providers.find((candidate) => candidate.provider_driver === driver);
  if (!provider) throw new Error(`unknown T1.5 Provider ${driver}`);
  await session.aicc.call("provider.add", {
    provider_instance_name: instance,
    provider_type: "cloud_api",
    provider_profile_id: PROFILE_IDS[driver],
    protocol_adapter_id: provider.contracts[0].protocol_adapter_id,
    endpoint: providerEndpoint(driver, mockBaseUrl),
    credentials: { type: credentialType(driver), secret: `t15-mock-${driver}` },
    auto_sync_models: true,
    enabled: true,
  });
}

function inventories(value: unknown): ProviderInventory[] {
  const providers = value && typeof value === "object" ? (value as { providers?: unknown }).providers : undefined;
  if (!Array.isArray(providers)) throw new Error("models.list.providers must be an array");
  return providers as ProviderInventory[];
}

async function waitInventory(session: GatewaySession, instance: string, timeoutMs: number): Promise<ProviderInventory> {
  const deadline = Date.now() + timeoutMs;
  let last: ProviderInventory[] = [];
  while (Date.now() < deadline) {
    last = inventories(await session.aicc.call("models.list", {}));
    const found = last.find((inventory) => inventory.provider_instance_name === instance);
    if (found && found.models.length > 0) return found;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 300));
  }
  throw new Error(`Provider inventory ${instance} did not converge; found=${last.map((item) => item.provider_instance_name).join(",")}`);
}

const MODEL_IDS: Record<string, Record<string, string>> = {
  openai: { llm: "gpt-5.4", "vision.ocr": "gpt-5.4", "vision.caption": "gpt-5.4", "agent.computer_use": "gpt-5.4", "embedding.text": "text-embedding-3-small", "image.txt2img": "gpt-image-1", "image.img2img": "gpt-image-1", "image.inpaint": "gpt-image-1", "audio.tts": "tts-1", "audio.asr": "whisper-1", "video.txt2video": "sora-2", "video.img2video": "sora-2" },
  claude: { llm: "claude-3-7-sonnet-20250219", "vision.ocr": "claude-3-7-sonnet-20250219", "vision.caption": "claude-3-7-sonnet-20250219" },
  "google-gemini": { llm: "gemini-3.5-pro", "vision.ocr": "gemini-3.5-pro", "vision.caption": "gemini-3.5-pro", "vision.detect": "gemini-3.5-pro", "vision.segment": "gemini-3.5-pro", "audio.asr": "gemini-3.5-pro", "agent.computer_use": "gemini-3.5-pro", "embedding.text": "gemini-embedding-2", "embedding.multimodal": "gemini-embedding-2" },
  fal: { "image.upscale": "fal-ai/esrgan", "image.bg_remove": "fal-ai/imageutils/rembg", "audio.enhance": "fal-ai/deepfilternet3", "video.upscale": "fal-ai/video-upscaler" },
  minimax: { llm: "MiniMax-M2.5", "audio.tts": "speech-2.8-hd", "image.txt2img": "image-01", "image.img2img": "image-01", "video.txt2video": "MiniMax-Hailuo-02", "video.img2video": "MiniMax-Hailuo-02", "audio.music": "music-2.0" },
  openrouter: { llm: "openai/gpt-5.4" },
  "sn-ai-provider": { llm: "gpt-5.4", "vision.ocr": "gpt-5.4", "vision.caption": "gpt-5.4", "agent.computer_use": "gpt-5.4" },
};

function exactModel(testCase: AcceptanceCase, inventory: ProviderInventory): string {
  if (testCase.model_selector?.kind === "exact") return testCase.model_selector.value;
  const id = MODEL_IDS[testCase.provider_driver ?? ""]?.[testCase.api_type ?? ""];
  const model = inventory.models.find((candidate) => candidate.provider_model_id === id) ??
    inventory.models.find((candidate) => candidate.api_types.includes(testCase.api_type ?? ""));
  if (!model) throw new Error(`no exact model for ${testCase.provider_driver}/${testCase.api_type}`);
  return model.exact_model;
}

function resource(mime: string): Record<string, unknown> {
  return { kind: "base64", mime, data_base64: Buffer.from("t15-fixture").toString("base64") };
}

export function buildT15TypedParams(apiType: string, exactModelId: string, runId: string): Record<string, unknown> {
  const common = { exact_model: exactModelId, idempotency_key: `${runId}:${apiType}` };
  switch (apiType) {
    case "llm": return { ...common, messages: [{ role: "user", content: [{ type: "text", text: "Return BUCKYOS-AICC-4827." }] }], max_output_tokens: 32 };
    case "embedding.text": return { ...common, items: [{ type: "text", id: "item-1", text: "BUCKYOS-AICC-4827" }] };
    case "embedding.multimodal": return { ...common, items: [{ id: "item-1", text: "marker", image: resource("image/png") }] };
    case "image.txt2img": return { ...common, prompt: "A blue square marked 4827" };
    case "image.img2img": return { ...common, prompt: "Preserve the image", image: resource("image/png") };
    case "image.inpaint": return { ...common, prompt: "Fill the mask", image: resource("image/png"), mask: resource("image/png") };
    case "image.upscale": return { ...common, image: resource("image/png"), scale: 2 };
    case "image.bg_remove": return { ...common, image: resource("image/png") };
    case "vision.ocr": return { ...common, image: resource("image/png"), prompt: "Read marker 4827" };
    case "vision.caption": return { ...common, image: resource("image/png"), prompt: "Caption the image" };
    case "vision.detect": return { ...common, image: resource("image/png"), prompt: "Detect objects" };
    case "vision.segment": return { ...common, image: resource("image/png"), prompt: "Segment objects" };
    case "audio.tts": return { ...common, text: "BuckyOS 4827", voice: "alloy" };
    case "audio.asr": return { ...common, audio: resource("audio/wav") };
    case "audio.music": return { ...common, prompt: "A short calm instrumental" };
    case "audio.enhance": return { ...common, audio: resource("audio/wav"), operation: "denoise" };
    case "video.txt2video": return { ...common, prompt: "A paper plane moves across a desk" };
    case "video.img2video": return { ...common, prompt: "Subtle motion", image: resource("image/png") };
    case "video.video2video": return { ...common, video: resource("video/mp4"), prompt: "Preserve motion" };
    case "video.extend": return { ...common, video: resource("video/mp4"), duration_seconds: 2 };
    case "video.upscale": return { ...common, video: resource("video/mp4") };
    case "agent.computer_use": return { ...common, task: "Read the page title", environment: "browser" };
    default: throw new Error(`no T1.5 typed request fixture for ${apiType}`);
  }
}

async function terminal(session: GatewaySession, value: unknown, timeoutMs: number): Promise<unknown> {
  if (!value || typeof value !== "object") throw new Error("typed response must be an object");
  const response = value as Record<string, unknown>;
  if (response.status === "failed") throw new Error(`AICC returned failed: ${JSON.stringify(response.result ?? response)}`);
  if (response.status === "succeeded") return response;
  if (response.status !== "running" || typeof response.task_id !== "string") {
    throw new Error(`unexpected typed response: ${JSON.stringify(response)}`);
  }
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const raw = await session.taskManager.call("get_task", { task_id: response.task_id }) as Record<string, unknown>;
    const task = raw.task && typeof raw.task === "object" ? raw.task as Record<string, unknown> : raw;
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") throw new Error(`task ended ${String(task.outcome)}: ${JSON.stringify(task.error ?? {})}`);
      return task;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`task ${response.task_id} timed out`);
}

async function executeCase(
  session: GatewaySession,
  testCase: AcceptanceCase,
  inventory: ProviderInventory,
  controlUrl: string,
  runId: string,
  timeoutMs: number,
): Promise<CaseResult> {
  await selectMock(controlUrl, testCase);
  let failed: unknown;
  try {
    const result = await session.aicc.call(
      testCase.method,
      buildT15TypedParams(testCase.api_type!, exactModel(testCase, inventory), runId),
    );
    await terminal(session, result, timeoutMs);
  } catch (error) {
    failed = error;
  }
  const requests = await capturedRequests(controlUrl);
  const validationErrors = requests.flatMap((request) => request.validation_errors ?? []);
  const expectsFailure = Boolean(testCase.expected_error_class);
  const diagnostics: string[] = [];
  if (requests.length === 0) diagnostics.push("Provider mock received no request");
  if (validationErrors.length > 0) diagnostics.push(`wire contract violations: ${JSON.stringify(validationErrors)}`);
  const selectedModel = inventory.models.find((model) =>
    testCase.model_selector?.kind === "exact" && model.exact_model === testCase.model_selector.value
  );
  if (selectedModel?.provider_actual_model_id) {
    const providerRequest = requests.at(-1);
    const body = providerRequest?.body && typeof providerRequest.body === "object"
      ? providerRequest.body as Record<string, unknown>
      : {};
    if (body.model !== selectedModel.provider_actual_model_id &&
        !providerRequest?.pathname?.includes(encodeURIComponent(selectedModel.provider_actual_model_id))) {
      diagnostics.push(`variant model was not lowered to ${selectedModel.provider_actual_model_id}`);
    }
    for (const [name, expected] of Object.entries(selectedModel.provider_options ?? {})) {
      if (JSON.stringify(body[name]) !== JSON.stringify(expected)) {
        diagnostics.push(`variant Provider option ${name} was not lowered to ${JSON.stringify(expected)}`);
      }
    }
  }
  if (expectsFailure && !failed) diagnostics.push("official Provider error fixture was not mapped to a failed AICC call/task");
  if (!expectsFailure && failed) diagnostics.push(String(failed));
  return {
    case_id: testCase.case_id,
    provider_driver: testCase.provider_driver,
    protocol_contract_id: testCase.protocol_contract_id,
    scenario: testCase.mock_scenario,
    status: diagnostics.length === 0 ? "passed" : "failed",
    diagnostic: diagnostics.length > 0 ? diagnostics.join("; ") : undefined,
    captured_requests: requests.length,
  };
}

function variantCells(catalog: ProviderProtocolCatalog, inventory: ProviderInventory) {
  return inventory.models.filter((model) =>
    Boolean(model.provider_actual_model_id) || model.provider_model_id.includes(":")
  ).flatMap((model) => model.api_types.map((apiType) => {
    const contract = catalog.providers.find((provider) => provider.provider_driver === inventory.provider_driver)
      ?.contracts.find((candidate) => candidate.api_types.includes(apiType));
    if (!contract) return undefined;
    return { provider_driver: inventory.provider_driver, contract_id: contract.id, api_type: apiType, model };
  })).filter((value): value is { provider_driver: string; contract_id: string; api_type: string; model: ProviderModel } => Boolean(value));
}

async function main(): Promise<void> {
  const input = options(process.argv.slice(2));
  await runPreflight();
  const catalog = await loadProviderProtocolCatalog();
  const selectedProviders = input.providers.length > 0 ? new Set(input.providers) : new Set(catalog.providers.map((provider) => provider.provider_driver));
  for (const driver of selectedProviders) {
    if (!catalog.providers.some((provider) => provider.provider_driver === driver)) throw new Error(`unknown --provider ${driver}`);
  }
  let mockProcess: ChildProcess | undefined;
  if (input.startLocalMock) {
    mockProcess = spawn(process.execPath, ["--experimental-strip-types", join(here, "t15_mock_provider.ts"), "--port", String(input.mockPort)], {
      stdio: ["ignore", "inherit", "inherit"],
    });
  }
  const runId = `t15-${new Date().toISOString().replace(/[^0-9]/g, "").slice(0, 14)}-${process.pid}`;
  const created: string[] = [];
  const results: CaseResult[] = [];
  let session: GatewaySession | undefined;
  try {
    await waitMock(input.mockControlUrl);
    session = await loginGateway({
      gatewayUrl: input.gatewayUrl,
      sessionToken: input.sessionToken,
      username: input.username,
      password: input.password,
      appId: input.appId,
    });
    process.stdout.write(`${JSON.stringify({
      layer: "T1.5",
      providers: [...selectedProviders],
      real_provider_calls: 0,
      estimated_cost_usd: 0,
      global_concurrency: 1,
      provider_concurrency: 1,
      provider_min_interval_ms: input.providerMinIntervalMs,
    }, null, 2)}\n`);
    for (const driver of selectedProviders) {
      const provider = catalog.providers.find((candidate) => candidate.provider_driver === driver)!;
      const bootstrap = buildT15Manifest(catalog).find((testCase) =>
        testCase.provider_driver === driver && testCase.protocol_contract_id === provider.contracts[0].id && testCase.mock_scenario === "success"
      )!;
      await selectMock(input.mockControlUrl, bootstrap);
      const instance = `${runId}-${driver}`.toLowerCase().replace(/[^a-z0-9_-]+/g, "-");
      await addProvider(session, catalog, driver, instance, input.mockBaseUrl);
      created.push(instance);
      const inventory = await waitInventory(session, instance, input.timeoutMs);
      const manifest = validateCaseManifest(buildT15Manifest(catalog, variantCells(catalog, inventory)))
        .filter((testCase) => testCase.provider_driver === driver)
        .filter((testCase) => input.caseIds.length === 0 || input.caseIds.includes(testCase.case_id));
      for (const [index, testCase] of manifest.entries()) {
        if (index > 0 && input.providerMinIntervalMs > 0) {
          await new Promise((resolvePromise) => setTimeout(resolvePromise, input.providerMinIntervalMs));
        }
        testCase.provider_instance = instance;
        testCase.expected_provider_instance = instance;
        results.push(await executeCase(session, testCase, inventory, input.mockControlUrl, runId, input.timeoutMs));
      }
      await session.aicc.call("provider.delete", { provider_instance_name: instance });
      created.splice(created.indexOf(instance), 1);
    }
  } finally {
    if (session) {
      for (const providerInstanceName of created.reverse()) {
        try {
          await session.aicc.call("provider.delete", { provider_instance_name: providerInstanceName });
        } catch (error) {
          results.push({ case_id: `t1.5.cleanup.${providerInstanceName}`, provider_driver: null, scenario: null, status: "failed", diagnostic: String(error), captured_requests: 0 });
        }
      }
    }
    mockProcess?.kill("SIGTERM");
  }
  await mkdir(input.reportDir, { recursive: true });
  const reportPath = resolve(input.reportDir, `${runId}.json`);
  const report = {
    run_id: runId,
    layer: "T1.5",
    protocol_evidence_revision: catalog.revision,
    official_evidence_checked_at: catalog.checked_at,
    real_provider_calls: 0,
    estimated_cost_usd: 0,
    limits: { global_concurrency: 1, provider_concurrency: 1, provider_min_interval_ms: input.providerMinIntervalMs },
    providers: [...selectedProviders],
    totals: {
      cases: results.length,
      passed: results.filter((result) => result.status === "passed").length,
      failed: results.filter((result) => result.status === "failed").length,
    },
    cases: results,
  };
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  process.stdout.write(`${JSON.stringify({ report: reportPath, ...report.totals }, null, 2)}\n`);
  if (report.totals.failed > 0) process.exitCode = 1;
}

if (process.argv[1] && resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1])) {
  main().catch((error) => {
    process.stderr.write(`T1.5 acceptance failed: ${String(error)}\n`);
    process.exitCode = 1;
  });
}
