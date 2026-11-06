import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import { ndm_proxy, ndn } from "buckyos";

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
type JsonObject = { [key: string]: JsonValue };

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type Owner = {
  user_id: string;
  app_id: string;
};

type EventEnvelope = {
  type: string;
  seq: number;
  actor: string;
  node_id?: string | null;
  payload?: JsonValue;
};

type HistoryResponse = {
  ok: true;
  events: EventEnvelope[];
  next_seq: number;
  current_seq: number;
};

type GraphResponse = {
  ok: true;
  run_id: string;
  workflow_id: string;
  workflow_name?: string;
  status: string;
  revision?: string;
  requested_by?: string | null;
  executor?: string;
  graph: JsonObject;
  nodes: JsonObject;
  node_states: Record<string, string>;
  node_outputs: Record<string, JsonValue>;
  node_errors?: Record<string, JsonObject>;
  node_sources?: Record<string, string>;
  human_waiting_nodes: string[];
  pending_thunks: JsonObject;
  metrics: JsonObject;
  output?: Record<string, JsonValue>;
  seq: number;
};

type CreateRunResponse = {
  ok: true;
  run_id: string;
  workflow_id: string;
  status: string;
  revision: string;
  reused: boolean;
  output: Record<string, JsonValue>;
  events: EventEnvelope[];
  seq: number;
};

type TaskSummary = {
  task_id: string;
  name: string;
  parent_id?: string | null;
  root_id: string;
  schema_id: string;
  phase: string;
  outcome?: string | null;
};

type UploadedInput = {
  obj_id: string;
  node_id: string;
  path: string;
};

type CaseResult =
  | { status: "passed"; caseId: string; reportDir: string }
  | { status: "blocked"; caseId: string; reason: string; reportDir: string }
  | { status: "failed"; caseId: string; error: string; reportDir: string };

class BlockedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "BlockedError";
  }
}

const assert = {
  ok(value: unknown, message?: string): void {
    if (!value) throw new Error(message ?? "assertion failed");
  },
  equal(actual: unknown, expected: unknown, message?: string): void {
    if (actual !== expected) {
      throw new Error(
        message ??
          `assertion failed: expected ${String(expected)}, got ${
            String(actual)
          }`,
      );
    }
  },
  notEqual(actual: unknown, expected: unknown, message?: string): void {
    if (actual === expected) {
      throw new Error(
        message ??
          `assertion failed: value should not equal ${String(expected)}`,
      );
    }
  },
  deepEqual(actual: unknown, expected: unknown, message?: string): void {
    const actualJson = JSON.stringify(actual);
    const expectedJson = JSON.stringify(expected);
    if (actualJson !== expectedJson) {
      throw new Error(
        message ??
          `assertion failed: expected ${expectedJson}, got ${actualJson}`,
      );
    }
  },
};

const DEFAULT_CASES = [
  "WF-DV-001",
  "WF-DV-002",
  "WF-DV-003",
  "WF-DV-005",
  "WF-DV-006",
  "WF-DV-007",
  "WF-DV-008",
  "WF-DV-009",
  "WF-DV-010",
  "WF-DV-011",
  "WF-DV-012",
  "WF-DV-013",
  "WF-DV-014",
  "WF-DV-015",
  "WF-DV-016",
];
const WORKFLOW_DV_CASES = getEnv("WORKFLOW_DV_CASES");
const WORKFLOW_REPORT_DIR = getEnv("WORKFLOW_REPORT_DIR") ??
  "reports/workflow_dv";
const WORKFLOW_TEST_IMAGE_URL = getEnv("WORKFLOW_TEST_IMAGE_URL") ??
  "https://www.gstatic.com/webp/gallery/1.jpg";
const WORKFLOW_WAIT_TIMEOUT_MS = Number(
  getEnv("WORKFLOW_WAIT_TIMEOUT_MS") ?? "180000",
);
const SYSTEM_DEFINITION_IDS = [
  "media.image",
  "media.pdf",
  "media.video",
  "media.document",
];
const POPPLER_TOOLS = ["pdfinfo", "pdftotext", "pdftoppm"];
const TERMINAL_STATUSES = ["completed", "failed", "aborted", "budget_exhausted"];

function getEnv(name: string): string | null {
  const value = Deno.env.get(name);
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function randomHex(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function safeFileSegment(value: string): string {
  const safe = value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return safe || "case";
}

function joinPath(base: string, ...segments: string[]): string {
  let result = base.replace(/\/+$/g, "");
  for (const segment of segments) {
    result = `${result}/${segment.replace(/^\/+|\/+$/g, "")}`;
  }
  return result;
}

async function writeJsonFile(path: string, value: unknown): Promise<void> {
  await Deno.writeTextFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

let aiccRpc: RpcClient;

async function loadFixture(name: string): Promise<JsonObject> {
  const url = new URL(`./fixtures/${name}`, import.meta.url);
  const fixture = JSON.parse(await Deno.readTextFile(url)) as JsonObject;
  await resolveFixtureModels(fixture);
  return fixture;
}

/** AICC's typed methods take an exact model, which differs per Zone: a fixture says `route:<api_type>`. */
async function resolveFixtureModels(value: JsonValue): Promise<void> {
  if (!value || typeof value !== "object") return;
  for (const item of Object.values(value)) await resolveFixtureModels(item);
  if (Array.isArray(value) || typeof value.exact_model !== "string") return;
  const apiType = value.exact_model.startsWith("route:") ? value.exact_model.slice(6) : null;
  if (!apiType) return;
  const route = await rpcCall<{ selected_exact_model: string }>(aiccRpc, "route.resolve", {
    api_type: apiType,
    logical_model: apiType,
  });
  value.exact_model = route.selected_exact_model;
}

function cloneJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function prepareDefinition(definition: JsonObject, runTag: string): JsonObject {
  const prepared = cloneJson(definition);
  prepared.id = `${String(prepared.id)}-${runTag}`;
  prepared.name = `${String(prepared.name)} ${runTag}`;
  overrideResourceUrls(prepared, WORKFLOW_TEST_IMAGE_URL);
  return prepared;
}

function overrideResourceUrls(value: JsonValue, imageUrl: string): void {
  if (!value || typeof value !== "object") return;
  if (Array.isArray(value)) {
    for (const item of value) overrideResourceUrls(item, imageUrl);
    return;
  }
  if (value.kind === "url" && typeof value.url === "string") {
    const url = value.url;
    if (url.includes("gstatic.com/webp/gallery/")) {
      value.url = imageUrl;
    }
  }
  for (const item of Object.values(value)) overrideResourceUrls(item, imageUrl);
}

function replaceGalleryImage(
  definition: JsonObject,
  imageUrl: string,
): JsonObject {
  const next = cloneJson(definition);
  overrideAllResourceUrls(next, imageUrl);
  return next;
}

function overrideAllResourceUrls(value: JsonValue, imageUrl: string): void {
  if (!value || typeof value !== "object") return;
  if (Array.isArray(value)) {
    for (const item of value) overrideAllResourceUrls(item, imageUrl);
    return;
  }
  if (value.kind === "url" && typeof value.url === "string") {
    value.url = imageUrl;
  }
  for (const item of Object.values(value)) {
    overrideAllResourceUrls(item, imageUrl);
  }
}

async function rpcOk<T>(
  rpc: RpcClient,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  const raw = await rpc.call(method, params);
  if (!raw || typeof raw !== "object") {
    throw new Error(`${method} returned non-object response`);
  }
  const body = raw as Record<string, unknown>;
  if (body.ok !== true) {
    const text = JSON.stringify(body, null, 2);
    if (isAiccUnavailableText(text)) {
      throw new BlockedError(`AICC unavailable for workflow step: ${text}`);
    }
    throw new Error(`${method} failed: ${text}`);
  }
  return body as T;
}

/**
 * task_mgr RPC handlers return the result struct directly (e.g. `{tasks:[...]}`,
 * `null`) without an `{ok:true}` envelope, unlike the workflow service. Use
 * this helper for those calls so we don't misinterpret a successful response
 * as a failure.
 */
async function rpcCall<T>(
  rpc: RpcClient,
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  return (await rpc.call(method, params)) as T;
}

function isAiccUnavailableText(text: string): boolean {
  const lowered = text.toLowerCase();
  return lowered.includes("service::aicc") ||
    lowered.includes("no_provider_available") ||
    lowered.includes("model_alias_not_mapped") ||
    lowered.includes("provider_unavailable") ||
    lowered.includes("no route candidate generated") ||
    lowered.includes("aicc client unavailable");
}

function isAiccWaitingFailure(
  history: HistoryResponse,
  expectedHumanNode: string,
): string | null {
  for (const event of history.events) {
    if (event.type !== "step.waiting_human") continue;
    if (event.node_id === expectedHumanNode) continue;
    const payload = JSON.stringify(event.payload ?? {});
    if (isAiccUnavailableText(payload)) {
      return `AICC step ${
        event.node_id ?? "<unknown>"
      } is waiting_human: ${payload}`;
    }
  }
  return null;
}

function assertNoAnalysisErrors(
  response: { analysis?: unknown },
  label: string,
): void {
  const analysis = response.analysis as { errors?: unknown[] } | undefined;
  assert.ok(analysis, `${label} should return analysis`);
  const errors = analysis?.errors;
  assert.equal(
    Array.isArray(errors),
    true,
    `${label} analysis.errors should be an array`,
  );
  assert.equal(
    errors?.length ?? 0,
    0,
    `${label} should have no analysis errors`,
  );
}

function assertAllowedStates(graph: GraphResponse): void {
  const runStates = new Set([
    "created",
    "running",
    "waiting_human",
    "completed",
    "failed",
    "paused",
    "aborted",
    "budget_exhausted",
  ]);
  const nodeStates = new Set([
    "pending",
    "ready",
    "running",
    "completed",
    "failed",
    "retrying",
    "waiting_human",
    "skipped",
    "aborted",
    "cancelled",
  ]);
  assert.ok(
    runStates.has(graph.status),
    `unexpected run status ${graph.status}`,
  );
  for (const [nodeId, state] of Object.entries(graph.node_states)) {
    assert.ok(
      nodeStates.has(state),
      `unexpected node state ${nodeId}=${state}`,
    );
  }
}

function assertMonotonicHistory(history: HistoryResponse): void {
  let last = 0;
  for (const event of history.events) {
    assert.ok(
      event.seq > last,
      `history seq should be strictly increasing at ${event.seq}`,
    );
    last = event.seq;
  }
}

async function assertHistoryResume(
  workflowRpc: RpcClient,
  runId: string,
  fullHistory: HistoryResponse,
): Promise<void> {
  const sinceSeq = Math.max(0, fullHistory.current_seq - 5);
  const resumed = await rpcOk<HistoryResponse>(workflowRpc, "get_history", {
    run_id: runId,
    since_seq: sinceSeq,
    limit: 20,
  });
  const expected = fullHistory.events.filter((event) => event.seq > sinceSeq);
  assert.deepEqual(
    resumed.events.map((event) => event.seq),
    expected.map((event) => event.seq),
    "get_history since_seq should align with full history tail",
  );
}

function eventsFor(
  history: HistoryResponse,
  nodeId: string,
  eventType?: string,
): EventEnvelope[] {
  return history.events.filter((event) =>
    event.node_id === nodeId && (!eventType || event.type === eventType)
  );
}

async function submitDefinitionPair(
  workflowRpc: RpcClient,
  definition: JsonObject,
): Promise<
  { workflowId: string; version: number; first: JsonObject; second: JsonObject }
> {
  const dryRun = await rpcOk<JsonObject>(workflowRpc, "dry_run", {
    definition,
  });
  assertNoAnalysisErrors(dryRun, "dry_run");

  const first = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    definition,
    tags: ["dv", "workflow"],
  });
  assertNoAnalysisErrors(first, "submit_definition");
  assert.equal(typeof first.workflow_id, "string");

  const second = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    definition,
    tags: ["dv", "workflow"],
  });
  assert.equal(
    second.workflow_id,
    first.workflow_id,
    "same owner + definition should keep workflow_id",
  );
  assert.ok(
    Number(second.version) >= Number(first.version),
    "definition version should not move backwards",
  );

  return {
    workflowId: String(first.workflow_id),
    version: Number(second.version),
    first,
    second,
  };
}

