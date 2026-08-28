import type { RpcClient } from "./gateway.ts";

type AiMethodResponse = {
  task_id: string;
  status: "succeeded" | "running" | "failed";
  result?: unknown;
};

export class JudgeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "JudgeError";
  }
}

function object(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

export function responseText(value: unknown, depth = 0): string[] {
  if (depth > 8 || value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap((item) => responseText(item, depth + 1));
  const record = object(value);
  if (!record) return [];
  const values: string[] = [];
  for (const key of ["text", "output_text"] as const) {
    if (typeof record[key] === "string") values.push(record[key]);
  }
  if (typeof record.content === "string") {
    values.push(record.content);
  } else {
    values.push(...responseText(record.content, depth + 1));
  }
  for (const [key, child] of Object.entries(record)) {
    if (!["text", "output_text", "content", "provider_io"].includes(key)) {
      values.push(...responseText(child, depth + 1));
    }
  }
  return [...new Set(values.filter(Boolean))];
}

function outputResources(value: unknown, depth = 0): Array<Record<string, unknown>> {
  if (depth > 8 || value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap((item) => outputResources(item, depth + 1));
  const record = object(value);
  if (!record) return [];
  const source = object(record.source);
  const resources: Array<Record<string, unknown>> = [];
  if (source && (typeof source.obj_id === "string" || typeof source.url === "string" || typeof source.data_base64 === "string")) {
    const mime = source.mime_hint ?? source.mime;
    resources.push({
      type: typeof mime === "string" && mime.startsWith("image/") ? "image" : "document",
      source,
    });
  }
  for (const [key, child] of Object.entries(record)) {
    if (key !== "provider_io") resources.push(...outputResources(child, depth + 1));
  }
  return resources;
}

async function terminal(taskManager: RpcClient, initial: AiMethodResponse, timeoutMs: number): Promise<unknown> {
  if (!initial.task_id) throw new JudgeError("Judge response omitted task_id");
  if (initial.status === "failed") throw new JudgeError("Judge request failed immediately");
  if (initial.status === "succeeded") return initial;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const raw = object(await taskManager.call("get_task", { task_id: initial.task_id })) ?? {};
    const task = object(raw.task) ?? raw;
    if (task.phase === "Terminal") {
      if (task.outcome !== "Succeeded") {
        throw new JudgeError(`Judge task ended ${String(task.outcome)}: ${JSON.stringify(task.error ?? {})}`);
      }
      const taskResult = object(task.result);
      return { task_id: initial.task_id, status: "succeeded", result: object(taskResult?.result)?.output };
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new JudgeError(`Judge task timed out after ${timeoutMs} ms`);
}

export async function runJudge(input: {
  aicc: RpcClient;
  taskManager: RpcClient;
  model: string;
  runId: string;
  caseId: string;
  rubricVersion: string;
  rubric: string[];
  testedModel: string;
  testedProviderInstance: string;
  preferDifferentProvider: boolean;
  threshold: number;
  terminalResponse: unknown;
  timeoutMs: number;
  invoke: (request: Record<string, unknown>) => Promise<AiMethodResponse>;
}): Promise<{
  taskId: string;
  terminalResponse: unknown;
  passed: boolean;
  score: number;
  reason: string;
  inputSummary: string;
}> {
  const texts = responseText(input.terminalResponse).join("\n").slice(0, 4_000);
  const resources = outputResources(input.terminalResponse).slice(0, 4);
  const inputSummary = `case=${input.caseId}; tested_model=${input.testedModel}; rubric_items=${input.rubric.length}; output_text_chars=${texts.length}; output_resources=${resources.length}`;
  const content: Array<Record<string, unknown>> = [{
    type: "text",
    text: `You are a strict acceptance-test judge using rubric version ${input.rubricVersion}. Evaluate only the supplied output against every rubric item. Return JSON only. pass must be false when score is below ${input.threshold}.\nRubric:\n- ${input.rubric.join("\n- ")}\nObserved output text:\n${texts || "<no text; inspect attached output resources>"}`,
  }, ...resources];
  const request: Record<string, unknown> = {
    capability: "llm",
    model: { alias: input.model },
    requirements: { must_features: ["json_output"], resp_format: "json" },
    ...(input.preferDifferentProvider
      ? { policy: { blocked_provider_instances: [input.testedProviderInstance] } }
      : {}),
    payload: {
      input_json: {
        messages: [{ role: "user", content }],
        max_output_tokens: 1024,
        response_format: {
          type: "json_schema",
          name: "aicc_t2_judge_verdict",
          strict: true,
          schema: {
            type: "object",
            properties: {
              pass: { type: "boolean" },
              score: { type: "number", minimum: 0, maximum: 1 },
              reason: { type: "string" },
            },
            required: ["pass", "score", "reason"],
            additionalProperties: false,
          },
        },
      },
      resources: [],
      tool_specs: [],
      options: { session_id: `${input.runId}:${input.caseId}:judge`, rootid: input.runId },
    },
    idempotency_key: `${input.runId}:${input.caseId}:judge`,
  };
  const initial = await input.invoke(request);
  const result = await terminal(input.taskManager, initial, input.timeoutMs);
  const text = responseText(result).join("\n");
  const match = /\{[\s\S]*\}/.exec(text);
  if (!match) throw new JudgeError(`Judge returned no JSON object: ${text.slice(0, 500)}`);
  let verdict: unknown;
  try {
    verdict = JSON.parse(match[0]);
  } catch (error) {
    throw new JudgeError(`Judge returned invalid JSON: ${String(error)}`);
  }
  const record = object(verdict);
  if (!record || typeof record.pass !== "boolean" || typeof record.score !== "number" ||
    !Number.isFinite(record.score) || record.score < 0 || record.score > 1 ||
    typeof record.reason !== "string" || (record.score < input.threshold && record.pass)) {
    throw new JudgeError(`Judge verdict has invalid schema: ${JSON.stringify(verdict)}`);
  }
  return {
    taskId: initial.task_id,
    terminalResponse: result,
    passed: record.pass && record.score >= input.threshold,
    score: record.score,
    reason: record.reason,
    inputSummary,
  };
}
