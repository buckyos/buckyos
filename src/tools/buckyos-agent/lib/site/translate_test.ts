import { MeteredModel, ModelError } from "../model.ts";
import type { ModelRequest } from "../model.ts";
import { assert, assertEquals, assertFails } from "./testing.ts";
import { createTranslator, sameLanguage, TRANSLATE_MODEL } from "./translate.ts";
import type { TranslateJob } from "./translate.ts";
import { SiteError } from "./types.ts";

const DAY = 86_400_000;
const NOW = 20_000 * DAY + 3_600_000;

interface Sent {
  source_language: string;
  target_language: string;
  topics: string[];
  write_summary: boolean;
  texts: string[];
  article?: string;
}

// A model that "translates" by wrapping each text, and tags everything with what `tags` says.
function mockModel(options: { tags?: string[]; reply?: (sent: Sent, n: number) => unknown; tokens?: number } = {}) {
  const requests: (ModelRequest & { sent: Sent })[] = [];
  const call = (request: ModelRequest) => {
    const sent = JSON.parse(request.user) as Sent;
    requests.push({ ...request, sent });
    const reply = options.reply?.(sent, requests.length) ?? {
      translations: sent.texts.map((text) => `译(${text})`),
      tags: options.tags ?? [],
      summary: sent.write_summary ? "模型写的摘要" : "",
    };
    if (reply instanceof Error) return Promise.reject(reply);
    const total = options.tokens ?? 100;
    return Promise.resolve({
      text: typeof reply === "string" ? reply : JSON.stringify(reply),
      model: "mock-mt",
      usage: { input_tokens: total - 20, output_tokens: 20, total_tokens: total },
    });
  };
  return { call, requests };
}

const job = (patch: Partial<TranslateJob> = {}): TranslateJob => ({
  id: "0123456789abcdef",
  from: "ja",
  to: "zh-CN",
  topics: ["国际", "科技", "体育"],
  texts: ["見出し", "", "第一段落", "第二段落", "第一段落"],
  summary: "",
  ...patch,
});

