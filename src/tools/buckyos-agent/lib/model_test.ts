import { aiccModel, aiccModelError, MeteredModel, ModelError, usageEnv } from "./model.ts";
import type { ModelAnswer, ModelCall, ModelRequest } from "./model.ts";

function assertEquals(actual: unknown, expected: unknown, what = ""): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) throw new Error(`${what} expected ${expectedJson}, got ${actualJson}`);
}

async function assertModelError(fn: () => Promise<unknown>, kind: string, includes = ""): Promise<ModelError> {
  try {
    await fn();
  } catch (err) {
    if (!(err instanceof ModelError) || err.kind !== kind || !err.message.includes(includes)) {
      throw new Error(`expected a ${kind} ModelError with ${JSON.stringify(includes)}, got ${err}`);
    }
    return err;
  }
  throw new Error(`expected a ${kind} ModelError`);
}

const REQUEST: ModelRequest = {
  model: "llm.translate",
  system: "Translate.",
  user: '{"texts":["こんにちは"]}',
  schema: { name: "translation", schema: { type: "object" } },
  traceId: "site:0123456789abcdef",
};

const answer = (total: number, model = "qwen-mt-plus@dashscope"): ModelAnswer => ({
  text: "{}",
  model,
  usage: { input_tokens: total - 10, output_tokens: 10, total_tokens: total },
});

Deno.test("the AICC call asks llm.translate for a json_schema answer and forbids fallback and failover", async () => {
  const sent: Record<string, unknown>[] = [];
  const call = aiccModel({
    helperLlmChat: (request: Record<string, unknown>) => {
      sent.push(request);
      return Promise.resolve({
        status: "succeeded",
        message: { role: "assistant", content: [{ type: "thinking", text: "…" }, { type: "text", text: '{"a":' }, { type: "text", text: "1}" }] },
        usage: { input_tokens: 120, output_tokens: 30 },
        route_trace: { attempts: [], final_model: "qwen-mt-plus@dashscope" },
      });
    },
  });
  assertEquals(await call(REQUEST), {
    text: '{"a":1}',
    model: "qwen-mt-plus@dashscope",
    usage: { input_tokens: 120, output_tokens: 30, total_tokens: 150 },
  });
  assertEquals(sent, [{
    logical_model: "llm.translate",
    messages: [
      { role: "system", content: [{ type: "text", text: "Translate." }] },
      { role: "user", content: [{ type: "text", text: '{"texts":["こんにちは"]}' }] },
    ],
    response_format: { type: "json_schema", json_schema: { name: "translation", schema: { type: "object" }, strict: true } },
    requirements: { json_schema: true },
    policy: { allow_fallback: false, runtime_failover: false },
    execution_mode: "immediate",
    trace_id: "site:0123456789abcdef",
  }]);
});

Deno.test("failures of the AICC call are told apart by their code", async () => {
  const failing = (failure: unknown, thrown = true) =>
    aiccModel({
      helperLlmChat: () => thrown ? Promise.reject(failure) : Promise.resolve(failure),
    })(REQUEST);
  const rpc = (code: string, message: string, data?: unknown) => Object.assign(new Error(message), { code, data });

  await assertModelError(() => failing(rpc("no_candidate_model", "llm.translate has no model")), "config", "no_candidate_model");
  await assertModelError(() => failing(rpc("quota_exhausted", "the daily quota is used up")), "config");
  await assertModelError(() => failing(rpc("resource_not_permitted", "not permitted")), "config");
  await assertModelError(() => failing(rpc("fallback_not_allowed", "the model is down and fallback is off")), "config");
  await assertModelError(() => failing(rpc("http_error", "RPC call error: 403", { status: 403 })), "config");
  await assertModelError(() => failing(new Error("RPC call error: route_failed: no route candidate generated")), "config");
  await assertModelError(() => failing(rpc("transport", "RPC call failed: connection reset")), "transient");
  await assertModelError(() => failing(rpc("quota_unavailable", "the quota rule cannot be read")), "transient");
  await assertModelError(() => failing(rpc("http_error", "RPC call error: 504", { status: 504 })), "transient");
  await assertModelError(() => failing(rpc("provider_error", "the provider answered 500", { retriable: true })), "transient");
  await assertModelError(() => failing(rpc("provider_error", "bad request")), "failed");
  await assertModelError(
    () => failing({ status: "failed", task_id: "t-1", error: { code: "timeout", message: "the provider timed out" } }, false),
    "transient",
  );
  await assertModelError(() => failing({ status: "running", task_id: "t-2" }, false), "config", "t-2");
  await assertModelError(() => failing({ status: "succeeded", message: { content: [] } }, false), "failed", "without a text");
  assertEquals(aiccModelError("x", "plain text").kind, "failed");
});

