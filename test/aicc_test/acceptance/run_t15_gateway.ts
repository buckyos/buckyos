import { type ChildProcess, spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildFinancialReport } from "./finance.ts";
import {
  type GatewaySession,
  loginGateway,
  loginSudoSystemConfig,
  type RpcClient,
} from "./gateway.ts";
import {
  buildT15OpenAiRules,
  t15OpenAiTombstone,
} from "./cloud_update_cases.ts";
import { CloudUpdateFixtureService } from "./cloud_update_fixture_service.ts";
import {
  backupCloudUpdateConfig,
  disableCloudUpdate,
  setCloudUpdateSource,
  waitCloudUpdateConverged,
} from "./cloud_update_transaction.ts";
import { validateCaseManifest } from "./manifest.ts";
import {
  buildT15Manifest,
  loadProviderProtocolCatalog,
  protocolContract,
  type ProviderProtocolCatalog,
  selectOfficialModels,
} from "./provider_protocol_contracts.ts";
import { defectFromFailure, writeReport } from "./report.ts";
import { inventoriesFromModelsList } from "./inventory.ts";
import { runPreflight } from "./preflight.ts";
import { withMockQuotaTruth } from "./quota_transaction.ts";
import type {
  AcceptanceCase,
  AcceptanceReport,
  CaseReport,
  ProviderInventory,
  ProviderModel,
} from "./types.ts";

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
  ndnGatewayBinary: string;
  ndnNamedStoreConfigPath: string;
  ndnGatewayControlUrl: string;
  ndnSystemRoot: string;
  cloudCacheRoot: string;
};

type CaseResult = {
  case_id: string;
  provider_driver: string | null;
  provider_instance?: string;
  exact_model?: string;
  api_type?: string;
  method: string;
  protocol_contract_id?: string;
  scenario: string | null;
  status: "passed" | "failed";
  diagnostic?: string;
  captured_requests: number;
  started_at: string;
  elapsed_ms: number;
};

type T15TypedOptions = {
  sessionId?: string;
  historyMessage?: Record<string, unknown>;
  foreignProviderState?: {
    provider: string;
    value: Record<string, unknown>;
  };
};

const here = dirname(fileURLToPath(import.meta.url));

async function commitId(): Promise<string> {
  return await new Promise((resolvePromise) => {
    const child = spawn("git", ["rev-parse", "HEAD"], {
      cwd: resolve(here, "../../.."),
      stdio: ["ignore", "pipe", "ignore"],
    });
    let output = "";
    child.stdout?.on("data", (chunk) => output += String(chunk));
    child.once("error", () => resolvePromise("unknown"));
    child.once(
      "close",
      (code) =>
        resolvePromise(code === 0 && output.trim() ? output.trim() : "unknown"),
    );
  });
}

function required(args: string[], index: number, name: string): string {
  const value = args[index + 1]?.trim();
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
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
    ndnGatewayBinary: process.env.AICC_NDN_GATEWAY_BINARY ??
      "/opt/buckyos/bin/cyfs-gateway/cyfs_gateway",
    ndnNamedStoreConfigPath: process.env.AICC_NDN_NAMED_STORE_CONFIG ??
      "/opt/buckyos/etc/named_store.json",
    ndnGatewayControlUrl: process.env.AICC_NDN_GATEWAY_CONTROL_URL ??
      "http://127.0.0.1:13451",
    ndnSystemRoot: process.env.AICC_NDN_SYSTEM_ROOT ?? "/opt/buckyos",
    cloudCacheRoot: process.env.AICC_CLOUD_CACHE_ROOT ??
      "/opt/buckyos/data/aicc/driver_metadata/cloud",
  };
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--") continue;
    else if (arg === "--gateway-url") {
      parsed.gatewayUrl = required(args, index++, arg);
    } else if (arg === "--session-token") {
      parsed.sessionToken = required(args, index++, arg);
    } else if (arg === "--username") {
      parsed.username = required(args, index++, arg);
    } else if (arg === "--password") {
      parsed.password = required(args, index++, arg);
    } else if (arg === "--provider") {
      parsed.providers.push(required(args, index++, arg));
    } else if (arg === "--case") {
      parsed.caseIds.push(required(args, index++, arg));
    } else if (arg === "--mock-base-url") {
      parsed.mockBaseUrl = required(args, index++, arg);
    } else if (arg === "--mock-control-url") {
      parsed.mockControlUrl = required(args, index++, arg);
    } else if (arg === "--mock-port") {
      parsed.mockPort = Number(required(args, index++, arg));
    } else if (arg === "--timeout-ms") {
      parsed.timeoutMs = Number(required(args, index++, arg));
    } else if (arg === "--provider-min-interval-ms") {
      parsed.providerMinIntervalMs = Number(required(args, index++, arg));
    } else if (arg === "--report-dir") {
      parsed.reportDir = required(args, index++, arg);
    } else if (arg === "--start-local-mock") parsed.startLocalMock = true;
    else if (arg === "--allow-config-mutation") parsed.cliAllowsMutation = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  parsed.gatewayUrl = parsed.gatewayUrl.replace(/\/+$/, "");
  if (!parsed.gatewayUrl) {
    throw new Error("--gateway-url or BUCKYOS_TEST_GATEWAY_URL is required");
  }
  if (
    !Number.isInteger(parsed.mockPort) || parsed.mockPort < 1 ||
    parsed.mockPort > 65535
  ) {
    throw new Error("--mock-port must be 1..65535");
  }
  if (!Number.isFinite(parsed.timeoutMs) || parsed.timeoutMs < 1_000) {
    throw new Error("--timeout-ms is invalid");
  }
  if (
    !Number.isFinite(parsed.providerMinIntervalMs) ||
    parsed.providerMinIntervalMs < 0
  ) {
    throw new Error("--provider-min-interval-ms is invalid");
  }
  if (!parsed.mockBaseUrl) {
    parsed.mockBaseUrl = `http://127.0.0.1:${parsed.mockPort}`;
  }
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

async function selectMock(
  baseUrl: string,
  testCase: AcceptanceCase,
  selectionSeed?: string,
): Promise<void> {
  const response = await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider_driver: testCase.provider_driver,
      contract_id: testCase.protocol_contract_id,
      api_type: testCase.api_type,
      scenario: testCase.mock_scenario,
      selection_seed: selectionSeed,
    }),
  });
  if (!response.ok) {
    throw new Error(
      `mock selection failed: ${response.status} ${await response.text()}`,
    );
  }
}

async function resetMock(baseUrl: string): Promise<void> {
  const response = await fetch(`${baseUrl}/__mock/reset`, { method: "POST" });
  if (!response.ok) {
    throw new Error(
      `mock reset failed: ${response.status} ${await response.text()}`,
    );
  }
}

async function capturedRequests(baseUrl: string): Promise<
  Array<{
    validation_errors?: unknown[];
    pathname?: string;
    body?: unknown;
    async_step?: "poll" | "result" | "cancel";
  }>
> {
  const response = await fetch(`${baseUrl}/__mock/requests`);
  if (!response.ok) {
    throw new Error(
      `mock request audit failed: ${response.status} ${await response.text()}`,
    );
  }
  const value = await response.json() as { requests?: unknown };
  if (!Array.isArray(value.requests)) {
    throw new Error("mock request audit is malformed");
  }
  return value.requests as Array<
    { validation_errors?: unknown[]; pathname?: string; body?: unknown }
  >;
}

async function addProvider(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  driver: string,
  instance: string,
  mockBaseUrl: string,
): Promise<void> {
  const provider = catalog.providers.find((candidate) =>
    candidate.provider_driver === driver
  );
  if (!provider) throw new Error(`unknown T1.5 Provider ${driver}`);
  const catalogOnly = new Set(["fal", "glm", "doubao", "qwen"]);
  const catalogModels = provider.official_first_party_model_ids ??
    Object.fromEntries(
      Object.entries(provider.test_model_ids).map((
        [apiType, modelId],
      ) => [apiType, [modelId]]),
    );
  const catalogDiscovery = catalogOnly.has(driver)
    ? {
      revision: `t15-${driver}-${instance}`,
      discovered_at_ms: Date.now(),
      health: "healthy",
      models: Object.entries(catalogModels).flatMap(([apiType, modelIds]) =>
        modelIds.map((providerModelId) => ({
          provider_model_id: providerModelId,
          api_types: [apiType],
          remote_methods: provider.contracts
            .filter((contract) => contract.api_types.includes(apiType))
            .map((contract) => contract.operation),
          availability: "available",
          deprecated: false,
        }))
      ),
    }
    : undefined;
  const draft = {
    provider_instance_name: instance,
    provider_type: "cloud_api",
    provider_profile_id: provider.provider_profile_id,
    protocol_adapter_id: provider.contracts[0].protocol_adapter_id,
    base_url: `${mockBaseUrl}${provider.endpoint_path}`,
    credentials: { api_token: { locked: `t15-mock-${driver}` } },
    ...provider.instance_fields,
    ...(catalogDiscovery ? { discovery: catalogDiscovery } : {}),
    auto_sync_models: true,
  };
  try {
    await session.aicc.call("provider.add", draft);
  } catch (error) {
    const validation = await session.aicc.call(
      "provider.validate",
      draft,
    ) as Record<string, unknown>;
    throw new Error(
      `${driver} add failed: ${String(error)}; validation=${
        JSON.stringify(validation)
      }`,
    );
  }
}

