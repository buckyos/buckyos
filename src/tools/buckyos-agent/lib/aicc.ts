// Thin wrapper over the aicc kRPC client + task-manager polling, copying the
// flow proven out in test/aicc_test/aicc_smoke.ts.

import {
  AiccMethodResponse,
  AiResponse,
  Capability,
  JsonValue,
  Profile,
  ResourceRef,
} from "./types.ts";
import { AiccRuntime } from "./runtime.ts";

// deno-lint-ignore no-explicit-any
type RpcClient = { call: (method: string, params: Record<string, unknown>) => Promise<any> };

export type { Profile };

export interface CallOptions {
  capability: Capability;
  method: string;
  modelAlias?: string;
  inputJson?: Record<string, unknown>;
  resources?: ResourceRef[];
  options?: Record<string, unknown>;
  requirements?: Record<string, unknown>;
  profile?: Profile;
  allowFallback?: boolean;
  runtimeFailover?: boolean;
  maxCostUsd?: number;
  maxLatencyMs?: number;
  idempotencyKey?: string;
  traceId?: string;
  // Wait timeout for the polling phase (when status is "running"), ms.
  waitTimeoutMs?: number;
}

export interface CallResult {
  taskId: string;
  status: "succeeded" | "failed";
  summary: AiResponse | null;
  rawResponse: AiccMethodResponse;
  finalTask?: TaskRecord;
}