async function createAndStartRun(
  workflowRpc: RpcClient,
  workflowId: string,
): Promise<{ runId: string; start: JsonObject }> {
  const created = await rpcOk<JsonObject>(workflowRpc, "create_run", {
    workflow_id: workflowId,
    input: {
      image_url: WORKFLOW_TEST_IMAGE_URL,
      mime_hint: "image/jpeg",
    },
  });
  assert.equal(created.status, "created");
  assert.equal(typeof created.run_id, "string");

  const runId = String(created.run_id);
  // start_run only admits the run; the driver advances it in the background.
  let start = await rpcOk<JsonObject>(workflowRpc, "start_run", {
    run_id: runId,
    wait_ms: 2000,
  });
  assert.equal(start.accepted, true, "start_run must report acceptance");
  const deadline = Date.now() + 15_000;
  while (String(start.status) === "created" && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 300));
    start = await rpcOk<JsonObject>(workflowRpc, "get_run_graph", { run_id: runId });
  }
  assert.ok(
    ["running", "waiting_human", "completed", "failed"].includes(
      String(start.status),
    ),
    `start_run status should advance, got ${start.status}`,
  );
  return { runId, start };
}

async function getGraph(
  workflowRpc: RpcClient,
  runId: string,
): Promise<GraphResponse> {
  const graph = await rpcOk<GraphResponse>(workflowRpc, "get_run_graph", {
    run_id: runId,
  });
  assertAllowedStates(graph);
  return graph;
}

async function getHistory(
  workflowRpc: RpcClient,
  runId: string,
): Promise<HistoryResponse> {
  const history = await rpcOk<HistoryResponse>(workflowRpc, "get_history", {
    run_id: runId,
    limit: 500,
  });
  assertMonotonicHistory(history);
  return history;
}

async function waitForGraph(
  workflowRpc: RpcClient,
  runId: string,
  label: string,
  predicate: (graph: GraphResponse, history: HistoryResponse) => boolean,
): Promise<{ graph: GraphResponse; history: HistoryResponse }> {
  const deadline = Date.now() + WORKFLOW_WAIT_TIMEOUT_MS;
  let lastGraph: GraphResponse | null = null;
  let lastHistory: HistoryResponse | null = null;
  while (Date.now() < deadline) {
    lastGraph = await getGraph(workflowRpc, runId);
    lastHistory = await getHistory(workflowRpc, runId);
    if (predicate(lastGraph, lastHistory)) {
      return { graph: lastGraph, history: lastHistory };
    }
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  throw new Error(
    `timed out waiting for ${label}: graph=${
      JSON.stringify(lastGraph)
    } history=${JSON.stringify(lastHistory)}`,
  );
}

function assertArtifacts(output: JsonValue, label: string): void {
  assert.ok(
    output && typeof output === "object" && !Array.isArray(output),
    `${label} output should be object`,
  );
  const artifacts = (output as JsonObject).artifacts;
  assert.ok(Array.isArray(artifacts), `${label}.artifacts should be array`);
  const artifactItems = artifacts as JsonValue[];
  assert.ok(artifactItems.length > 0, `${label}.artifacts should be non-empty`);
}

async function assertRunListed(
  workflowRpc: RpcClient,
  workflowId: string,
  runId: string,
  status: string,
): Promise<void> {
  const listed = await rpcOk<{ ok: true; runs: JsonObject[] }>(
    workflowRpc,
    "list_runs",
    {
      workflow_id: workflowId,
      status,
    },
  );
  assert.ok(
    listed.runs.some((run) => run.run_id === runId),
    `list_runs should include ${runId} with status=${status}`,
  );
}

async function runImageReviewCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  decision: "approved" | "rejected";
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, decision, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-image-review-enhance.json"),
    `${runTag}-${decision}`,
  );
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  await writeJsonFile(joinPath(reportDir, "definition.json"), submitted.second);

  const { runId, start } = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  await writeJsonFile(joinPath(reportDir, "start.json"), start);

  const waiting = await waitForGraph(
    workflowRpc,
    runId,
    "human_review waiting_human",
    (graph) =>
      graph.status === "waiting_human" &&
      graph.node_states.human_review === "waiting_human",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runId);
    const blocked = isAiccWaitingFailure(history, "human_review");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });
  await writeJsonFile(joinPath(reportDir, "waiting_graph.json"), waiting.graph);

  assert.equal(waiting.graph.node_states.caption_image, "completed");
  assert.equal(waiting.graph.node_states.ocr_image, "completed");
  assert.equal(waiting.graph.node_states.detect_objects, "completed");
  assert.equal(waiting.graph.node_states.inspect_fork, "completed");
  assert.equal(waiting.graph.node_states.summarize_review, "completed");
  assert.ok(waiting.graph.human_waiting_nodes.includes("human_review"));
  assert.ok(
    eventsFor(waiting.history, "human_review", "step.waiting_human").length > 0,
  );

  const humanOutput = decision === "approved"
    ? {
      decision: "approved",
      comment: "质检摘要可接受，继续增强。",
      final_subject: { text: "approved", extra: { approved: true } },
    }
    : {
      decision: "rejected",
      comment: "图片主体不适合当前商品，不要继续消耗增强额度。",
      final_subject: {
        text: "rejected",
        extra: { approved: false, reason: "wrong_product" },
      },
    };

  const submittedHuman = await rpcOk<JsonObject>(workflowRpc, "submit_step_output", {
    run_id: runId,
    node_id: "human_review",
    actor: "devtest",
    output: humanOutput as unknown as JsonValue,
  });
  await writeJsonFile(
    joinPath(reportDir, "submit_step_output.json"),
    submittedHuman as unknown as JsonObject,
  );

  const done = await waitForGraph(
    workflowRpc,
    runId,
    "completed image review run",
    (graph) => graph.status === "completed",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runId);
    const blocked = isAiccWaitingFailure(history, "human_review");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });
  await writeJsonFile(joinPath(reportDir, "final_graph.json"), done.graph);
  await writeJsonFile(joinPath(reportDir, "history.json"), done.history);

  if (decision === "approved") {
    assert.equal(done.graph.node_states.upscale_image, "completed");
    assert.equal(done.graph.node_states.remove_background, "completed");
    assert.equal(done.graph.node_states.completed_marker, "completed");
    assert.equal(done.graph.node_states.rejected_marker, "pending");
    assertArtifacts(done.graph.node_outputs.upscale_image, "upscale_image");
    assertArtifacts(
      done.graph.node_outputs.remove_background,
      "remove_background",
    );
    assert.equal(
      eventsFor(done.history, "rejected_marker", "step.started").length,
      0,
    );
  } else {
    assert.equal(done.graph.node_states.rejected_marker, "completed");
    assert.equal(done.graph.node_states.upscale_image, "pending");
    assert.equal(done.graph.node_states.remove_background, "pending");
    assert.equal(
      eventsFor(done.history, "upscale_image", "step.started").length,
      0,
    );
    assert.equal(
      eventsFor(done.history, "remove_background", "step.started").length,
      0,
    );
  }

  await assertHistoryResume(workflowRpc, runId, done.history);
  await assertRunListed(
    workflowRpc,
    submitted.workflowId,
    runId,
    "completed",
  );
}

