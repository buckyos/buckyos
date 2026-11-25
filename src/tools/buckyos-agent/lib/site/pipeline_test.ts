import { MeteredModel, ModelError } from "../model.ts";
import type { ModelRequest } from "../model.ts";
import { Net } from "./extract.ts";
import type { Resize } from "./images.ts";
import { readSourcePackage, sourceEntry } from "./packages.ts";
import { putSupplement, syncSource } from "./pipeline.ts";
import type { SyncEnv } from "./pipeline.ts";
import { articleAddress, articleProblems, expiryOf, feedProblems, IMMUTABLE, SiteBucket } from "./store.ts";
import { assert, assertEquals, assertFails, cannedFetch, memoryStore, tempDir, tinyPng } from "./testing.ts";
import type { Page } from "./testing.ts";
import { createTranslator } from "./translate.ts";
import { articleKey, EVENTS_KEY, feedKey, markdownKey, READING_KEY } from "./types.ts";
import type { Article, Events, Feed, RawArticle, Site, SourceDef } from "./types.ts";

const DAY = 86_400_000;
const T0 = 20_500 * DAY + 3_600_000;

// The extractor of the test sources: the list is JSON at <home>feed.json, an article is JSON at its URL.
const EXTRACT = `export default async function extract(ctx) {
  const feed = await ctx.fetch(ctx.source.home_url + "feed.json", { conditional: true });
  if (feed.notModified) return { list: [], items: [], state: ctx.state };
  if (feed.status === 401) throw new SiteError("auth", "the list asks to sign in");
  if (feed.status !== 200) throw new SiteError("program", "the list answered HTTP " + feed.status);
  const list = JSON.parse(feed.text);
  const items = [];
  for (const entry of list) {
    if (items.length >= ctx.limit || (ctx.known(entry.url) && !entry.again)) continue;
    const page = await ctx.fetch(entry.url);
    if (page.status !== 200) throw new SiteError("program", entry.url + " answered HTTP " + page.status);
    const article = JSON.parse(page.text);
    if (ctx.source.comments === 0) delete article.comments;
    items.push(article);
  }
  return { list, items, state: { runs: (ctx.state?.runs ?? 0) + 1 } };
}`;

function def(id: string, patch: Partial<SourceDef> = {}): SourceDef {
  return {
    schema: "buckyos.site.source/1",
    id,
    title: id,
    home_url: `https://${id.replaceAll("-", ".")}/`,
    match: [{ host: id.replaceAll("-", ".") }],
    lang: "ja",
    kind: "news",
    needs: "http",
    ...patch,
  };
}

const paragraph = (n: number | string, chars = 320) => `第${n}篇の本文。`.repeat(Math.ceil(chars / 7)).slice(0, chars);

function raw(home: string, n: number, patch: Partial<RawArticle> = {}): RawArticle {
  return {
    url: `${home}a/${n}`,
    title: `見出し${n}`,
    published_at: T0 - 3_600_000 + n * 60_000,
    byline: "共同通信",
    blocks: [{ type: "h2", text: `小見出し${n}` }, { type: "p", text: paragraph(n) }],
    comments: [{ author: "tak***", at: T0 - 1000, text: `コメント${n}` }, { text: "二つ目" }, { text: "三つ目" }],
    links: [{ url: `${home}a/${n - 1}`, text: "関連" }, { url: "https://www.meti.go.jp/press/1.html", text: "発表" }],
    ...patch,
  };
}

interface Sent {
  texts: string[];
  write_summary: boolean;
  topics: string[];
}

async function world(id = "news-example-com", options: { def?: Partial<SourceDef>; site?: Partial<Site> } = {}) {
  const sourceDef = def(id, options.def);
  const home = sourceDef.home_url;
  const dir = await tempDir({ "source.json": JSON.stringify(sourceDef), "extract.ts": EXTRACT });
  const data = await tempDir();
  const memory = memoryStore();
  const bucket = new SiteBucket(memory.store, "b");
  const pages: Record<string, Page | (() => Page)> = {};
  const web = cannedFetch(pages);
  const clock = { now: T0 };
  const logs: string[] = [];
  const source = { ...sourceEntry(sourceDef, "s1", "test", T0), state: "active" as const };
  const site: Site = {
    schema: "buckyos.site/1",
    title: "我的阅读",
    language: "zh-CN",
    owner_did: "did:bns:alice",
    topics: ["国际", "科技", "体育"],
    template: { version: "t1", params: {} },
    columns: [],
    sources: [source],
    upgrades: [],
    reading: { enabled: true },
    limits: { translate_tokens_per_day: 1_000_000 },
    maintainer: null,
    updated_at: T0,
    updated_by: "test",
    ...options.site,
  };

  // The model: wraps every text, tags with 科技, and takes `seconds` of the clock per call.
  const model = {
    requests: [] as Sent[],
    seconds: 0,
    tokens: 100,
    fail: null as ((sent: Sent, n: number) => Error | unknown | undefined) | null,
    env: {} as { log?: string; runId?: string; budget?: number },
  };
  const call = (request: ModelRequest) => {
    const sent = JSON.parse(request.user) as Sent;
    model.requests.push(sent);
    clock.now += model.seconds * 1000;
    const override = model.fail?.(sent, model.requests.length);
    if (override instanceof Error) return Promise.reject(override);
    const reply = override ?? {
      translations: sent.texts.map((text) => `译(${text})`),
      tags: ["科技", "不在清单里"],
      summary: sent.write_summary ? "模型写的摘要" : "",
    };
    return Promise.resolve({
      text: JSON.stringify(reply),
      model: "mock-mt",
      usage: { input_tokens: model.tokens - 10, output_tokens: 10, total_tokens: model.tokens },
    });
  };

  const articles = (list: RawArticle[], entries?: unknown[]) => {
    pages[`${home}feed.json`] = JSON.stringify(entries ?? list.map((item) => ({ url: item.url, title: item.title })));
    for (const item of list) pages[item.url] = JSON.stringify(item);
  };
  const sync = (overrides: Partial<SyncEnv> = {}) => {
    const metered = new MeteredModel(call, model.env, () => clock.now);
    return syncSource({
      bucket,
      site,
      source,
      def: sourceDef,
      dir,
      dataDir: data,
      net: new Net({ fetcher: web.fetcher, sleep: () => Promise.resolve(), now: () => clock.now }),
      translator: createTranslator({
        model: metered,
        cacheDir: `${data}/tr`,
        dailyCap: site.limits.translate_tokens_per_day,
        now: () => clock.now,
      }),
      resize: null,
      tokens: () => metered.tokens,
      now: () => clock.now,
      timeLimitMs: 480_000,
      log: (message) => logs.push(message),
      ...overrides,
    });
  };
  const feed = () => memory.json(feedKey(id)) as Feed;
  const article = async (url: string) => memory.json(articleKey((await articleAddress(url)).id)) as Article;
  const local = async (name: string) => JSON.parse(await Deno.readTextFile(`${data}/${name}`));
  const close = async () => {
    await Deno.remove(dir, { recursive: true });
    await Deno.remove(data, { recursive: true });
  };
  return { id, home, dir, data, memory, bucket, pages, web, clock, logs, source, site, model, articles, sync, feed, article, local, close };
}