async function addCustomProvider(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  driver: string,
  instance: string,
  mockBaseUrl: string,
  runId: string,
): Promise<void> {
  const provider = catalog.providers.find((candidate) =>
    candidate.provider_driver === driver
  );
  if (!provider) {
    throw new Error(`unknown T1.5 custom Provider protocol ${driver}`);
  }
  const selected = selectOfficialModels(catalog, driver, runId);
  const modelApiTypes = new Map<
    string,
    { apiTypes: string[]; remoteMethods: string[] }
  >();
  for (const [apiType, modelId] of Object.entries(selected)) {
    const model = modelApiTypes.get(modelId) ??
      { apiTypes: [], remoteMethods: [] };
    model.apiTypes.push(apiType);
    const operation = provider.contracts.find((contract) =>
      contract.api_types.includes(apiType)
    )?.operation;
    if (!operation) {
      throw new Error(`${driver} has no protocol operation for ${apiType}`);
    }
    model.remoteMethods.push(operation);
    modelApiTypes.set(modelId, model);
  }
  const draft = {
    provider_instance_name: instance,
    provider_type: "cloud_api",
    provider_profile_id: "custom",
    protocol_adapter_id: provider.contracts[0].protocol_adapter_id,
    base_url: `${mockBaseUrl}${provider.endpoint_path}`,
    credentials: { api_token: { locked: `t15-mock-custom-${driver}` } },
    discovery: {
      revision: `t15-custom-${driver}-${runId}`,
      discovered_at_ms: Date.now(),
      health: "healthy",
      models: [...modelApiTypes].map(([provider_model_id, model]) => ({
        provider_model_id,
        api_types: model.apiTypes,
        remote_methods: [...new Set(model.remoteMethods)],
        availability: "available",
        deprecated: false,
      })),
    },
    auto_sync_models: true,
  };
  const validation = await session.aicc.call(
    "provider.validate",
    draft,
  ) as Record<string, unknown>;
  if (
    (Array.isArray(validation.errors) && validation.errors.length > 0) ||
    (Array.isArray(validation.error_details) &&
      validation.error_details.length > 0)
  ) {
    throw new Error(
      `custom ${driver} validation failed: ${JSON.stringify(validation)}`,
    );
  }
  try {
    await session.aicc.call("provider.add", draft);
  } catch (error) {
    const repeatedValidation = await session.aicc.call(
      "provider.validate",
      draft,
    ) as Record<string, unknown>;
    throw new Error(
      `custom ${driver} add failed: ${String(error)}; validation=${
        JSON.stringify(repeatedValidation)
      }`,
    );
  }
}

function inventories(value: unknown): ProviderInventory[] {
  return inventoriesFromModelsList(value);
}

async function waitInventory(
  session: GatewaySession,
  instance: string,
  timeoutMs: number,
): Promise<ProviderInventory> {
  const deadline = Date.now() + timeoutMs;
  let last: ProviderInventory[] = [];
  while (Date.now() < deadline) {
    last = inventories(await session.aicc.call("models.list", {}));
    const found = last.find((inventory) =>
      inventory.provider_instance_name === instance
    );
    if (found && found.models.length > 0) return found;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 300));
  }
  throw new Error(
    `Provider inventory ${instance} did not converge; found=${
      last.map((item) => item.provider_instance_name).join(",")
    }`,
  );
}

async function waitInventoryAbsent(
  session: GatewaySession,
  instance: string,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let last: ProviderInventory[] = [];
  while (Date.now() < deadline) {
    last = inventories(await session.aicc.call("models.list", {}));
    if (
      !last.some((inventory) => inventory.provider_instance_name === instance)
    ) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 300));
  }
  throw new Error(`deleted Provider inventory ${instance} is still present`);
}

async function refreshProviderInventory(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  driver: string,
  instance: string,
  controlUrl: string,
  runId: string,
  timeoutMs: number,
): Promise<ProviderInventory> {
  const provider = catalog.providers.find((candidate) =>
    candidate.provider_driver === driver
  );
  if (!provider) throw new Error(`unknown T1.5 Provider ${driver}`);
  const bootstrap = buildT15Manifest(catalog).find((testCase) =>
    testCase.provider_driver === driver &&
    testCase.protocol_contract_id === provider.contracts[0].id &&
    testCase.mock_scenario === "success"
  );
  if (!bootstrap) throw new Error(`missing T1.5 bootstrap case for ${driver}`);
  await selectMock(controlUrl, bootstrap, `${runId}:inventory:${driver}`);
  await session.aicc.call("provider.refresh_models", {
    provider_instance_name: instance,
  });
  return await waitInventory(session, instance, timeoutMs);
}

async function refreshLogin(
  input: Options,
  session: GatewaySession,
): Promise<GatewaySession> {
  if (!input.username || !input.password) return session;
  return await loginGateway({
    gatewayUrl: input.gatewayUrl,
    username: input.username,
    password: input.password,
    appId: input.appId,
  });
}

function exactModel(
  catalog: ProviderProtocolCatalog,
  testCase: AcceptanceCase,
  inventory: ProviderInventory,
): string {
  if (testCase.model_selector?.kind === "exact") {
    return testCase.model_selector.value;
  }
  const id = catalog.providers.find((provider) =>
    provider.provider_driver === testCase.provider_driver
  )
    ?.test_model_ids[testCase.api_type ?? ""];
  const model =
    inventory.models.find((candidate) =>
      candidate.provider_model_id === id &&
      candidate.api_types.includes(testCase.api_type ?? "")
    ) ??
      inventory.models.find((candidate) =>
        candidate.api_types.includes(testCase.api_type ?? "")
      );
  if (!model) {
    throw new Error(
      `no exact model for ${testCase.provider_driver}/${testCase.api_type}`,
    );
  }
  return model.exact_model;
}

function providerStateNamespace(providerDriver: string): string {
  if (providerDriver === "google-gemini") return "gemini";
  return providerDriver;
}

function resource(mime: string): Record<string, unknown> {
  return {
    kind: "base64",
    mime,
    data_base64: Buffer.from("t15-fixture").toString("base64"),
  };
}