async function runAgentCallbackCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-agent-callback.json"),
    runTag,
  );
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  const { runId, start } = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  await writeJsonFile(joinPath(reportDir, "start.json"), start);

  const waiting = await waitForGraph(
    workflowRpc,
    runId,
    "request_manual_tag waiting_human",
    (graph) =>
      graph.status === "waiting_human" &&
      graph.node_states.request_manual_tag === "waiting_human",
  );
  assert.ok(waiting.graph.human_waiting_nodes.includes("request_manual_tag"));

  const output = {
    tags: ["sample", "product", "needs-review"],
    source: "agent_callback",
  };
  const submittedOutput = await rpcOk<JsonObject>(workflowRpc, "submit_step_output", {
    run_id: runId,
    node_id: "request_manual_tag",
    actor: "agent/dv-callback",
    output,
  });
  await writeJsonFile(
    joinPath(reportDir, "submit_step_output.json"),
    submittedOutput as unknown as JsonObject,
  );

  const done = await waitForGraph(
    workflowRpc,
    runId,
    "agent callback completed",
    (graph) => graph.status === "completed",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runId);
    const blocked = isAiccWaitingFailure(history, "request_manual_tag");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });

  assert.equal(done.graph.node_states.request_manual_tag, "completed");
  assert.equal(done.graph.node_states.echo_tags, "completed");
  assert.ok(
    done.history.events.some((event) =>
      event.type === "step.completed" &&
      event.node_id === "request_manual_tag" &&
      event.actor === "agent/dv-callback"
    ),
    "submit_step_output actor should be reflected in history",
  );

  await writeJsonFile(joinPath(reportDir, "final_graph.json"), done.graph);
  await writeJsonFile(joinPath(reportDir, "history.json"), done.history);
  await assertRunListed(
    workflowRpc,
    submitted.workflowId,
    runId,
    "completed",
  );

  const defaultActorRun = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  await waitForGraph(
    workflowRpc,
    defaultActorRun.runId,
    "default actor run waiting_human",
    (graph) =>
      graph.status === "waiting_human" &&
      graph.node_states.request_manual_tag === "waiting_human",
  );
  // Without an actor, submit_step_output records "agent".
  await rpcOk<JsonObject>(workflowRpc, "submit_step_output", {
    run_id: defaultActorRun.runId,
    node_id: "request_manual_tag",
    output: { tags: ["default-actor"], source: "agent_default" },
  });
  const defaultDone = await waitForGraph(
    workflowRpc,
    defaultActorRun.runId,
    "default actor run completed",
    (graph) => graph.status === "completed",
  );
  assert.ok(
    defaultDone.history.events.some((event) =>
      event.type === "step.completed" &&
      event.node_id === "request_manual_tag" &&
      event.actor === "agent"
    ),
    "missing actor should default to agent",
  );
  await writeJsonFile(
    joinPath(reportDir, "default_actor_history.json"),
    defaultDone.history,
  );
}

async function runCacheCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-idempotent-cache.json"),
    runTag,
  );
  // The step cache outlives this DV: a new key makes the first run really execute.
  ((definition.steps as JsonObject[])[0].input as JsonObject).idempotency_key = runTag;
  const submitted = await submitDefinitionPair(workflowRpc, definition);

  const runA = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  const doneA = await waitForGraph(
    workflowRpc,
    runA.runId,
    "cache run A completed",
    (graph) => graph.status === "completed",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runA.runId);
    const blocked = isAiccWaitingFailure(history, "caption");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });

  const runB = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  const doneB = await waitForGraph(
    workflowRpc,
    runB.runId,
    "cache run B completed",
    (graph) => graph.status === "completed",
  );

  const completedA = eventsFor(doneA.history, "caption", "step.completed");
  const completedB = eventsFor(doneB.history, "caption", "step.completed");
  assert.ok(completedA.length > 0, "run A should complete caption");
  assert.ok(completedB.length > 0, "run B should complete caption");
  assert.notEqual(
    JSON.stringify(completedA.at(-1)?.payload ?? {}).includes(
      '"source":"cache"',
    ),
    true,
    "first run should not hit cache",
  );
  assert.equal(
    JSON.stringify(completedB.at(-1)?.payload ?? {}).includes(
      '"source":"cache"',
    ),
    true,
    "second run should hit workflow cache",
  );
  assert.equal(eventsFor(doneB.history, "caption", "step.started").length, 0);
  assert.deepEqual(
    doneB.graph.node_outputs.caption,
    doneA.graph.node_outputs.caption,
  );

  const changed = replaceGalleryImage(
    definition,
    "https://www.gstatic.com/webp/gallery/2.jpg",
  );
  ((changed.steps as JsonObject[])[0].input as JsonObject).idempotency_key = `${runTag}-changed`;
  const submittedChanged = await submitDefinitionPair(
    workflowRpc,
    changed,
  );
  const runC = await createAndStartRun(
    workflowRpc,
    submittedChanged.workflowId,
  );
  const doneC = await waitForGraph(
    workflowRpc,
    runC.runId,
    "cache run C completed",
    (graph) => graph.status === "completed",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runC.runId);
    const blocked = isAiccWaitingFailure(history, "caption");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });
  const completedC = eventsFor(doneC.history, "caption", "step.completed");
  assert.notEqual(
    JSON.stringify(completedC.at(-1)?.payload ?? {}).includes(
      '"source":"cache"',
    ),
    true,
    "changed input should not hit previous cache",
  );

  await writeJsonFile(joinPath(reportDir, "run_a_graph.json"), doneA.graph);
  await writeJsonFile(joinPath(reportDir, "run_a_history.json"), doneA.history);
  await writeJsonFile(joinPath(reportDir, "run_b_graph.json"), doneB.graph);
  await writeJsonFile(joinPath(reportDir, "run_b_history.json"), doneB.history);
  await writeJsonFile(joinPath(reportDir, "run_c_graph.json"), doneC.graph);
  await writeJsonFile(joinPath(reportDir, "run_c_history.json"), doneC.history);
}

async function runSemanticCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-semantic-skill-review.json"),
    runTag,
  );
  const dryRun = await rpcOk<JsonObject>(workflowRpc, "dry_run", {
    definition,
  });
  assertNoAnalysisErrors(dryRun, "semantic dry_run");

  const submitted = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    definition,
    tags: ["dv", "workflow", "semantic"],
  });
  const created = await rpcOk<JsonObject>(workflowRpc, "create_run", {
    workflow_id: submitted.workflow_id,
  });

  await rpcOk<JsonObject>(workflowRpc, "start_run", { run_id: created.run_id });
  const runId = String(created.run_id);
  // start_run returns at once; the step fails when the run gets to it.
  const settled = await waitForGraph(
    workflowRpc,
    runId,
    "semantic step failed",
    (graph) => Boolean(graph.node_errors?.semantic_review) || Boolean(graph.node_errors?.caption) || isTerminal(graph.status),
  );
  await writeJsonFile(joinPath(reportDir, "graph.json"), settled.graph);
  assert.ok(
    JSON.stringify((settled.graph as unknown as JsonObject).nodes).includes("/skill/image-quality-review"),
    "the compiled graph keeps the semantic path",
  );
  const text = JSON.stringify(settled.graph.node_errors ?? {});
  if (isAiccUnavailableText(text)) {
    throw new BlockedError(
      `AICC unavailable before semantic executor check: ${text}`,
    );
  }
  assert.ok(
    /image-quality-review|unresolved|semantic|require_function_object/.test(text),
    `the semantic executor should fail explicitly at execution time: ${text}`,
  );
  assert.equal(settled.graph.status, "failed", "a step that cannot be scheduled ends the run");
}