Deno.test("a sync publishes what it fetched: images, Markdown and article first, the feed last", async () => {
  const w = await world();
  try {
    const image = `${w.home}img/1.png`;
    w.pages[image] = { body: tinyPng(), headers: { "content-type": "image/png" } };
    w.articles([
      raw(w.home, 1, { blocks: [{ type: "img", url: image, alt: "写真" }, { type: "p", text: paragraph(1) }, { type: "table", rows: [["年", "額"]] }] }),
      raw(w.home, 2, { summary: "元の要約", comments: undefined, links: undefined }),
      raw(w.home, 3),
    ]);
    const report = await w.sync();
    assertEquals(
      [report.ok, report.kind, report.metrics.listed, report.metrics.new, report.metrics.published, report.metrics.rejected, report.metrics.images, report.metrics.tokens],
      [true, undefined, 3, 3, 3, 0, 1, 300],
    );
    assertEquals(report.summary, "news-example-com: 3 listed, 3 new, 3 published");
    assertEquals([report.metrics.pending, report.metrics.pending_reason], [0, null]);

    // The order readers depend on.
    const puts = w.memory.log.filter((entry) => entry.startsWith("put "));
    const first = await articleAddress(`${w.home}a/1`);
    const imageKey = (await w.article(`${w.home}a/1`)).thumb!;
    assertEquals(puts.slice(0, 3), [`put ${imageKey}`, `put ${markdownKey(first.id)}`, `put ${articleKey(first.id)}`]);
    assertEquals([puts.at(-1), puts.filter((entry) => entry === `put ${feedKey(w.id)}`).length], [`put ${feedKey(w.id)}`, 1]);
    assertEquals(w.memory.objects.get(imageKey)!.headers, { "cache-control": IMMUTABLE });
    // The bucket's own expiry, for what no job will come back to clean up.
    assertEquals(
      [imageKey, markdownKey(first.id), articleKey(first.id)].map((key) => w.memory.objects.get(key)!.expiresAt),
      Array(3).fill(expiryOf(T0)),
    );
    assertEquals(w.memory.objects.get(feedKey(w.id))!.expiresAt, undefined);

    const feed = w.feed();
    assertEquals(feedProblems(feed), []);
    assertEquals(feed.status, { state: "ok", checked_at: T0, published_at: T0, error: null, pending: null, login_url: null });
    assertEquals(feed.items.map((item) => item.title), ["译(見出し3)", "译(見出し2)", "译(見出し1)"], "newest first");
    assertEquals(feed.items.map((item) => item.summary), ["模型写的摘要", "译(元の要約)", "模型写的摘要"]);
    assertEquals(feed.items.map((item) => [item.image, item.tags, item.rev]), [[null, ["科技"], 1], [null, ["科技"], 1], [imageKey, ["科技"], 1]]);

    const one = await w.article(`${w.home}a/1`);
    assertEquals(articleProblems(one), []);
    assertEquals(
      [one.source, one.lang, one.rev, one.fetched_at, one.title, one.summary, one.byline, one.thumb],
      ["news-example-com", "ja", 1, T0, { o: "見出し1", t: "译(見出し1)" }, { o: "", t: "模型写的摘要" }, "共同通信", imageKey],
    );
    const picture = one.blocks[0] as Extract<Article["blocks"][number], { type: "img" }>;
    assertEquals([picture.src, picture.alt, picture.w, picture.h], [imageKey, { o: "写真", t: "译(写真)" }, 1, 1]);
    assertEquals(one.images, { [picture.object!]: { key: imageKey, w: 1, h: 1 } });
    assertEquals(one.blocks[2], { type: "table", rows: [[{ o: "年", t: "译(年)" }, { o: "額", t: "译(額)" }]] });
    assertEquals(one.comments[0], { author: "tak***", at: T0 - 1000, text: { o: "コメント1", t: "译(コメント1)" } });
    assertEquals(one.comments[1], { author: "", at: null, text: { o: "二つ目", t: "译(二つ目)" } });
    assertEquals(one.object.content, { type: "article", title: "译(見出し1)", summary: "模型写的摘要", cover: picture.object, media: [{ object: picture.object, alt: "译(写真)" }] });
    assertEquals([one.object.publisher, one.object.iat, one.object.tags, one.object.base_on], ["did:bns:alice", Math.floor(T0 / 1000), ["科技"], undefined]);
    assertEquals(one.object.wraps, (await w.memory.store.get("b", markdownKey(first.id))).obj_id);
    assert(w.memory.text(markdownKey(first.id)).startsWith(`# 译(見出し1)\n\n原文：${w.home}a/1\n`), w.memory.text(markdownKey(first.id)));

    // A link to an article the site has gets its id; the third article links to the second.
    const three = await w.article(`${w.home}a/3`);
    assertEquals(three.links.map((link) => link.article), [(await articleAddress(`${w.home}a/2`)).id, undefined]);
    assertEquals([(await w.article(`${w.home}a/2`)).comments, (await w.article(`${w.home}a/2`)).summary], [[], { o: "元の要約", t: "译(元の要約)" }]);

    assertEquals(await w.local("pending.json"), { items: [] });
    const state = await w.local("state.json");
    assertEquals([state.extract, state.listed, state.lengths.length], [{ runs: 1 }, 3, 3]);
    assertEquals(Object.keys(await w.local("index.json")).length, 3);
  } finally {
    await w.close();
  }
});