export function buildT15TypedParams(
  apiType: string,
  exactModelId: string,
  runId: string,
  executionMode: AcceptanceCase["execution_mode"] = "immediate",
  requestKey = apiType,
  typedOptions: T15TypedOptions = {},
): Record<string, unknown> {
  const common = {
    exact_model: exactModelId,
    execution_mode: executionMode,
    idempotency_key: `${runId}:${requestKey}`,
    ...(typedOptions.sessionId ? { session_id: typedOptions.sessionId } : {}),
  };
  switch (apiType) {
    case "llm": {
      const toolHistory = requestKey.endsWith(".tool-history");
      const nativeHistory = requestKey.endsWith(".native-history");
      const reasoningHistory = requestKey.endsWith(".reasoning-history");
      const structuredOutput = requestKey.endsWith(".structured-output");
      const providerSwitch = requestKey.endsWith(".provider-switch");
      const providerState = typedOptions.foreignProviderState;
      return {
        ...common,
        messages: nativeHistory
          ? [
            {
              role: "user",
              content: [{ type: "text", text: "Remember BUCKYOS-AICC-4827." }],
            },
            typedOptions.historyMessage ?? {
              role: "assistant",
              content: [
                { type: "thinking", summary: "kept for other providers" },
                { type: "text", text: "BUCKYOS-AICC-4827" },
                {
                  type: "provider_state",
                  provider: "openai",
                  value: {
                    type: "reasoning",
                    id: "rs_t15_4827",
                    summary: [{
                      type: "summary_text",
                      text: "opaque reasoning",
                    }],
                    encrypted_content: "opaque-t15-reasoning",
                  },
                },
                {
                  type: "provider_state",
                  provider: "openai",
                  value: {
                    type: "message",
                    id: "msg_t15_4827",
                    status: "completed",
                    role: "assistant",
                    content: [{
                      type: "output_text",
                      text: "BUCKYOS-AICC-4827",
                      annotations: [],
                    }],
                  },
                },
              ],
            },
            {
              role: "user",
              content: [{ type: "text", text: "Return the marker." }],
            },
          ]
          : reasoningHistory
          ? [
            {
              role: "user",
              content: [{ type: "text", text: "Think before answering." }],
            },
            typedOptions.historyMessage ?? {
              role: "assistant",
              content: [
                { type: "text", text: "I considered the request." },
                {
                  type: "thinking",
                  provider_metadata: [{
                    type: "reasoning.encrypted",
                    id: "reason-t15-4827",
                    data: "opaque-t15-reasoning",
                  }],
                },
              ],
            },
            { role: "user", content: [{ type: "text", text: "Continue." }] },
          ]
          : toolHistory
          ? [
            {
              role: "user",
              content: [{
                type: "text",
                text: "What is the weather in Paris?",
              }],
            },
            {
              role: "assistant",
              content: [
                { type: "text", text: "I will check." },
                {
                  type: "tool_use",
                  call_id: "weather-call-4827",
                  name: "weather",
                  args: { city: "Paris" },
                },
              ],
            },
            {
              role: "tool",
              content: [{
                type: "tool_result",
                call_id: "weather-call-4827",
                content: [{ type: "text", text: "sunny" }],
              }],
            },
            {
              role: "user",
              content: [{ type: "text", text: "Summarize the result." }],
            },
          ]
          : providerSwitch
          ? [
            {
              role: "user",
              content: [{
                type: "text",
                text: "Remember provider switch marker BUCKYOS-AICC-4827.",
              }],
            },
            {
              role: "assistant",
              content: [
                {
                  type: "text",
                  text:
                    "I will remember provider switch marker BUCKYOS-AICC-4827.",
                },
                ...(providerState
                  ? [{
                    type: "provider_state",
                    provider: providerState.provider,
                    value: providerState.value,
                  }]
                  : []),
              ],
            },
            {
              role: "user",
              content: [{
                type: "text",
                text: "Return the provider switch marker now.",
              }],
            },
          ]
          : requestKey.endsWith(".history")
          ? [
            {
              role: "user",
              content: [{
                type: "text",
                text: "Remember marker BUCKYOS-AICC-4827.",
              }],
            },
            {
              role: "assistant",
              content: [{
                type: "text",
                text: "I will remember BUCKYOS-AICC-4827.",
              }],
            },
            {
              role: "user",
              content: [{ type: "text", text: "Return the marker now." }],
            },
          ]
          : [{
            role: "user",
            content: [{ type: "text", text: "Return BUCKYOS-AICC-4827." }],
          }],
        ...(toolHistory
          ? {
            tools: [{
              type: "function",
              name: "weather",
              description: "Look up weather",
              args_json_schema: {
                type: "object",
                properties: { city: { type: "string" } },
                required: ["city"],
              },
            }],
          }
          : {}),
        ...(structuredOutput
          ? {
            response_format: {
              type: "json_schema",
              json_schema: {
                name: "answer",
                strict: true,
                schema: {
                  type: "object",
                  properties: { answer: { type: "string" } },
                  required: ["answer"],
                  additionalProperties: false,
                },
              },
            },
          }
          : {}),
        max_output_tokens: 32,
      };
    }
    case "embedding.text":
      return {
        ...common,
        items: [{ type: "text", id: "item-1", text: "BUCKYOS-AICC-4827" }],
      };
    case "embedding.multimodal":
      return {
        ...common,
        items: [{ id: "item-1", text: "marker", image: resource("image/png") }],
      };
    case "image.txt2img":
      return { ...common, prompt: "A blue square marked 4827" };
    case "image.img2img":
      return {
        ...common,
        prompt: "Preserve the image",
        images: [resource("image/png")],
      };
    case "image.inpaint":
      return {
        ...common,
        prompt: "Fill the mask",
        image: resource("image/png"),
        mask: resource("image/png"),
      };
    case "image.upscale":
      return { ...common, image: resource("image/png"), scale: 2 };
    case "image.bg_remove":
      return { ...common, image: resource("image/png") };
    case "vision.ocr":
      return { ...common, document: resource("image/png") };
    case "vision.caption":
      return { ...common, image: resource("image/png") };
    case "vision.detect":
      return { ...common, image: resource("image/png") };
    case "vision.segment":
      return {
        ...common,
        image: resource("image/png"),
        prompt: { type: "text", text: "Segment objects" },
      };
    case "audio.tts":
      return { ...common, text: "BuckyOS 4827", voice: { voice_id: "alloy" } };
    case "audio.asr":
      return { ...common, audio: resource("audio/wav") };
    case "audio.music":
      return {
        ...common,
        prompt: "A short calm song",
        lyrics: "[Verse]\nBUCKYOS-AICC-4827",
      };
    case "audio.enhance":
      return { ...common, audio: resource("audio/wav"), task: "denoise" };
    case "video.txt2video":
      return { ...common, prompt: "A paper plane moves across a desk" };
    case "video.img2video":
      return {
        ...common,
        prompt: "Subtle motion",
        image: resource("image/png"),
      };
    case "video.video2video":
      return {
        ...common,
        video: resource("video/mp4"),
        prompt: "Preserve motion",
      };
    case "video.extend":
      return {
        ...common,
        video: resource("video/mp4"),
        prompt: "Continue the motion",
        duration_seconds: 2,
      };
    case "video.upscale":
      return {
        ...common,
        video: resource("video/mp4"),
        target_resolution: "1080p",
      };
    case "rerank":
      return {
        ...common,
        query: "Which document contains marker 4827?",
        documents: [
          { id: "wrong", text: "This record has no marker." },
          { id: "right", text: "The marker is BUCKYOS-AICC-4827." },
        ],
      };
    case "agent.computer_use":
      return {
        ...common,
        task: "Read the page title",
        environment: {
          environment_id: "aicc-t15-browser",
          session_id: `${runId}:computer`,
          screenshot: {
            kind: "base64",
            mime: "image/png",
            data_base64:
              "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
          },
          viewport: { width: 1280, height: 720 },
        },
        allowed_actions: ["left_click"],
      };
    default:
      throw new Error(`no T1.5 typed request fixture for ${apiType}`);
  }
}

async function terminal(
  session: GatewaySession,
  value: unknown,
  timeoutMs: number,
): Promise<unknown> {
  if (!value || typeof value !== "object") {
    throw new Error("typed response must be an object");
  }
  const response = value as Record<string, unknown>;
  if (response.status === "failed") {
    throw new Error(
      `AICC returned failed: ${JSON.stringify(response.result ?? response)}`,
    );
  }
  if (response.status === "succeeded") return response;
  if (response.status !== "running" || typeof response.task_id !== "string") {
    throw new Error(`unexpected typed response: ${JSON.stringify(response)}`);
  }
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const raw = await session.taskManager.call("get_task", {
      task_id: response.task_id,
    }) as Record<string, unknown>;
    const task = raw.task && typeof raw.task === "object"
      ? raw.task as Record<string, unknown>
      : raw;
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") {
        throw new Error(
          `task ended ${String(task.outcome)}: ${
            JSON.stringify(task.error ?? {})
          }`,
        );
      }
      return {
        ...task,
        provider_task_ref: response.provider_task_ref,
      };
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error(`task ${response.task_id} timed out`);
}

function hasMappedField(
  value: unknown,
  names: Set<string>,
  seen = new Set<object>(),
): boolean {
  if (!value || typeof value !== "object") return false;
  if (seen.has(value)) return false;
  seen.add(value);
  if (Array.isArray(value)) {
    return value.some((item) => hasMappedField(item, names, seen));
  }
  const record = value as Record<string, unknown>;
  if (Object.keys(record).some((key) => names.has(key))) return true;
  return Object.values(record).some((item) =>
    hasMappedField(item, names, seen)
  );
}

function findRecordField(
  value: unknown,
  field: string,
  seen = new Set<object>(),
): Record<string, unknown> | undefined {
  if (!value || typeof value !== "object" || seen.has(value)) return undefined;
  seen.add(value);
  if (!Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    const candidate = record[field];
    if (
      candidate && typeof candidate === "object" && !Array.isArray(candidate)
    ) {
      return candidate as Record<string, unknown>;
    }
    for (const child of Object.values(record)) {
      const found = findRecordField(child, field, seen);
      if (found) return found;
    }
    return undefined;
  }
  for (const child of value) {
    const found = findRecordField(child, field, seen);
    if (found) return found;
  }
  return undefined;
}

function containsBase64Resource(
  value: unknown,
  seen = new Set<object>(),
): boolean {
  if (!value || typeof value !== "object") return false;
  if (seen.has(value)) return false;
  seen.add(value);
  if (Array.isArray(value)) {
    return value.some((item) => containsBase64Resource(item, seen));
  }
  const record = value as Record<string, unknown>;
  if (record.kind === "base64") return true;
  return Object.values(record).some((item) =>
    containsBase64Resource(item, seen)
  );
}

function hasReferencedArtifact(
  value: unknown,
  seen = new Set<object>(),
): boolean {
  if (!value || typeof value !== "object") return false;
  if (seen.has(value)) return false;
  seen.add(value);
  if (Array.isArray(value)) {
    return value.some((item) => hasReferencedArtifact(item, seen));
  }
  const record = value as Record<string, unknown>;
  if (record.kind === "named_object" || record.kind === "url") return true;
  return Object.values(record).some((item) =>
    hasReferencedArtifact(item, seen)
  );
}