async function withCache<T>(run: (cacheDir: string) => Promise<T>): Promise<T> {
  const dir = await Deno.makeTempDir();
  try {
    return await run(`${dir}/tr`);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
}

Deno.test("one request per article: the distinct texts in, as many translations out, tags and a summary", async () => {
  await withCache(async (cacheDir) => {
    const mock = mockModel({ tags: ["科技", "国际"] });
    const translator = createTranslator({ model: new MeteredModel(mock.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    const out = await translator.translate(job());
    assertEquals(out, {
      texts: ["译(見出し)", "", "译(第一段落)", "译(第二段落)", "译(第一段落)"],
      tags: ["科技", "国际"],
      summary: "模型写的摘要",
    });
    assertEquals(mock.requests.length, 1);
    assertEquals(mock.requests[0].sent, {
      source_language: "ja",
      target_language: "zh-CN",
      topics: ["国际", "科技", "体育"],
      write_summary: true,
      texts: ["見出し", "第一段落", "第二段落"],
    });
    assertEquals([mock.requests[0].model, mock.requests[0].traceId], [TRANSLATE_MODEL, "site:0123456789abcdef"]);
    const schema = mock.requests[0].schema.schema as { required: string[]; additionalProperties: boolean };
    assertEquals([schema.required, schema.additionalProperties], [["translations", "tags", "summary"], false]);

    // An article that has its own summary gets it translated, none written.
    const own = await translator.translate(job({ id: "1111111111111111", texts: ["見出し", "要約"], summary: "要約" }));
    assertEquals([own.summary, own.texts, mock.requests[1].sent.write_summary], ["", ["译(見出し)", "译(要約)"], false]);
  });
});

Deno.test("translations are kept by the hash of the original: a corrected article has only its changed blocks done", async () => {
  await withCache(async (cacheDir) => {
    const mock = mockModel({ tags: ["科技"] });
    const translator = createTranslator({ model: new MeteredModel(mock.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    await translator.translate(job());
    const again = await translator.translate(job());
    assertEquals([mock.requests.length, again.texts[2], again.tags, again.summary], [1, "译(第一段落)", ["科技"], "模型写的摘要"]);

    const corrected = await translator.translate(job({ texts: ["見出し", "", "第一段落（訂正）", "第二段落", "第一段落（訂正）"] }));
    assertEquals(mock.requests.length, 2);
    assertEquals(mock.requests[1].sent.texts, ["第一段落（訂正）"]);
    assertEquals(mock.requests[1].sent.write_summary, false, "the summary written before is kept");
    assertEquals(corrected.texts, ["译(見出し)", "", "译(第一段落（訂正）)", "译(第二段落)", "译(第一段落（訂正）)"]);

    // Another site language is another cache.
    const english = await translator.translate(job({ to: "en" }));
    assertEquals([mock.requests.length, mock.requests[2].sent.texts.length, english.texts[0]], [3, 3, "译(見出し)"]);
  });
});

Deno.test("a wrong number of translations is asked again once, then the article is left for the next sync", async () => {
  await withCache(async (cacheDir) => {
    const short = (sent: Sent) => ({ translations: sent.texts.slice(1), tags: [], summary: "" });
    const once = mockModel({ reply: (sent, n) => n === 1 ? short(sent) : undefined });
    const model = new MeteredModel(once.call, {});
    const translator = createTranslator({ model, cacheDir, dailyCap: 1_000_000, now: () => NOW });
    const out = await translator.translate(job());
    assertEquals([once.requests.length, out.texts[0], model.tokens], [2, "译(見出し)", 200]);

    const always = mockModel({ reply: short });
    const stubborn = createTranslator({ model: new MeteredModel(always.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    const err = await assertFails(() => stubborn.translate(job({ id: "2222222222222222" })), "2 translations for 3 texts, twice");
    assert(err instanceof SiteError && err.kind === "transient", "a mismatch leaves the article for later");
    assertEquals(always.requests.length, 2);

    const garbage = mockModel({ reply: () => "Sure! Here is the translation:" });
    const noJson = createTranslator({ model: new MeteredModel(garbage.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    assertEquals((await assertFails(() => noJson.translate(job({ id: "3333333333333333" })), "JSON asked for") as SiteError).kind, "transient");
    // Nothing of a failed article is kept: the next sync asks for all of it again.
    const retry = mockModel();
    const later = createTranslator({ model: new MeteredModel(retry.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    await later.translate(job({ id: "2222222222222222" }));
    assertEquals(retry.requests[0].sent.texts.length, 3);
  });
});

Deno.test("over budget: the run's tokens or the day's cap stop translation before the model is asked", async () => {
  await withCache(async (cacheDir) => {
    const dir = cacheDir.slice(0, -3);
    const log = `${dir}/usage.jsonl`;
    const record = (at_ms: number, run_id: string, total_tokens: number) =>
      JSON.stringify({ at_ms, model: "m", run_id, input_tokens: total_tokens, output_tokens: 0, total_tokens });
    // Yesterday (UTC) does not count; two other runs of today do.
    await Deno.writeTextFile(log, [record(NOW - DAY, "run-1", 900_000), record(NOW - 1000, "run-8", 600), record(NOW - 500, "run-9", 300)].join("\n") + "\n");

    const mock = mockModel({ tokens: 150 });
    const model = new MeteredModel(mock.call, { log, runId: "run-10" }, () => NOW);
    const translator = createTranslator({ model, cacheDir, dailyCap: 1000, now: () => NOW });
    await translator.translate(job());
    assertEquals(model.spentSince(Math.floor(NOW / DAY) * DAY), 1050);
    const daily = await assertFails(() => translator.translate(job({ id: "4444444444444444" })), "today's translation budget is used up (1050 of 1000");
    assert(daily instanceof SiteError && daily.kind === "budget", "the daily cap is a budget refusal");
    assertEquals(mock.requests.length, 1, "nothing is asked once the cap is reached");

    // $OPENDAN_MODEL_BUDGET: what this run may use.
    const tight = new MeteredModel(mock.call, { log, runId: "run-11", budget: 100 }, () => NOW);
    const limited = createTranslator({ model: tight, cacheDir, dailyCap: 1_000_000, now: () => NOW });
    await limited.translate(job({ id: "5555555555555555" }));
    const run = await assertFails(() => limited.translate(job({ id: "6666666666666666" })), "model budget: 150 of 100");
    assertEquals([(run as SiteError).kind, mock.requests.length], ["budget", 2]);

    const zero = createTranslator({ model: new MeteredModel(mock.call, { budget: 0 }), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    assertEquals((await assertFails(() => zero.translate(job({ id: "7777777777777777" })), "0 of 0") as SiteError).kind, "budget");
  });
});

Deno.test("no model, no quota, no permission: a configuration problem, not retried", async () => {
  await withCache(async (cacheDir) => {
    for (const code of ["no_candidate_model", "quota_exhausted", "resource_not_permitted"]) {
      const mock = mockModel({ reply: () => new ModelError("config", `llm.translate: ${code}`, code) });
      const translator = createTranslator({ model: new MeteredModel(mock.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
      const err = await assertFails(() => translator.translate(job()), code);
      assert(err instanceof SiteError && err.kind === "config", `${code} is a config failure`);
      assertEquals(mock.requests.length, 1, "asked once");
    }
    const flaky = mockModel({ reply: () => new ModelError("transient", "connection reset") });
    const translator = createTranslator({ model: new MeteredModel(flaky.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    assertEquals((await assertFails(() => translator.translate(job()), "connection reset") as SiteError).kind, "transient");
  });
});

Deno.test("tags that are not among the site's topics are dropped, and at most three are kept", async () => {
  await withCache(async (cacheDir) => {
    const mock = mockModel({ tags: ["娱乐", "科技", "科技", "国际", "体育", "经济"] });
    const translator = createTranslator({ model: new MeteredModel(mock.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    assertEquals((await translator.translate(job({ topics: ["国际", "科技", "体育", "经济"] }))).tags, ["科技", "国际", "体育"]);
    assertEquals(mock.requests.length, 1, "a tag outside the topics is no reason to ask again");
    // The topics changed since: what was kept is filtered by the topics of now.
    assertEquals((await translator.translate(job({ topics: ["体育"] }))).tags, ["体育"]);
    const none = await translator.translate(job({ id: "8888888888888888", topics: [] }));
    assertEquals(none.tags, []);
  });
});

Deno.test("a source in the site's language is not translated, only tagged and summarized", async () => {
  await withCache(async (cacheDir) => {
    assertEquals([sameLanguage("zh-CN", "zh"), sameLanguage("en-US", "EN"), sameLanguage("ja", "zh-CN")], [true, true, false]);
    const mock = mockModel({ tags: ["科技"] });
    const translator = createTranslator({ model: new MeteredModel(mock.call, {}), cacheDir, dailyCap: 1_000_000, now: () => NOW });
    const out = await translator.translate(job({ from: "zh", texts: ["标题", "", "正文第一段"] }));
    assertEquals(out, { texts: ["标题", "", "正文第一段"], tags: ["科技"], summary: "模型写的摘要" });
    assertEquals([mock.requests[0].sent.texts, mock.requests[0].sent.article], [[], "标题\n\n正文第一段"]);

    // With its own summary and no topics there is nothing to ask the model.
    const quiet = await translator.translate(job({ id: "9999999999999999", from: "zh-TW", topics: [], texts: ["标题", "摘要"], summary: "摘要" }));
    assertEquals([quiet.texts, mock.requests.length], [["标题", "摘要"], 1]);
  });
});
