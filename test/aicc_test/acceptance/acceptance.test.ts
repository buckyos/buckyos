import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { createServer } from "node:http";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_API_TYPES,
  methodsForApiType,
  parseCanonicalAssociationsFromRequirements,
} from "./canonical.ts";
import { buildStaticManifest } from "./cases.ts";
import {
  analyzeProviderMatrix,
  buildProviderMatrix,
  validateCaseManifest,
  validateProviderBaseline,
} from "./manifest.ts";
import { runPreflight } from "./preflight.ts";
import {
  ACCEPTANCE_REPORT_SCHEMA_VERSION,
  assertNoSecrets,
  caseTotals,
  isProviderRestricted,
  redact,
  validateAcceptanceReport,
} from "./report.ts";
import { buildMockSettings, configValue } from "./mock_settings.ts";
import { inventoriesFromModelsList } from "./inventory.ts";
import { withAiccSettingsOverride, withMockSettings } from "./settings_transaction.ts";
import { ProviderScheduler } from "./scheduler.ts";
import { buildFinancialReport, CostBudget, extractFinance } from "./finance.ts";
import { indexUsageByTask, providerCoverage, queryUsageEvents, usageEventFinance } from "./usage_audit.ts";
import { selectSingleProviderInstances } from "./inventory_selection.ts";
import { assertResponseShape, buildExactRequest } from "./payloads.ts";
import { manifestCoverage } from "./run_t1_gateway.ts";
import { applyProviderTokens, configuredProviderTokens } from "./provider_credentials.ts";
import { filterPhysicalModels } from "./model_coverage.ts";
import { bindOfficialCatalogInstances, fetchOfficialModelIds } from "./official_catalog.ts";
import { refreshProviderInventoriesUntilSuccess } from "./inventory_refresh.ts";
import { buildNdnGatewayConfig, gatewayRouterArgs } from "./ndn_fixture_service.ts";
import {
  buildCloudUpdateFiles,
  cloudUpdateTombstones,
  CLOUD_TEST_MOUNT_V1,
  CLOUD_TEST_MOUNT_V2,
} from "./cloud_update_cases.ts";
import { backupCloudUpdateConfig } from "./cloud_update_transaction.ts";
import type { ProviderInventory } from "./types.ts";
import { buildT1Coverage } from "./coverage.ts";
import { callInference, type RpcClient } from "./gateway.ts";
import {
  buildT15Manifest,
  loadProviderProtocolCatalog,
  protocolContract,
  selectOfficialModels,
  type ProviderProtocolContract,
  validateProviderProtocolCatalog,
  validateProviderAuxiliaryRequest,
  validateProviderRequest,
} from "./provider_protocol_contracts.ts";
import { assertT15ResponseMapping, buildT15TypedParams } from "./run_t15_gateway.ts";
import { createT15MockHandler, T15_PROVIDER_DISCOVERY_CONTRACTS } from "./t15_mock_provider.ts";
import {
  MOCK_PROVIDER_CONTRACT_VERSION,
  MOCK_PROVIDER_MANAGEMENT_ROUTES,
  MOCK_PROVIDER_SCENARIOS,
  validateMockProviderContract,
} from "./mock_provider_contract.ts";
import {
  assertBackgroundRemovalTransparency,
  validateArtifactBytes,
  validateNamedArtifact,
} from "./artifact_validation.ts";
import { outputResources, parseJudgeVerdict, responseText, selectJudgeModel } from "./judge.ts";
import { parseToml } from "../../jarvis_media_dv/config.ts";
import {
  ASSET_LABEL,
  SCENARIOS,
} from "../../jarvis_media_dv/scenarios.ts";
import {
  buildEntryCoverage,
  productDefects,
  type StepResult,
} from "../../jarvis_media_dv/jarvis_media_dv.ts";

const here = dirname(fileURLToPath(import.meta.url));

function t15FieldValue(field: string, contract: ProviderProtocolContract, model: string): unknown {
  if (field === "model") return model;
  const type = contract.body_field_types[field]?.[0] ?? "string";
  if (type === "array") return [];
  if (type === "object") return {};
  if (type === "number") return 1;
  if (type === "boolean") return true;
  return `t15-${field}`;
}

function t15ProviderRequest(
  baseUrl: string,
  contract: ProviderProtocolContract,
  model: string,
  extraBody: Record<string, unknown> = {},
): { url: string; init: RequestInit } {
  let path = contract.path.replaceAll("{model}", encodeURIComponent(model));
  const headers = new Headers(contract.required_headers ?? {});
  if (contract.auth.kind === "header") headers.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
  else {
    const url = new URL(`${baseUrl}${path}`);
    url.searchParams.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
    path = `${url.pathname}${url.search}`;
  }
  const fields = Object.fromEntries(contract.required_body_fields.map((field) => [
    field,
    t15FieldValue(field, contract, model),
  ]));
  if (contract.content_type === "multipart/form-data") {
    const form = new FormData();
    for (const [field, value] of Object.entries({ ...fields, ...extraBody })) {
      if (value && typeof value === "object") form.append(field, new Blob(["t15"]), `${field}.bin`);
      else form.append(field, String(value));
    }
    return { url: `${baseUrl}${path}`, init: { method: contract.http_method, headers, body: form } };
  }
  headers.set("content-type", contract.content_type);
  return {
    url: `${baseUrl}${path}`,
    init: { method: contract.http_method, headers, body: JSON.stringify({ ...fields, ...extraBody }) },
  };
}

test("T2 fixture NDN service is isolated and routed only for the run lifetime", () => {
  const config = buildNdnGatewayConfig({
    controlPort: 13452,
    dataPort: 34080,
    routePrefix: "/aicc-test-ndn-run-1",
    namedStoreConfigPath: "/opt/buckyos/storage/named_store.json",
  }) as {
    stacks: Record<string, { bind: string }>;
    servers: Record<string, Record<string, unknown>>;
  };
  assert.equal(config.stacks.__control_server__.bind, "127.0.0.1:13452");
  assert.equal(config.stacks.aicc_ndn_http.bind, "127.0.0.1:34080");
  assert.deepEqual(config.servers.aicc_ndn, {
    type: "cyfs-dir",
    named_store_config_path: "/opt/buckyos/storage/named_store.json",
    url_prefix: "/aicc-test-ndn-run-1",
  });
  assert.deepEqual(gatewayRouterArgs({
    action: "add_router",
    routePrefix: "/aicc-test-ndn-run-1",
    dataPort: 34080,
    gatewayControlUrl: "http://127.0.0.1:13451",
  }), [
    "add_router",
    "--id",
    "server:node_gateway",
    "--uri",
    "/aicc-test-ndn-run-1",
    "--target",
    "http://127.0.0.1:34080",
    "--server",
    "http://127.0.0.1:13451",
  ]);
});

test("cloud update fixtures replace complete catalog files and tombstone every cloud identity", async () => {
  const first = await buildCloudUpdateFiles(42, "v1");
  const second = await buildCloudUpdateFiles(43, "v2");
  assert.deepEqual(first.map((file) => `${file.catalog_kind}:${file.catalog_id}`), [
    "model_driver:openai",
    "provider_rules:aicc-cloud-update-openai",
    "known_provider:aicc-cloud-update-openai",
  ]);
  const firstModels = first[0].contents.models as Array<Record<string, unknown>>;
  const secondModels = second[0].contents.models as Array<Record<string, unknown>>;
  assert.equal(firstModels.some((model) => model.id === "text-embedding-3-small"), false);
  assert.equal(secondModels.some((model) => model.id === "text-embedding-3-small"), true);
  assert.ok((firstModels.find((model) => model.id === "gpt-5.6")?.logical_mounts as string[]).includes(CLOUD_TEST_MOUNT_V1));
  assert.ok(!(firstModels.find((model) => model.id === "gpt-5.6")?.logical_mounts as string[]).includes(CLOUD_TEST_MOUNT_V2));
  assert.ok((secondModels.find((model) => model.id === "gpt-5.6")?.logical_mounts as string[]).includes(CLOUD_TEST_MOUNT_V2));
  assert.deepEqual(cloudUpdateTombstones(44).map((item) => `${item.catalog_kind}:${item.catalog_id}`), [
    "model_driver:openai",
    "provider_rules:aicc-cloud-update-openai",
    "known_provider:aicc-cloud-update-openai",
  ]);
});

test("cloud update config cleanup retries with refreshed authentication", async () => {
  let expiredDeletes = 0;
  let refreshedDeletes = 0;
  const expired: RpcClient = {
    call: async (method) => {
      if (method === "sys_config_get") return null;
      expiredDeletes += 1;
      throw new Error("ExpiredSignature");
    },
  };
  const refreshed: RpcClient = {
    call: async (method) => {
      assert.equal(method, "sys_config_delete");
      refreshedDeletes += 1;
      return null;
    },
  };
  const restore = await backupCloudUpdateConfig(expired);
  await assert.rejects(restore(), /ExpiredSignature/);
  await restore(refreshed);
  await restore(refreshed);
  assert.equal(expiredDeletes, 1);
  assert.equal(refreshedDeletes, 1);
});

test("judge model selection prefers current exact Gemini and honors overrides", () => {
  const inventories: ProviderInventory[] = [
    {
      provider_driver: "openai",
      provider_instance_name: "openai-main",
      models: [{
        exact_model: "gpt-5.6-sol@openai-main",
        provider_model_id: "gpt-5.6-sol",
        api_types: ["llm"],
        logical_mounts: [],
      }],
    },
    {
      provider_driver: "google-gemini",
      provider_instance_name: "google-gemini-main",
      models: [{
        exact_model: "gemini-3.7-flash@google-gemini-main",
        provider_model_id: "gemini-3.7-flash",
        api_types: ["llm"],
        logical_mounts: [],
      }],
    },
  ];
  assert.equal(
    selectJudgeModel("llm.plan.default", inventories),
    "gemini-3.7-flash@google-gemini-main",
  );
  assert.equal(selectJudgeModel("custom@judge", inventories), "custom@judge");
});

test("Judge verdict parser enforces the requested strict schema", () => {
  assert.deepEqual(parseJudgeVerdict('{"pass":true,"score":0.9,"reason":"meets rubric"}', 0.8), {
    passed: true,
    score: 0.9,
    reason: "meets rubric",
  });
  assert.throws(() => parseJudgeVerdict('{"pass":true,"score":0.9,"reasoning":"ok"}', 0.8));
  assert.throws(() => parseJudgeVerdict('{"pass":true,"score":0.9,"reason":"ok","extra":1}', 0.8));
  assert.throws(() => parseJudgeVerdict(`{"pass":true,"score":0.9,"reason":"${"x".repeat(241)}"}`, 0.8));
  assert.throws(() => parseJudgeVerdict('prefix {"pass":true,"score":0.9,"reason":"ok"}', 0.8));
});