Deno.test("a sync that finds nothing new makes one request and still says it checked", async () => {
  const w = await world();
  try {
    let modified = true;
    w.articles([raw(w.home, 1)]);
    const list = w.pages[`${w.home}feed.json`] as string;
    w.pages[`${w.home}feed.json`] = () => modified ? { body: list, headers: { etag: '"v1"' } } : { status: 304 };
    await w.sync();
    modified = false;
    w.clock.now += 1_800_000;
    const before = w.web.requests.length;
    const report = await w.sync();
    assertEquals([report.ok, report.metrics.listed, report.metrics.new, report.metrics.published, report.not_modified], [true, 0, 0, 0, 1]);
    assertEquals(w.web.requests.length - before, 1);
    assertEquals(w.web.requests.at(-1)!.headers["if-none-match"], '"v1"');
    assertEquals([w.feed().status.checked_at, w.feed().status.published_at, w.feed().items.length], [T0 + 1_800_000, T0, 1]);
    assertEquals(w.model.requests.length, 1);
  } finally {
    await w.close();
  }
});

Deno.test("the feed is rewritten after every 10 articles, so a long sync shows as it goes", async () => {
  const w = await world();
  try {
    w.articles(Array.from({ length: 12 }, (_, n) => raw(w.home, n + 1)));
    const seen: number[] = [];
    w.memory.hooks.beforePut = (key) => {
      if (key === feedKey(w.id)) seen.push(w.memory.keys("_data/a/").length);
    };
    const report = await w.sync();
    assertEquals([report.metrics.published, seen], [12, [10, 12]]);
    assertEquals(w.feed().status.pending, null);
  } finally {
    await w.close();
  }
});

Deno.test("resume: a sync cut by its time limit publishes the rest next time without fetching it again", async () => {
  const w = await world();
  try {
    w.articles(Array.from({ length: 5 }, (_, n) => raw(w.home, n + 1)));
    w.model.seconds = 200;
    const first = await w.sync({ timeLimitMs: 480_000 });
    assertEquals([first.ok, first.kind, first.metrics.new, first.metrics.published], [true, undefined, 5, 3]);
    assertEquals([first.metrics.pending, first.metrics.pending_reason], [2, "time"]);
    assertEquals(w.feed().items.length, 3, "what was published before the limit is visible");
    const pending = w.feed().status.pending!;
    assertEquals([pending.count, pending.reason], [2, "time"]);
    assertEquals((await w.local("pending.json")).items.map((item: { raw: RawArticle }) => item.raw.title), ["見出し4", "見出し5"]);

    const fetched = w.web.requests.length;
    w.clock.now += 1_800_000;
    w.model.seconds = 0;
    const second = await w.sync();
    assertEquals([second.ok, second.metrics.new, second.metrics.published], [true, 0, 2]);
    assertEquals(w.web.requests.slice(fetched).map((request) => request.url), [`${w.home}feed.json`], "only the list is read again");
    assertEquals([w.feed().items.length, w.feed().status.pending], [5, null]);
    assertEquals((await w.article(`${w.home}a/5`)).fetched_at, T0, "an article keeps the time it was fetched at");
  } finally {
    await w.close();
  }
});

Deno.test("resume: a sync that dies half-way loses nothing and repeats nothing it paid for", async () => {
  const w = await world();
  try {
    w.articles([raw(w.home, 1), raw(w.home, 2), raw(w.home, 3)]);
    const second = await articleAddress(`${w.home}a/2`);
    w.memory.hooks.beforePut = (key) => {
      if (key === articleKey(second.id)) throw new Error("the process was killed here");
    };
    await assertFails(() => w.sync(), "killed here");
    assertEquals(w.memory.keys("_data/a/").length, 1);
    assertEquals(w.memory.objects.has(feedKey(w.id)), false, "the feed was not reached");
    assertEquals((await w.local("pending.json")).items.length, 2);

    w.memory.hooks.beforePut = undefined;
    const asked = w.model.requests.length;
    const report = await w.sync();
    assertEquals([report.ok, report.metrics.published, w.feed().items.length], [true, 2, 3]);
    assertEquals(w.model.requests.length - asked, 1, "the article translated before the crash comes from the cache");
  } finally {
    await w.close();
  }
});

Deno.test("two sources syncing at the same time do not disturb each other", async () => {
  const a = await world("news-a-example");
  const b = await world("news-b-example");
  try {
    // One bucket, one web, two jobs with their own data.
    const shared = { bucket: a.bucket };
    const picture = "https://cdn.example.com/wire.png";
    for (const w of [a, b]) {
      w.pages[picture] = { body: tinyPng(), headers: { "content-type": "image/png" } };
      w.articles(Array.from({ length: 4 }, (_, n) =>
        raw(w.home, n + 1, { blocks: [{ type: "img", url: picture }, { type: "p", text: paragraph(`${w.id}-${n}`) }] })));
    }
    const [ra, rb] = await Promise.all([a.sync(shared), b.sync(shared)]);
    assertEquals([ra.ok, ra.metrics.published, rb.ok, rb.metrics.published], [true, 4, true, 4]);
    const feedA = a.memory.json(feedKey(a.id)) as Feed;
    const feedB = a.memory.json(feedKey(b.id)) as Feed;
    assertEquals([feedA.items.length, feedB.items.length], [4, 4]);
    assert(feedA.items.every((item) => item.url.startsWith(a.home)), "feed a lists only a");
    assert(feedB.items.every((item) => item.url.startsWith(b.home)), "feed b lists only b");
    assertEquals(a.memory.keys("_data/a/").length, 8);
    assertEquals(a.memory.keys("_data/img/").length, 1, "the same picture is one object");
    for (const key of a.memory.keys("_data/a/")) assertEquals(articleProblems(a.memory.json(key)), []);
    assertEquals([Object.keys(await a.local("index.json")).length, Object.keys(await b.local("index.json")).length], [4, 4]);
  } finally {
    await a.close();
    await b.close();
  }
});

