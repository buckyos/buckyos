import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

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
  status: string;
  graph: JsonObject;
  nodes: JsonObject;
  node_states: Record<string, string>;
  node_outputs: Record<string, JsonValue>;
  human_waiting_nodes: string[];
  pending_thunks: JsonObject;
  metrics: JsonObject;
  seq: number;
};

type CaseResult =
  | { status: "passed"; caseId: string; reportDir: string }
  | { status: "blocked"; caseId: string; reason: string; reportDir: string }
  | { status: "failed"; caseId: string; error: string; reportDir: string };

type TaskRecord = {
  id: number;
  name: string;
  task_type: string;
  status: string;
  data: JsonValue;
  root_id?: string | null;
  parent_id?: number | null;
};

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

const DEFAULT_CASES = ["WF-DV-001", "WF-DV-002", "WF-DV-006", "WF-DV-007"];
const WORKFLOW_DV_CASES = getEnv("WORKFLOW_DV_CASES");
const WORKFLOW_REPORT_DIR = getEnv("WORKFLOW_REPORT_DIR") ??
  "reports/workflow_dv";
const WORKFLOW_TEST_IMAGE_URL = getEnv("WORKFLOW_TEST_IMAGE_URL") ??
  "https://www.gstatic.com/webp/gallery/1.jpg";
const WORKFLOW_WAIT_TIMEOUT_MS = Number(
  getEnv("WORKFLOW_WAIT_TIMEOUT_MS") ?? "180000",
);
const TASKDATA_CASE_ID = "WF-DV-003";

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

async function loadFixture(name: string): Promise<JsonObject> {
  const url = new URL(`./fixtures/${name}`, import.meta.url);
  return JSON.parse(await Deno.readTextFile(url)) as JsonObject;
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
  owner: Owner,
  definition: JsonObject,
): Promise<
  { workflowId: string; version: number; first: JsonObject; second: JsonObject }
> {
  const dryRun = await rpcOk<JsonObject>(workflowRpc, "dry_run", {
    definition,
  });
  assertNoAnalysisErrors(dryRun, "dry_run");

  const first = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    owner,
    definition,
    tags: ["dv", "workflow"],
  });
  assertNoAnalysisErrors(first, "submit_definition");
  assert.equal(typeof first.workflow_id, "string");

  const second = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    owner,
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
  owner: Owner,
  workflowId: string,
): Promise<{ runId: string; start: JsonObject }> {
  const created = await rpcOk<JsonObject>(workflowRpc, "create_run", {
    workflow_id: workflowId,
    owner,
    input: {
      image_url: WORKFLOW_TEST_IMAGE_URL,
      mime_hint: "image/jpeg",
    },
  });
  assert.equal(created.status, "created");
  assert.equal(typeof created.run_id, "string");

  const runId = String(created.run_id);
  const start = await rpcOk<JsonObject>(workflowRpc, "start_run", {
    run_id: runId,
  });
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
  owner: Owner,
  workflowId: string,
  runId: string,
  status: string,
): Promise<void> {
  const listed = await rpcOk<{ ok: true; runs: JsonObject[] }>(
    workflowRpc,
    "list_runs",
    {
      owner,
      workflow_id: workflowId,
      status,
    },
  );
  assert.ok(
    listed.runs.some((run) => run.run_id === runId),
    `list_runs should include ${runId} with status=${status}`,
  );
}

/**
 * Drive a human_confirm node by writing TaskData via task_manager RPC,
 * mirroring the production flow (TaskMgr UI writes TaskData, workflow service
 * subscribes to the event and calls apply_task_data). The legacy path used
 * workflow.submit_step_output directly; switching here lets the smoke test
 * exercise the task_mgr → workflow event pipe end-to-end.
 */
async function submitHumanActionViaTaskData(args: {
  taskMgrRpc: RpcClient;
  runId: string;
  nodeId: string;
  actor: string;
  payload: JsonValue;
}): Promise<{ taskId: number }> {
  const { taskMgrRpc, runId, nodeId, actor, payload } = args;
  const task = await waitForStepTask(taskMgrRpc, runId, nodeId);
  await rpcCall<unknown>(taskMgrRpc, "update_task", {
    id: task.id,
    data: {
      human_action: {
        kind: "submit_output",
        actor,
        payload,
      },
    },
  });
  return { taskId: task.id };
}