test("shared TOML parser accepts finite decimal and exponent numbers", () => {
  assert.deepEqual(parseToml("cost = 0.01\nsmall = -2.5e-3\nwhole = 8\n"), {
    cost: 0.01,
    small: -0.0025,
    whole: 8,
  });
  assert.throws(() => parseToml("cost = 1e999\n"), /non-finite TOML number/);
});

test("Judge text extraction ignores echoed Provider request bodies", () => {
  const texts = responseText({
    result: {
      message: { content: [{ type: "text", text: '{"pass":true}' }] },
      extra: {
        candidate_text: "untrusted duplicated transcript",
        provider_io: {
          input: { messages: [{ content: "untrusted echoed judge prompt" }] },
        },
      },
    },
  });
  assert.deepEqual(texts, ['{"pass":true}']);
});

test("Judge resource extraction includes request resources and output artifacts", () => {
  assert.deepEqual(outputResources({
    payload: {
      resources: [{ kind: "base64", mime: "image/png", data_base64: "aW1hZ2U=" }],
    },
    result: {
      artifacts: [{ mime: "audio/mpeg", resource: { kind: "named_object", obj_id: "chunk:test" } }],
    },
  }), [{
    type: "image",
    source: { kind: "base64", mime: "image/png", data_base64: "aW1hZ2U=" },
  }, {
    type: "document",
    source: { kind: "named_object", obj_id: "chunk:test", mime_hint: "audio/mpeg" },
  }]);
});

test("LLM acceptance exposes only the breaking-change chat method", async () => {
  assert.deepEqual(methodsForApiType("llm"), ["chat.completions.create"]);
  assert.doesNotMatch(JSON.stringify(await baseline()), /llm\.completion/);
});

test("canonical requirements preserve separate api_type and method value sets", async () => {
  const source = await readFile(join(here, "../../../doc/aicc/aicc_e2e_test_requirements.md"), "utf8");
  const associations = parseCanonicalAssociationsFromRequirements(source);
  assert.deepEqual(associations.get("llm"), ["chat.completions.create"]);
  assert.deepEqual(associations.get("image.txt2img"), ["images.generate"]);
  assert.notEqual("image.txt2img", associations.get("image.txt2img")?.[0]);
  assert.deepEqual([...associations.keys()], CANONICAL_API_TYPES);
});

async function baseline() {
  return validateProviderBaseline(JSON.parse(
    await readFile(join(here, "provider_capability_baseline.json"), "utf8"),
  ));
}

function matrixInputs(inventories: ProviderInventory[]) {
  return { officialInventories: structuredClone(inventories), aiccInventories: inventories };
}

test("official catalog fetches paginated Provider inventory independently of AICC", async () => {
  const profile = (await baseline()).providers.find((item) => item.provider_driver === "claude")!;
  const requests: URL[] = [];
  const ids = await fetchOfficialModelIds({
    profile,
    token: "catalog-test-token",
    timeoutMs: 1_000,
    fetcher: async (input, init) => {
      const url = new URL(input.toString());
      requests.push(url);
      assert.equal(new Headers(init?.headers).get("x-api-key"), "catalog-test-token");
      return new Response(JSON.stringify(requests.length === 1
        ? { data: [{ id: "claude-opus-5" }], has_more: true, last_id: "page-1" }
        : { data: [{ id: "claude-fable-5" }], has_more: false }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    },
  });
  assert.deepEqual(ids, ["claude-fable-5", "claude-opus-5"]);
  assert.equal(requests[1].searchParams.get("after_id"), "page-1");
});

test("SN official catalog requires an independent bearer session token", async () => {
  const profile = (await baseline()).providers.find((item) => item.provider_driver === "sn-ai-provider")!;
  await assert.rejects(
    fetchOfficialModelIds({ profile, timeoutMs: 1_000, fetcher: async () => new Response() }),
    /official catalog credential is required/,
  );
  const ids = await fetchOfficialModelIds({
    profile,
    token: "sn-session-token",
    timeoutMs: 1_000,
    fetcher: async (_input, init) => {
      assert.equal(new Headers(init?.headers).get("authorization"), "Bearer sn-session-token");
      return new Response(JSON.stringify({ data: [{ id: "gpt-5" }] }), { status: 200 });
    },
  });
  assert.deepEqual(ids, ["gpt-5"]);
});

test("Fal official catalog verifies the parameterized endpoint protocol scope", async () => {
  const profile = (await baseline()).providers.find((item) => item.provider_driver === "fal")!;
  const expected = profile.official_catalog.endpoint_ids!;
  const ids = await fetchOfficialModelIds({
    profile,
    token: "fal-catalog-token",
    timeoutMs: 1_000,
    fetcher: async (input, init) => {
      const url = new URL(input.toString());
      assert.deepEqual(url.searchParams.getAll("endpoint_id"), expected);
      assert.equal(url.searchParams.get("status"), "active");
      assert.equal(url.searchParams.get("limit"), String(expected.length));
      assert.equal(new Headers(init?.headers).get("authorization"), "Key fal-catalog-token");
      return new Response(JSON.stringify({
        models: expected.map((endpoint_id) => ({ endpoint_id })),
        has_more: false,
        next_cursor: null,
      }), { status: 200 });
    },
  });
  assert.deepEqual(ids, [...expected].sort((left, right) => left.localeCompare(right)));
});

test("Fal scoped catalog fails closed when an endpoint is no longer active", async () => {
  const profile = (await baseline()).providers.find((item) => item.provider_driver === "fal")!;
  await assert.rejects(
    fetchOfficialModelIds({
      profile,
      token: "fal-catalog-token",
      timeoutMs: 1_000,
      fetcher: async () => new Response(JSON.stringify({
        models: profile.official_catalog.endpoint_ids!.slice(1).map((endpoint_id) => ({ endpoint_id })),
        has_more: false,
      }), { status: 200 }),
    }),
    /official catalog scope mismatch.*missing=fal-ai\/esrgan/,
  );
});

test("official catalog network failures do not expose query credentials", async () => {
  const profile = (await baseline()).providers.find((item) => item.provider_driver === "google-gemini")!;
  await assert.rejects(
    fetchOfficialModelIds({
      profile,
      token: "catalog-secret-value",
      timeoutMs: 1_000,
      fetcher: async (input) => {
        throw new Error(`failed ${input.toString()}`);
      },
    }),
    (error: unknown) => {
      assert.doesNotMatch(String(error), /catalog-secret-value/);
      assert.match(String(error), /network error/);
      return true;
    },
  );
});

test("T2 stops each selected Provider inventory refresh after its first success", async () => {
  const selected: ProviderInventory[] = ["openai", "claude"].map((driver) => ({
    provider_driver: driver,
    provider_instance_name: `${driver}-main`,
    inventory_revision: `${driver}-before`,
    models: [],
  }));
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  let reads = 0;
  const result = await refreshProviderInventoriesUntilSuccess({
    aicc: {
      call: async (method, params) => {
        calls.push({ method, params });
        return {
          ok: true,
          provider_instance_name: params.provider_instance_name,
          inventory_revision: `${params.provider_instance_name}-refresh`,
        };
      },
    },
    selectedInventories: selected,
    readInventories: async () => {
      reads += 1;
      return selected.map((inventory) => ({
        ...inventory,
        inventory_revision: `${inventory.provider_driver}-after`,
      }));
    },
    maxAttempts: 3,
    retryDelayMs: 0,
  });
  assert.equal(reads, 1);
  assert.deepEqual(calls, selected.map((inventory) => ({
    method: "provider.refresh_models",
    params: { provider_instance_name: inventory.provider_instance_name },
  })));
  assert.deepEqual(
    result.evidence.map((item) => item.after_inventory_revision),
    ["openai-after", "claude-after"],
  );
});

test("T2 stops before models.list when a Provider inventory refresh fails", async () => {
  let refreshCalls = 0;
  let reads = 0;
  await assert.rejects(
    refreshProviderInventoriesUntilSuccess({
      aicc: {
        call: async () => {
          refreshCalls += 1;
          return { ok: false };
        },
      },
      selectedInventories: [{
        provider_driver: "openai",
        provider_instance_name: "openai-main",
        models: [],
      }],
      readInventories: async () => {
        reads += 1;
        return [];
      },
      maxAttempts: 3,
      retryDelayMs: 0,
    }),
    /provider inventory refresh failed/,
  );
  assert.equal(refreshCalls, 3);
  assert.equal(reads, 0);
});

test("T2 inventory refresh backs off until the first success and then stops", async () => {
  let refreshCalls = 0;
  const delays: number[] = [];
  const result = await refreshProviderInventoriesUntilSuccess({
    aicc: {
      call: async (_method, params) => {
        refreshCalls += 1;
        if (refreshCalls < 3) throw new Error("rate limited");
        return {
          ok: true,
          provider_instance_name: params.provider_instance_name,
          inventory_revision: "openai-success",
        };
      },
    },
    selectedInventories: [{
      provider_driver: "openai",
      provider_instance_name: "openai-main",
      models: [],
    }],
    readInventories: async () => [{
      provider_driver: "openai",
      provider_instance_name: "openai-main",
      inventory_revision: "openai-success",
      models: [],
    }],
    maxAttempts: 5,
    retryDelayMs: 100,
    providerRetryDelayMs: { openai: 250 },
    sleep: async (delayMs) => {
      delays.push(delayMs);
    },
  });
  assert.equal(refreshCalls, 3);
  assert.deepEqual(delays, [250, 500]);
  assert.equal(result.evidence[0].attempt_count, 3);
});

test("official catalogs bind to refreshed AICC Provider instances", () => {
  const catalogs: ProviderInventory[] = [{
    provider_driver: "openai",
    provider_instance_name: "openai-catalog",
    models: [{
      exact_model: "gpt-5@openai-catalog",
      provider_model_id: "gpt-5",
      api_types: [],
      logical_mounts: [],
    }],
  }];
  const bound = bindOfficialCatalogInstances(catalogs, [{
    provider_driver: "openai",
    provider_instance_name: "openai-live",
    models: [],
  }]);
  assert.equal(bound[0].provider_instance_name, "openai-live");
  assert.equal(bound[0].models[0].exact_model, "gpt-5@openai-live");
});

test("official catalog is the inventory baseline and exposes AICC omissions", async () => {
  const instance = "openai-test-a";
  const officialInventories: ProviderInventory[] = [{
    provider_instance_name: instance,
    provider_driver: "openai",
    models: ["gpt-5.6-sol", "gpt-6"].map((id) => ({
      exact_model: `${id}@${instance}`,
      provider_model_id: id,
      api_types: [],
      logical_mounts: [],
    })),
  }];
  const aiccInventories: ProviderInventory[] = [{
    provider_instance_name: instance,
    provider_driver: "openai",
    models: [{
      exact_model: `gpt-5.6-sol@${instance}`,
      provider_model_id: "gpt-5.6-sol",
      api_types: ["llm", "vision.ocr", "vision.caption"],
      logical_mounts: [],
    }],
  }];
  const result = analyzeProviderMatrix({
    baseline: await baseline(),
    officialInventories,
    aiccInventories,
  });
  assert.ok(result.mismatches.includes("official_supported_but_aicc_missing openai/gpt-6"));
  assert.ok(result.cells.some((cell) => cell.provider_model_id === "gpt-5.6-sol"));
  assert.ok(result.cells.every((cell) => cell.provider_model_id !== "gpt-6"));
});

test("official catalog coverage rules remove logical aliases and deduplicate physical models", async () => {
  const providerBaseline = structuredClone(await baseline());
  const openai = providerBaseline.providers.find((item) => item.provider_driver === "openai")!;
  openai.coverage_rules = [
    ...(openai.coverage_rules ?? []),
    {
      model_pattern: "gpt-current",
      action: "alias",
      physical_model_id: "gpt-5.6-sol",
      reason: "logical_alias",
      source_urls: ["https://platform.openai.com/docs/api-reference/models"],
      evidence_summary: "Test alias for one physical model.",
    },
    {
      model_pattern: "gpt-cheapest",
      action: "exclude",
      reason: "logical_alias",
      source_urls: ["https://platform.openai.com/docs/api-reference/models"],
      evidence_summary: "Test routing alias rather than a physical model.",
    },
  ];
  const result = filterPhysicalModels({
    baseline: providerBaseline,
    source: "official_catalog",
    inventories: [{
      provider_instance_name: "openai-main",
      provider_driver: "openai",
    models: ["gpt-5.6-sol", "gpt-current", "gpt-cheapest"].map((id) => ({
        exact_model: `${id}@openai-main`,
        provider_model_id: id,
        api_types: [],
        logical_mounts: [],
      })),
    }],
  });
  assert.deepEqual(result.inventories[0].models.map((model) => model.provider_model_id), ["gpt-5.6-sol"]);
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gpt-current")?.reason, "duplicate_physical_model");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gpt-cheapest")?.reason, "logical_alias");
});