Deno.test("status: ok after a run that passed; transient keeps the state, auth asks to sign in, program fails", async () => {
  const w = await world();
  try {
    const list = `${w.home}feed.json`;

    // The very first run cannot reach the site: still preparing, with the error.
    w.pages[list] = { status: 503 };
    let report = await w.sync();
    assertEquals([report.ok, report.kind], [false, "transient"]);
    assertEquals([w.feed().status.state, w.feed().status.error?.kind, w.feed().status.checked_at], ["preparing", "transient", null]);

    w.clock.now += 1000;
    w.articles([raw(w.home, 1), raw(w.home, 2)]);
    report = await w.sync();
    assertEquals([report.ok, w.feed().status.state, w.feed().status.error, w.feed().status.checked_at], [true, "ok", null, T0 + 1000]);

    // The site asks to sign in: needs_login with where to do it; the articles stay.
    w.clock.now += 1000;
    w.pages[list] = { status: 401 };
    report = await w.sync();
    assertEquals([report.ok, report.kind, report.summary], [false, "auth", "news-example-com: auth: the list asks to sign in"]);
    let status = w.feed().status;
    assertEquals([status.state, status.login_url, status.error?.kind, status.checked_at, w.feed().items.length], ["needs_login", w.home, "auth", T0 + 1000, 2]);
    assertEquals(feedProblems(w.feed()), []);

    // A hiccup while it needs login changes nothing but the error.
    w.clock.now += 1000;
    w.pages[list] = { status: 500 };
    await w.sync();
    assertEquals([w.feed().status.state, w.feed().status.login_url, w.feed().status.error?.kind], ["needs_login", w.home, "transient"]);

    // The user signed in: back to ok by itself.
    w.clock.now += 1000;
    w.articles([raw(w.home, 1), raw(w.home, 2), raw(w.home, 3)]);
    report = await w.sync();
    status = w.feed().status;
    assertEquals([report.ok, status.state, status.login_url, status.error, status.checked_at, w.feed().items.length], [true, "ok", null, null, T0 + 4000, 3]);

    // The extractor breaks (more than half of the run fails the check): failed, nothing published, nothing advanced.
    w.clock.now += 1000;
    const state = await w.local("state.json");
    w.articles([raw(w.home, 4), raw(w.home, 5, { blocks: [{ type: "p", text: "短い" }] }), raw(w.home, 6, { blocks: [] })]);
    report = await w.sync();
    assertEquals([report.ok, report.kind, report.metrics.rejected, report.metrics.published], [false, "program", 2, 0]);
    status = w.feed().status;
    assertEquals([status.state, status.error?.kind, status.checked_at, w.feed().items.length], ["failed", "program", T0 + 4000, 3]);
    assert(status.error!.message.startsWith("2 of 3 articles did not pass the check"), status.error!.message);
    assertEquals(await w.local("state.json"), state, "a failed run leaves the job's state as it was");
    assertEquals((await w.local("pending.json")).items, []);

    // A hiccup after that: still failed.
    w.clock.now += 1000;
    w.pages[list] = { status: 502 };
    await w.sync();
    assertEquals([w.feed().status.state, w.feed().status.error?.kind], ["failed", "transient"]);

    // Fixed: ok again; the article that was fine all along is published now.
    w.clock.now += 1000;
    w.articles([raw(w.home, 4), raw(w.home, 5), raw(w.home, 6)]);
    report = await w.sync();
    assertEquals([report.ok, report.metrics.published, w.feed().status.state, w.feed().items.length], [true, 3, "ok", 6]);

    // The list had entries and is empty now: failed.
    w.clock.now += 1000;
    w.pages[list] = "[]";
    report = await w.sync();
    assertEquals([report.kind, w.feed().status.state], ["program", "failed"]);
    assertEquals(w.feed().status.error?.message, "the list had 3 entries last time and is empty now");
  } finally {
    await w.close();
  }
});

Deno.test("status: one bad article is left out, remembered for a day, and the run passes", async () => {
  const w = await world();
  try {
    w.articles([raw(w.home, 1), raw(w.home, 2, { title: "" }), raw(w.home, 3)]);
    const report = await w.sync();
    assertEquals([report.ok, report.metrics.published, report.metrics.rejected], [true, 2, 1]);
    assertEquals(report.rejected, [{ url: `${w.home}a/2`, title: "", reason: "the title is empty" }]);
    assertEquals(w.feed().status.state, "ok");

    const before = w.web.requests.length;
    w.clock.now += 1_800_000;
    await w.sync();
    assertEquals(w.web.requests.slice(before).map((request) => request.url), [`${w.home}feed.json`], "not fetched again the same day");
    w.clock.now += DAY;
    w.pages[`${w.home}a/2`] = JSON.stringify(raw(w.home, 2));
    const later = await w.sync();
    assertEquals(later.metrics.published, 1, "a day later it gets another chance");
  } finally {
    await w.close();
  }
});

Deno.test("translation refused by the budget: nothing untranslated is published, the next day goes on", async () => {
  const w = await world("news-example-com", { site: { limits: { translate_tokens_per_day: 250 } } });
  try {
    w.model.env = { log: `${w.data}/usage.jsonl`, runId: "run-1" };
    w.articles([raw(w.home, 1), raw(w.home, 2), raw(w.home, 3), raw(w.home, 4)]);
    const report = await w.sync();
    assertEquals([report.ok, report.kind, report.metrics.published, report.metrics.tokens], [true, "budget", 3, 300]);
    assertEquals([report.metrics.pending, report.metrics.pending_reason], [1, "translate_budget"], "what a check script sees of it");
    const status = w.feed().status;
    assertEquals([status.state, status.error, status.pending?.count, status.pending?.reason], ["ok", null, 1, "translate_budget"]);
    assert(status.pending!.message.includes("300 of 250"), status.pending!.message);
    assertEquals(w.feed().items.length, 3);
    assertEquals(
      (await Deno.readTextFile(`${w.data}/usage.jsonl`)).trimEnd().split("\n").map((line) => JSON.parse(line)),
      Array(3).fill({ at_ms: T0, model: "mock-mt", run_id: "run-1", input_tokens: 90, output_tokens: 10, total_tokens: 100 }),
    );

    // Later the same day: still refused, no model call.
    w.clock.now += 3_600_000;
    w.model.env.runId = "run-2";
    const asked = w.model.requests.length;
    const same = await w.sync();
    assertEquals([same.kind, same.metrics.published, w.model.requests.length - asked], ["budget", 0, 0]);

    // The next day (UTC) the count starts again.
    w.clock.now += DAY;
    w.model.env.runId = "run-3";
    const next = await w.sync();
    assertEquals([next.ok, next.kind, next.metrics.published, w.feed().items.length, w.feed().status.pending], [true, undefined, 1, 4, null]);
    assertEquals([next.metrics.pending, next.metrics.pending_reason], [0, null]);

    // $OPENDAN_MODEL_BUDGET of the run.
    w.clock.now += 1000;
    w.model.env = { log: `${w.data}/usage.jsonl`, runId: "run-4", budget: 0 };
    w.articles([raw(w.home, 5)]);
    const none = await w.sync();
    assertEquals([none.ok, none.kind, none.metrics.published, w.feed().status.pending?.reason], [true, "budget", 0, "translate_budget"]);
  } finally {
    await w.close();
  }
});