function artifactResultSummary(value: unknown): string {
  const record = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
  const result = record.result && typeof record.result === "object" && !Array.isArray(record.result)
    ? record.result as Record<string, unknown>
    : {};
  const output = result.output && typeof result.output === "object" && !Array.isArray(result.output)
    ? result.output as Record<string, unknown>
    : {};
  const artifacts = Array.isArray(output.artifacts) ? output.artifacts : [];
  return JSON.stringify({
    top_keys: Object.keys(record).sort(),
    result_keys: Object.keys(result).sort(),
    output_keys: Object.keys(output).sort(),
    artifact_count: artifacts.length,
    has_base64: containsBase64Resource(value),
    has_reference: hasReferencedArtifact(value),
  });
}

function taskResultPayload(value: unknown): unknown {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  const record = value as Record<string, unknown>;
  return record.result ?? value;
}

function canonicalJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, child]) => [key, canonicalJson(child)]),
  );
}

function sameJsonSemantics(left: unknown, right: unknown): boolean {
  return JSON.stringify(canonicalJson(left)) === JSON.stringify(canonicalJson(right));
}

export function assertT15ResponseMapping(
  apiType: string,
  value: unknown,
  contract: ReturnType<typeof protocolContract>,
): void {
  const expectedFields: Record<string, string[]> = {
    llm: ["message"],
    "embedding.text": ["data", "data_resource", "embeddings"],
    "embedding.multimodal": ["data", "data_resource", "embeddings"],
    rerank: ["results"],
    "image.txt2img": ["images", "artifacts"],
    "image.img2img": ["images", "image", "artifacts"],
    "image.inpaint": ["images", "image", "artifacts"],
    "image.upscale": ["image", "artifacts"],
    "image.bg_remove": ["image", "artifacts"],
    "vision.ocr": ["text", "pages", "artifacts"],
    "vision.caption": ["captions"],
    "vision.detect": ["detections"],
    "vision.segment": ["masks", "artifacts"],
    "audio.tts": ["audio", "artifacts"],
    "audio.asr": ["text", "segments"],
    "audio.music": ["audio", "artifacts"],
    "audio.enhance": ["audio", "artifacts"],
    "video.txt2video": ["video", "artifacts"],
    "video.img2video": ["video", "artifacts"],
    "video.video2video": ["video", "artifacts"],
    "video.extend": ["video", "artifacts"],
    "video.upscale": ["video", "artifacts"],
    "agent.computer_use": ["action", "actions"],
  };
  const fields = expectedFields[apiType];
  if (!fields || !hasMappedField(value, new Set(fields))) {
    throw new Error(
      `typed ${apiType} response is missing mapped field ${
        fields?.join("|") ?? "<unknown>"
      }`,
    );
  }
  if (
    hasMappedField(contract.success_fixture, new Set(["usage"])) &&
    !hasMappedField(value, new Set(["usage"]))
  ) {
    throw new Error(
      `typed ${apiType} response is missing mapped Provider usage`,
    );
  }
  if (
    contract.async_protocol &&
    !hasMappedField(
      value,
      new Set(["provider_task_ref", "provider_operation_id"]),
    )
  ) {
    throw new Error(
      `typed ${apiType} response is missing Provider operation attribution`,
    );
  }
}