test("preflight covers protocol, providers, and static cases", async () => {
  const result = await runPreflight();
  assert.equal(result.canonical_api_types, CANONICAL_API_TYPES.length);
  assert.ok(result.static_cases > CANONICAL_API_TYPES.length);
  assert.ok(result.t15_cases > CANONICAL_API_TYPES.length);
  assert.equal(result.mock_provider_contract_version, MOCK_PROVIDER_CONTRACT_VERSION);
  assert.deepEqual(result.provider_drivers, [
    "claude",
    "deepseek",
    "doubao",
    "fal",
    "glm",
    "google-gemini",
    "kimi",
    "minimax",
    "openai",
    "openrouter",
    "qwen",
    "sn-ai-provider",
  ]);
});

test("T1 Mock Provider uses a fixed versioned control contract", () => {
  assert.doesNotThrow(validateMockProviderContract);
  assert.deepEqual(MOCK_PROVIDER_MANAGEMENT_ROUTES.reset, {
    method: "POST",
    path: "/__mock/reset",
  });
  assert.ok(MOCK_PROVIDER_SCENARIOS.includes("stream_success"));
  assert.ok(MOCK_PROVIDER_SCENARIOS.includes("async_success"));
  assert.ok(MOCK_PROVIDER_SCENARIOS.includes("rate_limit"));
});

test("T3 manifest includes six inbound kinds and multi-attachment history", () => {
  const kinds = new Set(["message-text", ...Object.values(ASSET_LABEL).map((mime) =>
    mime === "application/zip" ? "archive" : mime.split("/")[0]
  )]);
  assert.deepEqual([...kinds].sort(), ["archive", "audio", "image", "message-text", "text", "video"]);
  const multi = SCENARIOS.find((scenario) => scenario.id === "multi_attachment_current_and_history");
  assert.ok(multi);
  assert.ok(multi.steps.some((step) => (step.attachments?.length ?? 0) >= 3));
  assert.ok(multi.steps.some((step) => Boolean(step.replyToStep)));
  const videoPackage = SCENARIOS.find((scenario) => scenario.id === "multi_output_video_subtitle_cover");
  assert.deepEqual(videoPackage?.steps[0].expect.artifacts, ["video/", "text/", "image/"]);
  assert.deepEqual(videoPackage?.steps[0].expect.attachmentCount, { min: 3, max: 3 });
  assert.equal(
    SCENARIOS.find((scenario) => scenario.id === "document_vector_retrieval")?.coverage?.status,
    "not_applicable",
  );
  for (const id of [
    "archive_valid_shapes",
    "archive_empty_rejected",
    "archive_corrupt_rejected",
    "archive_encrypted_rejected",
    "archive_path_traversal_rejected",
    "archive_many_files_rejected",
    "archive_large_expansion_rejected",
    "archive_deep_nesting_rejected",
  ]) {
    const scenario = SCENARIOS.find((item) => item.id === id);
    assert.ok(scenario, `missing T3 archive scenario ${id}`);
    assert.ok(scenario.requiredAssets.some((asset) => asset.startsWith("archive_")));
  }
  assert.equal(
    SCENARIOS.find((scenario) => scenario.id === "duplicate_inbound_idempotency")
      ?.steps[0].duplicateInbound,
    true,
  );
  assert.equal(
    SCENARIOS.find((scenario) => scenario.id === "outbound_delivery_idempotency")
      ?.steps[0].assertUniqueOutbound,
    true,
  );
  assert.equal(
    SCENARIOS.find((scenario) => scenario.id === "group_message_semantics")?.requiresGroup,
    true,
  );
  assert.equal(
    SCENARIOS.find((scenario) => scenario.id === "forwarded_message_source")?.steps[0].sourceDid,
    "did:bns:aicc-forward-origin",
  );
  assert.ok(
    SCENARIOS.filter((scenario) => !scenario.coverage)
      .flatMap((scenario) => scenario.steps)
      .some((step) => step.review.length > 0),
    "T3 must retain semantic rubrics for the parameterized LLM Judge",
  );
});

test("T3 entry audit proves six message kinds and multi-attachment in both directions", () => {
  const coverage = buildEntryCoverage({
    transports: ["msg-center"],
    scenarios: SCENARIOS,
    results: SCENARIOS.flatMap((scenario) => scenario.steps.map((step) => ({
      transport: "msg-center" as const,
      scenario_id: scenario.id,
      step_id: step.id,
      status: "passed" as const,
    }))),
  });
  assert.equal(coverage.length, 14);
  assert.deepEqual([...new Set(coverage.map((item) => item.kind))].sort(), [
    "archive",
    "audio",
    "document",
    "image",
    "multi_attachment",
    "text",
    "video",
  ]);
  assert.ok(coverage.every((item) => item.status === "covered"));
});

test("T3 report records product assertions with expected, observed, and evidence", () => {
  const base: StepResult = {
    transport: "msg-center",
    scenario_id: "text-basic",
    step_id: "reply",
    status: "failed",
    started_at: "2026-08-27T00:00:00.000Z",
    elapsed_ms: 1,
    prompt: "hello",
    reply_texts: [],
    reply_refs: [],
    automatic_checks: ["Jarvis returns a non-empty reply"],
    review: [],
    failure_class: "assertion_failed",
    error: "reply was empty",
  };
  const defects = productDefects([
    base,
    { ...base, step_id: "transport", failure_class: "message_transport_failed" },
  ]);
  assert.equal(defects.length, 1);
  assert.equal(defects[0].component, "Jarvis");
  assert.equal(defects[0].expected, "Jarvis returns a non-empty reply");
  assert.equal(defects[0].observed, "reply was empty");
  assert.deepEqual(defects[0].evidence_paths, [
    "summary.json",
    "conversations/msg-center-text-basic.md",
  ]);
});

test("T1 mock settings append run-scoped instances without mutating backup", () => {
  const original = {
    providers: [{ provider_instance_name: "production", provider_profile_id: "openai" }],
    session_config: { revision: "production" },
  };
  const serialized = JSON.stringify(original);
  const decoded = configValue({ value: serialized });
  const patched = buildMockSettings(decoded.parsed, {
    baseUrl: "http://127.0.0.1:4827/",
    runId: "run:one",
  });
  assert.equal(JSON.stringify(original), serialized);
  const providers = patched.providers as Array<Record<string, unknown>>;
  assert.equal(providers[0].provider_instance_name, "production");
  assert.deepEqual(providers.slice(1, 3).map((item) => item.provider_instance_name), [
    "dv-openai-a-run-one",
    "dv-openai-b-run-one",
  ]);
  assert.equal(providers.length, 12);
  assert.deepEqual(providers[1].credentials, { api_token: { locked: "mock-a-run-one" } });
  assert.deepEqual(
    providers.slice(8).map((item) => [item.provider_profile_id, item.protocol_adapter_id]),
    [
      ["custom", "openai-responses"],
      ["custom", "claude-messages"],
      ["custom", "gemini-interactions"],
      ["custom", "fal-queue"],
    ],
  );
  assert.equal((patched.session_config as { revision: string }).revision, "dv-routing-run-one");
  assert.equal(decoded.serialized, serialized);
});

test("models.list flat catalog is grouped into Provider inventories", () => {
  const inventories = inventoriesFromModelsList({
    generation: "g1",
    models: [
      {
        exact_model: "gpt-5.6@openai-main",
        provider_model_id: "gpt-5.6",
        provider_instance_name: "openai-main",
        provider_profile_id: "openai",
        inventory_revision: "r1",
        api_types: ["llm"],
        logical_mounts: ["llm.openai"],
      },
      {
        exact_model: "gemini-2.5-flash@gemini-main",
        provider_model_id: "gemini-2.5-flash",
        provider_instance_name: "gemini-main",
        provider_profile_id: "gemini",
        api_types: ["llm"],
        logical_mounts: ["llm.gemini"],
      },
    ],
  });
  assert.equal(inventories.length, 2);
  assert.equal(inventories[0].provider_driver, "openai");
  assert.equal(inventories[0].inventory_revision, "r1");
  assert.equal(inventories[1].provider_driver, "google-gemini");
});