Deno.test("translation refused for want of a model: a configuration problem on the status, no retry in the run", async () => {
  const w = await world();
  try {
    w.articles([raw(w.home, 1), raw(w.home, 2)]);
    w.model.fail = () => new ModelError("config", "llm.translate: no_candidate_model: no model is mapped", "no_candidate_model");
    const report = await w.sync();
    assertEquals([report.ok, report.kind, report.metrics.published, w.model.requests.length], [true, "budget", 0, 1]);
    assertEquals([report.metrics.pending, report.metrics.pending_reason], [2, "translate_config"]);
    const status = w.feed().status;
    assertEquals([status.state, status.error?.kind, status.pending?.count, status.pending?.reason], ["ok", "config", 2, "translate_config"]);
    assertEquals(feedProblems(w.feed()), []);

    w.clock.now += 1_800_000;
    w.model.fail = null;
    const fixed = await w.sync();
    assertEquals([fixed.metrics.published, w.feed().status.error, w.feed().status.pending], [2, null, null]);
  } finally {
    await w.close();
  }
});

Deno.test("a translation with the wrong length is left for the next sync, and given up after three syncs", async () => {
  const w = await world();
  try {
    w.articles([raw(w.home, 1), raw(w.home, 2)]);
    // The model keeps merging two paragraphs of the first article.
    w.model.fail = (sent) => sent.texts.includes("見出し1") ? { translations: sent.texts.slice(1), tags: [], summary: "" } : undefined;
    const report = await w.sync();
    assertEquals([report.ok, report.kind, report.metrics.published, w.model.requests.length], [true, undefined, 1, 3]);
    assertEquals([report.metrics.pending, report.metrics.pending_reason], [1, "translate_failed"]);
    assertEquals([w.feed().status.pending?.count, w.feed().status.pending?.reason, w.feed().items.length], [1, "translate_failed", 1]);
    assert(w.logs.some((line) => line.includes("twice") && line.includes("left for the next sync")), w.logs.join("\n"));

    w.clock.now += 1_800_000;
    await w.sync();
    w.clock.now += 1_800_000;
    const third = await w.sync();
    assertEquals([third.metrics.rejected, w.feed().status.pending, (await w.local("pending.json")).items.length], [1, null, 0]);
    assertEquals(third.rejected[0].url, `${w.home}a/1`);
  } finally {
    await w.close();
  }
});

Deno.test("images: one that does not come waits a sync, then the article is published without it", async () => {
  const w = await world();
  try {
    const good = `${w.home}img/good.png`;
    const bad = `${w.home}img/bad.png`;
    w.pages[good] = { body: tinyPng(), headers: { "content-type": "image/png" } };
    w.articles([
      raw(w.home, 1, { blocks: [{ type: "img", url: bad, alt: "来ない" }, { type: "img", url: good }, { type: "p", text: paragraph(1) }] }),
      raw(w.home, 2),
    ]);
    const first = await w.sync();
    assertEquals([first.ok, first.metrics.published, first.metrics.images], [true, 1, 1]);
    assertEquals([first.metrics.pending, first.metrics.pending_reason], [1, "images"]);
    assertEquals([w.feed().status.pending?.count, w.feed().status.pending?.reason], [1, "images"]);
    assertEquals(w.model.requests.length, 1, "an article that waits for an image is not translated yet");

    w.clock.now += 1_800_000;
    const before = w.web.requests.length;
    const second = await w.sync();
    assertEquals([second.metrics.published, second.metrics.images, w.feed().status.pending], [1, 0, null]);
    assertEquals(
      w.web.requests.slice(before).map((request) => request.url),
      [`${w.home}feed.json`, bad],
      "the image that was stored is not fetched again",
    );
    const article = await w.article(`${w.home}a/1`);
    assertEquals(article.blocks[0], { type: "img", src: null, object: null, alt: { o: "来ない", t: "译(来ない)" }, w: 0, h: 0, missing: true });
    const kept = article.blocks[1] as Extract<Article["blocks"][number], { type: "img" }>;
    assertEquals([kept.src === article.thumb, kept.missing, Object.keys(article.images).length], [true, undefined, 1]);
    assertEquals(article.object.content.cover, kept.object);
    assertEquals(second.published[0].missing_images, 1);

    // site try publishes at once what it cannot get.
    const t = await world("news-try-example");
    try {
      t.articles([raw(t.home, 1, { blocks: [{ type: "img", url: bad }, { type: "p", text: paragraph(1) }] })]);
      const tried = await t.sync({ imageTries: 1 });
      assertEquals([tried.metrics.published, tried.published[0].missing_images, (await t.article(`${t.home}a/1`)).thumb], [1, 1, null]);
    } finally {
      await t.close();
    }
  } finally {
    await w.close();
  }
});

Deno.test("images: what was fetched and is no picture is left out at once, and the article is published in the same sync", async () => {
  const w = await world();
  try {
    const good = `${w.home}img/good.png`;
    const icon = `${w.home}img/icon?id=linkedin`;
    const svgByName = "https://cdn.example.com/LinkIcons/svg/github.svg";
    const errorPage = `${w.home}img/gone.jpg`;
    const ico = `${w.home}favicon.ico`;
    w.pages[good] = { body: tinyPng(), headers: { "content-type": "image/png" } };
    w.pages[icon] = { body: "<svg xmlns='http://www.w3.org/2000/svg'><path d='M0 0h24v24H0z'/></svg>", headers: { "content-type": "image/svg+xml" } };
    w.pages[svgByName] = { body: "<svg xmlns='http://www.w3.org/2000/svg'/>", headers: { "content-type": "image/svg+xml" } };
    w.pages[errorPage] = { body: "<!doctype html><title>Not found</title>", headers: { "content-type": "text/html" } };
    w.pages[ico] = { body: Uint8Array.from([0, 0, 1, 0, 1, 0, 16, 16]), headers: { "content-type": "image/x-icon" } };
    w.articles([raw(w.home, 1, {
      blocks: [
        { type: "img", url: icon, alt: "LinkedIn" },
        { type: "p", text: paragraph(1) },
        { type: "img", url: svgByName },
        { type: "img", url: good, alt: "写真" },
        { type: "img", url: errorPage },
        { type: "img", url: ico },
      ],
    })]);
    const report = await w.sync();
    assertEquals([report.ok, report.metrics.published, report.metrics.images, report.metrics.pending, w.feed().status.pending], [true, 1, 1, 0, null]);
    assertEquals(report.published[0].missing_images, 0);
    const article = await w.article(`${w.home}a/1`);
    assertEquals(article.blocks.map((block) => block.type), ["p", "img"], "no block and no placeholder for what was left out");
    const kept = article.blocks[1] as Extract<Article["blocks"][number], { type: "img" }>;
    assertEquals([kept.alt.o, kept.missing, kept.src === article.thumb, Object.keys(article.images).length], ["写真", undefined, true, 1]);
    assertEquals(articleProblems(article), []);
    // The texts still line up around the pictures that were left out.
    assertEquals([article.title.t, (article.blocks[0] as { text: { t: string } }).text.t.startsWith("译(第1篇")], ["译(見出し1)", true]);
    assertEquals(w.memory.keys("_data/img/").length, 1);
    assertEquals(w.logs.filter((line) => line.startsWith("image ")), [
      `image ${icon}: not a JPEG, PNG, GIF, WebP or AVIF image — left out of the article`,
      `image ${svgByName}: an SVG is not a picture the site keeps — left out of the article`,
      `image ${errorPage}: not a JPEG, PNG, GIF, WebP or AVIF image — left out of the article`,
      `image ${ico}: not a JPEG, PNG, GIF, WebP or AVIF image — left out of the article`,
    ]);
    assertEquals(w.web.requests.filter((request) => request.url === svgByName).length, 0, "an address that ends in .svg is not asked for");
    assertEquals((await w.local("pending.json")).items, []);
  } finally {
    await w.close();
  }
});

