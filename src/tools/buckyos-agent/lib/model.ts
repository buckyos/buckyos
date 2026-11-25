// One text call to a model through AICC, for the commands that use a model as
// a step of a program (a Goal job translating articles). The model never
// changes behind the caller's back: fallback and failover are off. Under a
// Goal job the tokens are written to $OPENDAN_USAGE_LOG in the format `xllm`
// writes, and $OPENDAN_MODEL_BUDGET is kept.

export interface ModelRequest {
  /** A logical model name, e.g. `llm.translate`. */
  model: string;
  system: string;
  user: string;
  /** The JSON the answer must be. */
  schema: { name: string; schema: unknown };
  traceId?: string;
}

export interface ModelUsage {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
}

export interface ModelAnswer {
  text: string;
  /** The model that answered; the requested name when the service does not say. */
  model: string;
  usage: ModelUsage | null;
}

export type ModelCall = (request: ModelRequest) => Promise<ModelAnswer>;

/**
 * `config`: no model, no quota or no permission — nothing a retry changes;
 * `budget`: the tokens this run was given are used up; `transient`: try again
 * later; `failed`: anything else.
 */
export class ModelError extends Error {
  constructor(
    readonly kind: "config" | "budget" | "transient" | "failed",
    message: string,
    readonly code?: string,
  ) {
    super(message);
    this.name = "ModelError";
  }
}

const CONFIG_CODES = [
  "quota_exhausted",
  "quota_budget_exceeded",
  "resource_not_permitted",
  "attribution_unavailable",
  "policy_denied",
  "budget_exceeded",
  "no_provider_available",
  "no_candidate_model",
  "fallback_not_allowed",
  "invalid_model_name",
  "unsupported_execution_mode",
];
const CONFIG_TEXT = [
  "no_candidate_model",
  "no_provider_available",
  "no route candidate generated",
  "provider_unavailable",
  "route_failed",
  "model_alias_not_mapped",
];

interface AiccFailure {
  code?: unknown;
  message?: unknown;
  retriable?: unknown;
  data?: { status?: unknown; retriable?: unknown } | null;
}

/** How a failed AICC call counts: by its code, never by guessing from a successful-looking answer. */
export function aiccModelError(context: string, failure: unknown): ModelError {
  const f = (failure ?? {}) as AiccFailure;
  const code = typeof f.code === "string" ? f.code : undefined;
  const text = typeof f.message === "string" ? f.message : String(failure);
  const message = `${context}: ${code && !text.includes(code) ? `${code}: ` : ""}${text}`;
  const status = typeof f.data?.status === "number" ? f.data.status : undefined;
  if (code && CONFIG_CODES.includes(code)) return new ModelError("config", message, code);
  if (code === "http_error" && (status === 401 || status === 403)) return new ModelError("config", message, code);
  if (
    code === "quota_unavailable" || code === "timeout" || code === "transport" || f.retriable === true ||
    f.data?.retriable === true || (code === "http_error" && (status === 408 || (status ?? 0) >= 500))
  ) {
    return new ModelError("transient", message, code);
  }
  const lowered = text.toLowerCase();
  if (CONFIG_TEXT.some((part) => lowered.includes(part))) return new ModelError("config", message, code);
  return new ModelError("failed", message, code);
}

interface AiccChat {
  helperLlmChat(request: Record<string, unknown>): Promise<{
    status: string;
    message?: { content: { type: string; text?: string }[] };
    usage?: { input_tokens?: number; output_tokens?: number; total_tokens?: number };
    route_trace?: { final_model?: string };
    error?: unknown;
    task_id?: string;
  }>;
}

/** `helper.llm_chat` of the SDK's AICC client as a ModelCall: one answer, `json_schema`, no fallback, no failover. */
export function aiccModel(aicc: unknown): ModelCall {
  const client = aicc as AiccChat;
  return async (request) => {
    const text = (value: string) => [{ type: "text", text: value }];
    let response;
    try {
      response = await client.helperLlmChat({
        logical_model: request.model,
        messages: [
          { role: "system", content: text(request.system) },
          { role: "user", content: text(request.user) },
        ],
        response_format: {
          type: "json_schema",
          json_schema: { name: request.schema.name, schema: request.schema.schema, strict: true },
        },
        requirements: { json_schema: true },
        policy: { allow_fallback: false, runtime_failover: false },
        execution_mode: "immediate",
        ...(request.traceId ? { trace_id: request.traceId } : {}),
      });
    } catch (err) {
      throw aiccModelError(`${request.model}`, err);
    }
    if (response.status === "failed") throw aiccModelError(request.model, response.error);
    if (response.status !== "succeeded") {
      throw new ModelError(
        "config",
        `${request.model} answered with a task (${response.task_id}) instead of a text: it needs a model that answers at once`,
      );
    }
    const answer = (response.message?.content ?? []).flatMap((part) =>
      part.type === "text" && typeof part.text === "string" ? [part.text] : []
    ).join("");
    if (!answer) throw new ModelError("failed", `${request.model} answered without a text`);
    const usage = response.usage;
    const input = usage?.input_tokens ?? 0;
    const output = usage?.output_tokens ?? 0;
    return {
      text: answer,
      model: response.route_trace?.final_model || request.model,
      usage: usage ? { input_tokens: input, output_tokens: output, total_tokens: usage.total_tokens ?? input + output } : null,
    };
  };
}