test("T1 settings transaction restores exact backup after execution failure", async () => {
  const backup = JSON.stringify({ openai: { enabled: false } }, null, 2);
  const writes: string[] = [];
  let current = backup;
  let reloads = 0;
  const systemConfig = {
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === "sys_config_get") return { value: current };
      assert.equal(method, "sys_config_set");
      current = String(params.value);
      writes.push(current);
      return {};
    },
  };
  const aicc = { call: async () => { reloads += 1; return {}; } };
  await assert.rejects(
    withMockSettings({
      systemConfig,
      aicc,
      baseUrl: "http://127.0.0.1:4827",
      runId: "failure",
      execute: async () => { throw new Error("injected"); },
    }),
    /injected/,
  );
  assert.equal(writes.length, 2);
  assert.equal(writes[1], backup);
  assert.equal(reloads, 2);
});

test("settings transaction reauthenticates when the original cleanup session expires", async () => {
  const backup = JSON.stringify({ original: true });
  let current = backup;
  let originalWrites = 0;
  let refreshedWrites = 0;
  const originalSystemConfig = {
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === "sys_config_get") return { value: current };
      originalWrites += 1;
      if (originalWrites > 1) throw new Error("session expired");
      current = String(params.value);
      return {};
    },
  };
  const refreshedSystemConfig = {
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === "sys_config_get") return { value: current };
      current = String(params.value);
      refreshedWrites += 1;
      return {};
    },
  };
  const result = await withAiccSettingsOverride({
    systemConfig: originalSystemConfig,
    aicc: { call: async () => ({}) },
    description: "reauth cleanup",
    patch: (settings) => ({ ...settings, temporary: true }),
    execute: async () => "completed",
    refreshClients: async () => ({
      systemConfig: refreshedSystemConfig,
      aicc: { call: async () => ({}) },
    }),
  });
  assert.equal(result.result, "completed");
  assert.equal(result.cleanup, "restored");
  assert.equal(refreshedWrites, 1);
});

test("Provider credentials patch only the selected runtime instance without mutating input", () => {
  const original = {
    openai: {
      instances: [
        { provider_instance_name: "openai-one", provider_driver: "openai", api_token: "old-one" },
        { provider_instance_name: "openai-two", provider_driver: "openai", api_token: "old-two" },
        { provider_instance_name: "router", provider_driver: "openrouter", api_token: "old-router" },
      ],
    },
    gemini: { instances: [{ provider_instance_name: "gemini", api_token: "old-gemini" }] },
  };
  const patched = applyProviderTokens(original, {
    openai: "new-openai",
    openrouter: "new-router",
    "google-gemini": "new-gemini",
  }, {
    openai: "openai-two",
    openrouter: "router",
    "google-gemini": "gemini",
  }) as typeof original;
  assert.equal(original.openai.instances[1].api_token, "old-two");
  assert.equal(patched.openai.instances[0].api_token, "old-one");
  assert.equal(patched.openai.instances[1].api_token, "new-openai");
  assert.equal(patched.openai.instances[2].api_token, "new-router");
  assert.equal(patched.gemini.instances[0].api_token, "new-gemini");
  assert.throws(() => applyProviderTokens(original, { openai: "secret" }, {}), /multiple configured instances/);
});

test("Provider credentials accept TOML values or provider-specific environment variables", () => {
  const tokens = configuredProviderTokens({
    "provider_credentials.openai.api_token": "toml-openai",
  }, (name) => name === "AICC_CLAUDE_API_TOKEN" ? "env-claude" : undefined);
  assert.deepEqual(tokens, { openai: "toml-openai", claude: "env-claude" });
});

test("Provider credentials create one current-schema instance when the section is absent", () => {
  const patched = applyProviderTokens({
    "sn-ai-provider": { enabled: true, instances: [] },
  }, {
    openai: "openai-token",
    "google-gemini": "gemini-token",
    openrouter: "router-token",
  }, {}) as Record<string, { enabled: boolean; instances: Array<Record<string, unknown>> }>;
  assert.equal(patched.openai.instances.length, 2);
  assert.deepEqual(
    patched.openai.instances.map((instance) => instance.provider_driver),
    ["openai", "openrouter"],
  );
  assert.equal(patched.google.instances[0].provider_instance_name, "google-gemini-main");
  assert.equal(patched.google.instances[0].base_url, "https://generativelanguage.googleapis.com/v1beta");
});

test("provider scheduler runs sessions concurrently within global and provider limits", async () => {
  const scheduler = new ProviderScheduler(3, { maxConcurrency: 2, minIntervalMs: 0 });
  let globalActive = 0;
  let globalPeak = 0;
  const providerActive = new Map<string, number>();
  const providerPeaks = new Map<string, number>();
  const items = ["openai", "openai", "openai", "claude", "claude", "claude"]
    .map((provider_driver, id) => ({ provider_driver, id }));
  const started = Date.now();
  const results = await scheduler.run(items, async (item) => {
    globalActive += 1;
    globalPeak = Math.max(globalPeak, globalActive);
    const active = (providerActive.get(item.provider_driver) ?? 0) + 1;
    providerActive.set(item.provider_driver, active);
    providerPeaks.set(item.provider_driver, Math.max(providerPeaks.get(item.provider_driver) ?? 0, active));
    await new Promise((resolve) => setTimeout(resolve, 40));
    providerActive.set(item.provider_driver, active - 1);
    globalActive -= 1;
    return item.id;
  });
  assert.deepEqual(results, [0, 1, 2, 3, 4, 5]);
  assert.equal(globalPeak, 3);
  assert.ok([...providerPeaks.values()].every((peak) => peak <= 2));
  assert.ok(Date.now() - started < 210, "execution unexpectedly became fully serial");
});

test("provider scheduler enforces request start interval per provider", async () => {
  const scheduler = new ProviderScheduler(4, { maxConcurrency: 3, minIntervalMs: 30 });
  const starts: number[] = [];
  await scheduler.run(
    Array.from({ length: 4 }, () => ({ provider_driver: "openai" })),
    async () => { starts.push(Date.now()); },
  );
  for (let index = 1; index < starts.length; index += 1) {
    assert.ok(starts[index] - starts[index - 1] >= 24, `start interval was ${starts[index] - starts[index - 1]}ms`);
  }
});

test("finance extracts AICC usage and USD cost without treating missing cost as zero", () => {
  assert.deepEqual(extractFinance({ result: {
    usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15 },
    cost: { amount: 0.0012, currency: "USD" },
  } }), {
    usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15, request_units: undefined },
    actualCostUsd: 0.0012,
    rawCostUsd: undefined,
    creditAppliedUsd: undefined,
  });
  assert.deepEqual(extractFinance({ result: { usage: { total_tokens: 2 } } }), {
    usage: { input_tokens: undefined, output_tokens: undefined, total_tokens: 2, request_units: undefined },
    actualCostUsd: undefined,
    rawCostUsd: undefined,
    creditAppliedUsd: undefined,
  });
  assert.deepEqual(extractFinance({ result: {
    cost: { amount: 0.02, currency: "USD" },
    extra: { billing: { raw_cost_usd: 0.05, sn_ai_provider_credit_applied_usd: 0.03 } },
  } }), {
    usage: undefined,
    actualCostUsd: 0.02,
    rawCostUsd: 0.05,
    creditAppliedUsd: 0.03,
  });
});

test("finance budget reserves concurrent attempts and reports unknown exposure", () => {
  const budget = new CostBudget(0.03);
  const first = budget.reserve(0.01);
  const second = budget.reserve(0.01);
  assert.throws(() => budget.reserve(0.02), /budget exhausted/);
  budget.settle(first, 0.012);
  budget.settle(second);
  const report = buildFinancialReport({
    budgetUsd: 0.03,
    plannedMaxCalls: 2,
    plannedMaxCostUsd: 0.02,
    entries: [
      { case_id: "a", attempt: 1, provider_driver: "openai", provider_instance: "one", exact_model: "m@one", api_type: "llm", method: "chat.completions.create", started_at: "now", status: "passed", estimated_cost_usd: 0.01, actual_cost_usd: 0.012, cost_status: "actual" },
      { case_id: "b", attempt: 1, provider_driver: "openai", provider_instance: "one", exact_model: "m@one", api_type: "llm", method: "chat.completions.create", started_at: "now", status: "failed", estimated_cost_usd: 0.01, cost_status: "unknown" },
    ],
  });
  assert.equal(report.actual_cost_usd, 0.012);
  assert.equal(report.estimated_exposure_usd, 0.01);
  assert.equal(report.unknown_cost_calls, 1);
  assert.equal(report.by_provider[0].calls, 2);
});

test("durable usage audit paginates and preserves finance snapshot", async () => {
  let page = 0;
  const events = await queryUsageEvents({
    aicc: { call: async (_method, params) => {
      page += 1;
      assert.equal((params.time_range as { kind: string }).kind, "explicit");
      return page === 1
        ? {
          events: [{ event_id: "e1", task_id: "t1", capability: "llm", request_model: "m", provider_model: "m@p", input_tokens: 3, output_tokens: 2, total_tokens: 5, usage_json: {}, finance_snapshot_json: { amount: 0.02, currency: "USD", billing: { raw_cost_usd: 0.05, sn_ai_provider_credit_applied_usd: 0.03 } }, created_at_ms: 1 }],
          next_cursor: "next",
        }
        : { events: [{ event_id: "e2", task_id: "t2", capability: "image", request_model: "i", provider_model: "i@p", request_units: 1, usage_json: {}, created_at_ms: 2 }] };
    } },
    startTimeMs: 0,
    endTimeMs: 3,
    taskIds: ["t1", "t2"],
  });
  assert.equal(events.length, 2);
  assert.equal(indexUsageByTask(events).get("t1")?.length, 1);
  assert.deepEqual(usageEventFinance(events[0]), {
    usage: { input_tokens: 3, output_tokens: 2, total_tokens: 5, request_units: undefined },
    actualCostUsd: 0.02,
    rawCostUsd: 0.05,
    creditAppliedUsd: 0.03,
  });
});

test("T3 provider audit derives driver coverage from exact models and runtime inventory", () => {
  const coverage = providerCoverage({
    exactModels: ["vendor@model@openai-main", "claude-4@claude-main", "malformed"],
    inventories: [
      { provider_instance_name: "openai-main", provider_driver: "openai" },
      { provider_instance_name: "claude-main", provider_driver: "claude" },
    ],
    expectedDrivers: ["openai", "claude", "fal"],
  });
  assert.deepEqual(coverage.observedInstances, ["claude-main", "openai-main"]);
  assert.deepEqual(coverage.observedDrivers, ["claude", "openai"]);
  assert.deepEqual(coverage.missingExpectedDrivers, ["fal"]);
});

