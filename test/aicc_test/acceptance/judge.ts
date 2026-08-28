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

function judgeResource(
  sourceValue: unknown,
  fallbackMime?: unknown,
): Record<string, unknown> | undefined {
  const source = object(sourceValue);
  if (!source) return undefined;
  const hasReference = typeof source.obj_id === "string" || typeof source.url === "string" ||
    typeof source.data_base64 === "string";
  if (!hasReference) return undefined;
  const mime = source.mime_hint ?? source.mime ?? fallbackMime;
  const normalizedSource = typeof mime === "string" && !source.mime_hint && !source.mime
    ? { ...source, mime_hint: mime }
    : source;
  return {
    type: typeof mime === "string" && mime.startsWith("image/") ? "image" : "document",
    source: normalizedSource,
  };
}

export function outputResources(value: unknown, depth = 0): Array<Record<string, unknown>> {
  if (depth > 8 || value === null || value === undefined) return [];
  if (Array.isArray(value)) return value.flatMap((item) => outputResources(item, depth + 1));
  const record = object(value);
  if (!record) return [];
  const resources: Array<Record<string, unknown>> = [];
  const direct = judgeResource(record);
  const source = judgeResource(record.source, record.mime);
  const artifact = judgeResource(record.resource, record.mime);
  for (const resource of [direct, source, artifact]) if (resource) resources.push(resource);
  for (const [key, child] of Object.entries(record)) {
    if (!["provider_io", "source", "resource"].includes(key)) {
      resources.push(...outputResources(child, depth + 1));
    }
  }
  return [...new Map(resources.map((resource) => [JSON.stringify(resource), resource])).values()];
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
  testedRequest: unknown;
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
  const sourceResources = outputResources(input.testedRequest).slice(0, 4);
  const outputResourcesForJudge = outputResources(input.terminalResponse).slice(0, 4);
  const inputSummary = `case=${input.caseId}; tested_model=${input.testedModel}; rubric_items=${input.rubric.length}; output_text_chars=${texts.length}; input_resources=${sourceResources.length}; output_resources=${outputResourcesForJudge.length}`;
  const content: Array<Record<string, unknown>> = [{
    type: "text",
    text: `You are a strict acceptance-test judge using rubric version ${input.rubricVersion}. Compare the source input resources and observed output against every rubric item. Return exactly one JSON object with pass, score, and reason. pass must be false when score is below ${input.threshold}.\nRubric:\n- ${input.rubric.join("\n- ")}\nObserved output text:\n${texts || "<no text; inspect attached output resources>"}\nThe next ${sourceResources.length} attachment(s) are source inputs; the final ${outputResourcesForJudge.length} attachment(s) are observed outputs.`,
  }, ...sourceResources, ...outputResourcesForJudge];
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
  const reason = record && ["reason", "reasoning", "details", "explanation", "rationale"]
    .map((key) => record[key])
    .find((value): value is string => typeof value === "string" && value.trim().length > 0);
  if (!record || typeof record.pass !== "boolean" || typeof record.score !== "number" ||
    !Number.isFinite(record.score) || record.score < 0 || record.score > 1 ||
    (record.score < input.threshold && record.pass)) {
    throw new JudgeError(`Judge verdict has invalid schema: ${JSON.stringify(verdict)}`);
  }
  return {
    taskId: initial.task_id,
    terminalResponse: result,
    passed: record.pass && record.score >= input.threshold,
    score: record.score,
    reason: reason ?? `Judge returned pass=${record.pass} score=${record.score}`,
    inputSummary,
  };
}