async function waitForStepTask(
  taskMgrRpc: RpcClient,
  runId: string,
  nodeId: string,
): Promise<TaskRecord> {
  // The workflow tracker creates the step task lazily — give it a few seconds
  // after the run lands in waiting_human. list_tasks filters by root_id =
  // run_id (the tracker pins root_id to run.run_id for every step task).
  const deadline = Date.now() + 10_000;
  let lastSnapshot: TaskRecord[] = [];
  while (Date.now() < deadline) {
    const listed = await rpcCall<{ tasks?: TaskRecord[] }>(
      taskMgrRpc,
      "list_tasks",
      { root_id: runId },
    );
    lastSnapshot = listed.tasks ?? [];
    const match = lastSnapshot.find((task) => {
      const data = task.data as JsonObject | null;
      const workflow = data && typeof data === "object" && !Array.isArray(data)
        ? (data.workflow as JsonObject | undefined)
        : undefined;
      return workflow?.node_id === nodeId;
    });
    if (match) return match;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(
    `task_mgr.list_tasks did not surface step task for run=${runId} node=${nodeId}; saw=${
      JSON.stringify(lastSnapshot.map((t) => ({ id: t.id, name: t.name })))
    }`,
  );
}

async function runImageReviewCase(args: {
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  owner: Owner;
  runTag: string;
  decision: "approved" | "rejected";
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, taskMgrRpc, owner, runTag, decision, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-image-review-enhance.json"),
    `${runTag}-${decision}`,
  );
  const submitted = await submitDefinitionPair(workflowRpc, owner, definition);
  await writeJsonFile(joinPath(reportDir, "definition.json"), submitted.second);

  const { runId, start } = await createAndStartRun(
    workflowRpc,
    owner,
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

  const submittedHuman = await submitHumanActionViaTaskData({
    taskMgrRpc,
    runId,
    nodeId: "human_review",
    actor: "devtest",
    payload: humanOutput as unknown as JsonValue,
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
    owner,
    submitted.workflowId,
    runId,
    "completed",
  );
}

async function runAgentCallbackCase(args: {
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, taskMgrRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-agent-callback.json"),
    runTag,
  );
  const submitted = await submitDefinitionPair(workflowRpc, owner, definition);
  const { runId, start } = await createAndStartRun(
    workflowRpc,
    owner,
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
  const submittedOutput = await submitHumanActionViaTaskData({
    taskMgrRpc,
    runId,
    nodeId: "request_manual_tag",
    actor: "agent/dv-callback",
    payload: output,
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
    owner,
    submitted.workflowId,
    runId,
    "completed",
  );

  const defaultActorRun = await createAndStartRun(
    workflowRpc,
    owner,
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
  // This sub-case specifically pins down the workflow RPC's default-actor
  // behavior ("agent" when actor omitted). The apply_task_data path defaults
  // to "human" instead, so the test stays on submit_step_output here.
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
  const submitted = await submitDefinitionPair(workflowRpc, owner, definition);

  const runA = await createAndStartRun(
    workflowRpc,
    owner,
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
    owner,
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
  const submittedChanged = await submitDefinitionPair(
    workflowRpc,
    owner,
    changed,
  );
  const runC = await createAndStartRun(
    workflowRpc,
    owner,
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
  const graphText = JSON.stringify(dryRun.graph);
  assert.ok(
    graphText.includes("SemanticPath") ||
      graphText.includes("/skill/image-quality-review"),
  );

  const submitted = await rpcOk<JsonObject>(workflowRpc, "submit_definition", {
    owner,
    definition,
    tags: ["dv", "workflow", "semantic"],
  });
  const created = await rpcOk<JsonObject>(workflowRpc, "create_run", {
    workflow_id: submitted.workflow_id,
    owner,
  });

  const raw = await workflowRpc.call("start_run", { run_id: created.run_id });
  await writeJsonFile(joinPath(reportDir, "start_raw.json"), raw);
  const text = JSON.stringify(raw);
  if (isAiccUnavailableText(text)) {
    throw new BlockedError(
      `AICC unavailable before semantic executor check: ${text}`,
    );
  }
  assert.ok(
    text.includes("image-quality-review") ||
      text.includes("unresolved") ||
      text.includes("semantic") ||
      text.includes("require_function_object"),
    "semantic executor should be preserved and fail explicitly at execution time",
  );
}

async function runFailureHumanRetryCase(args: {
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportDir: string;
}): Promise<void> {
  const { workflowRpc, taskMgrRpc, owner, runTag, reportDir } = args;
  const definition = prepareDefinition(
    await loadFixture("wf-dv-failure-human-retry.json"),
    runTag,
  );
  const submitted = await submitDefinitionPair(workflowRpc, owner, definition);
  const { runId } = await createAndStartRun(
    workflowRpc,
    owner,
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

  await submitHumanActionViaTaskData({
    taskMgrRpc,
    runId,
    nodeId: "caption_bad",
    actor: "devtest",
    payload: { text: "manually-supplied caption text" },
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

async function runTaskDataCase(): Promise<void> {
  throw new BlockedError(
    "WF-DV-003 is blocked in the current service: apply_task_data exists in orchestrator, but workflow service has no exposed RPC/listener path for TaskData updates.",
  );
}

function selectedCaseIds(): string[] {
  if (!WORKFLOW_DV_CASES) return DEFAULT_CASES;
  return WORKFLOW_DV_CASES.split(",").map((item) => item.trim()).filter(
    Boolean,
  );
}

async function runOneCase(args: {
  caseId: string;
  workflowRpc: RpcClient;
  taskMgrRpc: RpcClient;
  owner: Owner;
  runTag: string;
  reportRoot: string;
}): Promise<CaseResult> {
  const { caseId, workflowRpc, taskMgrRpc, owner, runTag, reportRoot } = args;
  const reportDir = joinPath(reportRoot, safeFileSegment(caseId));
  await Deno.mkdir(reportDir, { recursive: true });
  try {
    if (caseId === "WF-DV-001") {
      await runImageReviewCase({
        workflowRpc,
        taskMgrRpc,
        owner,
        runTag,
        decision: "approved",
        reportDir,
      });
    } else if (caseId === "WF-DV-002") {
      await runImageReviewCase({
        workflowRpc,
        taskMgrRpc,
        owner,
        runTag,
        decision: "rejected",
        reportDir,
      });
    } else if (caseId === TASKDATA_CASE_ID) {
      await runTaskDataCase();
    } else if (caseId === "WF-DV-006") {
      await runAgentCallbackCase({
        workflowRpc,
        taskMgrRpc,
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
        taskMgrRpc,
        owner,
        runTag,
        reportDir,
      });
    } else if (caseId === "WF-DV-004") {
      throw new BlockedError(
        "WF-DV-004 is blocked: appservice adapter is not registered.",
      );
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
      owner,
      runTag,
      reportRoot,
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