test("T1 manifest coverage does not count declared but unexecuted cases", () => {
  const coverage = manifestCoverage([{
    run_id: "run",
    case_id: "t1.route.exact_model_hits_instance",
    layer: "T1",
    status: "passed",
    api_type: "llm",
    method: "chat.completions.create",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  }]);
  assert.equal(coverage.executed, 1);
  assert.equal(coverage.passed, 1);
  assert.ok(coverage.total > coverage.executed);
  assert.ok(coverage.unexecuted_case_ids.includes("t1.route.logical_model_selects_candidate"));
});

test("T1 report separates requirement branches from combination cells", () => {
  const manifest = buildStaticManifest();
  const selected = manifest.find((item) => item.case_id === "t1.task.concurrent_idempotency");
  assert.ok(selected);
  const coverage = buildT1Coverage(manifest, [{
    run_id: "run",
    case_id: selected.case_id,
    layer: "T1",
    status: "failed",
    method: selected.method,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  }]);
  const branch = coverage.branches.find((item) => item.branch_id === "task.concurrent_idempotency");
  assert.equal(branch?.status, "failed");
  assert.ok(coverage.total_branches > 50);
  assert.equal(
    coverage.combination_groups.find((item) => item.group_id === "cross_cutting")?.executed_cells,
    1,
  );
  assert.equal(
    coverage.combination_groups.find((item) => item.group_id === "canonical_api_routes")?.executed_cells,
    0,
  );
});

test("T1 requirement coverage keeps optional credential skips distinct from failures", () => {
  const manifest = buildStaticManifest();
  const selected = manifest.find((item) => item.case_id === "t1.security.cross_tenant");
  assert.ok(selected);
  const coverage = buildT1Coverage(manifest, [{
    run_id: "run",
    case_id: selected.case_id,
    layer: "T1",
    status: "skipped",
    method: selected.method,
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  }]);
  assert.equal(
    coverage.branches.find((item) => item.branch_id === "security.cross_tenant")?.status,
    "skipped",
  );
  assert.equal(coverage.skipped_branches, 1);
  assert.equal(coverage.failed_branches, 0);
});

test("manifest rejects duplicate case ids", () => {
  const manifest = buildStaticManifest();
  assert.throws(
    () => validateCaseManifest([manifest[0], manifest[0]]),
    /duplicate case_id/,
  );
});

test("manifest rejects an illegal method and api_type association", () => {
  const testCase = structuredClone(buildStaticManifest().find((item) => item.api_type === "image.txt2img")!);
  testCase.method = "image.txt2img";
  assert.throws(() => validateCaseManifest([testCase]), /method is not valid for api_type/);
});

test("provider baseline requires profile, adapter, and model driver identities", async () => {
  const valid = await baseline();
  const invalid = structuredClone(valid) as unknown as Record<string, unknown>;
  const providers = invalid.providers as Array<Record<string, unknown>>;
  delete providers[0].provider_profile_id;
  assert.throws(() => validateProviderBaseline(invalid), /provider_profile_id/);
});

test("T2 provider matrix has one minimal cell per physical model and API type", async () => {
  const cells = buildProviderMatrix({
    baseline: await baseline(),
    ...matrixInputs([{
      provider_instance_name: "fal-test-a",
      provider_driver: "fal",
      models: [{
        exact_model: "fal-ai/esrgan@fal-test-a",
        provider_model_id: "fal-ai/esrgan",
        api_types: ["image.upscale"],
        logical_mounts: ["image.upscale"],
      }],
    }]),
  });
  assert.equal(cells.length, 1);
  assert.equal(cells[0].exact_model, "fal-ai/esrgan@fal-test-a");
  assert.equal(cells[0].method, "image.upscale");
  assert.equal(cells[0].resource_representation, undefined);
  assert.equal(cells[0].variant, undefined);
});

test("T2 selects one configured instance per provider and rejects ambiguity", () => {
  const inventories = ["one", "two"].map((name) => ({
    provider_instance_name: name,
    provider_driver: "openai",
    models: [],
  }));
  assert.throws(() => selectSingleProviderInstances({
    inventories,
    drivers: ["openai"],
    configured: {},
  }), /multiple instances/);
  assert.deepEqual(selectSingleProviderInstances({
    inventories,
    drivers: ["openai"],
    configured: { openai: "two" },
  }).map((item) => item.provider_instance_name), ["two"]);
});

test("T2 embedding assertion verifies usage, cost, space, count, and finite dimensions", () => {
  const cell = {
    case_id: "embedding",
    provider_driver: "openai",
    provider_instance: "one",
    exact_model: "embed@one",
    provider_model_id: "embed",
    api_type: "embedding.text",
    method: "embedding.text",
    baseline_status: "active" as const,
    input_kinds: ["text"],
    output_kinds: ["embedding"],
    source_urls: [],
  };
  assert.doesNotThrow(() => assertResponseShape(cell, {
    task_id: "task",
    status: "succeeded",
    result: {
      message: { role: "assistant", content: [] },
      usage: { input_tokens: 2 },
      cost: { amount: 0.001, currency: "USD" },
      extra: {
        embedding: {
          embedding_space_id: "openai:embed:2",
          data: [
            { index: 0, embedding: [0.1, 0.2] },
            { index: 1, embedding: [0.3, 0.4] },
          ],
        },
      },
    },
  }));
  assert.throws(() => assertResponseShape(cell, {
    task_id: "task",
    status: "succeeded",
    result: {
      message: { role: "assistant", content: [] },
      usage: { input_tokens: 2 },
      cost: { amount: 0.001, currency: "USD" },
      extra: { embedding: { embedding_space_id: "space", data: [{ embedding: [0.1, Number.NaN] }] } },
    },
  }), /item count|finite vector/);
});

test("T2 successful protocol response cannot omit durable accounting fields", () => {
  const cell = {
    case_id: "llm",
    provider_driver: "openai",
    provider_instance: "one",
    exact_model: "gpt@one",
    provider_model_id: "gpt",
    api_type: "llm",
    method: "chat.completions.create",
    baseline_status: "active" as const,
    input_kinds: ["text"],
    output_kinds: ["text"],
    source_urls: [],
  };
  assert.throws(() => assertResponseShape(cell, {
    task_id: "task",
    status: "succeeded",
    result: { message: { role: "assistant", content: [{ type: "text", text: "ok" }] } },
  }), /usage/);
});

test("T2 LLM output variants build and assert JSON schema and tool-call contracts", () => {
  const base = {
    case_id: "llm",
    provider_driver: "openai",
    provider_instance: "one",
    exact_model: "gpt@one",
    provider_model_id: "gpt",
    api_type: "llm",
    method: "chat.completions.create",
    baseline_status: "active" as const,
    input_kinds: ["text"],
    source_urls: [],
  };
  const jsonCell = { ...base, output_kinds: ["json"] };
  const jsonRequest = buildExactRequest({ cell: jsonCell, runId: "run", fixtures: {} });
  assert.deepEqual(jsonRequest.requirements, { must_features: ["json_output"], resp_format: "json" });
  assert.doesNotThrow(() => assertResponseShape(jsonCell, {
    task_id: "task",
    status: "succeeded",
    result: {
      message: { role: "assistant", content: [{ type: "text", text: '{"marker":"BUCKYOS-AICC-4827"}' }] },
      usage: {},
      cost: {},
    },
  }));
  const toolCell = { ...base, output_kinds: ["tool_call"] };
  const toolRequest = buildExactRequest({ cell: toolCell, runId: "run", fixtures: {} });
  assert.equal(
    ((((toolRequest.payload as Record<string, unknown>).input_json as Record<string, unknown>)
      .tool_specs) as unknown[]).length,
    1,
  );
  assert.doesNotThrow(() => assertResponseShape(toolCell, {
    task_id: "task",
    status: "succeeded",
    result: {
      message: {
        role: "assistant",
        content: [{ type: "tool_use", call_id: "call-1", name: "echo_marker", args: { marker: "BUCKYOS-AICC-4827" } }],
      },
      usage: {},
      cost: {},
    },
  }));
});

test("typed inference adapter removes the legacy request envelope", async () => {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const client: RpcClient = {
    call: (method, params) => {
      calls.push({ method, params });
      return Promise.resolve({ task_id: "task", status: "running" });
    },
  };
  const request = {
    model: { alias: "gpt@test" },
    payload: {
      input_json: {
        messages: [{ role: "user", content: [{ type: "text", text: "hello" }] }],
      },
      resources: [{ kind: "url", url: "https://example.invalid/image.png", mime_hint: "image/png" }],
      options: {},
    },
    idempotency_key: "idem",
  };

  await callInference(client, "chat.completions.create", request);
  await callInference(client, "images.generate", {
    ...request,
    payload: { input_json: { prompt: "a fox" }, resources: [], options: {} },
  });

  assert.equal(calls[0].method, "chat.completions.create");
  assert.equal(calls[0].params.exact_model, "gpt@test");
  assert.equal("model" in calls[0].params, false);
  assert.equal("payload" in calls[0].params, false);
  assert.equal(
    ((calls[0].params.messages as Array<{ content: unknown[] }>)[0].content).length,
    2,
  );
  assert.equal(calls[1].method, "images.generate");
  assert.equal(calls[1].params.prompt, "a fox");
  assert.equal("payload" in calls[1].params, false);
});

test("active official model families are not excluded by lifecycle filters", async () => {
  const providerBaseline = await baseline();
  const rules = (driver: string) => providerBaseline.providers
    .find((provider) => provider.provider_driver === driver)?.coverage_rules ?? [];
  assert.equal(rules("openai").some((rule) => rule.model_pattern === "gpt-5.5*"), false);
  assert.equal(rules("claude").some((rule) => rule.model_pattern === "claude-opus-4-*"), false);
  assert.equal(rules("google-gemini").some((rule) =>
    ["gemini-2.5-*", "gemini-3.1-*", "gemini-3.5-*", "gemini-3.6-*"].includes(rule.model_pattern)
  ), false);
  for (const driver of ["google-gemini", "fal"]) {
    assert.equal(providerBaseline.providers.find((provider) => provider.provider_driver === driver)
      ?.rules.some((rule) => rule.model_pattern === "*" && rule.status === "removed"), false);
  }
});

test("T2 Veo extension requests the protocol-fixed seven second duration", () => {
  const request = buildExactRequest({
    cell: {
      case_id: "veo-extend",
      provider_driver: "google-gemini",
      provider_instance: "gemini-main",
      exact_model: "veo-3.1-generate-preview@gemini-main",
      provider_model_id: "veo-3.1-generate-preview",
      api_type: "video.extend",
      method: "video.extend",
      baseline_status: "active",
      input_kinds: ["video"],
      output_kinds: ["video"],
      source_urls: [],
      resource_representation: "base64",
    },
    runId: "run",
    fixtures: { video: { kind: "base64", mime: "video/mp4", data_base64: "AA==" } },
  });
  assert.equal(
    ((request.payload as Record<string, unknown>).input_json as Record<string, unknown>).duration_seconds,
    7,
  );
});