async function runFailureHumanRetryCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-failure-human-retry.json"),
    runTag,
  );
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  const { runId } = await createAndStartRun(
    workflowRpc,
    submitted.workflowId,
  );
  const waiting = await waitForGraph(
    workflowRpc,
    runId,
    "caption_bad waiting_human",
    (graph) =>
      graph.status === "waiting_human" &&
      graph.node_states.caption_bad === "waiting_human",
  );
  assert.ok(
    eventsFor(waiting.history, "caption_bad", "step.retrying").length >= 1,
  );
  assert.ok(
    eventsFor(waiting.history, "caption_bad", "step.waiting_human").length >= 1,
  );

  await rpcOk<JsonObject>(workflowRpc, "submit_step_output", {
    run_id: runId,
    node_id: "caption_bad",
    actor: "devtest",
    output: { text: "manually-supplied caption text" },
  });
  const done = await waitForGraph(
    workflowRpc,
    runId,
    "failure retry completed",
    (graph) => graph.status === "completed",
  ).catch(async (err) => {
    const history = await getHistory(workflowRpc, runId);
    const blocked = isAiccWaitingFailure(history, "caption_bad");
    if (blocked) throw new BlockedError(blocked);
    throw err;
  });
  const captionBadOutput = done.graph.node_outputs.caption_bad;
  assert.ok(
    captionBadOutput && typeof captionBadOutput === "object" &&
      !Array.isArray(captionBadOutput),
    "caption_bad output should be object",
  );
  assert.equal(
    (captionBadOutput as JsonObject).text,
    "manually-supplied caption text",
  );
  assert.equal(done.graph.node_states.echo_caption, "completed");
  await writeJsonFile(joinPath(reportDir, "final_graph.json"), done.graph);
  await writeJsonFile(joinPath(reportDir, "history.json"), done.history);
}


function isTerminal(status: string): boolean {
  return TERMINAL_STATUSES.includes(status);
}

async function waitTerminal(
  workflowRpc: RpcClient,
  runId: string,
  label: string,
): Promise<{ graph: GraphResponse; history: HistoryResponse }> {
  return await waitForGraph(
    workflowRpc,
    runId,
    label,
    (graph) => isTerminal(graph.status),
  );
}

/** Expects `{ok:false, error:<code>}`; a thrown RPC error (NoPermission) counts as `rpc_error`. */
async function expectRejected(
  rpc: RpcClient,
  method: string,
  params: Record<string, unknown>,
  expectedCodes: string[],
): Promise<string> {
  let raw: unknown;
  try {
    raw = await rpc.call(method, params);
  } catch (err) {
    if (!expectedCodes.includes("rpc_error")) {
      throw new Error(
        `${method} threw instead of returning a coded error: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }
    return "rpc_error";
  }
  const body = raw as Record<string, unknown>;
  assert.equal(body.ok, false, `${method} must be rejected: ${JSON.stringify(body)}`);
  const code = String(body.error);
  assert.ok(
    expectedCodes.includes(code),
    `${method} rejected with ${code}, expected one of ${expectedCodes.join("|")}`,
  );
  return code;
}

/**
 * Uploads a fixture through NFSP (the same path the file browser uses) and
 * returns the FileObject id the workflow service accepts as `obj_id`; the
 * caller principal therefore has a real read right on the object.
 */
async function uploadInput(
  zoneHost: string,
  sessionToken: string,
  runTag: string,
  fixture: string,
): Promise<UploadedInput> {
  const bytes = await Deno.readFile(new URL(`./fixtures/${fixture}`, import.meta.url));
  const chunkId = ndn.ChunkId.fromMix256Result(bytes.byteLength, ndn.sha256Bytes(bytes)).toString();
  const proxy = ndm_proxy.createNdmProxyClient({
    endpoint: `https://${zoneHost}`,
    sessionToken,
    fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
      const target = typeof request === "string"
        ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
        : request instanceof URL
        ? new URL(request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"))
        : request;
      return fetch(target, init);
    },
  });
  await proxy.putChunk(chunkId, bytes);
  const file = new ndn.FileObject(`${runTag}-${fixture}`, bytes.byteLength, chunkId);
  file.content_obj.create_time = Date.now();
  const [objId, objStr] = file.genObjId();
  await proxy.putObject({ obj_id: objId.toString(), obj_data: objStr });
  return { obj_id: objId.toString(), node_id: chunkId, path: `${runTag}/${fixture}` };
}

/**
 * LibreOffice stamps every PDF differently, so a document is only reusable if
 * the first conversion is what later runs get: every step of the second run,
 * to_pdf included, comes from the step cache.
 */
async function runDocumentCacheCase(args: {
  workflowRpc: RpcClient;
  reportDir: string;
  note: UploadedInput;
}): Promise<void> {
  const { workflowRpc, reportDir, note } = args;
  const input: JsonObject = { obj_id: note.obj_id, format: "txt", dpi: 72, limits: { max_pages: 2 } };
  const first = await createSystemRun(workflowRpc, "media.document", input);
  const doneA = await waitTerminal(workflowRpc, first.run_id, "media.document run A");
  await writeJsonFile(joinPath(reportDir, "run-a.json"), doneA.graph);
  assert.equal(doneA.graph.status, "completed", JSON.stringify(doneA.graph.node_errors));
  const second = await createSystemRun(workflowRpc, "media.document", input);
  const doneB = await waitTerminal(workflowRpc, second.run_id, "media.document run B");
  await writeJsonFile(joinPath(reportDir, "run-b.json"), doneB.graph);
  assert.equal(doneB.graph.status, "completed", JSON.stringify(doneB.graph.node_errors));
  assert.equal(doneB.graph.node_sources?.capabilities, "executed");
  for (const step of ["probe", "to_pdf", "extract_pdf_text", "render_pdf_pages", "build_catalog"]) {
    assert.equal(doneB.graph.node_sources?.[step], "cache", `${step} of the second run comes from the cache`);
  }
  const catalog = (graph: typeof doneA.graph) => (graph.output?.build_catalog as JsonObject)?.manifest_obj_id;
  assert.equal(catalog(doneB.graph), catalog(doneA.graph), "both runs read the same catalog");
}

/**
 * media.video reads its input through `/ndn/` with a read token bound to one key,
 * not a temp copy (bucket design §21 N17): the token and key records prove which path
 * ran, and both are gone when the step ends.
 */
async function runMediaVideoNdnCase(args: {
  workflowRpc: RpcClient;
  bucketRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
  video: UploadedInput;
}): Promise<void> {
  const { workflowRpc, bucketRpc, runTag, reportDir, video } = args;
  // The step cache outlives a Zone restart: a new edge makes extract_frames really run.
  const edge = 128 + (parseInt(runTag.slice(-6), 36) % 1000);
  const input: JsonObject = { obj_id: video.obj_id, max_frames: 2, frame_max_edge: edge };
  const first = await createSystemRun(workflowRpc, "media.video", input);
  const doneA = await waitTerminal(workflowRpc, first.run_id, "media.video run A");
  await writeJsonFile(joinPath(reportDir, "run-a.json"), doneA.graph);
  assert.equal(doneA.graph.status, "completed", JSON.stringify(doneA.graph.node_errors));
  assert.equal(doneA.graph.node_sources?.extract_frames, "executed");
  const output = doneA.graph.node_outputs as Record<string, JsonObject>;
  const frames = output.extract_frames.frames as JsonObject[];
  assert.equal(frames.length, 2);
  assert.ok(frames.every((frame) => frame.status === "ok"), JSON.stringify(frames));
  assert.equal(output.extract_audio.status, "ok", JSON.stringify(output.extract_audio));
  assert.equal(output.probe.has_audio, true);
  assert.equal((doneA.graph.output as Record<string, JsonObject>).build_catalog.complete, true);

  const scratch = "zone/workflow/_/scratch";
  const prefix = `inputs/${first.run_id}/`;
  const tokens = await rpcCall<JsonObject[]>(bucketRpc, "bucket.list_access_tokens", { bucket_id: scratch });
  const ours = tokens.filter((token) => String(token.key ?? "").startsWith(prefix));
  await writeJsonFile(joinPath(reportDir, "tokens.json"), ours);
  assert.ok(ours.length >= 1, "the step read through a token, so it did not fall back to a temp copy");
  assert.ok(ours.every((token) => token.revoked_at), `every token is revoked: ${JSON.stringify(ours)}`);
  const listed = await rpcCall<{ items: JsonObject[] }>(bucketRpc, "bucket.list_objects", { bucket_id: scratch, prefix });
  assert.deepEqual(listed.items, [], "the input key is unbound when the step ends");

  const shown = JSON.stringify([doneA.graph, doneA.history]);
  assert.ok(!shown.includes("bkt_") && !shown.includes("hint="), "no token in run data");

  // Same thunk, same cache key as ever: the URL input changed nothing about it.
  const second = await createSystemRun(workflowRpc, "media.video", input);
  const doneB = await waitTerminal(workflowRpc, second.run_id, "media.video run B");
  assert.equal(doneB.graph.status, "completed", JSON.stringify(doneB.graph.node_errors));
  assert.equal(doneB.graph.node_sources?.extract_frames, "cache");
  assert.deepEqual(doneB.graph.node_outputs.extract_frames, output.extract_frames);
}

async function createSystemRun(
  workflowRpc: RpcClient,
  workflowId: string,
  input: JsonObject,
  extra: Record<string, unknown> = {},
): Promise<CreateRunResponse> {
  return await rpcOk<CreateRunResponse>(workflowRpc, "create_run", {
    workflow_id: workflowId,
    input,
    auto_start: true,
    wait_ms: 1000,
    ...extra,
  });
}

async function runStepCacheCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
  image: UploadedInput;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir, image } = args;

  // (a) user definition on test.echo: no media tools involved.
  const definition = prepareDefinition(await loadFixture("wf-dv-echo-cache.json"), runTag);
  // The step cache is persistent across service restarts, so the first run
  // must carry a value no earlier DV run has computed.
  ((definition.steps as JsonObject[])[0].input as JsonObject).value = `dv-echo-${runTag}`;
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  const first = await createSystemRun(workflowRpc, submitted.workflowId, {});
  const doneA = await waitTerminal(workflowRpc, first.run_id, "echo run A");
  assert.equal(doneA.graph.status, "completed", JSON.stringify(doneA.graph.node_errors));
  assert.equal(doneA.graph.node_sources?.a, "executed");
  assert.equal(doneA.graph.node_sources?.b, "executed");
  const second = await createSystemRun(workflowRpc, submitted.workflowId, {});
  assert.notEqual(second.run_id, first.run_id, "completed runs are never reused");
  const doneB = await waitTerminal(workflowRpc, second.run_id, "echo run B");
  assert.equal(doneB.graph.status, "completed");
  assert.equal(doneB.graph.node_sources?.a, "cache", "echo A must come from the step cache");
  assert.equal(doneB.graph.node_sources?.b, "cache", "echo B must come from the step cache");
  assert.deepEqual(doneB.graph.output, doneA.graph.output);
  assert.equal(eventsFor(doneB.history, "a", "step.started").length, 0);

  // (b) system media.image on an uploaded image.
  const uniqueEdge = 300 + (parseInt(runTag.slice(-6), 36) % 900);
  const imageInput: JsonObject = { obj_id: image.obj_id, max_long_edge: uniqueEdge };
  const runA = await createSystemRun(workflowRpc, "media.image", imageInput);
  assert.equal(runA.workflow_id, "media.image");
  const imgA = await waitTerminal(workflowRpc, runA.run_id, "media.image run A");
  assert.equal(imgA.graph.status, "completed", JSON.stringify(imgA.graph.node_errors));
  assert.equal(imgA.graph.node_sources?.capabilities, "executed");
  assert.equal(imgA.graph.node_sources?.fit_image, "executed");
  const fitA = imgA.graph.output?.fit_image as JsonObject;
  assert.equal(typeof fitA?.model_media_obj_id, "string", "fit_image output carries model_media_obj_id");
  const runB = await createSystemRun(workflowRpc, "media.image", imageInput);
  const imgB = await waitTerminal(workflowRpc, runB.run_id, "media.image run B");
  assert.equal(imgB.graph.status, "completed", JSON.stringify(imgB.graph.node_errors));
  assert.equal(imgB.graph.node_sources?.capabilities, "executed", "capabilities is impure and re-executes");
  assert.equal(imgB.graph.node_sources?.probe, "cache");
  assert.equal(imgB.graph.node_sources?.fit_image, "cache", "fit_image must hit the step cache");
  const fitB = imgB.graph.output?.fit_image as JsonObject;
  assert.equal(fitB?.model_media_obj_id, fitA?.model_media_obj_id, "cached fit_image returns the same artifact");

  // A different effective spec misses the cache.
  const runC = await createSystemRun(workflowRpc, "media.image", { obj_id: image.obj_id, max_long_edge: uniqueEdge + 1 });
  const imgC = await waitTerminal(workflowRpc, runC.run_id, "media.image run C");
  assert.equal(imgC.graph.status, "completed", JSON.stringify(imgC.graph.node_errors));
  assert.equal(imgC.graph.node_sources?.fit_image, "executed", "changed max_long_edge must recompute");

  await writeJsonFile(joinPath(reportDir, "echo_a.json"), doneA.graph);
  await writeJsonFile(joinPath(reportDir, "echo_b.json"), doneB.graph);
  await writeJsonFile(joinPath(reportDir, "image_a.json"), imgA.graph);
  await writeJsonFile(joinPath(reportDir, "image_b.json"), imgB.graph);
  await writeJsonFile(joinPath(reportDir, "image_c.json"), imgC.graph);
}

