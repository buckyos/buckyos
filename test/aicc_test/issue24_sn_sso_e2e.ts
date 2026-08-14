import { initTestRuntime } from "../test_helpers/buckyos_client.ts";

type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

type ModelEntry = {
  exact_model: string;
  api_types?: string[];
  logical_mounts?: string[];
};

type ProviderEntry = {
  provider_instance_name: string;
  inventory_revision?: string | null;
  models?: ModelEntry[];
};

type ModelsListResponse = { providers?: ProviderEntry[] };

type AiccResponse = {
  task_id?: string;
  status?: string;
  result?: unknown;
};

type TaskRecord = {
  id: number;
  status: string;
  message?: string | null;
  data?: {
    request?: { external_task_id?: string };
    result?: { output?: unknown };
    error?: unknown;
  };
};

const PROVIDER_NAME = env("ISSUE24_PROVIDER_NAME") ?? "sn-ai-provider-default";
const MODEL_ALIAS_OVERRIDE = env("ISSUE24_MODEL_ALIAS");
const WAIT_TIMEOUT_MS = Number(env("ISSUE24_WAIT_TIMEOUT_MS") ?? "90000");
const REQUIRE_CACHE_EVIDENCE = env("ISSUE24_REQUIRE_CACHE_EVIDENCE") === "1";
const REPORT_PATH =
  env("ISSUE24_REPORT_PATH") ??
  `reports/issue24-sn-sso-${Date.now().toString(36)}.json`;

function env(name: string): string | undefined {
  const value = Deno.env.get(name)?.trim();
  return value ? value : undefined;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function taskList(value: unknown): TaskRecord[] {
  if (Array.isArray(value)) return value as TaskRecord[];
  if (!value || typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  for (const key of ["tasks", "items", "data", "result"]) {
    if (Array.isArray(record[key])) return record[key] as TaskRecord[];
  }
  return [];
}

function taskRecord(value: unknown): TaskRecord | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const candidate = record.task ?? record.data ?? record.result ?? value;
  if (!candidate || typeof candidate !== "object") return null;
  const task = candidate as TaskRecord;
  return typeof task.id === "number" && typeof task.status === "string"
    ? task
    : null;
}

async function awaitTask(
  taskManager: RpcClient,
  externalTaskId: string,
  appId: string,
  userId: string,
): Promise<unknown> {
  const deadline = Date.now() + WAIT_TIMEOUT_MS;
  let task: TaskRecord | undefined;
  while (Date.now() < deadline && !task) {
    const raw = await taskManager.call("list_tasks", {
      app_id: appId,
      task_type: "aicc.compute",
      source_user_id: userId,
      source_app_id: appId,
    });
    task = taskList(raw).find(
      (item) => item.data?.request?.external_task_id === externalTaskId,
    );
    if (!task) await sleep(1000);
  }
  if (!task) throw new Error(`AICC task not found: ${externalTaskId}`);

  while (Date.now() < deadline) {
    const latest = taskRecord(
      await taskManager.call("get_task", { id: task.id }),
    );
    if (!latest) throw new Error(`Invalid task response for ${task.id}`);
    if (latest.status === "Completed") return latest.data?.result?.output;
    if (["Failed", "Canceled"].includes(latest.status)) {
      throw new Error(
        `AICC task ${task.id} ${latest.status}: ${JSON.stringify(
          latest.data?.error ?? latest.message,
        )}`,
      );
    }
    await sleep(1000);
  }
  throw new Error(`AICC task timed out: ${externalTaskId}`);
}

async function invokeChat(
  aicc: RpcClient,
  taskManager: RpcClient,
  modelAlias: string,
  appId: string,
  userId: string,
  runId: string,
): Promise<unknown> {
  const raw = (await aicc.call("llm.chat", {
    capability: "llm",
    model: { alias: modelAlias },
    requirements: {},
    payload: {
      input_json: {
        messages: [
          {
            role: "user",
            content: [{ type: "text", text: "Reply with exactly: issue24-ok" }],
          },
        ],
        temperature: 0,
        max_output_tokens: 32,
      },
      resources: [],
      options: { session_id: "issue24-sso-cache-window", rootid: runId },
    },
    idempotency_key: runId,
  })) as AiccResponse;
  if (!raw?.task_id || !raw.status) {
    throw new Error(`Invalid AICC response: ${JSON.stringify(raw)}`);
  }
  if (raw.status === "failed") {
    throw new Error(`AICC inference failed: ${JSON.stringify(raw.result)}`);
  }
  if (raw.status === "succeeded" && raw.result) return raw.result;
  return await awaitTask(taskManager, raw.task_id, appId, userId);
}

function cacheEvidence(value: unknown): Record<string, unknown> {
  const evidence: Record<string, unknown> = {};
  function visit(node: unknown, path: string): void {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) {
      node.forEach((item, index) => visit(item, `${path}[${index}]`));
      return;
    }
    for (const [key, child] of Object.entries(
      node as Record<string, unknown>,
    )) {
      const next = path ? `${path}.${key}` : key;
      if (/cache|reuse|refresh|login_count/i.test(key)) {
        evidence[next] =
          child === null || typeof child !== "object" ? child : "<present>";
      }
      if (!/token|authorization|secret|private.?key/i.test(key))
        visit(child, next);
    }
  }
  visit(value, "");
  return evidence;
}

