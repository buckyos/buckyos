/**
 * test_fal.ts — fal.ai provider 远程冒烟测试
 *
 * 覆盖 fal provider 提供的 3 个 ai method:
 *   - image.upscale
 *   - image.bg_remove
 *   - video.upscale
 *
 * 运行前提：远端 AICC 已配置 settings.fal（至少 enabled + api_token）。
 * 若返回 `no_provider_available` 之类的路由错误，本测试会标记为 SKIPPED 而非失败，
 * 方便在未启用 fal 的环境下也能跑通整套用例。
 *
 * 环境变量:
 *   FAL_TEST_IMAGE_URL    — 用于 image.upscale / image.bg_remove 的输入图片 URL
 *   FAL_TEST_VIDEO_URL    — 用于 video.upscale 的输入视频 URL
 *   FAL_WAIT_TIMEOUT_MS   — 单个用例最长等待时长，默认 240000 ms
 */

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

type AiccMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: AiResponseSummary | null;
  event_ref?: string | null;
};

type AiArtifact = {
  name: string;
  resource: { kind: string; url?: string; mime_hint?: string };
  mime?: string | null;
  metadata?: JsonValue | null;
};

type AiResponseSummary = {
  text?: string | null;
  artifacts?: AiArtifact[];
  finish_reason?: string | null;
  provider_task_ref?: string | null;
  extra?: JsonValue | null;
};

type TaskRecord = {
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
};

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type CaseResult =
  | { status: "passed"; method: string; summary: AiResponseSummary }
  | { status: "skipped"; method: string; reason: string }
  | { status: "failed"; method: string; error: string };

// 默认测试输入选用稳定的公网公共资源（HEAD 200、体积合适）：
//   - 图片: gstatic webp gallery 1.jpg（约 44KB,JPEG）
//   - 视频: test-videos.co.uk Big Buck Bunny 360p 10s 样片（约 1MB,MP4）
// 也可通过 FAL_TEST_IMAGE_URL / FAL_TEST_VIDEO_URL 覆盖。
const FAL_TEST_IMAGE_URL = getEnv("FAL_TEST_IMAGE_URL") ??
  "https://www.gstatic.com/webp/gallery/1.jpg";
const FAL_TEST_VIDEO_URL = getEnv("FAL_TEST_VIDEO_URL") ??
  "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/360/Big_Buck_Bunny_360_10s_1MB.mp4";
const FAL_WAIT_TIMEOUT_MS = Number(
  getEnv("FAL_WAIT_TIMEOUT_MS") ?? "240000",
);

