import { buckyos } from "buckyos";

import type { AiccRuntime } from "./runtime.ts";
import type { AiccInferenceResponse, JsonValue, Profile } from "./types.ts";

type AiccClient = ReturnType<typeof buckyos.getAiccClient>;
type TaskManagerClient = ReturnType<typeof buckyos.getTaskManagerClient>;
type RoutedRequest<T> = Omit<
  T,
  "exact_model" | "trace_id" | "idempotency_key" | "task_options"
>;

export type TypedRequestMap = {
  "image.img2img": RoutedRequest<Parameters<AiccClient["imageToImage"]>[0]>;
  "image.inpaint": RoutedRequest<Parameters<AiccClient["imageInpaint"]>[0]>;
  "image.upscale": RoutedRequest<Parameters<AiccClient["imageUpscale"]>[0]>;
  "image.bg_remove": RoutedRequest<
    Parameters<AiccClient["imageBackgroundRemove"]>[0]
  >;
  "vision.ocr": RoutedRequest<Parameters<AiccClient["visionOcr"]>[0]>;
  "vision.caption": RoutedRequest<Parameters<AiccClient["visionCaption"]>[0]>;
  "vision.detect": RoutedRequest<Parameters<AiccClient["visionDetect"]>[0]>;
  "vision.segment": RoutedRequest<Parameters<AiccClient["visionSegment"]>[0]>;
  "audio.tts": RoutedRequest<Parameters<AiccClient["audioTextToSpeech"]>[0]>;
  "audio.asr": RoutedRequest<
    Parameters<AiccClient["audioSpeechRecognition"]>[0]
  >;
  "audio.music": RoutedRequest<Parameters<AiccClient["audioMusic"]>[0]>;
  "audio.enhance": RoutedRequest<Parameters<AiccClient["audioEnhance"]>[0]>;
  "video.txt2video": RoutedRequest<
    Parameters<AiccClient["videoTextToVideo"]>[0]
  >;
  "video.img2video": RoutedRequest<
    Parameters<AiccClient["videoImageToVideo"]>[0]
  >;
  "video.video2video": RoutedRequest<Parameters<AiccClient["videoToVideo"]>[0]>;
  "video.extend": RoutedRequest<Parameters<AiccClient["videoExtend"]>[0]>;
  "video.upscale": RoutedRequest<Parameters<AiccClient["videoUpscale"]>[0]>;
};

export type TypedMethod = keyof TypedRequestMap;

const API_TYPE_BY_METHOD: {
  [M in TypedMethod]: Parameters<AiccClient["routeResolve"]>[0]["api_type"];
} = {
  "image.img2img": "image.img2img",
  "image.inpaint": "image.inpaint",
  "image.upscale": "image.upscale",
  "image.bg_remove": "image.bg_remove",
  "vision.ocr": "vision.ocr",
  "vision.caption": "vision.caption",
  "vision.detect": "vision.detect",
  "vision.segment": "vision.segment",
  "audio.tts": "audio.tts",
  "audio.asr": "audio.asr",
  "audio.music": "audio.music",
  "audio.enhance": "audio.enhance",
  "video.txt2video": "video.txt2video",
  "video.img2video": "video.img2video",
  "video.video2video": "video.video2video",
  "video.extend": "video.extend",
  "video.upscale": "video.upscale",
};

export interface RouteOptions {
  requirements?: Parameters<AiccClient["routeResolve"]>[0]["requirements"];
  disable?: Parameters<AiccClient["routeResolve"]>[0]["disable"];
  profile?: Profile;
  allowFallback?: boolean;
  runtimeFailover?: boolean;
  localOnly?: boolean;
  allowedProviderInstances?: string[];
  blockedProviderInstances?: string[];
  maxCostUsd?: number;
  maxLatencyMs?: number;
  idempotencyKey?: string;
  traceId?: string;
  waitTimeoutMs?: number;
}

export interface TypedCallOptions<M extends TypedMethod> extends RouteOptions {
  method: M;
  model: string;
  request: TypedRequestMap[M];
}

export interface CallResult {
  taskId: string;
  status: "succeeded" | "failed";
  summary: AiccInferenceResponse | null;
  rawResponse: AiccInferenceResponse;
  finalTask?: TaskRecord;
}

interface TaskRecord {
  task_id: string;
  phase: string;
  outcome?: string | null;
  message?: string | null;
  result?: unknown;
  error?: unknown;
}

const AGENT_TOOL_PROGRESS_PREFIX = "__BUCKYOS_AGENT_PROGRESS__";

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function emitAiccProgress(
  method: string,
  stage: "running" | "finalizing",
  taskId: string,
  elapsedMs: number,
): void {
  console.error(`${AGENT_TOOL_PROGRESS_PREFIX}${
    JSON.stringify({
      agent_tool_progress: "1",
      kind: "aicc",
      method,
      stage,
      task_id: taskId,
      elapsed_ms: elapsedMs,
    })
  }`);
}