test("provider matrix fails on AICC capability over-advertising", async () => {
  const providerBaseline = await baseline();
  assert.throws(() => buildProviderMatrix({
    baseline: providerBaseline,
    ...matrixInputs([{
      provider_instance_name: "openai-test-a",
      provider_driver: "openai",
      models: [{
        exact_model: "gpt-5.6-sol@openai-test-a",
        provider_model_id: "gpt-5.6-sol",
        api_types: ["llm", "rerank", "vision.ocr", "vision.caption"],
        logical_mounts: ["llm.openai.gpt-5.6-sol"],
      }],
    }]),
  }), /official_not_supported_but_aicc_advertised/);
});

test("provider matrix exposes baseline mismatches for reporting", async () => {
  const result = analyzeProviderMatrix({
    baseline: await baseline(),
    ...matrixInputs([{
      provider_instance_name: "openai-test-a",
      provider_driver: "openai",
      models: [{
        exact_model: "gpt-5.6-sol@openai-test-a",
        provider_model_id: "gpt-5.6-sol",
        api_types: ["llm", "rerank", "vision.ocr", "vision.caption"],
        logical_mounts: [],
      }],
    }]),
  });
  assert.ok(result.mismatches.some((item) => item.includes("aicc_advertised")));
});

test("physical model coverage excludes lifecycle and logical aliases while retaining variants", async () => {
  const result = filterPhysicalModels({
    baseline: await baseline(),
    inventories: [
      {
        provider_instance_name: "openai-default",
        provider_driver: "openai",
        models: [
          { exact_model: "gpt-image-1@openai-default", provider_model_id: "gpt-image-1", api_types: ["image.txt2img"], logical_mounts: [] },
          { exact_model: "sora-2@openai-default", provider_model_id: "sora-2", api_types: ["video.txt2video"], logical_mounts: [] },
          { exact_model: "gpt-3.5-turbo@openai-default", provider_model_id: "gpt-3.5-turbo", api_types: ["llm"], logical_mounts: [] },
          { exact_model: "gpt-5.1-codex-mini@openai-default", provider_model_id: "gpt-5.1-codex-mini", api_types: ["llm"], logical_mounts: [] },
          { exact_model: "o4-mini@openai-default", provider_model_id: "o4-mini", api_types: ["llm"], logical_mounts: [] },
          { exact_model: "gpt-5.6-sol@openai-default", provider_model_id: "gpt-5.6-sol", api_types: ["llm"], logical_mounts: [] },
          { exact_model: "gpt-5.6-sol:reasoning-high@openai-default", provider_model_id: "gpt-5.6-sol:reasoning-high", provider_actual_model_id: "gpt-5.6-sol", api_types: ["llm"], logical_mounts: [] },
        ],
      },
      {
        provider_instance_name: "gemini-default",
        provider_driver: "google-gemini",
        models: [
          { exact_model: "gemini-flash-latest@gemini-default", provider_model_id: "gemini-flash-latest", api_types: ["llm"], logical_mounts: [] },
          { exact_model: "gemini-3.7-flash@gemini-default", provider_model_id: "gemini-3.7-flash", api_types: ["llm"], logical_mounts: [] },
        ],
      },
    ],
  });
  assert.deepEqual(result.inventories[0].models.map((model) => model.provider_model_id), [
    "gpt-5.6-sol:reasoning-high",
    "gpt-5.6-sol",
  ]);
  assert.deepEqual(result.inventories[1].models.map((model) => model.provider_model_id), ["gemini-3.7-flash"]);
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gpt-image-1")?.reason, "deprecated_or_retiring");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "sora-2")?.reason, "deprecated_or_retiring");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gpt-3.5-turbo")?.reason, "deprecated_or_retiring");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gpt-5.1-codex-mini")?.reason, "deprecated_or_retiring");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "o4-mini")?.reason, "deprecated_or_retiring");
  assert.equal(result.coverage.find((item) => item.provider_model_id === "gemini-flash-latest")?.reason, "logical_alias");
  assert.equal(result.coverage.find((item) => item.provider_model_id.includes("reasoning-high"))?.status, "included");
});

test("SN matrix uses its inventory and OpenAI capability evidence", async () => {
  const cells = buildProviderMatrix({
    baseline: await baseline(),
    ...matrixInputs([{
      provider_instance_name: "sn-default",
      provider_driver: "sn-ai-provider",
      models: [{
        exact_model: "gpt-5.6-sol@sn-default",
        provider_model_id: "gpt-5.6-sol",
        api_types: ["llm", "vision.ocr", "vision.caption", "image.txt2img", "image.img2img"],
        logical_mounts: [],
      }],
    }]),
  });
  assert.deepEqual([...new Set(cells.map((cell) => cell.api_type))].sort(), [
    "image.img2img",
    "image.txt2img",
    "llm",
    "vision.caption",
    "vision.ocr",
  ]);
  assert.ok(cells.every((cell) => cell.provider_driver === "sn-ai-provider"));
});

test("T2 Gemini Embedding 2 has one minimal cell per API type and no variant cells", async () => {
  const cells = buildProviderMatrix({
    baseline: await baseline(),
    ...matrixInputs([{
      provider_instance_name: "gemini-default",
      provider_driver: "google-gemini",
      models: [{
        exact_model: "gemini-embedding-2@gemini-default",
        provider_model_id: "gemini-embedding-2",
        api_types: ["embedding.text", "embedding.multimodal"],
        logical_mounts: [],
      }],
    }]),
  });
  assert.equal(cells.length, 2);
  assert.deepEqual(cells.map((cell) => cell.api_type).sort(), ["embedding.multimodal", "embedding.text"]);
  assert.ok(cells.every((cell) => cell.variant === undefined));
  assert.ok(cells.every((cell) => cell.resource_representation === undefined));
});

test("T1.5 protocol catalog is independent, traceable, and strict on Provider wire", async () => {
  const catalog = await loadProviderProtocolCatalog();
  assert.equal(catalog.providers.length, 12);
  assert.deepEqual(
    catalog.providers.map((provider) => provider.provider_driver).sort(),
    ["claude", "deepseek", "doubao", "fal", "glm", "google-gemini", "kimi", "minimax", "openai", "openrouter", "qwen", "sn-ai-provider"],
  );
  const contract = protocolContract(catalog, "claude", "anthropic.messages.2023-06-01");
  assert.deepEqual(validateProviderRequest(contract, {
    method: "POST",
    pathname: "/v1/messages",
    query: new URLSearchParams(),
    headers: new Headers({
      "content-type": "application/json",
      "x-api-key": "test-key",
      "anthropic-version": "2023-06-01",
    }),
    body: { model: "claude-test", messages: [], max_tokens: 16 },
  }), []);
  assert.deepEqual(validateProviderRequest(contract, {
    method: "POST",
    pathname: "/v1/messages",
    query: new URLSearchParams(),
    headers: new Headers({
      "content-type": "application/json",
      "x-api-key": "test-key",
      "anthropic-version": "2023-06-01",
    }),
    body: { model: "claude-test", messages: [], max_tokens: 16, invented_by_aicc: true },
  }), ["unknown body field invented_by_aicc"]);
  assert.deepEqual(validateProviderRequest(contract, {
    method: "POST",
    pathname: "/v1/messages",
    query: new URLSearchParams(),
    headers: new Headers({
      "content-type": "application/json",
      "x-api-key": "test-key",
      "anthropic-version": "2023-06-01",
    }),
    body: { model: "claude-test", messages: {}, max_tokens: "16" },
  }), ["body field messages has invalid type", "body field max_tokens has invalid type"]);
  assert.ok(contract.official_sources.every((source) => source.startsWith("https://")));
  const sn = protocolContract(catalog, "sn-ai-provider", "sn.openai-responses.v1");
  assert.equal(sn.path, "/api/v1/ai/responses");
  assert.ok(sn.official_sources.every((source) => source.includes("buckyos/sn-business/blob/f765081")));
  assert.equal(catalog.providers.find((provider) => provider.provider_driver === "qwen")?.instance_fields?.workspace, "t15-workspace");
  const selected = selectOfficialModels(catalog, "google-gemini", "seed-4827");
  assert.deepEqual(selected, selectOfficialModels(catalog, "google-gemini", "seed-4827"));
  const gemini = catalog.providers.find((provider) => provider.provider_driver === "google-gemini")!;
  for (const [apiType, modelId] of Object.entries(selected)) {
    assert.ok(gemini.official_first_party_model_ids?.[apiType].includes(modelId));
  }
  const invalidCatalog = structuredClone(catalog);
  invalidCatalog.providers[0].contracts[0].official_sources = ["https://example.com/not-provider-evidence"];
  assert.throws(() => validateProviderProtocolCatalog(invalidCatalog), /Provider official domain/);
});

test("T1.5 Provider mock rejects non-official wire and redacts captured credentials", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;
  assert.equal((await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider_driver: "qwen",
      contract_id: "qwen.responses.compatible-v1",
      scenario: "success",
    }),
  })).status, 200);
  assert.equal((await fetch(`${baseUrl}/compatible-mode/v1/models`, {
    headers: { authorization: "Bearer t15-secret-value" },
  })).status, 404);
  assert.equal((await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider_driver: "openai",
      contract_id: "openai.responses.v1",
      scenario: "success",
    }),
  })).status, 200);
  const unauthenticatedDiscovery = await fetch(`${baseUrl}/v1/models`);
  assert.equal(unauthenticatedDiscovery.status, 401);
  const discovery = await fetch(`${baseUrl}/v1/models`, {
    headers: { authorization: "Bearer t15-secret-value" },
  });
  assert.equal(discovery.status, 200);
  assert.ok(((await discovery.json()) as { data: Array<{ id: string }> }).data.some((model) => model.id === "gpt-5.6"));
  assert.equal((await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider_driver: "sn-ai-provider",
      contract_id: "sn.openai-responses.v1",
      scenario: "success",
    }),
  })).status, 200);
  const snDiscovery = await fetch(`${baseUrl}/api/v1/ai/models`, {
    headers: { authorization: "Bearer t15-secret-value" },
  });
  assert.equal(snDiscovery.status, 200);
  assert.ok(Array.isArray(((await snDiscovery.json()) as { items: unknown[] }).items));
  const select = async () => {
    const response = await fetch(`${baseUrl}/__mock/select`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        provider_driver: "claude",
        contract_id: "anthropic.messages.2023-06-01",
        scenario: "success",
      }),
    });
    assert.equal(response.status, 200);
  };
  await select();
  const headers = {
    "content-type": "application/json",
    "x-api-key": "t15-secret-value",
    "anthropic-version": "2023-06-01",
  };
  const valid = await fetch(`${baseUrl}/v1/messages`, {
    method: "POST",
    headers,
    body: JSON.stringify({ model: "claude-test", messages: [], max_tokens: 16 }),
  });
  assert.equal(valid.status, 200);
  const audit = await (await fetch(`${baseUrl}/__mock/requests`)).json() as {
    requests: Array<{ headers: Record<string, string>; validation_errors: string[] }>;
  };
  assert.equal(audit.requests[0].headers["x-api-key"], "[REDACTED]");
  assert.deepEqual(audit.requests[0].validation_errors, []);
  await select();
  const invalid = await fetch(`${baseUrl}/v1/messages`, {
    method: "POST",
    headers,
    body: JSON.stringify({ model: "claude-test", messages: [], max_tokens: 16, invented: true }),
  });
  assert.equal(invalid.status, 400);
  assert.match(await invalid.text(), /unknown body field invented/);
});