function getEnv(name: string): string | null {
  const value = Deno.env.get(name);
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function randomHex(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function normalizeTaskList(result: unknown): TaskRecord[] {
  if (Array.isArray(result)) return result as TaskRecord[];
  if (
    result && typeof result === "object" &&
    Array.isArray((result as { tasks?: unknown }).tasks)
  ) {
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

function isSkippableError(message: string): boolean {
  const lowered = message.toLowerCase();
  return (
    lowered.includes("no_provider_available") ||
    lowered.includes("model_alias_not_mapped") ||
    lowered.includes("provider_unavailable") ||
    lowered.includes("fal provider api_token is empty")
  );
}

async function findTaskByExternalId(
  taskManagerRpc: RpcClient,
  externalTaskId: string,
  deadlineMs: number,
  appId: string,
  userId: string,
): Promise<TaskRecord | null> {
  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call("list_tasks", {
      app_id: appId,
      task_type: "aicc.compute",
      source_user_id: userId,
      source_app_id: appId,
    });
    const tasks = normalizeTaskList(result).sort(
      (a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0),
    );
    const matched = tasks.find(
      (task) => task.data?.aicc?.external_task_id === externalTaskId,
    );
    if (matched) return matched;
    await sleep(1000);
  }
  return null;
}

async function waitForFinalTaskResult(
  taskManagerRpc: RpcClient,
  externalTaskId: string,
  appId: string,
  userId: string,
): Promise<TaskRecord> {
  const deadlineMs = Date.now() + FAL_WAIT_TIMEOUT_MS;
  const task = await findTaskByExternalId(
    taskManagerRpc,
    externalTaskId,
    deadlineMs,
    appId,
    userId,
  );
  if (!task) {
    throw new Error(
      `Timed out while locating AICC task for external_task_id=${externalTaskId}`,
    );
  }
  while (Date.now() < deadlineMs) {
    const result = await taskManagerRpc.call("get_task", { id: task.id });
    const latest = normalizeTask(result);
    if (["Completed", "Failed", "Canceled"].includes(latest.status)) {
      return latest;
    }
    await sleep(1500);
  }
  throw new Error(`Timed out while waiting for AICC task ${task.id} to finish`);
}

function pickSummaryFromTask(task: TaskRecord): AiResponseSummary | null {
  const output = task.data?.aicc?.output;
  if (output && typeof output === "object" && !Array.isArray(output)) {
    return output as unknown as AiResponseSummary;
  }
  return null;
}

function pickErrorFromTask(task: TaskRecord): string {
  const error = task.data?.aicc?.error ?? task.message ?? null;
  return typeof error === "string" ? error : JSON.stringify(error ?? "<empty>");
}

function buildPayload(args: {
  method: "image.upscale" | "image.bg_remove" | "video.upscale";
  runId: string;
  resourceUrl: string;
  mimeHint: string;
  inputJson?: Record<string, unknown>;
  options?: Record<string, unknown>;
}): Record<string, unknown> {
  const capability = args.method.startsWith("video.") ? "video" : "image";
  const alias = args.method;
  const inputJson = { ...(args.inputJson ?? {}) };
  return {
    capability,
    model: { alias },
    requirements: {},
    payload: {
      input_json: inputJson,
      resources: [
        {
          kind: "url",
          url: args.resourceUrl,
          mime_hint: args.mimeHint,
        },
      ],
      options: {
        session_id: args.runId,
        rootid: args.runId,
        ...(args.options ?? {}),
      },
    },
    idempotency_key: args.runId,
  };
}

function assertArtifacts(
  method: string,
  summary: AiResponseSummary | null,
): AiResponseSummary {
  if (!summary) {
    throw new Error(`${method}: no summary returned`);
  }
  const artifacts = summary.artifacts ?? [];
  if (artifacts.length === 0) {
    throw new Error(
      `${method}: response contained no artifacts; summary=${
        JSON.stringify(summary)
      }`,
    );
  }
  const first = artifacts[0];
  const url = first.resource?.url;
  if (!url || typeof url !== "string" || !url.startsWith("http")) {
    throw new Error(
      `${method}: artifact missing http url; got=${JSON.stringify(first)}`,
    );
  }
  if (method === "video.upscale") {
    const mime = first.mime ?? first.resource?.mime_hint ?? "";
    if (!mime.toLowerCase().includes("video")) {
      console.warn(
        `[warn] video.upscale artifact mime is not video/*: mime=${mime}`,
      );
    }
  } else {
    const mime = first.mime ?? first.resource?.mime_hint ?? "";
    if (!mime.toLowerCase().includes("image")) {
      console.warn(
        `[warn] ${method} artifact mime is not image/*: mime=${mime}`,
      );
    }
  }
  return summary;
}

async function runCase(args: {
  method: "image.upscale" | "image.bg_remove" | "video.upscale";
  aiccRpc: RpcClient;
  taskManagerRpc: RpcClient;
  appId: string;
  userId: string;
  payload: Record<string, unknown>;
}): Promise<CaseResult> {
  const { method, aiccRpc, taskManagerRpc, appId, userId, payload } = args;
  let response: AiccMethodResponse;
  try {
    response = await aiccRpc.call(method, payload) as AiccMethodResponse;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (isSkippableError(message)) {
      return { status: "skipped", method, reason: message };
    }
    return { status: "failed", method, error: message };
  }

  if (!response?.task_id || !response?.status) {
    return {
      status: "failed",
      method,
      error: `invalid response: ${JSON.stringify(response)}`,
    };
  }

  if (response.status === "failed") {
    const summary = response.result ?? null;
    const errorText = JSON.stringify(summary ?? response);
    if (isSkippableError(errorText)) {
      return { status: "skipped", method, reason: errorText };
    }
    return { status: "failed", method, error: errorText };
  }

  let summary: AiResponseSummary | null = response.result ?? null;

  if (response.status === "running" && !summary) {
    let finalTask: TaskRecord;
    try {
      finalTask = await waitForFinalTaskResult(
        taskManagerRpc,
        response.task_id,
        appId,
        userId,
      );
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return { status: "failed", method, error: msg };
    }
    if (finalTask.status !== "Completed") {
      const errText = pickErrorFromTask(finalTask);
      if (isSkippableError(errText)) {
        return { status: "skipped", method, reason: errText };
      }
      return {
        status: "failed",
        method,
        error: `task ${finalTask.id} ended with ${finalTask.status}: ${errText}`,
      };
    }
    summary = pickSummaryFromTask(finalTask);
  }

  try {
    const verified = assertArtifacts(method, summary);
    return { status: "passed", method, summary: verified };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { status: "failed", method, error: message };
  }
}

async function main(): Promise<void> {
  const runId = `aicc-fal-${Date.now().toString(36)}-${randomHex(3)}`;
  const { buckyos, userId, zoneHost } = await initTestRuntime();
  const appId = buckyos.getAppId?.() ?? getEnv("BUCKYOS_TEST_APP_ID") ??
    "buckycli";
  const aiccRpc = buckyos.getServiceRpcClient("aicc") as RpcClient;
  const taskManagerRpc = buckyos.getServiceRpcClient(
    "task-manager",
  ) as RpcClient;

  console.log("=== AICC fal Provider Test ===");
  console.log(`Zone: ${zoneHost}`);
  console.log(`App ID: ${appId}`);
  console.log(`User ID: ${userId}`);
  console.log(`Run ID: ${runId}`);
  console.log(`Image URL: ${FAL_TEST_IMAGE_URL}`);
  console.log(`Video URL: ${FAL_TEST_VIDEO_URL}`);

  const cases: Array<Promise<CaseResult>> = [];

  cases.push(runCase({
    method: "image.upscale",
    aiccRpc,
    taskManagerRpc,
    appId,
    userId,
    payload: buildPayload({
      method: "image.upscale",
      runId: `${runId}-upscale`,
      resourceUrl: FAL_TEST_IMAGE_URL,
      mimeHint: "image/jpeg",
      inputJson: { scale: 2 },
    }),
  }));

  cases.push(runCase({
    method: "image.bg_remove",
    aiccRpc,
    taskManagerRpc,
    appId,
    userId,
    payload: buildPayload({
      method: "image.bg_remove",
      runId: `${runId}-bgremove`,
      resourceUrl: FAL_TEST_IMAGE_URL,
      mimeHint: "image/jpeg",
    }),
  }));

  cases.push(runCase({
    method: "video.upscale",
    aiccRpc,
    taskManagerRpc,
    appId,
    userId,
    payload: buildPayload({
      method: "video.upscale",
      runId: `${runId}-vupscale`,
      resourceUrl: FAL_TEST_VIDEO_URL,
      mimeHint: "video/mp4",
    }),
  }));

  const results = await Promise.all(cases);

  let passed = 0;
  let skipped = 0;
  let failed = 0;
  for (const result of results) {
    if (result.status === "passed") {
      passed += 1;
      const url = result.summary.artifacts?.[0]?.resource?.url ?? "<no url>";
      console.log(`[PASS] ${result.method} -> ${url}`);
    } else if (result.status === "skipped") {
      skipped += 1;
      console.log(`[SKIP] ${result.method}: ${result.reason}`);
    } else {
      failed += 1;
      console.log(`[FAIL] ${result.method}: ${result.error}`);
    }
  }

  console.log("\n--- summary ---");
  console.log(`passed=${passed} skipped=${skipped} failed=${failed}`);

  buckyos.logout(false);

  if (failed > 0) {
    Deno.exit(1);
  }
  if (passed === 0 && skipped > 0) {
    // Provider 未配置时，不视为失败但用退出码 2 提示。
    Deno.exit(2);
  }
}

main().catch((error) => {
  console.error("AICC fal provider test failed");
  console.error(error);
  Deno.exit(1);
});