/** What a Goal job is given about its model use (Agent State Protocol §9.9). */
export interface UsageEnv {
  /** $OPENDAN_USAGE_LOG: one JSON record per call is appended here. */
  log?: string;
  /** $OPENDAN_JOB_RUN_ID: the run the records belong to. */
  runId?: string;
  /** $OPENDAN_MODEL_BUDGET: the tokens this run may use. */
  budget?: number;
}

export function usageEnv(env: Record<string, string | undefined> = Deno.env.toObject()): UsageEnv {
  const budget = Number(env.OPENDAN_MODEL_BUDGET?.trim() || NaN);
  return {
    ...(env.OPENDAN_USAGE_LOG ? { log: env.OPENDAN_USAGE_LOG } : {}),
    ...(env.OPENDAN_JOB_RUN_ID ? { runId: env.OPENDAN_JOB_RUN_ID } : {}),
    ...(Number.isSafeInteger(budget) && budget >= 0 ? { budget } : {}),
  };
}

/** A model whose calls are counted, logged and kept within the run's budget. */
export class MeteredModel {
  /** Tokens the calls through this object used. */
  tokens = 0;
  calls = 0;
  /** The model that answered the last call. */
  model: string | null = null;

  constructor(
    private readonly call: ModelCall,
    private readonly env: UsageEnv = usageEnv(),
    private readonly now: () => number = Date.now,
  ) {}

  private records(): { at_ms?: unknown; run_id?: unknown; total_tokens?: unknown }[] {
    if (!this.env.log) return [];
    let text: string;
    try {
      text = Deno.readTextFileSync(this.env.log);
    } catch {
      return [];
    }
    return text.split("\n").flatMap((line) => {
      try {
        const record = JSON.parse(line);
        return record && typeof record === "object" ? [record] : [];
      } catch {
        return [];
      }
    });
  }

  private sum(keep: (record: { at_ms?: unknown; run_id?: unknown }) => boolean): number {
    return this.records().reduce(
      (total, record) => total + (keep(record) && typeof record.total_tokens === "number" ? record.total_tokens : 0),
      0,
    );
  }

  /** Tokens of this run: its records in the usage log (earlier attempts of the run included). */
  spentThisRun(): number {
    const run = this.env.runId;
    return Math.max(this.tokens, run ? this.sum((record) => record.run_id === run) : 0);
  }

  /** Tokens every run recorded in the usage log since `ms`; without a log, those of this object. */
  spentSince(ms: number): number {
    return Math.max(this.tokens, this.sum((record) => typeof record.at_ms === "number" && record.at_ms >= ms));
  }

  async ask(request: ModelRequest): Promise<ModelAnswer> {
    const budget = this.env.budget;
    if (budget !== undefined) {
      const used = this.spentThisRun();
      if (used >= budget) {
        throw new ModelError("budget", `model budget: ${used} of ${budget} tokens used`, "model_budget");
      }
    }
    this.calls += 1;
    const answer = await this.call(request);
    this.model = answer.model;
    const usage = answer.usage;
    if (usage && usage.total_tokens > 0) {
      this.tokens += usage.total_tokens;
      if (this.env.log) {
        const record = {
          at_ms: this.now(),
          model: answer.model,
          run_id: this.env.runId ?? null,
          input_tokens: usage.input_tokens,
          output_tokens: usage.output_tokens,
          total_tokens: usage.total_tokens,
        };
        try {
          await Deno.writeTextFile(this.env.log, `${JSON.stringify(record)}\n`, { append: true, create: true });
        } catch (err) {
          console.error(`usage log ${this.env.log}: ${(err as Error).message}`);
        }
      }
    }
    return answer;
  }
}