test("T1.5 Provider mock serves every contract for all 12 Providers", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  assert.equal(catalog.providers.length, 12);
  assert.deepEqual(
    new Set(Object.keys(T15_PROVIDER_DISCOVERY_CONTRACTS)),
    new Set(catalog.providers.map((provider) => provider.provider_driver)),
  );
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;

  for (const provider of catalog.providers) {
    for (const contract of provider.contracts) {
      const selected = await fetch(`${baseUrl}/__mock/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: contract.id, scenario: "success" }),
      });
      assert.equal(selected.status, 200, `${provider.provider_driver}/${contract.id} selection`);
      const model = provider.test_model_ids[contract.api_types[0]];
      const providerRequest = t15ProviderRequest(baseUrl, contract, model);
      const response = await fetch(providerRequest.url, providerRequest.init);
      assert.equal(response.status, 200, `${provider.provider_driver}/${contract.id}: ${await response.clone().text()}`);
      const audit = await (await fetch(`${baseUrl}/__mock/requests`)).json() as {
        requests: Array<{ headers: Record<string, string>; validation_errors: string[] }>;
      };
      assert.equal(audit.requests.length, 1, `${provider.provider_driver}/${contract.id} audit count`);
      assert.deepEqual(audit.requests[0].validation_errors, [], `${provider.provider_driver}/${contract.id} wire validation`);
      const authHeader = contract.auth.kind === "header" ? contract.auth.name.toLowerCase() : undefined;
      if (authHeader) assert.equal(audit.requests[0].headers[authHeader], "[REDACTED]");
    }
  }
});

test("T1.5 Provider mock implements each machine discovery contract and rejects catalog-only discovery", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;

  for (const provider of catalog.providers) {
    const discovery = T15_PROVIDER_DISCOVERY_CONTRACTS[provider.provider_driver];
    const contract = provider.contracts[0];
    assert.equal((await fetch(`${baseUrl}/__mock/select`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: contract.id, scenario: "success" }),
    })).status, 200);
    if (discovery.mode === "catalog_only") {
      assert.equal((await fetch(`${baseUrl}${provider.endpoint_path}/models`)).status, 404, provider.provider_driver);
      continue;
    }
    const unauthenticated = await fetch(`${baseUrl}${discovery.path}`);
    assert.equal(unauthenticated.status, 401, `${provider.provider_driver} unauthenticated discovery`);
    const url = new URL(`${baseUrl}${discovery.path}`);
    for (const [name, value] of Object.entries(discovery.required_query ?? {})) url.searchParams.set(name, value);
    const headers = new Headers(discovery.required_headers ?? {});
    if (contract.auth.kind === "header") headers.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
    else url.searchParams.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
    const response = await fetch(url, { headers });
    assert.equal(response.status, 200, `${provider.provider_driver} discovery: ${await response.clone().text()}`);
    const fixture = await response.json() as Record<string, unknown>;
    if (discovery.response_shape === "gemini") assert.ok(Array.isArray(fixture.models));
    else if (discovery.response_shape === "sn") assert.ok(Array.isArray(fixture.items));
    else assert.ok(Array.isArray(fixture.data));
  }
});

test("T1.5 Provider mock serves every declared streaming and official error fixture", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;

  for (const provider of catalog.providers) {
    const primary = provider.contracts[0];
    const model = provider.test_model_ids[primary.api_types[0]];
    for (const fixture of catalog.error_fixtures[provider.provider_driver]) {
      assert.equal((await fetch(`${baseUrl}/__mock/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: primary.id, scenario: fixture.scenario }),
      })).status, 200);
      const providerRequest = t15ProviderRequest(baseUrl, primary, model);
      assert.equal((await fetch(providerRequest.url, providerRequest.init)).status, fixture.status,
        `${provider.provider_driver}/${fixture.scenario}`);
    }
    for (const contract of provider.contracts.filter((candidate) => candidate.stream_protocol)) {
      assert.equal((await fetch(`${baseUrl}/__mock/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: contract.id, scenario: "stream_success" }),
      })).status, 200);
      const providerRequest = t15ProviderRequest(
        baseUrl,
        contract,
        provider.test_model_ids[contract.api_types[0]],
        { stream: true },
      );
      const response = await fetch(providerRequest.url, providerRequest.init);
      assert.equal(response.status, 200, `${provider.provider_driver}/${contract.id} stream`);
      assert.match(response.headers.get("content-type") ?? "", /^text\/event-stream/);
      assert.match(await response.text(), /BUCKYOS-AICC-4827/);
      assert.equal((await fetch(`${baseUrl}/__mock/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: contract.id, scenario: "stream_interrupted" }),
      })).status, 200);
      const interruptedRequest = t15ProviderRequest(
        baseUrl,
        contract,
        provider.test_model_ids[contract.api_types[0]],
        { stream: true },
      );
      const interrupted = await fetch(interruptedRequest.url, interruptedRequest.init);
      assert.equal(interrupted.status, 200, `${provider.provider_driver}/${contract.id} interrupted stream`);
      const interruptedBody = await interrupted.text();
      const terminalMarker = contract.stream_protocol === "openai_responses"
        ? "response.completed"
        : contract.stream_protocol === "claude_messages"
        ? "message_stop"
        : contract.stream_protocol === "gemini_interactions"
        ? "interaction.completed"
        : "[DONE]";
      assert.equal(interruptedBody.includes(terminalMarker), false, `${contract.id} interrupted terminal marker`);
    }
  }
});

test("T1.5 async lifecycle validates and captures official poll/result wire", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  const contract = protocolContract(catalog, "fal", "fal.esrgan.queue-v1");
  assert.deepEqual(validateProviderAuxiliaryRequest(contract, {
    method: "GET",
    pathname: "/fal-ai/esrgan/requests/fal_mock_1/status",
    query: new URLSearchParams(),
    headers: new Headers({ authorization: "Key test-key" }),
  }).errors, []);
  assert.match(validateProviderAuxiliaryRequest(contract, {
    method: "GET",
    pathname: "/fal-ai/esrgan/requests/fal_mock_1/status",
    query: new URLSearchParams(),
    headers: new Headers(),
  }).errors.join(";"), /authentication/);

  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;
  assert.equal((await fetch(`${baseUrl}/__mock/select`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ provider_driver: "fal", contract_id: contract.id, scenario: "async_success" }),
  })).status, 200);
  const headers = { authorization: "Key test-key" };
  assert.equal((await fetch(`${baseUrl}/fal-ai/esrgan/requests/fal_mock_1/status`, { headers })).status, 200);
  assert.equal((await fetch(`${baseUrl}/fal-ai/esrgan/requests/fal_mock_1`, { headers })).status, 200);
  const audit = await (await fetch(`${baseUrl}/__mock/requests`)).json() as {
    requests: Array<{ async_step?: string; validation_errors: string[] }>;
  };
  assert.deepEqual(audit.requests.map((request) => request.async_step), ["poll", "result"]);
  assert.ok(audit.requests.every((request) => request.validation_errors.length === 0));
});

test("T1.5 Provider mock completes every declared async lifecycle", async (context) => {
  const catalog = await loadProviderProtocolCatalog();
  const handler = createT15MockHandler(catalog);
  const server = createServer((request, response) => void handler(request, response));
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolvePromise);
  });
  context.after(() => new Promise<void>((resolvePromise, reject) =>
    server.close((error) => error ? reject(error) : resolvePromise())
  ));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const baseUrl = `http://127.0.0.1:${address.port}`;
  const asyncContracts = catalog.providers.flatMap((provider) => provider.contracts
    .filter((contract) => contract.async_protocol)
    .map((contract) => ({ provider, contract })));
  assert.deepEqual(new Set(asyncContracts.map(({ contract }) => contract.async_protocol)),
    new Set(["openai_video", "google_lro", "fal_queue", "minimax_video"]));

  for (const { provider, contract } of asyncContracts) {
    assert.equal((await fetch(`${baseUrl}/__mock/select`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider_driver: provider.provider_driver, contract_id: contract.id, scenario: "async_success" }),
    })).status, 200);
    const submit = t15ProviderRequest(baseUrl, contract, provider.test_model_ids[contract.api_types[0]]);
    assert.equal((await fetch(submit.url, submit.init)).status, 200, `${contract.id} submit`);
    for (const step of contract.async_steps ?? []) {
      let path = step.path
        .replaceAll("{request_id}", "fal_mock_1")
        .replaceAll("{operation_id}", contract.async_protocol === "google_lro" ? "gemini_mock_1" : "video_mock_1");
      const url = new URL(`${baseUrl}${path}`);
      for (const field of step.required_query_fields ?? []) {
        url.searchParams.set(field, field === "task_id" ? "minimax_video_mock_1" : "minimax_file_mock_1");
      }
      const headers = new Headers();
      if (contract.auth.kind === "header") headers.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
      else url.searchParams.set(contract.auth.name, `${contract.auth.prefix}t15-secret`);
      const response = await fetch(url, { method: step.http_method, headers });
      assert.ok([200, 202].includes(response.status), `${contract.id}/${step.name}: ${response.status} ${await response.clone().text()}`);
      if (step.name !== "result" || !response.headers.get("content-type")?.startsWith("application/json")) continue;
      const result = await response.json() as Record<string, unknown>;
      const serialized = JSON.stringify(result);
      assert.equal(serialized.includes("http://mock"), false, `${contract.id} leaked placeholder artifact authority`);
      const artifactUrl = serialized.match(/http:\/\/127\.0\.0\.1:\d+\/artifacts\/[^"\\]+/)?.[0];
      if (artifactUrl) assert.equal((await fetch(artifactUrl)).status, 200, `${contract.id} artifact`);
    }
    const audit = await (await fetch(`${baseUrl}/__mock/requests`)).json() as {
      requests: Array<{ async_step?: string; validation_errors: string[] }>;
    };
    assert.ok(audit.requests.every((request) => request.validation_errors.length === 0), contract.id);
    assert.deepEqual(
      new Set(audit.requests.map((request) => request.async_step).filter(Boolean)),
      new Set((contract.async_steps ?? []).map((step) => step.name)),
      `${contract.id} lifecycle audit`,
    );
  }
});