function envNumber(name: string, fallback: number): number {
  const raw = Deno.env.get(name);
  if (!raw) return fallback;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function defaultProfile(): Profile {
  const raw = (Deno.env.get("AICC_DEFAULT_PROFILE") ?? "").toLowerCase();
  return raw === "cheap" || raw === "fast" || raw === "quality" ||
      raw === "balanced"
    ? raw
    : "balanced";
}

function buildPolicy(
  opts: RouteOptions,
): NonNullable<Parameters<AiccClient["routeResolve"]>[0]["policy"]> {
  return {
    profile: opts.profile ?? defaultProfile(),
    allow_fallback: opts.allowFallback ?? true,
    runtime_failover: opts.runtimeFailover ?? true,
    ...(opts.localOnly ? { local_only: true } : {}),
    ...(opts.allowedProviderInstances?.length
      ? { allowed_provider_instances: opts.allowedProviderInstances }
      : {}),
    ...(opts.blockedProviderInstances?.length
      ? { blocked_provider_instances: opts.blockedProviderInstances }
      : {}),
    ...(typeof opts.maxCostUsd === "number"
      ? { max_cost: { amount: opts.maxCostUsd, currency: "USD" } }
      : {}),
    ...(typeof opts.maxLatencyMs === "number"
      ? { max_latency_ms: opts.maxLatencyMs }
      : {}),
  };
}

async function resolveExactModel<M extends TypedMethod>(
  client: AiccClient,
  opts: TypedCallOptions<M>,
): Promise<string> {
  if (opts.model.includes("@")) return opts.model;
  const resolved = await client.routeResolve({
    ...(opts.traceId ? { trace_id: opts.traceId } : {}),
    api_type: API_TYPE_BY_METHOD[opts.method],
    logical_model: opts.model,
    requirements: opts.requirements ?? {},
    disable: opts.disable ?? {},
    policy: buildPolicy(opts),
  });
  return resolved.selected_exact_model;
}

async function invokeTyped(
  client: AiccClient,
  method: TypedMethod,
  request: Record<string, unknown>,
): Promise<AiccInferenceResponse> {
  switch (method) {
    case "image.img2img":
      return await client.imageToImage(
        request as Parameters<AiccClient["imageToImage"]>[0],
      );
    case "image.inpaint":
      return await client.imageInpaint(
        request as Parameters<AiccClient["imageInpaint"]>[0],
      );
    case "image.upscale":
      return await client.imageUpscale(
        request as Parameters<AiccClient["imageUpscale"]>[0],
      );
    case "image.bg_remove":
      return await client.imageBackgroundRemove(
        request as Parameters<AiccClient["imageBackgroundRemove"]>[0],
      );
    case "vision.ocr":
      return await client.visionOcr(
        request as Parameters<AiccClient["visionOcr"]>[0],
      );
    case "vision.caption":
      return await client.visionCaption(
        request as Parameters<AiccClient["visionCaption"]>[0],
      );
    case "vision.detect":
      return await client.visionDetect(
        request as Parameters<AiccClient["visionDetect"]>[0],
      );
    case "vision.segment":
      return await client.visionSegment(
        request as Parameters<AiccClient["visionSegment"]>[0],
      );
    case "audio.tts":
      return await client.audioTextToSpeech(
        request as Parameters<AiccClient["audioTextToSpeech"]>[0],
      );
    case "audio.asr":
      return await client.audioSpeechRecognition(
        request as Parameters<AiccClient["audioSpeechRecognition"]>[0],
      );
    case "audio.music":
      return await client.audioMusic(
        request as Parameters<AiccClient["audioMusic"]>[0],
      );
    case "audio.enhance":
      return await client.audioEnhance(
        request as Parameters<AiccClient["audioEnhance"]>[0],
      );
    case "video.txt2video":
      return await client.videoTextToVideo(
        request as Parameters<AiccClient["videoTextToVideo"]>[0],
      );
    case "video.img2video":
      return await client.videoImageToVideo(
        request as Parameters<AiccClient["videoImageToVideo"]>[0],
      );
    case "video.video2video":
      return await client.videoToVideo(
        request as Parameters<AiccClient["videoToVideo"]>[0],
      );
    case "video.extend":
      return await client.videoExtend(
        request as Parameters<AiccClient["videoExtend"]>[0],
      );
    case "video.upscale":
      return await client.videoUpscale(
        request as Parameters<AiccClient["videoUpscale"]>[0],
      );
  }
}

function normalizeTask(value: unknown): TaskRecord {
  if (
    value && typeof value === "object" && !Array.isArray(value) &&
    "task" in value
  ) {
    return (value as { task: TaskRecord }).task;
  }
  return value as TaskRecord;
}

async function waitForFinalTask(
  taskMgr: TaskManagerClient,
  taskId: string,
  method: string,
  deadlineMs: number,
): Promise<TaskRecord> {
  const startedAt = Date.now();
  let lastProgressAt = 0;
  while (Date.now() < deadlineMs) {
    const now = Date.now();
    if (now - lastProgressAt >= 5_000) {
      emitAiccProgress(method, "running", taskId, now - startedAt);
      lastProgressAt = now;
    }
    const next = normalizeTask(await taskMgr.getTask(taskId));
    if (next.phase === "Terminal") {
      emitAiccProgress(method, "finalizing", taskId, Date.now() - startedAt);
      return next;
    }
    await sleep(1_000);
  }
  throw new Error(`timed out while waiting for AICC task ${taskId} to finish`);
}

function taskOutput(task: TaskRecord): AiccInferenceResponse | null {
  if (
    !task.result || typeof task.result !== "object" ||
    Array.isArray(task.result)
  ) return null;
  const data = task.result as Record<string, unknown>;
  const nested = data.result;
  if (nested && typeof nested === "object" && !Array.isArray(nested)) {
    const output = (nested as Record<string, unknown>).output;
    if (output && typeof output === "object" && !Array.isArray(output)) {
      return output as AiccInferenceResponse;
    }
  }
  const output = data.output;
  return output && typeof output === "object" && !Array.isArray(output)
    ? output as AiccInferenceResponse
    : null;
}

async function finishCall(
  taskMgr: TaskManagerClient,
  method: string,
  response: AiccInferenceResponse,
  waitTimeoutMs: number,
): Promise<CallResult> {
  if (response.status === "succeeded" || response.status === "failed") {
    return {
      taskId: response.task_id,
      status: response.status,
      summary: response,
      rawResponse: response,
    };
  }
  const finalTask = await waitForFinalTask(
    taskMgr,
    response.task_id,
    method,
    Date.now() + waitTimeoutMs,
  );
  return {
    taskId: response.task_id,
    status: finalTask.outcome === "Succeeded" ? "succeeded" : "failed",
    summary: taskOutput(finalTask),
    rawResponse: response,
    finalTask,
  };
}

export async function callAicc<M extends TypedMethod>(
  runtime: AiccRuntime,
  opts: TypedCallOptions<M>,
): Promise<CallResult> {
  const client = runtime.buckyos.getAiccClient();
  const exactModel = await resolveExactModel(client, opts);
  const request = {
    ...opts.request,
    exact_model: exactModel,
    ...(opts.traceId ? { trace_id: opts.traceId } : {}),
    ...(opts.idempotencyKey ? { idempotency_key: opts.idempotencyKey } : {}),
  };
  const response = await invokeTyped(client, opts.method, request);
  return await finishCall(
    runtime.buckyos.getTaskManagerClient(),
    opts.method,
    response,
    opts.waitTimeoutMs ?? envNumber("AICC_DEFAULT_TIMEOUT", 900_000),
  );
}

export interface TextToImageOptions extends RouteOptions {
  model?: string;
  request: Omit<
    Parameters<AiccClient["helperTextToImage"]>[0],
    | "logical_model"
    | "requirements"
    | "disable"
    | "trace_id"
    | "policy"
    | "idempotency_key"
    | "task_options"
    | "session_overlay"
  >;
}

export async function textToImage(
  runtime: AiccRuntime,
  opts: TextToImageOptions,
): Promise<CallResult> {
  const response = await runtime.buckyos.getAiccClient().helperTextToImage({
    ...opts.request,
    logical_model: opts.model ?? "image.txt2img",
    requirements: opts.requirements ?? {},
    disable: opts.disable ?? {},
    ...(opts.traceId ? { trace_id: opts.traceId } : {}),
    policy: buildPolicy(opts),
    ...(opts.idempotencyKey ? { idempotency_key: opts.idempotencyKey } : {}),
  });
  return await finishCall(
    runtime.buckyos.getTaskManagerClient(),
    "helper.text_to_image",
    response,
    opts.waitTimeoutMs ?? envNumber("AICC_DEFAULT_TIMEOUT", 900_000),
  );
}

export function describeFailure(result: CallResult): string {
  if (result.finalTask) {
    const taskResult = result.finalTask.result;
    const nestedError =
      taskResult && typeof taskResult === "object" && !Array.isArray(taskResult)
        ? (taskResult as Record<string, unknown>).error
        : undefined;
    const error = result.finalTask.error ?? nestedError ??
      result.finalTask.message;
    if (error) return typeof error === "string" ? error : JSON.stringify(error);
    return `task ended with ${
      result.finalTask.outcome ?? result.finalTask.phase
    }`;
  }
  const error = (result.rawResponse as unknown as { error?: JsonValue }).error;
  return error ? JSON.stringify(error) : "aicc call failed";
}

interface CommonFlagsShape {
  profile?: Profile;
  noFallback: boolean;
  maxCostUsd?: number;
  maxLatencyMs?: number;
  idempotencyKey?: string;
  traceId?: string;
  timeoutMs?: number;
}

export function commonPolicyOptions(common: CommonFlagsShape): RouteOptions {
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