function chooseModel(provider: ProviderEntry): string {
  if (MODEL_ALIAS_OVERRIDE) return MODEL_ALIAS_OVERRIDE;
  for (const model of provider.models ?? []) {
    if (model.api_types?.includes("llm.chat")) {
      const logical = model.logical_mounts?.find((item) =>
        item.startsWith("llm."),
      );
      return logical ?? model.exact_model;
    }
  }
  throw new Error(`${PROVIDER_NAME} has no llm.chat model`);
}

async function main(): Promise<void> {
  const { buckyos, userId, zoneHost } = await initTestRuntime();
  const appId =
    buckyos.getAppId?.() ?? env("BUCKYOS_TEST_APP_ID") ?? "buckycli";
  const aicc = buckyos.getServiceRpcClient("aicc") as RpcClient;
  const taskManager = buckyos.getServiceRpcClient("task-manager") as RpcClient;
  const report: Record<string, unknown> = {
    issue: "buckyos/sn-business#24",
    zone_host: zoneHost,
    app_id: appId,
    user_id: userId,
    provider_instance_name: PROVIDER_NAME,
  };
  const reportDir = REPORT_PATH.includes("/")
    ? REPORT_PATH.slice(0, REPORT_PATH.lastIndexOf("/"))
    : ".";

  try {
    const before = (await aicc.call("models.list", {})) as ModelsListResponse;
    const providerBefore = before.providers?.find(
      (item) => item.provider_instance_name === PROVIDER_NAME,
    );
    if (!providerBefore)
      throw new Error(`Provider not found: ${PROVIDER_NAME}`);
    if (!providerBefore.models?.length) {
      throw new Error(`${PROVIDER_NAME} inventory is empty before refresh`);
    }

    const refresh = (await aicc.call("provider.refresh_models", {
      provider_instance_name: PROVIDER_NAME,
    })) as Record<string, unknown>;
    if (refresh.ok !== true)
      throw new Error(`Refresh failed: ${JSON.stringify(refresh)}`);

    const after = (await aicc.call("models.list", {})) as ModelsListResponse;
    const providerAfter = after.providers?.find(
      (item) => item.provider_instance_name === PROVIDER_NAME,
    );
    if (!providerAfter?.models?.length) {
      throw new Error(`${PROVIDER_NAME} inventory is empty after refresh`);
    }
    const modelAlias = chooseModel(providerAfter);
    report.inventory = {
      before_revision: providerBefore.inventory_revision ?? null,
      refresh_revision: refresh.inventory_revision ?? null,
      after_revision: providerAfter.inventory_revision ?? null,
      model_count: providerAfter.models.length,
      selected_model_alias: modelAlias,
    };

    const first = await invokeChat(
      aicc,
      taskManager,
      modelAlias,
      appId,
      userId,
      `issue24-first-${crypto.randomUUID()}`,
    );
    const second = await invokeChat(
      aicc,
      taskManager,
      modelAlias,
      appId,
      userId,
      `issue24-second-${crypto.randomUUID()}`,
    );
    const evidence = { ...cacheEvidence(first), ...cacheEvidence(second) };
    if (REQUIRE_CACHE_EVIDENCE && Object.keys(evidence).length === 0) {
      throw new Error("Provider response exposed no cache reuse evidence");
    }
    report.inference = {
      consecutive_requests: 2,
      cache_evidence: evidence,
      cache_evidence_status: Object.keys(evidence).length
        ? "observed"
        : "not_exposed",
    };

    let invalidProviderRejected = false;
    try {
      await aicc.call("provider.refresh_models", {
        provider_instance_name: `${PROVIDER_NAME}-missing-issue24`,
      });
    } catch {
      invalidProviderRejected = true;
    }
    if (!invalidProviderRejected) {
      throw new Error("Unknown provider refresh was not rejected");
    }
    report.error_path = { unknown_provider_refresh_rejected: true };
    report.status = "passed";
    await Deno.mkdir(reportDir, { recursive: true });
    await Deno.writeTextFile(
      REPORT_PATH,
      `${JSON.stringify(report, null, 2)}\n`,
    );
    console.log(`Issue #24 SN SSO E2E passed: ${REPORT_PATH}`);
  } catch (error) {
    report.status = "failed";
    report.error = error instanceof Error ? error.message : String(error);
    await Deno.mkdir(reportDir, { recursive: true });
    await Deno.writeTextFile(
      REPORT_PATH,
      `${JSON.stringify(report, null, 2)}\n`,
    );
    throw error;
  } finally {
    buckyos.logout(false);
  }
}

main().catch((error) => {
  console.error("Issue #24 SN SSO E2E failed");
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exit(1);
});