Deno.test("images: a picture that could not be fetched still waits one sync; one that came and cannot be kept does not", async () => {
  const w = await world();
  try {
    const gone = `${w.home}img/gone.png`;
    const down = "https://cdn.example.com/photo.jpg";
    const huge = `${w.home}img/huge.png`;
    w.pages[down] = { status: 503 };
    const big = new Uint8Array(2 * 1024 * 1024 + 1);
    big.set(tinyPng());
    w.pages[huge] = { body: big, headers: { "content-type": "image/png" } };
    // Article 1: a 404 and a server error. Article 2: a real PNG too large to keep without ImageMagick.
    w.articles([
      raw(w.home, 1, { blocks: [{ type: "img", url: gone }, { type: "img", url: down }, { type: "p", text: paragraph(1) }] }),
      raw(w.home, 2, { blocks: [{ type: "img", url: huge, alt: "大きい" }, { type: "p", text: paragraph(2) }] }),
    ]);
    const first = await w.sync();
    assertEquals([first.metrics.published, first.metrics.pending, first.metrics.pending_reason], [1, 1, "images"]);
    assertEquals(w.logs.filter((line) => line.startsWith("image ")), [
      `image ${gone}: ${gone}: HTTP 404 (try 1 of 2)`,
      `image ${down}: ${down}: HTTP 503 (try 1 of 2)`,
      `image ${huge}: 2097153 bytes, and without ImageMagick an image is kept only up to 2097152 — published as missing`,
    ]);
    const second = await w.article(`${w.home}a/2`);
    assertEquals(second.blocks[0], { type: "img", src: null, object: null, alt: { o: "大きい", t: "译(大きい)" }, w: 0, h: 0, missing: true });

    w.clock.now += 1_800_000;
    const next = await w.sync();
    assertEquals([next.metrics.published, next.metrics.pending, next.published[0].missing_images], [1, 0, 2]);
    assertEquals((await w.article(`${w.home}a/1`)).blocks.map((block) => (block as { missing?: boolean }).missing), [true, true, undefined]);
  } finally {
    await w.close();
  }
});

Deno.test("images: with ImageMagick the article gets the shrunk picture and the list a smaller one", async () => {
  const w = await world();
  try {
    const picture = `${w.home}img/big.png`;
    w.pages[picture] = { body: tinyPng(), headers: { "content-type": "image/png" } };
    w.articles([raw(w.home, 1, { blocks: [{ type: "img", url: picture }, { type: "img", url: picture }, { type: "p", text: paragraph(1) }] })]);
    const resize: Resize = (_bytes, _from, _to, edge) => Promise.resolve(tinyPng(edge === 1280 ? 1 : 2));
    const report = await w.sync({ resize });
    const article = await w.article(`${w.home}a/1`);
    const [first, second] = article.blocks as Extract<Article["blocks"][number], { type: "img" }>[];
    assertEquals([report.metrics.images, first.src === second.src, article.thumb !== first.src, w.memory.keys("_data/img/").length], [1, true, true, 2]);
    assertEquals(w.feed().items[0].image, article.thumb);
    assertEquals(w.web.requests.filter((request) => request.url === picture)[0].headers.referer, `${w.home}a/1`);
  } finally {
    await w.close();
  }
});

Deno.test("a corrected article is a new revision based on the one before; an unchanged one is left alone", async () => {
  const w = await world();
  try {
    w.articles([raw(w.home, 1)]);
    await w.sync();
    const before = await w.article(`${w.home}a/1`);

    // The extractor fetches it again and it is the same: nothing is written.
    w.clock.now += 1_800_000;
    w.articles([raw(w.home, 1)], [{ url: `${w.home}a/1`, title: "見出し1", again: true }]);
    const puts = w.memory.log.length;
    const same = await w.sync();
    assertEquals([same.metrics.new, same.metrics.published], [1, 0]);
    assertEquals(w.memory.log.slice(puts), [`put ${feedKey(w.id)}`]);

    // A paragraph was corrected.
    w.clock.now += 1_800_000;
    const corrected = raw(w.home, 1);
    corrected.blocks = [{ type: "h2", text: "小見出し1" }, { type: "p", text: `${paragraph(1)}（訂正）` }];
    w.articles([corrected], [{ url: `${w.home}a/1`, title: "見出し1", again: true }]);
    const asked = w.model.requests.length;
    const report = await w.sync();
    const after = await w.article(`${w.home}a/1`);
    assertEquals([report.metrics.published, after.rev, after.object.base_on, after.fetched_at], [1, 2, before.object_id, T0 + 3_600_000]);
    assert(after.object_id !== before.object_id, "a correction is a new object");
    assertEquals(w.model.requests.slice(asked).map((sent) => sent.texts), [[`${paragraph(1)}（訂正）`]], "only the changed block is translated");
    assertEquals([w.feed().items.length, w.feed().items[0].rev, w.feed().items[0].object_id], [1, 2, after.object_id]);
  } finally {
    await w.close();
  }
});