Deno.test("the environment of a Goal job is read once", () => {
  assertEquals(usageEnv({}), {});
  assertEquals(
    usageEnv({ OPENDAN_USAGE_LOG: "/data/usage.jsonl", OPENDAN_JOB_RUN_ID: "run-7", OPENDAN_MODEL_BUDGET: " 5000 " }),
    { log: "/data/usage.jsonl", runId: "run-7", budget: 5000 },
  );
  assertEquals(usageEnv({ OPENDAN_MODEL_BUDGET: "0" }), { budget: 0 });
  assertEquals(usageEnv({ OPENDAN_MODEL_BUDGET: "lots" }), {});
});

Deno.test("every call is appended to the usage log in the format xllm writes", async () => {
  const dir = await Deno.makeTempDir();
  const log = `${dir}/usage.jsonl`;
  try {
    const answers = [answer(150), answer(60, "llm.translate"), { text: "{}", model: "m", usage: null }, answer(0)];
    const model = new MeteredModel(() => Promise.resolve(answers.shift()!), { log, runId: "run-7" }, () => 1764001700000);
    for (let n = 0; n < 4; n += 1) await model.ask(REQUEST);
    const lines = (await Deno.readTextFile(log)).trimEnd().split("\n");
    assertEquals(lines, [
      '{"at_ms":1764001700000,"model":"qwen-mt-plus@dashscope","run_id":"run-7","input_tokens":140,"output_tokens":10,"total_tokens":150}',
      '{"at_ms":1764001700000,"model":"llm.translate","run_id":"run-7","input_tokens":50,"output_tokens":10,"total_tokens":60}',
    ], "a call that reports no tokens is not recorded");
    assertEquals([model.tokens, model.calls, model.model], [210, 4, "qwen-mt-plus@dashscope"]);

    // Outside a job there is no run: the record says so. Without a log nothing is written.
    const outside = new MeteredModel(() => Promise.resolve(answer(40)), { log }, () => 1764001800000);
    await outside.ask(REQUEST);
    assertEquals(JSON.parse((await Deno.readTextFile(log)).trimEnd().split("\n").at(-1)!).run_id, null);
    const quiet = new MeteredModel(() => Promise.resolve(answer(40)), {});
    await quiet.ask(REQUEST);
    assertEquals([quiet.tokens, (await Deno.readTextFile(log)).trimEnd().split("\n").length], [40, 3]);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("the run's budget counts what the usage log has for the run, earlier attempts included", async () => {
  const dir = await Deno.makeTempDir();
  const log = `${dir}/usage.jsonl`;
  try {
    await Deno.writeTextFile(
      log,
      [
        { at_ms: 1764000000000, model: "m", run_id: "run-7", input_tokens: 300, output_tokens: 100, total_tokens: 400 },
        { at_ms: 1764000000000, model: "m", run_id: "run-6", input_tokens: 900, output_tokens: 100, total_tokens: 1000 },
        { at_ms: 1763900000000, model: "m", run_id: "run-1", input_tokens: 90, output_tokens: 10, total_tokens: 100 },
      ].map((record) => JSON.stringify(record)).join("\n") + "\nnot json\n",
    );
    let calls = 0;
    const call: ModelCall = () => {
      calls += 1;
      return Promise.resolve(answer(350));
    };
    const model = new MeteredModel(call, { log, runId: "run-7", budget: 1000 }, () => 1764001700000);
    assertEquals([model.spentThisRun(), model.spentSince(1764000000000), model.spentSince(0)], [400, 1400, 1500]);
    await model.ask(REQUEST);
    await model.ask(REQUEST);
    assertEquals(model.spentThisRun(), 1100);
    const refused = await assertModelError(() => model.ask(REQUEST), "budget", "1100 of 1000");
    assertEquals([refused.code, calls], ["model_budget", 2], "a refused call does not reach the model");

    const none = new MeteredModel(call, { budget: 0 });
    await assertModelError(() => none.ask(REQUEST), "budget", "0 of 0");
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
