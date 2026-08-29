import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { CANONICAL_API_TYPES, methodsForApiType } from "./canonical.ts";
import { buildStaticManifest } from "./cases.ts";
import {
  analyzeProviderMatrix,
  buildProviderMatrix,
  validateCaseManifest,
  validateProviderBaseline,
} from "./manifest.ts";
import { runPreflight } from "./preflight.ts";
import { assertNoSecrets, caseTotals, isProviderRestricted, redact } from "./report.ts";
import { buildMockSettings, configValue } from "./mock_settings.ts";
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
import type { ProviderInventory } from "./types.ts";
import { buildT1Coverage } from "./coverage.ts";
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
  assert.deepEqual(methodsForApiType("llm"), ["llm.chat"]);
  assert.doesNotMatch(JSON.stringify(await baseline()), /llm\.completion/);
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
  assert.deepEqual(result.provider_drivers, [
    "claude",
    "fal",
    "google-gemini",
    "minimax",
    "openai",
    "openrouter",
    "sn-ai-provider",
  ]);
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
  const original = { openai: { enabled: true, instances: [{ provider_instance_name: "production" }] } };
  const serialized = JSON.stringify(original);
  const decoded = configValue({ value: serialized });
  const patched = buildMockSettings(decoded.parsed, {
    baseUrl: "http://127.0.0.1:4827/",
    runId: "run:one",
  });
  assert.equal(JSON.stringify(original), serialized);
  const openai = patched.openai as { instances: { provider_instance_name: string }[] };
  assert.equal(openai.instances[0].provider_instance_name, "production");
  assert.deepEqual(openai.instances.slice(1).map((item) => item.provider_instance_name), [
    "dv-openai-a-run-one",
    "dv-openai-b-run-one",
  ]);
  assert.equal(decoded.serialized, serialized);
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
      { case_id: "a", attempt: 1, provider_driver: "openai", provider_instance: "one", exact_model: "m@one", api_type: "llm", method: "llm.chat", started_at: "now", status: "passed", estimated_cost_usd: 0.01, actual_cost_usd: 0.012, cost_status: "actual" },
      { case_id: "b", attempt: 1, provider_driver: "openai", provider_instance: "one", exact_model: "m@one", api_type: "llm", method: "llm.chat", started_at: "now", status: "failed", estimated_cost_usd: 0.01, cost_status: "unknown" },
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
    case_id: "t1.mock.error.rate_limit",
    layer: "T1",
    status: "passed",
    api_type: "llm",
    method: "llm.chat",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  }]);
  assert.equal(coverage.executed, 1);
  assert.equal(coverage.passed, 1);
  assert.ok(coverage.total > coverage.executed);
  assert.ok(coverage.unexecuted_case_ids.includes("t1.route.exact_model_hits_instance"));
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
    coverage.combination_groups.find((item) => item.group_id === "api_method_x_mock_scenario")?.executed_cells,
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

test("provider matrix expands exact model and method", async () => {
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
  assert.equal(cells.length, 3);
  assert.equal(cells[0].exact_model, "fal-ai/esrgan@fal-test-a");
  assert.equal(cells[0].method, "image.upscale");
  assert.deepEqual([...new Set(cells.map((cell) => cell.resource_representation))].sort(), [
    "base64",
    "named_object",
    "url",
  ]);
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
    method: "llm.chat",
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
    method: "llm.chat",
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
        api_types: ["llm", "vision.ocr", "vision.caption"],
        logical_mounts: [],
      }],
    }]),
  });
  assert.deepEqual([...new Set(cells.map((cell) => cell.api_type))].sort(), ["llm", "vision.caption", "vision.ocr"]);
  assert.ok(cells.every((cell) => cell.provider_driver === "sn-ai-provider"));
});

test("Gemini Embedding 2 expands official multimodal combinations and one large artifact per API", async () => {
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
  const multimodal = cells.filter((cell) =>
    cell.api_type === "embedding.multimodal" && cell.variant === "default"
  );
  assert.deepEqual([...new Set(multimodal.map((cell) => cell.input_kinds.join("+")))].sort(), [
    "audio",
    "document",
    "image",
    "text",
    "text+image",
    "video",
  ]);
  assert.deepEqual(
    [...new Set(multimodal.filter((cell) => cell.input_kinds.includes("document")).map((cell) => cell.document_format))],
    ["pdf"],
  );
  assert.equal(cells.filter((cell) => cell.variant === "embedding_large_artifact").length, 2);
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
    layer: "T1",
    method: "llm.chat",
    outbound_message_ids: [],
    artifact_ids: [],
    attempts: [],
  } as const;
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