test("T1.5 manifest owns Provider normal, streaming, async, error, and variant cells", async () => {
  const catalog = await loadProviderProtocolCatalog();
  const manifest = validateCaseManifest(buildT15Manifest(catalog, [{
    provider_driver: "openai",
    contract_id: "openai.responses.v1",
    api_type: "llm",
    model: {
      exact_model: "gpt-5.4:reasoning-high@t15-openai",
      provider_model_id: "gpt-5.4:reasoning-high",
      provider_actual_model_id: "gpt-5.4",
      provider_options: { reasoning: { effort: "high" } },
      api_types: ["llm"],
      logical_mounts: [],
    },
  }]));
  assert.ok(manifest.some((item) => item.mock_scenario === "stream_success"));
  assert.ok(manifest.filter((item) => item.mock_scenario === "stream_success" ||
    item.mock_scenario === "stream_interrupted").every((item) => item.execution_mode === "stream"));
  assert.ok(manifest.filter((item) => item.mock_scenario !== "stream_success" &&
    item.mock_scenario !== "stream_interrupted").every((item) => item.execution_mode === "immediate"));
  assert.ok(manifest.some((item) => item.mock_scenario === "async_success"));
  assert.ok(manifest.some((item) => item.mock_scenario === "async_failed"));
  assert.ok(manifest.some((item) => item.mock_scenario === "async_cancel"));
  assert.ok(manifest.some((item) => item.expected_error_class === "provider_protocol_failed"));
  assert.ok(manifest.filter((item) => item.tags.includes("official_error"))
    .every((item) => typeof item.expected_retriable === "boolean" && !("expected_retryable" in item)));
  assert.ok(manifest.some((item) => item.tags.includes("variant")));
  assert.ok(manifest.every((item) => item.layer === "T1.5" && item.semantic_rubric.length === 0));
  assert.ok(buildStaticManifest().every((item) => !item.case_id.startsWith("t1.protocol.")));
});

test("T1.5 typed request fixtures use current provider-neutral methods without legacy envelopes", () => {
  const params = buildT15TypedParams("llm", "gpt-5.4@t15-openai", "run");
  assert.equal(params.exact_model, "gpt-5.4@t15-openai");
  assert.equal(params.execution_mode, "immediate");
  assert.ok(Array.isArray(params.messages));
  assert.equal("model" in params, false);
  assert.equal("payload" in params, false);
  assert.equal("stream" in params, false);

  const streamParams = buildT15TypedParams("llm", "gpt-5.4@t15-openai", "run", "stream");
  assert.equal(streamParams.execution_mode, "stream");
  assert.equal("stream" in streamParams, false);
});

test("T1.5 success mapping requires canonical output, usage, and async attribution", async () => {
  const catalog = await loadProviderProtocolCatalog();
  const llm = protocolContract(catalog, "openai", "openai.responses.v1");
  assert.doesNotThrow(() => assertT15ResponseMapping("llm", {
    status: "succeeded",
    message: { role: "assistant", content: [{ type: "text", text: "ok" }] },
    usage: { input_tokens: 1, output_tokens: 1 },
  }, llm));
  assert.throws(() => assertT15ResponseMapping("llm", {
    status: "succeeded",
    usage: { input_tokens: 1 },
  }, llm), /missing mapped field message/);
  const video = protocolContract(catalog, "openai", "openai.videos.v1");
  assert.throws(() => assertT15ResponseMapping("video.txt2video", {
    status: "succeeded",
    video: { kind: "named_object", obj_id: "chunk:video" },
  }, video), /operation attribution/);
});

test("T2 excludes independently callable metadata variants", async () => {
  const inventories: ProviderInventory[] = [{
    provider_instance_name: "openai-main",
    provider_driver: "openai",
    models: [
      { exact_model: "gpt-5.4@openai-main", provider_model_id: "gpt-5.4", api_types: ["llm", "vision.ocr", "vision.caption", "image.txt2img", "image.img2img"], logical_mounts: [] },
      { exact_model: "gpt-5.4:reasoning-high@openai-main", provider_model_id: "gpt-5.4:reasoning-high", provider_actual_model_id: "gpt-5.4", api_types: ["llm"], logical_mounts: [] },
    ],
  }];
  const cells = buildProviderMatrix({ baseline: await baseline(), ...matrixInputs(inventories) });
  assert.ok(cells.length > 0);
  assert.ok(cells.every((cell) => !cell.provider_model_id.includes(":")));
});

test("report redaction removes secrets and totals statuses", () => {
  const safe = redact({
    api_key: "sk-secret-value-1234567890",
    nested: { authorization: "Bearer abc.def.ghi" },
  });
  assert.deepEqual(safe, {
    api_key: "[REDACTED]",
    nested: { authorization: "[REDACTED]" },
  });
  assert.doesNotThrow(() => assertNoSecrets(safe));
  const base = {
    run_id: "run",
    layer: "T1" as const,
    method: "chat.completions.create",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  const totals = caseTotals([
    { ...base, case_id: "passed", status: "passed" },
    { ...base, case_id: "restricted", status: "provider_restricted" },
  ]);
  assert.equal(totals.passed, 1);
  assert.equal(totals.provider_restricted, 1);
  assert.equal(totals.failed, 0);
  assert.equal(totals.skipped, 0);
  assert.equal(isProviderRestricted(new Error("request not allowed for this model")), true);
  assert.equal(isProviderRestricted(new Error("provider request failed")), false);
});

test("report schema rejects version drift and duplicate case ids", () => {
  const caseReport = {
    run_id: "run-1",
    case_id: "t1.schema",
    layer: "T1",
    status: "passed",
    method: "route.resolve",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  };
  const report = {
    schema_version: ACCEPTANCE_REPORT_SCHEMA_VERSION,
    run_id: "run-1",
    started_at: "2026-09-02T00:00:00.000Z",
    finished_at: "2026-09-02T00:00:01.000Z",
    commit: "test-commit",
    baseline_revision: "test-baseline",
    allow_real_model_calls: false,
    planned_real_calls: 0,
    actual_real_calls: 0,
    estimated_cost_usd: 0,
    actual_cost_usd: 0,
    raw_cost_usd: 0,
    credit_applied_usd: 0,
    finance: buildFinancialReport({
      entries: [],
      budgetUsd: 0,
      plannedMaxCalls: 0,
      plannedMaxCostUsd: 0,
    }),
    cases: [caseReport],
    product_defects: [],
    cleanup: { status: "passed", details: [] },
    protocol_evidence_revision: "official-provider-protocols-test",
    official_evidence_checked_at: "2026-09-02",
    providers: ["openai"],
    limits: { global_concurrency: 1, provider_concurrency: 1, provider_min_interval_ms: 50 },
  };
  assert.doesNotThrow(() => validateAcceptanceReport(report));
  assert.throws(
    () => validateAcceptanceReport({ ...report, schema_version: 2 }),
    /unsupported acceptance report schema_version/,
  );
  assert.throws(
    () => validateAcceptanceReport({ ...report, cases: [caseReport, caseReport] }),
    /duplicate report case_id/,
  );
  assert.throws(
    () => validateAcceptanceReport({ ...report, limits: { ...report.limits, provider_concurrency: -1 } }),
    /limits.provider_concurrency/,
  );
});

test("named artifact validation reads and verifies ZIP entries", async () => {
  const bytes = new Uint8Array(await readFile(join(here, "../../jarvis_media_dv/assets/archive_mixed.zip")));
  const audit = await validateNamedArtifact({
    openReader: async () => ({
      totalSize: bytes.length,
      body: new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(bytes);
          controller.close();
        },
      }),
    }),
  }, { obj_id: "mix256:test", label: "application/zip" });
  assert.equal(audit.size, bytes.length);
  assert.match(audit.sha256, /^[0-9a-f]{64}$/);
  assert.ok((audit.archive_entries?.length ?? 0) > 0);
});

test("legacy DOC and PPT fixtures are genuine OLE Office binaries", async () => {
  for (const [name, stream] of [
    ["facts.doc", "WordDocument"],
    ["facts.ppt", "PowerPoint Document"],
  ] as const) {
    const bytes = await readFile(join(here, "../fixtures", name));
    assert.deepEqual([...bytes.subarray(0, 8)], [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    assert.ok(bytes.includes(Buffer.from(stream, "utf16le")), `${name} is missing ${stream} OLE stream`);
  }
  assert.ok((await readFile(join(here, "../fixtures/facts.doc"))).includes("I am a test document"));
  assert.ok((await readFile(join(here, "../fixtures/facts.ppt"))).includes("AICC-FIXTURE-7319"));
});

test("artifact validation records media metadata", async () => {
  const png = new Uint8Array(await readFile(join(here, "../fixtures/mask.png")));
  const pngAudit = await validateArtifactBytes(png, { id: "inline", label: "image/png" });
  assert.equal(typeof pngAudit.metadata?.width, "number");
  assert.equal(typeof pngAudit.metadata?.height, "number");
  const transparent = new Uint8Array(await readFile(join(here, "../fixtures/transparent.png")));
  const transparentAudit = await validateArtifactBytes(transparent, { id: "inline", label: "image/png" });
  assert.equal(transparentAudit.metadata?.alpha_min, 0);
  assert.equal(transparentAudit.metadata?.alpha_max, 255);
  assert.equal(transparentAudit.metadata?.transparent_pixels, 1);
  assert.equal(transparentAudit.metadata?.opaque_pixels, 1);
  assert.equal(transparentAudit.metadata?.transparent_ratio, 0.5);
  assert.equal(transparentAudit.metadata?.opaque_ratio, 0.5);
  assert.doesNotThrow(() => assertBackgroundRemovalTransparency([transparentAudit]));
  assert.throws(() => assertBackgroundRemovalTransparency([{
    ...transparentAudit,
    metadata: {
      format: "png",
      width: 100,
      height: 100,
      transparent_pixels: 1,
      opaque_pixels: 9999,
      transparent_ratio: 0.0001,
      opaque_ratio: 0.9999,
    },
  }]));
  const wav = new Uint8Array(await readFile(join(here, "../../jarvis_media_dv/assets/audio_speech.wav")));
  const wavAudit = await validateArtifactBytes(wav, { id: "inline", label: "audio/wav" });
  assert.equal(typeof wavAudit.metadata?.sample_rate_hz, "number");
  assert.ok(Number(wavAudit.metadata?.duration_seconds) > 0);
});