async function runEnsureCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(await loadFixture("wf-dv-ensure-sleep.json"), runTag);
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  const input: JsonObject = { ms: 4000 };
  const parallel = await Promise.all(
    Array.from({ length: 4 }, () =>
      createSystemRun(workflowRpc, submitted.workflowId, input, { ensure: true, wait_ms: 0 })),
  );
  const runIds = new Set(parallel.map((r) => r.run_id));
  assert.equal(runIds.size, 1, `concurrent ensure requests must share one run, got ${[...runIds].join(",")}`);
  const reused = parallel.filter((r) => r.reused).length;
  assert.equal(reused, 3, `exactly one request creates the run, ${reused} reused`);
  const runId = parallel[0].run_id;

  // Same input, different normalization form (defaults applied server-side) still merges.
  const defaulted = await createSystemRun(workflowRpc, submitted.workflowId, {}, { ensure: true, wait_ms: 0 });
  assert.equal(defaulted.run_id, runId, "schema defaults normalize into the same ensure key");
  assert.equal(defaulted.reused, true);

  // A different input is a different ensure scope.
  const other = await createSystemRun(workflowRpc, submitted.workflowId, { ms: 4500 }, { ensure: true, wait_ms: 0 });
  assert.notEqual(other.run_id, runId);

  const done = await waitTerminal(workflowRpc, runId, "ensure run completed");
  assert.equal(done.graph.status, "completed", JSON.stringify(done.graph.node_errors));
  await waitTerminal(workflowRpc, other.run_id, "other ensure run completed");
  const after = await createSystemRun(workflowRpc, submitted.workflowId, input, { ensure: true, wait_ms: 0 });
  assert.notEqual(after.run_id, runId, "terminal runs leave the ensure index");
  assert.equal(after.reused, false);
  await waitTerminal(workflowRpc, after.run_id, "post-terminal ensure run");

  await writeJsonFile(joinPath(reportDir, "parallel.json"), parallel);
  await writeJsonFile(joinPath(reportDir, "after.json"), after);
}

async function runMissingToolsCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  reportDir: string;
  pdf: UploadedInput;
}): Promise<void> {
  const { workflowRpc, owner, reportDir, pdf } = args;
  const caps = await rpcOk<{ ok: true; environment: { missing: string[]; tools: JsonObject; revision: string } }>(
    workflowRpc,
    "get_capabilities",
    {},
  );
  const missingPoppler = POPPLER_TOOLS.filter((t) => caps.environment.missing.includes(t));
  const created = await createSystemRun(workflowRpc, "media.pdf", {
    obj_id: pdf.obj_id,
    dpi: 72,
    limits: { max_pages: 2 },
  });
  const done = await waitTerminal(workflowRpc, created.run_id, "media.pdf run");
  const branch = missingPoppler.length > 0 ? "tools_missing" : "tools_present";
  if (branch === "tools_missing") {
    assert.equal(done.graph.status, "failed", "missing poppler must fail the run");
    const failed = Object.entries(done.graph.node_errors ?? {}).find(([, err]) => err.code === "tools_missing");
    assert.ok(failed, `a step must fail with tools_missing: ${JSON.stringify(done.graph.node_errors)}`);
    const details = (failed![1].details as JsonObject | undefined) ?? {};
    const listed = (details.tools_missing as string[] | undefined) ?? [];
    assert.ok(
      listed.some((t) => POPPLER_TOOLS.includes(t)),
      `tools_missing details must name a poppler tool: ${JSON.stringify(details)}`,
    );
    assert.equal(done.graph.human_waiting_nodes.length, 0, "media runs never wait for a human");
    assert.ok(
      done.history.events.some((e) => e.type === "step.failed" && JSON.stringify(e.payload).includes("tools_missing")),
      "step.failed event carries the structured error",
    );
  } else {
    assert.equal(done.graph.status, "completed", JSON.stringify(done.graph.node_errors));
    const catalog = done.graph.output?.build_catalog as JsonObject;
    assert.equal(typeof catalog?.manifest_obj_id, "string", "build_catalog output has manifest_obj_id");
    const summary = catalog.summary as JsonObject;
    assert.equal(summary.family, "pdf");
    assert.equal(summary.total, 2);
    assert.equal(summary.complete, true);
    assert.ok(Number(summary.text_entries) >= 1, "text entries extracted");
    assert.ok(Number(summary.image_entries) >= 1, "page images rendered");
    const output = await rpcOk<{ ok: true; terminal: boolean; delivery: { artifacts: string[] } }>(
      workflowRpc,
      "get_run_output",
      { run_id: created.run_id },
    );
    assert.equal(output.terminal, true);
    assert.ok(output.delivery.artifacts.includes(String(catalog.manifest_obj_id)));
  }
  await writeJsonFile(joinPath(reportDir, "branch.json"), {
    branch,
    missing: caps.environment.missing,
    revision: caps.environment.revision,
  });
  await writeJsonFile(joinPath(reportDir, "graph.json"), done.graph);
  await writeJsonFile(joinPath(reportDir, "history.json"), done.history);
}