interface TaskRecord {
  id: number;
  status: string;
  message?: string | null;
  updated_at?: number;
  data?: {
    aicc?: {
      external_task_id?: string;
      output?: JsonValue;
      error?: JsonValue;
      status?: string;
    };
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function envNumber(name: string, fallback: number): number {
  const raw = Deno.env.get(name);
  if (!raw) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function defaultProfile(): Profile {
  const raw = (Deno.env.get("AICC_DEFAULT_PROFILE") ?? "").toLowerCase();
  if (raw === "cheap" || raw === "fast" || raw === "quality" || raw === "balanced") return raw;
  return "balanced";
}

function buildPolicy(opts: CallOptions): Record<string, unknown> {
  const policy: Record<string, unknown> = {
    profile: opts.profile ?? defaultProfile(),
    allow_fallback: opts.allowFallback ?? true,
    runtime_failover: opts.runtimeFailover ?? true,
  };
  if (typeof opts.maxCostUsd === "number") policy.max_cost_usd = opts.maxCostUsd;
  if (typeof opts.maxLatencyMs === "number") policy.max_latency_ms = opts.maxLatencyMs;
  return policy;
}

function buildRequest(opts: CallOptions): Record<string, unknown> {
  const modelAlias = opts.modelAlias ?? opts.method;
  const req: Record<string, unknown> = {
    capability: opts.capability,
    model: { alias: modelAlias },
    requirements: opts.requirements ?? {},
    payload: {
      input_json: opts.inputJson ?? {},
      resources: opts.resources ?? [],
      options: opts.options ?? {},
    },
    policy: buildPolicy(opts),
  };
  if (opts.idempotencyKey) req.idempotency_key = opts.idempotencyKey;
  if (opts.traceId) req.trace_id = opts.traceId;
  return req;
}

function normalizeTaskList(result: unknown): TaskRecord[] {
  if (Array.isArray(result)) return result as TaskRecord[];
  if (result && typeof result === "object" && Array.isArray((result as { tasks?: unknown }).tasks)) {
    return (result as { tasks: TaskRecord[] }).tasks;
  }
  return [];
}

function normalizeTask(result: unknown): TaskRecord {
  if (result && typeof result === "object" && "task" in result) {
    return (result as { task: TaskRecord }).task;
  }
  return result as TaskRecord;
}

function asAiResponse(value: unknown): AiResponse | null {
  if (
    value && typeof value === "object" && !Array.isArray(value) &&
    "message" in value
  ) {
    return value as AiResponse;
  }
  return null;
}

async function waitForFinalTask(
  taskMgr: RpcClient,
  externalTaskId: string,
  appId: string,
  userId: string,
  deadlineMs: number,
): Promise<TaskRecord> {
  while (Date.now() < deadlineMs) {
    const raw = await taskMgr.call("list_tasks", {
      app_id: appId,
      task_type: "aicc.compute",
      source_user_id: userId,
      source_app_id: appId,
    });
    const tasks = normalizeTaskList(raw).sort(
      (a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0),
    );
    const matched = tasks.find((t) => t.data?.aicc?.external_task_id === externalTaskId);
    if (matched) {
      while (Date.now() < deadlineMs) {
        const next = normalizeTask(await taskMgr.call("get_task", { id: matched.id }));
        if (["Completed", "Failed", "Canceled"].includes(next.status)) return next;
        await sleep(1000);
      }
      throw new Error(`timed out while waiting for AICC task ${matched.id} to finish`);
    }
    await sleep(1000);
  }
  throw new Error(`timed out while locating AICC task for external_task_id=${externalTaskId}`);
}

export async function callAicc(runtime: AiccRuntime, opts: CallOptions): Promise<CallResult> {
  // deno-lint-ignore no-explicit-any
  const aiccRpc = (runtime.buckyos as any).getServiceRpcClient("aicc") as RpcClient;
  // deno-lint-ignore no-explicit-any
  const taskMgr = (runtime.buckyos as any).getServiceRpcClient("task-manager") as RpcClient;

  const request = buildRequest(opts);
  const waitTimeoutMs = opts.waitTimeoutMs ?? envNumber("AICC_DEFAULT_TIMEOUT", 180_000);

  let response: AiccMethodResponse;
  try {
    response = await aiccRpc.call(opts.method, request) as AiccMethodResponse;
  } catch (err) {
    throw err instanceof Error ? err : new Error(String(err));
  }

  if (!response?.task_id || !response?.status) {
    throw new Error(`invalid AICC response: ${JSON.stringify(response)}`);
  }

  if (response.status === "succeeded") {
    return {
      taskId: response.task_id,
      status: "succeeded",
      summary: asAiResponse(response.result ?? null),
      rawResponse: response,
    };
  }
  if (response.status === "failed") {
    return {
      taskId: response.task_id,
      status: "failed",
      summary: asAiResponse(response.result ?? null),
      rawResponse: response,
    };
  }

  // status === "running" — fall back to task-manager polling.
  const deadline = Date.now() + waitTimeoutMs;
  const finalTask = await waitForFinalTask(
    taskMgr,
    response.task_id,
    runtime.appId,
    runtime.userId,
    deadline,
  );
  if (finalTask.status === "Completed") {
    return {
      taskId: response.task_id,
      status: "succeeded",
      summary: asAiResponse(finalTask.data?.aicc?.output ?? null),
      rawResponse: response,
      finalTask,
    };
  }
  return {
    taskId: response.task_id,
    status: "failed",
    summary: asAiResponse(finalTask.data?.aicc?.output ?? null),
    rawResponse: response,
    finalTask,
  };
}

export function describeFailure(result: CallResult): string {
  if (result.finalTask) {
    const err = result.finalTask.data?.aicc?.error ?? result.finalTask.message;
    if (err) return typeof err === "string" ? err : JSON.stringify(err);
    return `task ended with ${result.finalTask.status}`;
  }
  if (result.rawResponse?.result) {
    return JSON.stringify(result.rawResponse.result);
  }
  return "aicc call failed";
}

// Map parsed CLI common flags (§2.2) onto callAicc options. Spread into your
// CallOptions like:
//   await callAicc(runtime, { method, capability, inputJson, ...commonPolicyOptions(parsed.common) })
// Local definition (kept structural to avoid a runtime cycle with cli.ts).
// Matches the public CommonFlags shape exported from lib/cli.ts.
interface CommonFlagsShape {
  profile?: Profile;
  noFallback: boolean;
  maxCostUsd?: number;
  maxLatencyMs?: number;
  idempotencyKey?: string;
  traceId?: string;
  timeoutMs?: number;
  model?: string;
  json: boolean;
}

export function commonPolicyOptions(common: CommonFlagsShape): Partial<CallOptions> {
  return {
    profile: common.profile,
    allowFallback: common.noFallback ? false : undefined,
    runtimeFailover: common.noFallback ? false : undefined,
    maxCostUsd: common.maxCostUsd,
    maxLatencyMs: common.maxLatencyMs,
    idempotencyKey: common.idempotencyKey,
    traceId: common.traceId,
    waitTimeoutMs: common.timeoutMs,
  };
}

// Most artifact-producing AICC methods accept these hints to make the provider
// return a named_object obj_id (preferred) rather than a base64 blob.
export function requestNamedObjectOutput(
  input: Record<string, unknown>,
): Record<string, unknown> {
  const output = (input.output && typeof input.output === "object" && !Array.isArray(input.output))
    ? input.output as Record<string, unknown>
    : {};
  return {
    ...input,
    response_format: input.response_format ?? "object_id",
    output: {
      ...output,
      resource_format: (output as Record<string, unknown>).resource_format ?? "named_object",
    },
  };
}