Deno.test("clean-up: articles past their keep time go with their Markdown and the images nobody else uses", async () => {
  const w = await world("news-example-com", { def: { keep_days: 30 } });
  try {
    const own = `${w.home}img/own.png`;
    const shared = `${w.home}img/shared.png`;
    w.pages[own] = { body: tinyPng(1), headers: { "content-type": "image/png" } };
    w.pages[shared] = { body: tinyPng(2), headers: { "content-type": "image/png" } };
    const picture = (n: number, url: string) =>
      raw(w.home, n, { blocks: [{ type: "img", url }, { type: "p", text: paragraph(n) }] });
    // Day 0: three articles; 1 has its own picture, 2 shares one with an article of day 20, 3 is read, 4 is in an event.
    w.articles([picture(1, own), picture(2, shared), raw(w.home, 3), raw(w.home, 4)]);
    await w.sync();
    const ids = await Promise.all([1, 2, 3, 4, 5].map(async (n) => (await articleAddress(`${w.home}a/${n}`)).id));
    const ownKey = (await w.article(`${w.home}a/1`)).thumb!;
    const sharedKey = (await w.article(`${w.home}a/2`)).thumb!;
    assertEquals((await w.local("state.json")).cleaned_at, T0);

    w.clock.now = T0 + 20 * DAY;
    w.articles([picture(1, own), picture(2, shared), raw(w.home, 3), raw(w.home, 4), picture(5, shared)]);
    await w.sync();
    assertEquals(w.feed().items.length, 5);
    assertEquals(
      [w.memory.objects.get(ownKey)!.expiresAt, w.memory.objects.get(sharedKey)!.expiresAt],
      [expiryOf(T0), expiryOf(T0 + 20 * DAY)],
      "a picture that is used again expires later",
    );

    await w.memory.write(READING_KEY, JSON.stringify({ schema: "buckyos.site.reading/1", updated_at: 0, since: 0, articles: { [ids[2]]: { source: w.id, opens: 1, dwell_ms: 1, max_scroll: 1, last_opened_at: 1 } }, sources: {}, tags: {} }));
    await w.memory.write(EVENTS_KEY, JSON.stringify({ schema: "buckyos.site.events/1", events: [{ id: "e-1", title: "事件", summary: "", articles: [{ id: ids[3], adds: "", title: "译(見出し4)", source: w.id, at: T0 }], supplements: [], updated_at: 1 }], updated_at: 1 }));

    // Day 31: the first two are past 30 days; the read one and the one in an event stay 180 days.
    w.clock.now = T0 + 31 * DAY;
    const log = w.memory.log.length;
    const report = await w.sync();
    assertEquals(report.removed, 5);
    assertEquals(
      w.memory.log.slice(log).filter((entry) => entry.startsWith("delete ")).sort(),
      [articleKey(ids[0]), articleKey(ids[1]), markdownKey(ids[0]), markdownKey(ids[1]), ownKey].map((key) => `delete ${key}`).sort(),
    );
    const feedPut = w.memory.log.slice(log).indexOf(`put ${feedKey(w.id)}`);
    assertEquals(w.memory.log.slice(log).findIndex((entry) => entry.startsWith("delete ")) > feedPut, true, "the feed stops listing them first");
    assertEquals(w.feed().items.map((item) => item.id).sort(), [ids[2], ids[3], ids[4]].sort());
    assertEquals(w.memory.objects.has(sharedKey), true, "a picture an article still uses stays");
    const index = await w.local("index.json");
    assertEquals(Object.keys(index).filter((id) => !index[id].gone).sort(), [ids[2], ids[3], ids[4]].sort());
    assertEquals((await Deno.stat(`${w.data}/tr/${ids[0]}.json`).catch(() => null)), null, "its translations go too");

    // Once a day: nothing more is looked at the same day. The source still lists the two that went: they are not fetched anew.
    w.clock.now += 3_600_000;
    const sameDay = await w.sync();
    assertEquals([sameDay.removed, sameDay.metrics.new, sameDay.metrics.published], [0, 0, 0]);

    // Day 181: the kept ones go; the article of day 20 and the shared picture went on day 51.
    w.clock.now = T0 + 51 * DAY;
    assertEquals((await w.sync()).removed, 3);
    assertEquals(w.memory.objects.has(sharedKey), false);
    w.clock.now = T0 + 181 * DAY;
    assertEquals((await w.sync()).removed, 4);
    assertEquals([w.memory.keys("_data/a/"), w.memory.keys("_data/md/"), w.memory.keys("_data/img/"), w.feed().items], [[], [], [], []]);
    assertEquals(w.feed().status.state, "ok");

    // Once the source no longer lists an article, nothing is remembered of it.
    w.clock.now += DAY;
    w.pages[`${w.home}feed.json`] = JSON.stringify([{ url: `${w.home}a/5`, title: "見出し5" }]);
    await w.sync();
    assertEquals(Object.keys(await w.local("index.json")), [ids[4]]);
  } finally {
    await w.close();
  }
});

Deno.test("clean-up removes at most 500 objects a day", async () => {
  const w = await world("news-example-com", { def: { max_new: 100 } });
  try {
    for (let batch = 0; batch < 3; batch += 1) {
      w.articles(Array.from({ length: 100 }, (_, n) => raw(w.home, batch * 100 + n + 1, { comments: [], links: [] })));
      await w.sync({ translator: null });
      w.clock.now += 1000;
    }
    assertEquals(w.memory.keys("_data/a/").length, 300);
    w.clock.now = T0 + 40 * DAY;
    w.pages[`${w.home}feed.json`] = JSON.stringify([{ url: `${w.home}a/300`, title: "見出し300" }]);
    assertEquals((await w.sync({ translator: null })).removed, 500);
    assertEquals(w.memory.keys("_data/a/").length, 50);
    w.clock.now += DAY;
    assertEquals((await w.sync({ translator: null })).removed, 100);
  } finally {
    await w.close();
  }
});

Deno.test("without translation the original is published as it is; the site's comment limit applies", async () => {
  const w = await world();
  try {
    w.source.comments = 2;
    w.articles([raw(w.home, 1)]);
    const report = await w.sync({ translator: null });
    const article = await w.article(`${w.home}a/1`);
    assertEquals([report.metrics.tokens, article.title, article.tags, article.comments.length], [0, { o: "見出し1", t: "見出し1" }, [], 2]);
    assertEquals(articleProblems(article), []);

    // comments: 0 reaches the extractor as ctx.source.comments.
    w.source.comments = 0;
    w.clock.now += 1000;
    w.articles([raw(w.home, 2)]);
    await w.sync({ translator: null });
    assertEquals((await w.article(`${w.home}a/2`)).comments, []);
  } finally {
    await w.close();
  }
});