async function runObservabilityCase(args: {
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  owner: Owner;
  reportDir: string;
  image: UploadedInput;
}): Promise<void> {
  const { workflowRpc, taskMgrRpc, owner, reportDir, image } = args;
  const listed = await rpcOk<{ ok: true; definitions: JsonObject[] }>(workflowRpc, "list_definitions", {
    source: "system",
  });
  const systemIds = listed.definitions.map((d) => String(d.id)).sort();
  assert.deepEqual(systemIds, [...SYSTEM_DEFINITION_IDS].sort());
  for (const def of listed.definitions) {
    assert.equal(def.read_only, true, `${def.id} is read-only`);
    assert.equal(def.source, "system");
    assert.equal(typeof def.revision, "string");
  }
  const pdfDef = await rpcOk<{ ok: true; definition: JsonObject }>(workflowRpc, "get_definition", {
    workflow_id: "media.pdf",
  });
  const resolved = pdfDef.definition.resolved_functions as Record<string, string>;
  assert.ok(Object.keys(resolved).length >= 5, "every media.pdf step binds a function");
  for (const [node, funId] of Object.entries(resolved)) {
    assert.ok(funId.startsWith("func:"), `${node} bound to a real fun_id, got ${funId}`);
  }
  const functions = await rpcOk<{ ok: true; functions: JsonObject[] }>(workflowRpc, "list_functions", {
    prefix: "media.",
  });
  const names = functions.functions.map((f) => (f.registration as JsonObject).name).sort();
  assert.equal(names.length, 9, `9 media functions, got ${names.join(",")}`);
  const probe = await rpcOk<{ ok: true; function: JsonObject }>(workflowRpc, "get_function", {
    name: "media.probe",
    version: "1",
  });
  assert.equal(probe.function.published, true, "FunctionObject is readable from the object store");
  assert.ok(String(probe.function.fun_id).startsWith("func:"));

  const created = await createSystemRun(workflowRpc, "media.image", { obj_id: image.obj_id });
  const done = await waitTerminal(workflowRpc, created.run_id, "observability run");
  assert.equal(done.graph.revision, created.revision);
  assert.ok(String(done.graph.requested_by).startsWith(`${owner.user_id}/`), `requested_by ${done.graph.requested_by} belongs to ${owner.user_id}`);
  assert.equal(done.graph.executor, "workflow");
  assert.ok(done.graph.node_sources && Object.keys(done.graph.node_sources).length >= 3);
  await assertRunListed(workflowRpc, "media.image", created.run_id, done.graph.status);

  // A run has one id: it is its root task's id.
  const { task: root } = await rpcCall<{ task: TaskSummary }>(taskMgrRpc, "get_task", { task_id: created.run_id });
  assert.equal(root.task_id, created.run_id, "the run id is the root task id");
  assert.equal(root.schema_id, "workflow.run_tree/v1");
  const tree = await rpcCall<{ tasks?: TaskSummary[] }>(taskMgrRpc, "get_task_tree", {
    root_id: root.root_id,
    depth: 4,
  });
  const schemas = new Set((tree.tasks ?? []).map((t) => t.schema_id));
  assert.ok(schemas.has("workflow.step/v1"), `step tasks under the run: ${[...schemas].join(",")}`);
  assert.ok(schemas.has("workflow.thunk/v1"), `thunk tasks under the steps: ${[...schemas].join(",")}`);
  const thunks = (tree.tasks ?? []).filter((t) => t.schema_id === "workflow.thunk/v1");
  assert.ok(
    thunks.every((t) => String(t.phase).toLowerCase() === "terminal"),
    `thunk tasks reach a terminal phase: ${JSON.stringify(thunks.map((t) => [t.name, t.phase]))}`,
  );

  await writeJsonFile(joinPath(reportDir, "definitions.json"), listed);
  await writeJsonFile(joinPath(reportDir, "functions.json"), functions);
  await writeJsonFile(joinPath(reportDir, "graph.json"), done.graph);
  await writeJsonFile(joinPath(reportDir, "task_tree.json"), tree);
}


/** A second, non-admin principal (`WORKFLOW_DV_SECOND_USER=user:password`,
 * default the make_config seed user `bob`) logged in through control-panel. */
async function secondPrincipal(
  zoneHost: string,
): Promise<{ userId: string; rpc: RpcClient; token: string } | null> {
  const spec = getEnv("WORKFLOW_DV_SECOND_USER") ?? "bob:buckyos2026";
  const sep = spec.indexOf(":");
  if (sep <= 0) return null;
  const username = spec.slice(0, sep);
  const password = spec.slice(sep + 1);
  const sha256b64 = async (text: string) => {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
    return btoa(String.fromCharCode(...new Uint8Array(digest)));
  };
  const nonce = Date.now();
  const base = await sha256b64(`${password}${username}.buckyos`);
  const hashed = await sha256b64(`${base}${nonce}`);
  const response = await fetch(`https://${zoneHost}/kapi/control-panel`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      method: "auth.login",
      params: {
        username,
        password: hashed,
        target: { kind: "system", service_id: "control-panel" },
        login_nonce: nonce,
      },
      sys: [nonce],
    }),
  });
  if (!response.ok) return null;
  const body = await response.json() as { result?: { session_token?: string }; error?: unknown };
  const token = body.result?.session_token;
  if (typeof token !== "string") {
    console.warn(`[second-principal] login(${username}) failed: ${JSON.stringify(body.error ?? body)}`);
    return null;
  }
  const { buckyos } = await import("buckyos");
  const rpc = new buckyos.kRPCClient(`https://${zoneHost}/kapi/workflow`, token) as RpcClient;
  return { userId: username, rpc, token };
}

async function runReadOnlyCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
  image: UploadedInput;
  zoneHost: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir, image, zoneHost } = args;
  const definition = prepareDefinition(await loadFixture("wf-dv-echo-cache.json"), runTag);
  const reserved = cloneJson(definition);
  reserved.id = "media.x";
  reserved.name = "media.x";
  const results: Record<string, string> = {};
  results.reserved_id = await expectRejected(workflowRpc, "submit_definition", {
    definition: reserved,
  }, ["system_definition_read_only"]);
  results.system_source = await expectRejected(workflowRpc, "submit_definition", {
    definition,
    source: "system",
  }, ["rpc_error"]);
  results.archive_system = await expectRejected(workflowRpc, "archive_definition", {
    workflow_id: "media.pdf",
  }, ["system_definition_read_only"]);
  results.forged_owner_create = await expectRejected(workflowRpc, "create_run", {
    workflow_id: "media.image",
    owner: { user_id: `${owner.user_id}-other`, app_id: owner.app_id },
    input: { obj_id: image.obj_id },
  }, ["rpc_error"]);
  // An admin session may filter by any owner; a plain session is rejected.
  // Either way no run of another owner may leak.
  results.forged_owner_list = await (async () => {
    try {
      const listed = await rpcOk<{ runs: JsonObject[] }>(workflowRpc, "list_runs", {
        owner: { user_id: `${owner.user_id}-other`, app_id: owner.app_id },
      });
      assert.equal(listed.runs.length, 0, "no runs of another owner may be listed");
      return "allowed_empty(admin)";
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      assert.ok(/permission/i.test(text), `unexpected list_runs failure: ${text}`);
      return "rejected";
    }
  })();
  results.content_id_stripped = "n/a";
  const created = await createSystemRun(workflowRpc, "media.image", {
    obj_id: image.obj_id,
    content_id: "sha256:0000",
  } as JsonObject);
  const graph = await getGraph(workflowRpc, created.run_id);
  const trigger = graph.node_outputs.trigger as JsonObject;
  assert.notEqual(trigger.content_id, "sha256:0000", "callers cannot set content_id");
  results.content_id_stripped = "ok";

  const second = await secondPrincipal(zoneHost);
  if (!second) {
    await writeJsonFile(joinPath(reportDir, "results.json"), results);
    throw new BlockedError(
      `rejections verified (${JSON.stringify(results)}); foreign-principal checks need WORKFLOW_DV_SECOND_USER=user:password`,
    );
  }
  assert.notEqual(second.userId, owner.user_id, "second principal must be a different user");
  const foreign = second.rpc;
  for (const method of ["get_run_graph", "get_run_output", "get_history", "cancel_run"]) {
    results[`foreign_${method}`] = await expectRejected(foreign, method, { run_id: created.run_id }, ["rpc_error"]);
  }
  const foreignList = await rpcOk<{ runs: JsonObject[] }>(foreign, "list_runs", {});
  assert.ok(
    foreignList.runs.every((run) => (run.owner as JsonObject)?.user_id !== owner.user_id),
    `another principal must not list ${owner.user_id}'s runs: ${JSON.stringify(foreignList.runs)}`,
  );
  results.foreign_list_runs = `ok(${foreignList.runs.length})`;
  results.foreign_list_owner_filter = await expectRejected(foreign, "list_runs", {
    owner: { user_id: owner.user_id, app_id: owner.app_id },
  }, ["rpc_error"]);
  // Knowing an obj_id is not a read right: the second principal cannot start a
  // conversion on the first principal's object.
  results.foreign_input_obj_id = await expectRejected(foreign, "create_run", {
    workflow_id: "media.image",
    input: { obj_id: image.obj_id },
  }, ["rpc_error"]);
  const stillOwn = await getGraph(workflowRpc, created.run_id);
  assert.notEqual(stillOwn.status, "aborted", "foreign cancel_run must not touch the run");
  await writeJsonFile(joinPath(reportDir, "results.json"), results);
}

/**
 * What a system pipeline produced becomes readable to whoever asked for it, by the
 * path Agents and AICC read (`get_object`, `object/open`), and to nobody else. An
 * object id alone stays worthless, whichever workflow surface it is shown to.
 */
