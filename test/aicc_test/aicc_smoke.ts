import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

type AiccMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: unknown;
  event_ref?: string | null;
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

const AICC_MODEL_ALIAS = getEnv("AICC_MODEL_ALIAS") ??
  "llm.gpt5";
const AICC_TEST_INPUT = getEnv("AICC_TEST_INPUT") ??
  "今天天气如何，我在sanjose";
const AICC_WAIT_TIMEOUT_MS = Number(
  getEnv("AICC_WAIT_TIMEOUT_MS") ?? "90000",
);
const AICC_LLM_CHAT_METHOD = "llm.chat";

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

function getEnv(name: string): string | null {
  const value = Deno.env.get(name);
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function normalizeTaskList(result: unknown): TaskRecord[] {
  if (Array.isArray(result)) {
    return result as TaskRecord[];
  }
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

function renderSummary(summary: unknown): string {
  if (!summary || typeof summary !== "object") {
    return "<empty>";
  }

  const text = (summary as { text?: unknown }).text;
  if (typeof text === "string" && text.trim()) {
    return text.trim();
  }

  return JSON.stringify(summary, null, 2);
}

function extractTaskSummary(task: TaskRecord): unknown {
  return task.data?.aicc?.output ?? null;
}

function extractTaskError(task: TaskRecord): unknown {
  return task.data?.aicc?.error ?? task.message ?? null;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
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
      (left, right) => (right.updated_at ?? 0) - (left.updated_at ?? 0),
    );
    const matched = tasks.find(
      (task) => task.data?.aicc?.external_task_id === externalTaskId,
    );
    if (matched) {
      return matched;
    }

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
  const deadlineMs = Date.now() + AICC_WAIT_TIMEOUT_MS;
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
    await sleep(1000);
  }

  throw new Error(`Timed out while waiting for AICC task ${task.id} to finish`);
}

function randomHex(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function buildLlmChatPayload(runId: string): Record<string, unknown> {
  return {
    capability: "llm",
    model: {
      alias: AICC_MODEL_ALIAS,
    },
    requirements: {},
    payload: {
      input_json: {
        messages: [
          {
            role: "user",
            content: AICC_TEST_INPUT,
          },
        ],
        temperature: 0.2,
        max_output_tokens: 2560,
      },
      resources: [],
      options: {
        session_id: runId,
        rootid: runId,
      },
    },
    idempotency_key: runId,
  };
}

async function main(): Promise<void> {
  const runId = `aicc-smoke-${Date.now().toString(36)}-${randomHex(3)}`;

  const { buckyos, userId, ownerUserId, zoneHost } = await initTestRuntime();
  const appId = buckyos.getAppId?.() ?? getEnv("BUCKYOS_TEST_APP_ID") ??
    "buckycli";

  const aiccRpc = buckyos.getServiceRpcClient("aicc") as RpcClient;
  const taskManagerRpc = buckyos.getServiceRpcClient(
    "task-manager",
  ) as RpcClient;

  const response = await aiccRpc.call(
    AICC_LLM_CHAT_METHOD,
    buildLlmChatPayload(runId),
  ) as AiccMethodResponse;

  if (!response?.task_id || !response?.status) {
    throw new Error(
      `invalid AICC response: ${JSON.stringify(response, null, 2)}`,
    );
  }

  let summary: unknown = response.result ?? null;
  let terminalStatus: string = response.status;

  if (response.status === "failed") {
    throw new Error(
      `AICC llm.chat failed: ${JSON.stringify(response, null, 2)}`,
    );
  }

  if (response.status === "running" && !summary) {
    const finalTask = await waitForFinalTaskResult(
      taskManagerRpc,
      response.task_id,
      appId,
      userId,
    );
    terminalStatus = finalTask.status.toLowerCase();

    if (finalTask.status !== "Completed") {
      throw new Error(
        `AICC task ${finalTask.id} ended with ${finalTask.status}: ${
          JSON.stringify(extractTaskError(finalTask), null, 2)
        }`,
      );
    }

    summary = extractTaskSummary(finalTask);
  }

  console.log("=== AICC Smoke Test ===");
  console.log(`Zone: ${zoneHost}`);
  console.log(`App ID: ${appId}`);
  console.log(`User ID: ${userId}`);
  console.log(`Owner User ID: ${ownerUserId}`);
  console.log(`Model Alias: ${AICC_MODEL_ALIAS}`);
  console.log(`Task ID: ${response.task_id}`);
  console.log(`Status: ${terminalStatus}`);
  console.log("Input:");
  console.log(AICC_TEST_INPUT);
  console.log("Output:");
  console.log(renderSummary(summary));

  buckyos.logout(false);
}

main().catch((error) => {
  console.error("AICC smoke test failed");
  console.error(error);
  Deno.exit(1);
});