async function executeCase(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  testCase: AcceptanceCase,
  inventory: ProviderInventory,
  controlUrl: string,
  runId: string,
  timeoutMs: number,
): Promise<CaseResult> {
  const started = Date.now();
  const startedAt = new Date(started).toISOString();
  await selectMock(controlUrl, testCase, runId);
  let selectedExactModel: string | undefined;
  let failed: unknown;
  let terminalValue: unknown;
  try {
    selectedExactModel = exactModel(catalog, testCase, inventory);
    let historyMessage: Record<string, unknown> | undefined;
    if (
      testCase.tags.includes("native_history") ||
      testCase.tags.includes("reasoning_history")
    ) {
      const seedResult = await session.aicc.call(
        testCase.method,
        buildT15TypedParams(
          testCase.api_type!,
          selectedExactModel,
          runId,
          "immediate",
          `${testCase.case_id}.seed`,
        ),
      ) as Record<string, unknown>;
      const seedTerminal = await terminal(
        session,
        seedResult,
        Math.min(timeoutMs, testCase.timeout_ms),
      );
      historyMessage = findRecordField(seedTerminal, "message");
      if (!historyMessage) {
        throw new Error(
          "Provider history seed did not return a typed assistant message",
        );
      }
      await selectMock(controlUrl, testCase, `${runId}:replay`);
    }
    const result = await session.aicc.call(
      testCase.method,
      buildT15TypedParams(
        testCase.api_type!,
        selectedExactModel,
        runId,
        testCase.execution_mode,
        testCase.case_id,
        { historyMessage },
      ),
    ) as Record<string, unknown>;
    if (testCase.mock_scenario === "async_cancel") {
      if (result.status !== "running" || typeof result.task_id !== "string") {
        throw new Error(
          `async cancel requires running task: ${JSON.stringify(result)}`,
        );
      }
      const cancelled = await session.aicc.call("cancel", {
        task_id: result.task_id,
      }) as Record<string, unknown>;
      if (cancelled.accepted !== true) {
        throw new Error(
          `AICC did not accept Provider cancellation: ${
            JSON.stringify(cancelled)
          }`,
        );
      }
    } else {
      terminalValue = await terminal(
        session,
        result,
        Math.min(timeoutMs, testCase.timeout_ms),
      );
      if (!testCase.expected_error_class) {
        assertT15ResponseMapping(
          testCase.api_type!,
          terminalValue,
          protocolContract(
            catalog,
            testCase.provider_driver!,
            testCase.protocol_contract_id!,
          ),
        );
        if (testCase.tags.includes("task_result_artifact")) {
          const resultPayload = taskResultPayload(terminalValue);
          if (containsBase64Resource(resultPayload)) {
            throw new Error(
              `TaskMgr result retained inline base64 resource instead of a stable artifact reference: ${
                artifactResultSummary(terminalValue)
              }`,
            );
          }
          if (!hasReferencedArtifact(resultPayload)) {
            throw new Error(
              `TaskMgr result did not expose a named object or URL artifact reference: ${
                artifactResultSummary(terminalValue)
              }`,
            );
          }
        }
      }
    }
  } catch (error) {
    failed = error;
  }
  const requests = await capturedRequests(controlUrl);
  const validationErrors = requests.flatMap((request) =>
    request.validation_errors ?? []
  );
  const expectsFailure = Boolean(testCase.expected_error_class);
  const diagnostics: string[] = [];
  if (requests.length === 0) {
    diagnostics.push("Provider mock received no request");
  }
  if (validationErrors.length > 0) {
    diagnostics.push(
      `wire contract violations: ${JSON.stringify(validationErrors)}`,
    );
  }
  const wireBody = requests.find((request) => request.body)?.body as
    | Record<string, unknown>
    | undefined;
  if (testCase.tags.includes("native_history")) {
    const input = Array.isArray(wireBody?.input) ? wireBody.input : [];
    const reasoning = input.find((item) =>
      item && typeof item === "object" &&
      (item as Record<string, unknown>).id === "rs_t15_4827"
    ) as Record<string, unknown> | undefined;
    const message = input.find((item) =>
      item && typeof item === "object" &&
      (item as Record<string, unknown>).id === "msg_t15_4827"
    ) as Record<string, unknown> | undefined;
    if (
      reasoning?.encrypted_content !== "opaque-t15-reasoning" ||
      message?.status !== "completed"
    ) {
      diagnostics.push(
        "OpenAI native output items were not replayed unchanged",
      );
    }
  }
  if (testCase.tags.includes("reasoning_history")) {
    const messages = Array.isArray(wireBody?.messages) ? wireBody.messages : [];
    const assistant = messages.find((item) =>
      item && typeof item === "object" &&
      (item as Record<string, unknown>).role === "assistant"
    ) as Record<string, unknown> | undefined;
    const expectedReasoningDetails = [{
        type: "reasoning.encrypted",
        id: "reason-t15-4827",
        data: "opaque-t15-reasoning",
      }];
    if (!sameJsonSemantics(assistant?.reasoning_details, expectedReasoningDetails)) {
      diagnostics.push(
        `OpenRouter reasoning_details were not replayed unchanged: ${
          JSON.stringify(assistant?.reasoning_details ?? null)
        }`,
      );
    }
  }
  if (testCase.tags.includes("structured_output")) {
    const outputConfig = wireBody?.output_config as
      | Record<string, unknown>
      | undefined;
    const format = outputConfig?.format as Record<string, unknown> | undefined;
    if (
      format?.type !== "json_schema" ||
      !format.schema || typeof format.schema !== "object"
    ) {
      diagnostics.push(
        "Claude canonical JSON Schema was not lowered to output_config.format",
      );
    }
  }
  const contract = protocolContract(
    catalog,
    testCase.provider_driver!,
    testCase.protocol_contract_id!,
  );
  const observedAsyncSteps = new Set(
    requests.map((request) => request.async_step).filter(Boolean),
  );
  if (testCase.mock_scenario === "async_success") {
    for (
      const step of contract.async_steps?.filter((candidate) =>
        candidate.name !== "cancel"
      ) ?? []
    ) {
      if (!observedAsyncSteps.has(step.name)) {
        diagnostics.push(`missing async ${step.name} wire request`);
      }
    }
  }
  if (
    ["async_failed", "async_poll_timeout", "async_artifact_unavailable"]
      .includes(testCase.mock_scenario ?? "") && !observedAsyncSteps.has("poll")
  ) {
    diagnostics.push("missing async poll wire request before failure mapping");
  }
  if (
    testCase.mock_scenario === "async_cancel" &&
    !observedAsyncSteps.has("cancel")
  ) {
    diagnostics.push("missing Provider async cancel wire request");
  }
  const selectedModel = inventory.models.find((model) =>
    testCase.model_selector?.kind === "exact" &&
    model.exact_model === testCase.model_selector.value
  );
  if (selectedModel?.provider_actual_model_id) {
    const providerRequest = requests.at(-1);
    const body =
      providerRequest?.body && typeof providerRequest.body === "object"
        ? providerRequest.body as Record<string, unknown>
        : {};
    if (
      body.model !== selectedModel.provider_actual_model_id &&
      !providerRequest?.pathname?.includes(
        encodeURIComponent(selectedModel.provider_actual_model_id),
      )
    ) {
      diagnostics.push(
        `variant model was not lowered to ${selectedModel.provider_actual_model_id}`,
      );
    }
    for (
      const [name, expected] of Object.entries(
        testCase.expected_provider_options ?? {},
      )
    ) {
      if (JSON.stringify(body[name]) !== JSON.stringify(expected)) {
        diagnostics.push(
          `variant Provider option ${name} was not lowered to ${
            JSON.stringify(expected)
          }`,
        );
      }
    }
  }
  if (testCase.tags.includes("cloud_update")) {
    const body = requests.at(-1)?.body;
    const record = body && typeof body === "object" && !Array.isArray(body)
      ? body as Record<string, unknown>
      : {};
    if (record.service_tier !== "default") {
      diagnostics.push(
        `cloud Provider Rules did not lower service_tier=default: ${
          JSON.stringify(record.service_tier)
        }`,
      );
    }
  }
  if (expectsFailure && !failed) {
    diagnostics.push(
      "official Provider error fixture was not mapped to a failed AICC call/task",
    );
  }
  if (!expectsFailure && failed) diagnostics.push(String(failed));
  if (testCase.tags.includes("official_error") && failed) {
    const evidence = String(failed);
    if (
      !evidence.toLowerCase().includes(
        testCase.expected_aicc_error_code!.toLowerCase(),
      )
    ) {
      diagnostics.push(
        `missing stable AICC error code ${testCase.expected_aicc_error_code}`,
      );
    }
    if (
      !evidence.toLowerCase().includes(
        testCase.expected_provider_error_code!.toLowerCase(),
      )
    ) {
      diagnostics.push(
        `missing Provider error summary code ${testCase.expected_provider_error_code}`,
      );
    }
    const retriable = new RegExp(
      `retriable[\\s\"':=]+${String(testCase.expected_retriable)}`,
      "i",
    );
    if (testCase.expected_retriable === true && !retriable.test(evidence)) {
      diagnostics.push("missing retriable=true mapping");
    }
    if (
      testCase.expected_retriable === false &&
      /retriable[\\s\"':=]+true/i.test(evidence)
    ) {
      diagnostics.push("unexpected retriable=true mapping");
    }
  }
  return {
    case_id: testCase.case_id,
    provider_driver: testCase.provider_driver,
    provider_instance: testCase.provider_instance ?? undefined,
    exact_model: selectedExactModel,
    api_type: testCase.api_type ?? undefined,
    method: testCase.method,
    protocol_contract_id: testCase.protocol_contract_id,
    scenario: testCase.mock_scenario,
    status: diagnostics.length === 0 ? "passed" : "failed",
    diagnostic: diagnostics.length > 0 ? diagnostics.join("; ") : undefined,
    captured_requests: requests.length,
    started_at: startedAt,
    elapsed_ms: Date.now() - started,
  };
}

async function executeProviderSwitchCase(
  session: GatewaySession,
  catalog: ProviderProtocolCatalog,
  testCase: AcceptanceCase,
  sourceInventory: ProviderInventory,
  targetInventory: ProviderInventory,
  controlUrl: string,
  runId: string,
  timeoutMs: number,
): Promise<CaseResult> {
  const started = Date.now();
  const startedAt = new Date(started).toISOString();
  const sourceProvider = testCase.switch_source_provider_driver!;
  const sourceContractId = testCase.switch_source_contract_id!;
  const targetProvider = testCase.provider_driver!;
  const targetContractId = testCase.protocol_contract_id!;
  const sourceCase: AcceptanceCase = {
    ...testCase,
    case_id: `${testCase.case_id}.source`,
    tags: testCase.tags.filter((tag) => tag !== "provider_switch_matrix"),
    provider_driver: sourceProvider,
    provider_instance: sourceInventory.provider_instance_name,
    expected_provider_instance: sourceInventory.provider_instance_name,
    protocol_contract_id: sourceContractId,
    api_type: "llm",
    mock_scenario: "success",
    expected_error_class: null,
  };
  testCase.provider_instance = targetInventory.provider_instance_name;
  testCase.expected_provider_instance = targetInventory.provider_instance_name;
  const sessionId = `${runId}:${testCase.case_id}`;
  let sourceExactModel: string | undefined;
  let targetExactModel: string | undefined;
  let failed: unknown;
  let sourceRequests: Awaited<ReturnType<typeof capturedRequests>> = [];
  let targetRequests: Awaited<ReturnType<typeof capturedRequests>> = [];
  let terminalValue: unknown;
  try {
    sourceInventory = await refreshProviderInventory(
      session,
      catalog,
      sourceProvider,
      sourceInventory.provider_instance_name,
      controlUrl,
      runId,
      timeoutMs,
    );
    targetInventory = await refreshProviderInventory(
      session,
      catalog,
      targetProvider,
      targetInventory.provider_instance_name,
      controlUrl,
      runId,
      timeoutMs,
    );
    sourceExactModel = exactModel(catalog, sourceCase, sourceInventory);
    await selectMock(controlUrl, sourceCase, `${runId}:source`);
    const sourceResult = await session.aicc.call(
      sourceCase.method,
      buildT15TypedParams(
        "llm",
        sourceExactModel,
        runId,
        "immediate",
        `${testCase.case_id}.source`,
        {
          sessionId,
        },
      ),
    ) as Record<string, unknown>;
    await terminal(
      session,
      sourceResult,
      Math.min(timeoutMs, testCase.timeout_ms),
    );
    sourceRequests = await capturedRequests(controlUrl);

    targetExactModel = exactModel(catalog, testCase, targetInventory);
    await selectMock(controlUrl, testCase, `${runId}:target`);
    const targetResult = await session.aicc.call(
      testCase.method,
      buildT15TypedParams(
        "llm",
        targetExactModel,
        runId,
        "immediate",
        `${testCase.case_id}.provider-switch`,
        {
          sessionId,
          foreignProviderState: {
            provider: providerStateNamespace(sourceProvider),
            value: {
              type: "provider_switch_history",
              summary: [{
                type: "summary_text",
                text: `source=${sourceProvider}; marker=BUCKYOS-AICC-4827`,
              }],
              content: [{
                type: "output_text",
                text: `prior source model ${sourceExactModel}`,
              }],
            },
          },
        },
      ),
    ) as Record<string, unknown>;
    terminalValue = await terminal(
      session,
      targetResult,
      Math.min(timeoutMs, testCase.timeout_ms),
    );
    targetRequests = await capturedRequests(controlUrl);
    assertT15ResponseMapping(
      "llm",
      terminalValue,
      protocolContract(catalog, targetProvider, targetContractId),
    );
  } catch (error) {
    failed = error;
  }
  const sourceValidationErrors = sourceRequests.flatMap((request) =>
    request.validation_errors ?? []
  );
  const targetValidationErrors = targetRequests.flatMap((request) =>
    request.validation_errors ?? []
  );
  const diagnostics: string[] = [];
  if (sourceRequests.length === 0) {
    diagnostics.push("Provider switch source mock received no request");
  }
  if (targetRequests.length === 0) {
    diagnostics.push("Provider switch target mock received no request");
  }
  if (sourceValidationErrors.length > 0) {
    diagnostics.push(
      `source wire contract violations: ${
        JSON.stringify(sourceValidationErrors)
      }`,
    );
  }
  if (targetValidationErrors.length > 0) {
    diagnostics.push(
      `target wire contract violations: ${
        JSON.stringify(targetValidationErrors)
      }`,
    );
  }
  if (!failed && sourceExactModel === targetExactModel) {
    diagnostics.push(
      `Provider switch did not use distinct models: ${sourceExactModel}`,
    );
  }
  if (failed) diagnostics.push(String(failed));
  return {
    case_id: testCase.case_id,
    provider_driver: targetProvider,
    provider_instance: targetInventory.provider_instance_name,
    exact_model: targetExactModel,
    api_type: "llm",
    method: testCase.method,
    protocol_contract_id: targetContractId,
    scenario: testCase.mock_scenario,
    status: diagnostics.length === 0 ? "passed" : "failed",
    diagnostic: [
      `source_provider=${sourceProvider}`,
      sourceExactModel ? `source_exact_model=${sourceExactModel}` : undefined,
      ...diagnostics,
    ].filter(Boolean).join("; ") || undefined,
    captured_requests: sourceRequests.length + targetRequests.length,
    started_at: startedAt,
    elapsed_ms: Date.now() - started,
  };
}

export function variantCells(
  catalog: ProviderProtocolCatalog,
  inventory: ProviderInventory,
) {
  const provider = catalog.providers.find((candidate) =>
    candidate.provider_driver === inventory.provider_driver
  );
  if (!provider) {
    throw new Error(`missing protocol provider ${inventory.provider_driver}`);
  }
  const runtimeVariants = inventory.models.filter((model) =>
    Boolean(model.provider_actual_model_id) ||
    model.provider_model_id.includes(":")
  );
  const rules = provider.official_variant_rules ?? [];
  if (runtimeVariants.length > 0 && rules.length === 0) {
    throw new Error(
      `${inventory.provider_driver} exposes metadata variants without official T1.5 expectations`,
    );
  }
  const expected = new Map<string, Record<string, unknown>>();
  for (const rule of rules) {
    for (const modelId of rule.model_ids) {
      const baseExists = inventory.models.some((model) =>
        model.provider_model_id === modelId ||
        model.provider_actual_model_id === modelId
      );
      if (!baseExists) {
        throw new Error(
          `${inventory.provider_driver} is missing official base model ${modelId}`,
        );
      }
      for (const [variant, options] of Object.entries(rule.variants)) {
        const key = `${modelId}:${variant}`;
        if (expected.has(key)) {
          throw new Error(
            `${inventory.provider_driver} has duplicate official variant expectation ${key}`,
          );
        }
        expected.set(key, options);
      }
    }
  }
  const runtime = new Map(
    runtimeVariants.map((model) => [model.provider_model_id, model]),
  );
  for (const key of expected.keys()) {
    if (!runtime.has(key)) {
      throw new Error(
        `${inventory.provider_driver} is missing official metadata variant ${key}`,
      );
    }
  }
  for (const key of runtime.keys()) {
    if (!expected.has(key)) {
      throw new Error(
        `${inventory.provider_driver} exposes undocumented metadata variant ${key}`,
      );
    }
  }
  return runtimeVariants.flatMap((model) =>
    model.api_types.map((apiType) => {
      const operation = inventory.provider_driver === "openai" &&
          model.provider_model_id.startsWith("gpt-5") &&
          ["image.txt2img", "image.img2img"].includes(apiType)
        ? "responses.create"
        : undefined;
      const contract = catalog.providers.find((provider) =>
        provider.provider_driver === inventory.provider_driver
      )
        ?.contracts.find((candidate) =>
          operation
            ? candidate.operation === operation
            : candidate.api_types.includes(apiType)
        );
      if (!contract) {
        throw new Error(
          `${inventory.provider_driver} metadata variant ${model.provider_model_id} has no T1.5 contract for ${apiType}`,
        );
      }
      return {
        provider_driver: inventory.provider_driver,
        contract_id: contract.id,
        api_type: apiType,
        model,
        expected_provider_options: expected.get(model.provider_model_id),
      };
    })
  );
}

async function main(): Promise<void> {
  const input = options(process.argv.slice(2));
  const startedAt = new Date().toISOString();
  await runPreflight();
  const catalog = await loadProviderProtocolCatalog();
  const selectedProviders = input.providers.length > 0
    ? new Set(input.providers)
    : new Set(catalog.providers.map((provider) => provider.provider_driver));
  for (const driver of selectedProviders) {
    if (
      !catalog.providers.some((provider) => provider.provider_driver === driver)
    ) throw new Error(`unknown --provider ${driver}`);
  }
  const staticManifest = validateCaseManifest(buildT15Manifest(catalog));
  const inProviderScope = (testCase: AcceptanceCase) => {
    if (!selectedProviders.has(testCase.provider_driver ?? "")) return false;
    if (testCase.tags.includes("provider_switch_matrix")) {
      return selectedProviders.has(
        testCase.switch_source_provider_driver ?? "",
      );
    }
    return true;
  };
  const requestedCase = (testCase: AcceptanceCase) =>
    input.caseIds.length === 0 || input.caseIds.includes(testCase.case_id);
  const scopedStaticManifest = staticManifest.filter(inProviderScope).filter(
    requestedCase,
  );
  const providerSwitchCases = scopedStaticManifest.filter((testCase) =>
    testCase.tags.includes("provider_switch_matrix")
  );
  const switchProviderDrivers = new Set(
    providerSwitchCases.flatMap((testCase) =>
      [
        testCase.provider_driver,
        testCase.switch_source_provider_driver,
      ].filter((driver): driver is string =>
        typeof driver === "string" && driver.length > 0
      )
    ),
  );
  let mockProcess: ChildProcess | undefined;
  if (input.startLocalMock) {
    mockProcess = spawn(process.execPath, [
      "--experimental-strip-types",
      join(here, "t15_mock_provider.ts"),
      "--port",
      String(input.mockPort),
    ], {
      stdio: ["ignore", "inherit", "inherit"],
    });
  }
  const runId = `t15-${
    new Date().toISOString().replace(/[^0-9]/g, "").slice(0, 14)
  }-${process.pid}`;
  const created: string[] = [];
  const results: CaseResult[] = [];
  const providerInstances = new Map<string, string>();
  const providerInventories = new Map<string, ProviderInventory>();
  const plannedCaseIds = new Set(
    scopedStaticManifest.map((testCase) => testCase.case_id),
  );
  const unmatchedCaseIds = new Set(input.caseIds);
  let session: GatewaySession | undefined;
  let fatalError: unknown;
  let phase = "startup";
  const cloudCaseId = "t1.5.openai.openai.responses.v1.llm.cloud-update";
  const cloudCaseRequested = selectedProviders.has("openai") &&
    (input.caseIds.length === 0 || input.caseIds.includes(cloudCaseId));
  let cloudFixture: CloudUpdateFixtureService | undefined;
  let cloudAdmin: RpcClient | undefined;
  let cloudSystemConfig: RpcClient | undefined;
  let restoreCloudConfig:
    | ((refreshedSystemConfig?: RpcClient) => Promise<void>)
    | undefined;
  let cloudRevision = 0;
  let cloudCleanupRevision = 0;
  let cloudActive = false;
  try {
    await waitMock(input.mockControlUrl);
    session = await loginGateway({
      gatewayUrl: input.gatewayUrl,
      sessionToken: input.sessionToken,
      username: input.username,
      password: input.password,
      appId: input.appId,
    });
    let sudoSystemConfig = await loginSudoSystemConfig({
      gatewayUrl: input.gatewayUrl,
      username: input.username,
      password: input.password,
      appId: input.appId,
    });
    if (cloudCaseRequested) {
      cloudAdmin = session.aicc;
      cloudSystemConfig = sudoSystemConfig;
      restoreCloudConfig = await backupCloudUpdateConfig(sudoSystemConfig);
      const cloudView = await cloudAdmin.call(
        "driver_metadata_update.get",
        {},
      ) as { metadata_target_seq?: unknown };
      const currentSeq = typeof cloudView.metadata_target_seq === "number"
        ? cloudView.metadata_target_seq
        : 1;
      cloudRevision = Math.max(Date.now(), currentSeq + 10);
      cloudCleanupRevision = cloudRevision + 1;
      cloudFixture = await CloudUpdateFixtureService.start({
        gatewayUrl: input.gatewayUrl,
        sessionToken: session.sessionToken,
        runId,
        gatewayBinary: input.ndnGatewayBinary,
        namedStoreConfigPath: input.ndnNamedStoreConfigPath,
        gatewayControlUrl: input.ndnGatewayControlUrl,
        systemRoot: input.ndnSystemRoot,
      });
    }
    process.stdout.write(`${
      JSON.stringify(
        {
          layer: "T1.5",
          providers: [...selectedProviders],
          real_provider_calls: 0,
          estimated_cost_usd: 0,
          global_concurrency: 1,
          provider_concurrency: 1,
          provider_min_interval_ms: input.providerMinIntervalMs,
        },
        null,
        2,
      )
    }\n`);
    for (const driver of selectedProviders) {
      phase = `provider:${driver}:setup`;
      if (input.username && input.password) {
        session = await loginGateway({
          gatewayUrl: input.gatewayUrl,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
        sudoSystemConfig = await loginSudoSystemConfig({
          gatewayUrl: input.gatewayUrl,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
      }
      const provider = catalog.providers.find((candidate) =>
        candidate.provider_driver === driver
      )!;
      phase = `provider:${driver}:select`;
      const bootstrap = buildT15Manifest(catalog).find((testCase) =>
        testCase.provider_driver === driver &&
        testCase.protocol_contract_id === provider.contracts[0].id &&
        testCase.mock_scenario === "success"
      )!;
      await selectMock(input.mockControlUrl, bootstrap, runId);
      phase = `provider:${driver}:add`;
      const instance = `${runId}-${driver}`.toLowerCase().replace(
        /[^a-z0-9_-]+/g,
        "-",
      );
      await addProvider(session, catalog, driver, instance, input.mockBaseUrl);
      created.push(instance);
      phase = `provider:${driver}:inventory`;
      const inventory = await waitInventory(session, instance, input.timeoutMs);
      providerInstances.set(driver, instance);
      providerInventories.set(driver, inventory);
      const manifest = validateCaseManifest(
        buildT15Manifest(catalog, variantCells(catalog, inventory)),
      )
        .filter((testCase) => testCase.provider_driver === driver)
        .filter((testCase) => !testCase.tags.includes("provider_switch_matrix"))
        .filter((testCase) => !testCase.tags.includes("custom_provider"))
        .filter(requestedCase);
      for (const testCase of manifest) plannedCaseIds.add(testCase.case_id);
      for (const testCase of manifest) {
        unmatchedCaseIds.delete(testCase.case_id);
      }
      await withMockQuotaTruth({
        systemConfig: sudoSystemConfig,
        userId: session.userId,
        appId: "system:control-panel",
        inventories: [inventory],
        execute: async () => {
          phase = `provider:${driver}:cases`;
          for (const [index, testCase] of manifest.entries()) {
            if (index > 0 && input.providerMinIntervalMs > 0) {
              await new Promise((resolvePromise) =>
                setTimeout(resolvePromise, input.providerMinIntervalMs)
              );
            }
            testCase.provider_instance = instance;
            testCase.expected_provider_instance = instance;
            let effectiveInventory = inventory;
            if (testCase.tags.includes("cloud_update")) {
              if (!cloudFixture || !cloudAdmin) {
                throw new Error("cloud update fixture was not initialized");
              }
              const release = await cloudFixture.publish({
                revisionSeq: cloudRevision,
                files: await buildT15OpenAiRules(cloudRevision),
              });
              await setCloudUpdateSource(cloudAdmin, release.sourceUrl);
              await session!.aicc.call("provider.refresh_models", {
                provider_instance_name: instance,
              });
              await waitCloudUpdateConverged(
                cloudAdmin,
                cloudRevision,
                input.timeoutMs,
              );
              cloudActive = true;
              const cacheState = JSON.parse(
                await readFile(
                  join(
                    input.cloudCacheRoot,
                    "revisions",
                    String(cloudRevision),
                    "state.json",
                  ),
                  "utf8",
                ),
              ) as { target_seq?: unknown; files?: unknown[] };
              if (
                cacheState.target_seq !== cloudRevision ||
                cacheState.files?.length !== 1
              ) {
                throw new Error(
                  `T1.5 cloud Provider Rules revision ${cloudRevision} was not committed to cache`,
                );
              }
              effectiveInventory = await waitInventory(
                session!,
                instance,
                input.timeoutMs,
              );
            }
            results.push(
              await executeCase(
                session!,
                catalog,
                testCase,
                effectiveInventory,
                input.mockControlUrl,
                runId,
                input.timeoutMs,
              ),
            );
            if (testCase.tags.includes("cloud_update")) {
              const cleanup = await cloudFixture!.publish({
                revisionSeq: cloudCleanupRevision,
                files: [],
                tombstones: t15OpenAiTombstone(cloudCleanupRevision),
              });
              await setCloudUpdateSource(cloudAdmin!, cleanup.sourceUrl);
              await session!.aicc.call("provider.refresh_models", {
                provider_instance_name: instance,
              });
              await waitCloudUpdateConverged(
                cloudAdmin!,
                cloudCleanupRevision,
                input.timeoutMs,
              );
              providerInventories.set(
                driver,
                await refreshProviderInventory(
                  session!,
                  catalog,
                  driver,
                  instance,
                  input.mockControlUrl,
                  runId,
                  input.timeoutMs,
                ),
              );
              cloudActive = false;
            }
          }
        },
      });
      if (!switchProviderDrivers.has(driver)) {
        session = await refreshLogin(input, session);
        phase = `provider:${driver}:delete`;
        await session.aicc.call("provider.delete", {
          provider_instance_name: instance,
        });
        await waitInventoryAbsent(session, instance, input.timeoutMs);
        created.splice(created.indexOf(instance), 1);
        providerInstances.delete(driver);
        providerInventories.delete(driver);
      }
    }
    if (providerSwitchCases.length > 0) {
      session = await refreshLogin(input, session);
      await withMockQuotaTruth({
        systemConfig: sudoSystemConfig,
        userId: session.userId,
        appId: "system:control-panel",
        inventories: [...providerInventories.values()],
        execute: async () => {
          for (const [index, testCase] of providerSwitchCases.entries()) {
            if (index > 0 && input.providerMinIntervalMs > 0) {
              await new Promise((resolvePromise) =>
                setTimeout(resolvePromise, input.providerMinIntervalMs)
              );
            }
            plannedCaseIds.add(testCase.case_id);
            unmatchedCaseIds.delete(testCase.case_id);
            const sourceInventory = providerInventories.get(
              testCase.switch_source_provider_driver!,
            );
            const targetInventory = providerInventories.get(
              testCase.provider_driver!,
            );
            if (!sourceInventory || !targetInventory) {
              throw new Error(
                `Provider switch case ${testCase.case_id} is missing source or target inventory`,
              );
            }
            session = await refreshLogin(input, session!);
            results.push(
              await executeProviderSwitchCase(
                session!,
                catalog,
                testCase,
                sourceInventory,
                targetInventory,
                input.mockControlUrl,
                runId,
                input.timeoutMs,
              ),
            );
          }
        },
      });
      for (const driver of switchProviderDrivers) {
        const instance = providerInstances.get(driver);
        if (!instance) continue;
        session = await refreshLogin(input, session);
        await session.aicc.call("provider.delete", {
          provider_instance_name: instance,
        });
        await waitInventoryAbsent(session, instance, input.timeoutMs);
        created.splice(created.indexOf(instance), 1);
        providerInstances.delete(driver);
        providerInventories.delete(driver);
      }
    }
    for (const driver of ["openai", "claude", "google-gemini", "fal"]) {
      phase = `custom:${driver}:setup`;
      if (driver === "openai") {
        session = await loginGateway({
          gatewayUrl: input.gatewayUrl,
          sessionToken: input.sessionToken,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
        sudoSystemConfig = await loginSudoSystemConfig({
          gatewayUrl: input.gatewayUrl,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
      }
      if (!selectedProviders.has(driver)) continue;
      const customManifest = validateCaseManifest(buildT15Manifest(catalog))
        .filter((testCase) => testCase.provider_driver === driver)
        .filter((testCase) => testCase.tags.includes("custom_provider"))
        .filter((testCase) =>
          input.caseIds.length === 0 || input.caseIds.includes(testCase.case_id)
        );
      if (customManifest.length === 0) continue;
      const bootstrap = customManifest[0];
      await selectMock(input.mockControlUrl, bootstrap, runId);
      phase = `custom:${driver}:add`;
      const instance = `${runId}-custom-${driver}`.toLowerCase().replace(
        /[^a-z0-9_-]+/g,
        "-",
      );
      await addCustomProvider(
        session,
        catalog,
        driver,
        instance,
        input.mockBaseUrl,
        runId,
      );
      created.push(instance);
      phase = `custom:${driver}:inventory`;
      const inventory = await waitInventory(session, instance, input.timeoutMs);
      await withMockQuotaTruth({
        systemConfig: sudoSystemConfig,
        userId: session.userId,
        appId: "system:control-panel",
        inventories: [inventory],
        execute: async () => {
          phase = `custom:${driver}:cases`;
          for (const testCase of customManifest) {
            plannedCaseIds.add(testCase.case_id);
            unmatchedCaseIds.delete(testCase.case_id);
            testCase.provider_instance = instance;
            testCase.expected_provider_instance = instance;
            const result = await executeCase(
              session!,
              catalog,
              testCase,
              inventory,
              input.mockControlUrl,
              runId,
              input.timeoutMs,
            );
            result.diagnostic = [
              `seed=${runId}`,
              `selected_official_model=${
                result.exact_model?.split("@")[0] ?? "unknown"
              }`,
              result.diagnostic,
            ].filter(Boolean).join("; ");
            results.push(result);
          }
        },
      });
      session = await refreshLogin(input, session);
      phase = `custom:${driver}:delete`;
      await session.aicc.call("provider.delete", {
        provider_instance_name: instance,
      });
      await waitInventoryAbsent(session, instance, input.timeoutMs);
      created.splice(created.indexOf(instance), 1);
    }
    if (unmatchedCaseIds.size > 0) {
      throw new Error(
        `unknown or out-of-scope --case: ${
          [...unmatchedCaseIds].sort().join(", ")
        }`,
      );
    }
  } catch (error) {
    fatalError = error;
    results.push({
      case_id: "t1.5.runner",
      provider_driver: null,
      method: "provider.add/models.list",
      scenario: null,
      status: "failed",
      diagnostic: `${phase}: ${String(error)}`,
      captured_requests: 0,
      started_at: new Date().toISOString(),
      elapsed_ms: 0,
    });
  } finally {
    if (session && input.username && input.password) {
      try {
        session = await loginGateway({
          gatewayUrl: input.gatewayUrl,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
        cloudAdmin = session.aicc;
        cloudSystemConfig = await loginSudoSystemConfig({
          gatewayUrl: input.gatewayUrl,
          username: input.username,
          password: input.password,
          appId: input.appId,
        });
      } catch {
      }
    }
    if (session) {
      for (const providerInstanceName of created.reverse()) {
        try {
          await session.aicc.call("provider.delete", {
            provider_instance_name: providerInstanceName,
          });
          await waitInventoryAbsent(
            session,
            providerInstanceName,
            input.timeoutMs,
          );
        } catch (error) {
          results.push({
            case_id: `t1.5.cleanup.${providerInstanceName}`,
            provider_driver: null,
            provider_instance: providerInstanceName,
            method: "provider.delete",
            scenario: null,
            status: "failed",
            diagnostic: String(error),
            captured_requests: 0,
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
          });
        }
      }
    }
    if (cloudFixture && cloudAdmin) {
      let shouldRemoveCloudRules = cloudActive;
      if (!shouldRemoveCloudRules) {
        try {
          const view = await cloudAdmin.call(
            "driver_metadata_update.get",
            {},
          ) as { active_revision?: unknown };
          shouldRemoveCloudRules = view.active_revision === cloudRevision;
        } catch {
        }
      }
      if (shouldRemoveCloudRules) {
        try {
          const cleanup = await cloudFixture.publish({
            revisionSeq: cloudCleanupRevision,
            files: [],
            tombstones: t15OpenAiTombstone(cloudCleanupRevision),
          });
          await setCloudUpdateSource(cloudAdmin, cleanup.sourceUrl);
          await waitCloudUpdateConverged(
            cloudAdmin,
            cloudCleanupRevision,
            input.timeoutMs,
          );
        } catch (error) {
          results.push({
            case_id: "t1.5.cleanup.cloud-update",
            provider_driver: null,
            method: "driver_metadata_update.set",
            scenario: null,
            status: "failed",
            diagnostic: String(error),
            captured_requests: 0,
            started_at: new Date().toISOString(),
            elapsed_ms: 0,
          });
        }
      }
      await disableCloudUpdate(cloudAdmin).catch((error) =>
        results.push({
          case_id: "t1.5.cleanup.cloud-update-disable",
          provider_driver: null,
          method: "driver_metadata_update.set",
          scenario: null,
          status: "failed",
          diagnostic: String(error),
          captured_requests: 0,
          started_at: new Date().toISOString(),
          elapsed_ms: 0,
        })
      );
    }
    await restoreCloudConfig?.(cloudSystemConfig).catch((error) =>
      results.push({
        case_id: "t1.5.cleanup.cloud-update-config",
        provider_driver: null,
        method: "sys_config_set",
        scenario: null,
        status: "failed",
        diagnostic: String(error),
        captured_requests: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
      })
    );
    await cloudFixture?.stop().catch((error) =>
      results.push({
        case_id: "t1.5.cleanup.cloud-update-ndn",
        provider_driver: null,
        method: "cyfs-gateway.remove_router",
        scenario: null,
        status: "failed",
        diagnostic: String(error),
        captured_requests: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
      })
    );
    try {
      await resetMock(input.mockControlUrl);
    } catch (error) {
      results.push({
        case_id: "t1.5.cleanup.mock",
        provider_driver: null,
        method: "mock.reset",
        scenario: null,
        status: "failed",
        diagnostic: String(error),
        captured_requests: 0,
        started_at: new Date().toISOString(),
        elapsed_ms: 0,
      });
    }
    mockProcess?.kill("SIGTERM");
  }
  const cases: CaseReport[] = results.map((result) => ({
    run_id: runId,
    case_id: result.case_id,
    layer: "T1.5",
    status: result.status,
    provider_driver: result.provider_driver ?? undefined,
    provider_instance: result.provider_instance,
    exact_model: result.exact_model,
    api_type: result.api_type,
    method: result.method,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [{
      attempt: 1,
      started_at: result.started_at,
      elapsed_ms: result.elapsed_ms,
      status: result.status,
      failure_class: result.status === "failed"
        ? result.case_id.startsWith("t1.5.cleanup.")
          ? "cleanup_failed"
          : "provider_protocol_failed"
        : undefined,
      diagnostic: [
        result.protocol_contract_id
          ? `contract=${result.protocol_contract_id}`
          : undefined,
        result.scenario ? `scenario=${result.scenario}` : undefined,
        `captured_requests=${result.captured_requests}`,
        result.diagnostic,
      ].filter(Boolean).join("; "),
      estimated_cost_usd: 0,
      actual_cost_usd: 0,
      cost_status: "actual",
    }],
  }));
  const executedCaseIds = new Set(
    cases.map((item) => item.case_id).filter((caseId) =>
      plannedCaseIds.has(caseId)
    ),
  );
  const passedCaseIds = new Set(
    cases.filter((item) => item.status === "passed").map((item) =>
      item.case_id
    ),
  );
  const failedCaseIds = new Set(
    cases.filter((item) => item.status === "failed").map((item) =>
      item.case_id
    ),
  );
  const cleanupFailures = cases.filter((item) =>
    item.case_id.startsWith("t1.5.cleanup.") && item.status === "failed"
  );
  const finance = buildFinancialReport({
    entries: [],
    budgetUsd: 0,
    plannedMaxCalls: 0,
    plannedMaxCostUsd: 0,
  });
  const report: AcceptanceReport = {
    schema_version: 1,
    run_id: runId,
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    commit: await commitId(),
    baseline_revision: catalog.revision,
    allow_real_model_calls: false,
    planned_real_calls: 0,
    actual_real_calls: 0,
    estimated_cost_usd: 0,
    actual_cost_usd: 0,
    raw_cost_usd: 0,
    credit_applied_usd: 0,
    finance,
    cases,
    product_defects: cases.filter((item) =>
      item.status === "failed" && !item.case_id.startsWith("t1.5.cleanup.")
    )
      .map((item) =>
        defectFromFailure({
          component: "AICC",
          caseReport: item,
          expected:
            "AICC maps the official Provider protocol fixture through its real Adapter",
          observed: item.attempts.at(-1)?.diagnostic ?? "T1.5 case failed",
          evidencePaths: [`cases/${item.case_id}.json`],
        })
      ),
    cleanup: cleanupFailures.length === 0
      ? {
        status: "passed",
        details: [
          "temporary Provider instances were removed and the Mock selection was reset",
        ],
      }
      : {
        status: "failed",
        details: cleanupFailures.map((item) =>
          item.attempts.at(-1)?.diagnostic ?? item.case_id
        ),
      },
    manifest_coverage: {
      total: plannedCaseIds.size,
      executed: executedCaseIds.size,
      passed: [...executedCaseIds].filter((caseId) => passedCaseIds.has(caseId))
        .length,
      failed: [...executedCaseIds].filter((caseId) => failedCaseIds.has(caseId))
        .length,
      coverage_rate: plannedCaseIds.size === 0
        ? 1
        : executedCaseIds.size / plannedCaseIds.size,
      unexecuted_case_ids: [...plannedCaseIds].filter((caseId) =>
        !executedCaseIds.has(caseId)
      ).sort(),
    },
    protocol_evidence_revision: catalog.revision,
    official_evidence_checked_at: catalog.checked_at,
    limits: {
      global_concurrency: 1,
      provider_concurrency: 1,
      provider_min_interval_ms: input.providerMinIntervalMs,
    },
    providers: [...selectedProviders],
    targeted_retest_command: failedCaseIds.size === 0 ? undefined : [
      "AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 --",
      "--gateway-url",
      JSON.stringify(input.gatewayUrl),
      "--mock-base-url",
      JSON.stringify(input.mockBaseUrl),
      "--mock-control-url",
      JSON.stringify(input.mockControlUrl),
      "--allow-config-mutation",
      ...[...failedCaseIds].filter((caseId) => plannedCaseIds.has(caseId))
        .slice(0, 20)
        .flatMap((caseId) => ["--case", caseId]),
    ].join(" "),
  };
  const reportDir = resolve(input.reportDir, runId);
  await writeReport(reportDir, report);
  process.stdout.write(`${
    JSON.stringify(
      {
        report: join(reportDir, "summary.json"),
        cases: cases.length,
        passed: cases.filter((item) => item.status === "passed").length,
        failed: cases.filter((item) => item.status === "failed").length,
      },
      null,
      2,
    )
  }\n`);
  if (fatalError || cases.some((item) => item.status === "failed")) {
    process.exitCode = 1;
  }
}

if (
  process.argv[1] &&
  resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1])
) {
  main().catch((error) => {
    process.stderr.write(`T1.5 acceptance failed: ${String(error)}\n`);
    process.exitCode = 1;
  });
}