async function runDeliveredHoldingsCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
  pdf: UploadedInput;
  zoneHost: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir, pdf, zoneHost } = args;
  const second = await secondPrincipal(zoneHost);
  if (!second) {
    throw new BlockedError("needs a non-admin principal: WORKFLOW_DV_SECOND_USER=user:password");
  }
  assert.notEqual(second.userId, owner.user_id, "second principal must be a different user");
  const post = async (path: string, body: JsonObject, token: string) => {
    const response = await fetch(`https://${zoneHost}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify(body),
    });
    await response.body?.cancel();
    return response.status;
  };
  const getObject = (objId: string) => post("/ndm/v1/store/get_object", { obj_id: objId }, second.token);
  const openObject = (objId: string) => post("/ndm/proxy/v1/read/object/open", { obj_id: objId }, second.token);

  // Holdings outlive the run: a new dpi gives this round a catalog nobody holds yet.
  const dpi = 72 + (parseInt(runTag.slice(-6), 36) % 72);
  const input: JsonObject = { obj_id: pdf.obj_id, dpi };
  const first = await createSystemRun(workflowRpc, "media.pdf", input);
  const doneA = await waitTerminal(workflowRpc, first.run_id, "media.pdf of the first principal");
  assert.equal(doneA.graph.status, "completed", JSON.stringify(doneA.graph.node_errors));
  const manifestId = String((doneA.graph.output?.build_catalog as JsonObject).manifest_obj_id);
  const results: Record<string, unknown> = {};

  // Knowing the ids reads nothing; another principal's run hands nothing over.
  assert.equal(await getObject(manifestId), 403, "the manifest id alone is no read right");
  results.foreign_output = await expectRejected(second.rpc, "get_run_output", { run_id: first.run_id }, ["rpc_error"]);
  assert.equal(await getObject(manifestId), 403, "a refused get_run_output grants nothing");

  // Naming an object in the output of one's own workflow grants nothing either.
  const naming = prepareDefinition(await loadFixture("wf-dv-echo-cache.json"), `${runTag}-naming`);
  ((naming.steps as JsonObject[])[0].input as JsonObject).value = manifestId;
  const submitted = await rpcOk<{ workflow_id: string }>(second.rpc, "submit_definition", { definition: naming });
  const namingRun = await rpcOk<CreateRunResponse>(second.rpc, "create_run", {
    workflow_id: submitted.workflow_id,
    auto_start: true,
    wait_ms: 1000,
  });
  const namingDone = await waitTerminal(second.rpc, namingRun.run_id, "echo naming a foreign object");
  assert.equal(namingDone.graph.status, "completed", JSON.stringify(namingDone.graph.node_errors));
  const namingOutput = await rpcOk<{ delivery: { artifacts: string[]; held: number } }>(
    second.rpc,
    "get_run_output",
    { run_id: namingRun.run_id },
  );
  assert.ok(JSON.stringify(namingOutput).includes(manifestId), "the run does name the manifest");
  assert.equal(namingOutput.delivery.held, 0, "a user definition's output grants nothing");
  assert.equal(await getObject(manifestId), 403, "naming the manifest is no read right");

  // Nor can a run of a system pipeline be made to say something else.
  const bobPdf = await uploadInput(zoneHost, second.token, `${runTag}-second`, "dv-doc.pdf");
  const idle = await rpcOk<CreateRunResponse>(second.rpc, "create_run", {
    workflow_id: "media.pdf",
    input: { obj_id: bobPdf.obj_id, dpi },
    auto_start: false,
  });
  results.forged_step_output = await expectRejected(second.rpc, "submit_step_output", {
    run_id: idle.run_id,
    node_id: "build_catalog",
    output: { manifest_obj_id: manifestId },
  }, ["system_definition_read_only"]);
  await rpcOk(second.rpc, "cancel_run", { run_id: idle.run_id });

  // A schedule has no caller to authorize its input against, so it cannot start one at all.
  const scheduled = await second.rpc.call("create_scheduled_task", {
    name: `${runTag}-schedule`,
    owner: (await rpcOk<{ owner: Owner }>(second.rpc, "get_run_graph", { run_id: idle.run_id })).owner,
    schedule: { kind: "cron", expr: "0 0 * * *" },
    target: {
      task_type: "workflow.run",
      name_template: "convert",
      data_template: { request: { workflow_id: "media.pdf", input: { obj_id: bobPdf.obj_id, content_id: pdf.node_id } } },
    },
  }).then((value) => JSON.stringify(value), (err) => String(err));
  assert.ok(/cannot be scheduled/.test(scheduled), `a system pipeline cannot be scheduled: ${scheduled}`);
  results.scheduled_system_pipeline = "rejected";

  // Their own request for the same content is served from the shared cache and delivered.
  const own = await createSystemRun(second.rpc, "media.pdf", { obj_id: bobPdf.obj_id, dpi });
  const doneB = await waitTerminal(second.rpc, own.run_id, "media.pdf of the second principal");
  assert.equal(doneB.graph.status, "completed", JSON.stringify(doneB.graph.node_errors));
  assert.equal(doneB.graph.node_sources?.build_catalog, "cache", "the same catalog comes from the shared cache");
  assert.equal((doneB.graph.output?.build_catalog as JsonObject).manifest_obj_id, manifestId);
  assert.equal(await getObject(manifestId), 403, "a run alone grants nothing before its output is delivered");
  const delivered = await rpcOk<{ delivery: { held: number; hold_error?: string | null } }>(
    second.rpc,
    "get_run_output",
    { run_id: own.run_id },
  );
  assert.equal(delivered.delivery.hold_error ?? null, null, "the grant succeeded");
  assert.ok(delivered.delivery.held > 1, `manifest and its artifacts, got ${delivered.delivery.held}`);
  assert.equal(await getObject(manifestId), 200, "the requester reads the manifest");

  const manifest = await ndmManifest(zoneHost, second.token, manifestId);
  const entries = manifest.entries as JsonObject[];
  const artifacts = entries.flatMap((entry) =>
    [entry.text_obj_id, entry.image_obj_id, entry.audio_obj_id].filter((id): id is string => typeof id === "string")
  );
  assert.ok(artifacts.length >= 2, `artifacts of the manifest: ${JSON.stringify(entries)}`);
  for (const artifact of artifacts) {
    assert.equal(await getObject(artifact), 200, `the requester reads ${artifact}`);
    assert.equal(await openObject(artifact), 200, `the requester opens ${artifact}`);
  }
  results.delivered = { held: delivered.delivery.held, artifacts: artifacts.length };
  await writeJsonFile(joinPath(reportDir, "results.json"), results);
}

async function ndmManifest(zoneHost: string, token: string, manifestId: string): Promise<JsonObject> {
  const response = await fetch(`https://${zoneHost}/ndm/v1/store/get_object`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify({ obj_id: manifestId }),
  });
  assert.equal(response.status, 200);
  const body = await response.json() as { obj_data?: string };
  return JSON.parse(String(body.obj_data)) as JsonObject;
}

/** A person's decision goes through `submit_human_action`: the run moves on, and the actor is who really called. */
async function runHumanActionCase(args: {
  workflowRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
  zoneHost: string;
}): Promise<void> {
  const { workflowRpc, owner, runTag, reportDir, zoneHost } = args;
  const definition = prepareDefinition(await loadFixture("wf-dv-human-actions.json"), runTag);
  const submitted = await submitDefinitionPair(workflowRpc, definition);
  const waitingRun = async (label: string) => {
    const { runId } = await createAndStartRun(workflowRpc, submitted.workflowId);
    await waitForGraph(workflowRpc, runId, label, (graph) =>
      graph.status === "waiting_human" && graph.node_states.review === "waiting_human");
    return runId;
  };
  const decide = (runId: string, kind: string, payload?: JsonObject) =>
    rpcOk<JsonObject>(workflowRpc, "submit_human_action", { run_id: runId, node_id: "review", kind, payload });

  const approved = await waitingRun("run to approve");
  await decide(approved, "approve");
  const approvedDone = await waitForGraph(workflowRpc, approved, "approved run", (graph) => graph.status === "completed");
  assert.equal(approvedDone.graph.node_states.publish, "completed");
  assert.equal(approvedDone.graph.node_states.discard, "pending");
  const action = approvedDone.history.events.find((event) => event.type === "human.action");
  assert.ok(String(action?.actor).startsWith(`${owner.user_id}/`), `the actor is the verified caller, got ${action?.actor}`);
  await writeJsonFile(joinPath(reportDir, "approved_history.json"), approvedDone.history);

  const modified = await waitingRun("run to modify");
  await decide(modified, "modify", { modified_subject: { echo: "edited" } });
  const modifiedDone = await waitForGraph(workflowRpc, modified, "modified run", (graph) => graph.status === "completed");
  assert.deepEqual((modifiedDone.graph.node_outputs.publish as JsonObject).echo, { echo: "edited" });

  const rejected = await waitingRun("run to reject");
  await decide(rejected, "reject", { feedback: "not this one" });
  const rejectedDone = await waitForGraph(workflowRpc, rejected, "rejected run", (graph) => graph.status === "completed");
  assert.equal(rejectedDone.graph.node_states.discard, "completed");
  assert.equal(rejectedDone.graph.node_states.publish, "pending");

  const refused = await waitingRun("run that refuses");
  const results: Record<string, string> = {};
  results.unknown_kind = await expectRejected(workflowRpc, "submit_human_action", { run_id: refused, node_id: "review", kind: "sign" }, ["invalid_human_action"]);
  results.not_skippable = await expectRejected(workflowRpc, "submit_human_action", { run_id: refused, node_id: "review", kind: "skip" }, ["node_not_skippable"]);
  const second = await secondPrincipal(zoneHost);
  if (second) {
    results.foreign_caller = await expectRejected(second.rpc, "submit_human_action", { run_id: refused, node_id: "review", kind: "approve" }, ["rpc_error"]);
  }
  const untouched = await getGraph(workflowRpc, refused);
  assert.equal(untouched.status, "waiting_human", "refused actions change nothing");
  const aborted = await decide(refused, "abort");
  assert.equal(aborted.status, "aborted");
  results.after_abort = await expectRejected(workflowRpc, "submit_human_action", { run_id: refused, node_id: "review", kind: "approve" }, ["run_terminal"]);
  await writeJsonFile(joinPath(reportDir, "results.json"), results);
}