Deno.test("the sample source packages publish their fixtures", async () => {
  const sources = new URL("../../site-sources/", import.meta.url);
  const expected: Record<string, { articles: number; comments: number; minChars: number }> = {
    "news-yahoo-co-jp": { articles: 3, comments: 10, minChars: 400 },
    "news-web-nhk": { articles: 3, comments: 0, minChars: 40 },
    "news-ycombinator-com": { articles: 3, comments: 5, minChars: 400 },
  };
  for (const [id, want] of Object.entries(expected)) {
    const dir = decodeURIComponent(new URL(id, sources).pathname);
    const { def: sourceDef } = await readSourcePackage(dir);
    assertEquals(sourceDef.id, id);
    const index = JSON.parse(await Deno.readTextFile(`${dir}/fixtures/index.json`)) as Record<string, string>;
    const pages: Record<string, Page> = {};
    for (const [url, file] of Object.entries(index)) pages[url] = await Deno.readTextFile(`${dir}/fixtures/${file}`);
    const { fetcher } = cannedFetch(pages);
    const memory = memoryStore();
    const data = await tempDir();
    try {
      const source = { ...sourceEntry(sourceDef, "s1", "test", T0), state: "active" as const };
      const report = await syncSource({
        bucket: new SiteBucket(memory.store, "b"),
        site: { owner_did: "did:bns:alice", language: "zh-CN", topics: [], limits: { translate_tokens_per_day: 0 } } as unknown as Site,
        source,
        def: sourceDef,
        dir,
        dataDir: data,
        net: new Net({ fetcher, sleep: () => Promise.resolve() }),
        translator: null,
        resize: null,
        timeLimitMs: 480_000,
        limit: 3,
        imageTries: 1,
        cleanup: false,
        now: () => T0,
        log: () => {},
      });
      assertEquals([report.ok, report.metrics.published, report.rejected], [true, want.articles, []], `${id}: ${report.summary}`);
      for (const published of report.published) {
        assert(published.chars >= want.minChars, `${id}: ${published.url} has ${published.chars} characters`);
        assert(published.comments >= want.comments, `${id}: ${published.url} has ${published.comments} comments`);
        const article = memory.json(articleKey(published.id)) as Article;
        assertEquals(articleProblems(article), [], published.url);
        assert(article.title.o.trim() !== "" && sourceDef.match.some((rule) => new URL(article.url).hostname === rule.host), article.url);
      }
      assertEquals(feedProblems(memory.json(feedKey(id))), []);
    } finally {
      await Deno.remove(data, { recursive: true });
    }
  }
});

Deno.test("a supplement is an article of its own, entered in its event", async () => {
  const w = await world();
  try {
    const EVENTS = { schema: "buckyos.site.events/1", events: [{ id: "e-20261124-chip-export", title: "半导体设备出口新规", summary: "", articles: [], supplements: [], updated_at: 1 }], updated_at: 1 };
    await w.memory.write(EVENTS_KEY, JSON.stringify(EVENTS));
    const picture = "https://example.org/blog/chart.png";
    w.pages[picture] = { body: tinyPng(), headers: { "content-type": "image/png" } };
    const found: RawArticle = {
      url: "https://example.org/blog/chip-export-rules-explained#top",
      title: "Chip export rules, explained",
      lang: "en",
      byline: "example.org",
      blocks: [{ type: "img", url: picture, alt: "A chart" }, { type: "p", text: "The licence covers three kinds of equipment." }],
    };
    const metered = new MeteredModel((request: ModelRequest) => {
      const sent = JSON.parse(request.user) as Sent;
      return Promise.resolve({ text: JSON.stringify({ translations: sent.texts.map((text) => `译(${text})`), tags: ["科技"], summary: "逐条解释" }), model: "mock-mt", usage: null });
    }, {});
    const put = () =>
      putSupplement({
        bucket: w.bucket,
        site: w.site,
        raw: found,
        event: "e-20261124-chip-export",
        adds: "逐条解释了许可的适用范围",
        net: new Net({ fetcher: w.web.fetcher, sleep: () => Promise.resolve() }),
        translator: createTranslator({ model: metered, cacheDir: `${w.data}/tr`, dailyCap: 1_000_000 }),
        resize: null,
        now: T0,
        log: () => {},
      });
    const { article, images } = await put();
    assertEquals(articleProblems(article), []);
    assertEquals(
      [article.source, article.supplement, article.url, article.lang, article.title, article.summary.t, article.rev, images, article.thumb !== null],
      ["_supplement", { event: "e-20261124-chip-export", adds: "逐条解释了许可的适用范围" }, "https://example.org/blog/chip-export-rules-explained", "en", { o: "Chip export rules, explained", t: "译(Chip export rules, explained)" }, "逐条解释", 1, 1, true],
    );
    const events = w.memory.json(EVENTS_KEY) as Events;
    assertEquals(events.events[0].supplements, [{ article: article.id, source_url: article.url, adds: "逐条解释了许可的适用范围", title: "译(Chip export rules, explained)", at: T0 }]);
    assertEquals(w.memory.objects.get(articleKey(article.id))!.expiresAt, expiryOf(T0));
    assertEquals([events.events[0].updated_at, events.updated_at], [T0, T0]);
    assertEquals(w.memory.objects.has(feedKey("_supplement")), false, "a supplement is in no feed");

    // Put again after a correction: the same entry, the next revision.
    found.blocks.push({ type: "p", text: "A second paragraph." });
    const again = await put();
    assertEquals([again.article.rev, again.article.object.base_on, (w.memory.json(EVENTS_KEY) as Events).events[0].supplements.length], [2, article.object_id, 1]);

    await assertFails(() => putSupplement({ bucket: w.bucket, site: w.site, raw: found, event: "e-none", adds: "", net: new Net(), translator: null, resize: null, log: () => {} }), "no event e-none");
    await assertFails(() => putSupplement({ bucket: w.bucket, site: w.site, raw: { ...found, title: " " }, event: "e-20261124-chip-export", adds: "", net: new Net(), translator: null, resize: null, log: () => {} }), "title is empty");
    // What a source already has is not a supplement.
    w.articles([raw(w.home, 1)]);
    await w.sync();
    await assertFails(() => putSupplement({ bucket: w.bucket, site: w.site, raw: raw(w.home, 1), event: "e-20261124-chip-export", adds: "", net: new Net(), translator: null, resize: null, log: () => {} }), "of news-example-com already");
  } finally {
    await w.close();
  }
});