function selectedCaseIds(): string[] {
  if (!WORKFLOW_DV_CASES) return DEFAULT_CASES;
  return WORKFLOW_DV_CASES.split(",").map((item) => item.trim()).filter(
    Boolean,
  );
}

type DvInputs = {
  image: () => Promise<UploadedInput>;
  pdf: () => Promise<UploadedInput>;
  video: () => Promise<UploadedInput>;
  note: () => Promise<UploadedInput>;
};

function lazyInputs(zoneHost: string, sessionToken: string, runTag: string): DvInputs {
  const cache: Partial<Record<"image" | "pdf" | "video" | "note", Promise<UploadedInput>>> = {};
  const get = (key: "image" | "pdf" | "video" | "note", fixture: string) => () => {
    cache[key] ??= uploadInput(zoneHost, sessionToken, runTag, fixture);
    return cache[key]!;
  };
  return {
    image: get("image", "dv-image.png"),
    pdf: get("pdf", "dv-doc.pdf"),
    video: get("video", "dv-video.mp4"),
    note: get("note", "dv-note.txt"),
  };
}

async function runOneCase(args: {
  caseId: string;
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  bucketRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportRoot: string;
  inputs: DvInputs;
  zoneHost: string;
}): Promise<CaseResult> {
  const { caseId, workflowRpc, taskMgrRpc, bucketRpc, owner, runTag, reportRoot, inputs, zoneHost } = args;
  const reportDir = joinPath(reportRoot, safeFileSegment(caseId));
  await Deno.mkdir(reportDir, { recursive: true });
  try {
    if (caseId === "WF-DV-001") {
      await runImageReviewCase({
        workflowRpc,
        owner,
        runTag,
        decision: "approved",
        reportDir,
      });
    } else if (caseId === "WF-DV-002") {
      await runImageReviewCase({
        workflowRpc,
        owner,
        runTag,
        decision: "rejected",
        reportDir,
      });
    } else if (caseId === "WF-DV-003") {
      await runHumanActionCase({ workflowRpc, owner, runTag, reportDir, zoneHost });
    } else if (caseId === "WF-DV-006") {
      await runAgentCallbackCase({
        workflowRpc,
        owner,
        runTag,
        reportDir,
      });
    } else if (caseId === "WF-DV-007") {
      await runCacheCase({ workflowRpc, owner, runTag, reportDir });
    } else if (caseId === "WF-DV-005") {
      await runSemanticCase({ workflowRpc, owner, runTag, reportDir });
    } else if (caseId === "WF-DV-008") {
      await runFailureHumanRetryCase({
        workflowRpc,
        owner,
        runTag,
        reportDir,
      });
    } else if (caseId === "WF-DV-004") {
      throw new BlockedError(
        "WF-DV-004 is blocked: appservice adapter is not registered.",
      );
    } else if (caseId === "WF-DV-009") {
      await runStepCacheCase({
        workflowRpc,
        owner,
        runTag,
        reportDir,
        image: await inputs.image(),
      });
    } else if (caseId === "WF-DV-010") {
      await runEnsureCase({ workflowRpc, owner, runTag, reportDir });
    } else if (caseId === "WF-DV-011") {
      await runMissingToolsCase({
        workflowRpc,
        owner,
        reportDir,
        pdf: await inputs.pdf(),
      });
    } else if (caseId === "WF-DV-012") {
      await runObservabilityCase({
        workflowRpc,
        taskMgrRpc,
        owner,
        reportDir,
        image: await inputs.image(),
      });
    } else if (caseId === "WF-DV-016") {
      await runDeliveredHoldingsCase({
        workflowRpc,
        owner,
        runTag,
        reportDir,
        pdf: await inputs.pdf(),
        zoneHost,
      });
    } else if (caseId === "WF-DV-015") {
      await runDocumentCacheCase({ workflowRpc, reportDir, note: await inputs.note() });
    } else if (caseId === "WF-DV-014") {
      await runMediaVideoNdnCase({
        workflowRpc,
        bucketRpc,
        owner,
        runTag,
        reportDir,
        video: await inputs.video(),
      });
    } else if (caseId === "WF-DV-013") {
      await runReadOnlyCase({
        workflowRpc,
        owner,
        runTag,
        reportDir,
        image: await inputs.image(),
        zoneHost,
      });
    } else {
      throw new Error(`unknown workflow DV case ${caseId}`);
    }
    return { status: "passed", caseId, reportDir };
  } catch (err) {
    if (err instanceof BlockedError) {
      await writeJsonFile(joinPath(reportDir, "blocked.json"), {
        status: "blocked",
        reason: err.message,
      });
      return { status: "blocked", caseId, reason: err.message, reportDir };
    }
    const message = err instanceof Error
      ? `${err.name}: ${err.message}`
      : String(err);
    await writeJsonFile(joinPath(reportDir, "failed.json"), {
      status: "failed",
      error: message,
      stack: err instanceof Error ? err.stack : undefined,
    });
    return { status: "failed", caseId, error: message, reportDir };
  }
}

async function main(): Promise<void> {
  const runTag = `workflow-dv-${Date.now().toString(36)}-${randomHex(3)}`;
  const reportRoot = joinPath(WORKFLOW_REPORT_DIR, runTag);
  await Deno.mkdir(reportRoot, { recursive: true });

  const { buckyos, userId, ownerUserId, zoneHost } = await initTestRuntime();
  const appId = buckyos.getAppId?.() ?? getEnv("BUCKYOS_TEST_APP_ID") ??
    "buckycli";
  const owner: Owner = {
    user_id: ownerUserId || userId || "devtest",
    app_id: appId,
  };
  const workflowRpc = buckyos.getServiceRpcClient("workflow") as RpcClient;
  const taskMgrRpc = buckyos.getServiceRpcClient("task-manager") as RpcClient;
  const bucketRpc = buckyos.getServiceRpcClient("nfs-server") as RpcClient;
  aiccRpc = buckyos.getServiceRpcClient("aicc") as RpcClient;
  const sessionToken = (await buckyos.getAccountInfo())?.session_token ?? "";
  const inputs = lazyInputs(zoneHost, sessionToken, runTag);
  const cases = selectedCaseIds();

  console.log("=== Workflow DV Smoke Test ===");
  console.log(`Zone: ${zoneHost}`);
  console.log(`Owner: ${owner.user_id}/${owner.app_id}`);
  console.log(`Run Tag: ${runTag}`);
  console.log(`Cases: ${cases.join(",")}`);
  console.log(`Image URL: ${WORKFLOW_TEST_IMAGE_URL}`);
  console.log(`Report Dir: ${reportRoot}`);

  const results: CaseResult[] = [];
  for (const caseId of cases) {
    const result = await runOneCase({
      caseId,
      workflowRpc,
      taskMgrRpc,
      bucketRpc,
      owner,
      runTag,
      reportRoot,
      inputs,
      zoneHost,
    });
    results.push(result);
    if (result.status === "passed") {
      console.log(`[PASS] ${caseId} report=${result.reportDir}`);
    } else if (result.status === "blocked") {
      console.log(
        `[BLOCKED] ${caseId} report=${result.reportDir}: ${result.reason}`,
      );
    } else {
      console.log(
        `[FAIL] ${caseId} report=${result.reportDir}: ${result.error}`,
      );
    }
  }

  const summary = {
    run_tag: runTag,
    zone: zoneHost,
    owner,
    cases,
    passed: results.filter((item) => item.status === "passed").length,
    blocked: results.filter((item) => item.status === "blocked").length,
    failed: results.filter((item) => item.status === "failed").length,
    results,
  };
  await writeJsonFile(joinPath(reportRoot, "summary.json"), summary);

  console.log(
    `Summary: passed=${summary.passed} blocked=${summary.blocked} failed=${summary.failed}`,
  );
  if (summary.failed > 0) {
    Deno.exit(1);
  }
  if (summary.passed === 0 && summary.blocked > 0) {
    Deno.exit(2);
  }
}

main().catch((err) => {
  console.error("Workflow DV smoke failed:", err);
  Deno.exit(1);
});
