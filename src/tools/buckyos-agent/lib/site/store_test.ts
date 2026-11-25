import { reflowAddress } from "../reflow.ts";
import {
  articleAddress,
  articleProblems,
  displayState,
  emptyEvents,
  eventsProblems,
  expiryOf,
  feedProblems,
  IMMUTABLE,
  isReadRecord,
  isSourceId,
  SiteBucket,
  siteProblems,
  sourceDefProblems,
  sourceOfUrl,
} from "./store.ts";
import { assert, assertEquals, assertFails, fixture, memoryStore } from "./testing.ts";
import { articleKey, EVENTS_KEY, SITE_KEY, SiteError } from "./types.ts";
import type { Article, Events, Feed, Site } from "./types.ts";

const SITE = await fixture("site/site.json") as Site;
const FEED = await fixture("site/feed.json") as Feed;
const ARTICLE = await fixture("site/article.json") as Record<string, unknown>;
const clone = <T>(value: T): T => structuredClone(value);

function one(problems: string[], includes: string, what: unknown): void {
  if (problems.length !== 1 || !problems[0].includes(includes)) {
    throw new Error(`${JSON.stringify(what)}: expected one problem with "${includes}", got ${JSON.stringify(problems)}`);
  }
}

Deno.test("the shared samples are valid", async () => {
  assertEquals(siteProblems(SITE), []);
  assertEquals(feedProblems(FEED), []);
  assertEquals(articleProblems(ARTICLE), []);
  assertEquals(eventsProblems(await fixture("site/events.json")), []);
  for (const record of await fixture("site/read_batch.json") as unknown[]) assertEquals(isReadRecord(record), true);
});

Deno.test("an article id is the page of phase 1: same algorithm, same vectors", async () => {
  const vectors = await fixture("reflow_vectors.json") as { normalize: { url: string; normalized: string; page: string }[] };
  for (const vector of vectors.normalize) {
    assertEquals(await articleAddress(vector.url), { url: vector.normalized, id: vector.page }, vector.url);
    assertEquals((await reflowAddress(vector.url)).page, vector.page);
  }
  assertEquals((await articleAddress(ARTICLE.url as string)).id, ARTICLE.id);
  await assertFails(() => articleAddress("ftp://example.com/a"), "http(s)");
});

Deno.test("a source id is 1-48 of a-z, 0-9 and dashes, not at the ends", () => {
  for (const id of ["a", "news-yahoo-co-jp", "a".repeat(48), "0-9"]) assertEquals(isSourceId(id), true, id);
  for (const id of ["", "-a", "a-", "A", "a_b", "a.b", "a".repeat(49), "_supplement"]) assertEquals(isSourceId(id), false, id);
});

Deno.test("site.json is checked field by field", () => {
  const bad: [(site: Site) => void, string][] = [
    [(s) => (s as unknown as { schema: string }).schema = "buckyos.site/2", "schema"],
    [(s) => s.title = " ", "title"],
    [(s) => s.owner_did = "alice", "owner_did"],
    [(s) => s.topics = Array.from({ length: 17 }, (_, i) => `t${i}`), "topics"],
    [(s) => s.topics = ["a", "a"], "topics"],
    [(s) => s.template.version = "3", "template.version"],
    [(s) => s.template.params.home_order = "random" as "time", "home_order"],
    [(s) => s.template.params.per_column = 0, "per_column"],
    [(s) => s.sources[0].id = "News", "sources[0].id"],
    [(s) => s.sources[0].job = "job-1", "sources[0].job"],
    [(s) => s.sources[0].version = "v1", "sources[0].version"],
    [(s) => s.sources[0].every_secs = 299, "sources[0].every_secs"],
    [(s) => s.sources[0].comments = 11, "sources[0].comments"],
    [(s) => s.sources[0].match = [{ host: "News.Yahoo.co.jp" }], "sources[0].match[0].host"],
    [(s) => s.sources[0].match = [{ host: "news.yahoo.co.jp", path: "x" }], "sources[0].match[0].path"],
    [(s) => s.sources[0].state = "ok" as "active", "sources[0].state"],
    [(s) => s.sources[1].id = s.sources[0].id, "listed twice"],
    [(s) => s.upgrades[0].state = "done" as "open", "upgrades[0].state"],
    [(s) => (s.reading as unknown as { enabled: string }).enabled = "yes", "reading.enabled"],
    [(s) => s.limits.translate_tokens_per_day = -1, "limits"],
    [(s) => s.maintainer = { agent: "", goal_id: "g" }, "maintainer"],
    [(s) => s.updated_at = 1.5, "updated_at"],
  ];
  for (const [change, field] of bad) {
    const site = clone(SITE);
    change(site);
    const problems = siteProblems(site);
    // The duplicate id also breaks the job name of that entry.
    if (field === "listed twice") assert(problems.some((problem) => problem.includes(field)), JSON.stringify(problems));
    else one(problems, field, field);
  }
  const noMaintainer = clone(SITE);
  noMaintainer.maintainer = null;
  noMaintainer.sources[0].version = null;
  assertEquals(siteProblems(noMaintainer), []);
  assertEquals(siteProblems([]), ["site.json is not a JSON object"]);
});

Deno.test("a feed is checked field by field", () => {
  const bad: [(feed: Feed) => void, string][] = [
    [(f) => f.source = "_x", "source"],
    [(f) => f.status.state = "paused" as "ok", "status.state"],
    [(f) => f.status.checked_at = "now" as unknown as number, "checked_at"],
    [(f) => f.status.error = { kind: "budget" as "auth", message: "x", at: 1 }, "status.error"],
    [(f) => f.status.pending = { count: 0, reason: "time", message: "" }, "status.pending"],
    [(f) => f.status.pending = { count: 2, reason: "tired" as "time", message: "" }, "status.pending"],
    [(f) => f.status.pending = { count: 2, reason: "translate" as "time", message: "" }, "status.pending"],
    [(f) => f.status.login_url = "https://news.yahoo.co.jp/", "login_url"],
    [(f) => f.items[0].id = "xyz", "items[0].id"],
    [(f) => f.items[0].object_id = "cyfile:00", "items[0].object_id"],
    [(f) => f.items[0].image = "img/a.jpg", "items[0].image"],
    [(f) => f.items[0].tags = ["a", "b", "c", "d"], "items[0].tags"],
    [(f) => f.items[0].rev = 0, "items[0].rev"],
    [(f) => f.items = Array.from({ length: 101 }, () => f.items[0]), "at most 100"],
  ];
  for (const [change, field] of bad) {
    const feed = clone(FEED);
    change(feed);
    one(feedProblems(feed), field, field);
  }
  const login = clone(FEED);
  login.status.state = "needs_login";
  login.status.login_url = "https://news.yahoo.co.jp/";
  assertEquals(feedProblems(login), []);
  for (const reason of ["translate_budget", "translate_config", "translate_failed", "time", "images"] as const) {
    const waiting = clone(FEED);
    waiting.status.pending = { count: 1, reason, message: "" };
    assertEquals(feedProblems(waiting), [], reason);
  }
});

Deno.test("an article is checked field by field, its shell with the FeedObject rules", () => {
  type A = Record<string, unknown> & { blocks: Record<string, unknown>[]; object: Record<string, unknown> };
  const bad: [(article: A) => void, string][] = [
    [(a) => a.id = "7EEC8758D56A244A", "id"],
    [(a) => a.source = "Yahoo", "source"],
    [(a) => a.rev = 0, "rev"],
    [(a) => a.title = { o: "x" }, "title"],
    [(a) => a.tags = ["a", "b", "c", "d"], "tags"],
    [(a) => a.blocks[0] = { type: "p", text: "plain" }, "blocks[0]"],
    [(a) => a.blocks[0] = { type: "video" }, "blocks[0]"],
    [(a) => a.blocks[1].src = "https://example.com/a.jpg", "blocks[1]"],
    [(a) => a.blocks.find((b) => b.missing)!.src = "_data/img/" + "a".repeat(64) + ".jpg", "missing"],
    [(a) => a.comments = [{ author: "x", text: { o: "", t: "" } }], "comments"],
    [(a) => a.links = [{ url: "javascript:alert(1)", text: { o: "", t: "" } }], "links"],
    [(a) => a.images = { "x": { key: "k", w: 1, h: 1 } }, "images"],
    [(a) => a.thumb = "thumb.jpg", "thumb"],
    [(a) => a.supplement = { event: "e-1", adds: "x" }, "supplement"],
    [(a) => a.object.publisher = "alice", "object: publisher"],
    [(a) => a.object.wraps = "https://example.com/a.md", "object: wraps"],
  ];
  for (const [change, field] of bad) {
    const article = clone(ARTICLE) as A;
    change(article);
    one(articleProblems(article), field, field);
  }
  const supplement = clone(ARTICLE);
  supplement.source = "_supplement";
  supplement.supplement = { event: "e-20261124-chip-export", adds: "the original filing" };
  assertEquals(articleProblems(supplement), []);
});

Deno.test("events.json and source.json are checked", async () => {
  const events = await fixture("site/events.json") as { events: Record<string, unknown>[] };
  const bad: [(e: typeof events) => void, string][] = [
    [(e) => e.events[0].id = "chip-export", "events[0].id"],
    [(e) => e.events[0].id = `e-${"a".repeat(47)}`, "events[0].id"],
    [(e) => e.events[0].articles = [{ id: "nope", adds: "" }], "events[0].articles"],
    [(e) => e.events[0].supplements = [{ article: "01eabe90c23c8fe3", source_url: "x", adds: "" }], "events[0].supplements"],
    [(e) => e.events.push(e.events[0]), "listed twice"],
    [(e) => delete (e.events[0].articles as Record<string, unknown>[])[0].title, "events[0].articles must list { id, adds, title, source, at }"],
    [(e) => (e.events[0].articles as Record<string, unknown>[])[0].source = "Yahoo", "events[0].articles"],
    [(e) => (e.events[0].supplements as Record<string, unknown>[])[0].at = "today", "events[0].supplements must list { article, source_url, adds, title, at }"],
  ];
  for (const [change, field] of bad) {
    const copy = clone(events);
    change(copy);
    one(eventsProblems(copy), field, field);
  }
  // What a writer hands in need not carry what is copied from the articles.
  const written = clone(events);
  for (const entry of [...written.events[0].articles as Record<string, unknown>[], ...written.events[0].supplements as Record<string, unknown>[]]) {
    delete entry.title;
    delete entry.source;
    delete entry.at;
  }
  assertEquals([eventsProblems(written, false), eventsProblems(written).length], [[], 2]);
  one(eventsProblems({ ...written, events: [{ ...written.events[0], articles: [{ id: "nope", adds: "" }] }] }, false), "events[0].articles must list { id, adds }", "input");

  const def = {
    schema: "buckyos.site.source/1",
    id: "news-example-com",
    title: "Example",
    home_url: "https://news.example.com/",
    match: [{ host: "news.example.com", path: "/a/*" }],
    lang: "ja",
    kind: "news",
    needs: "http",
  };
  assertEquals(sourceDefProblems(def), []);
  assertEquals(sourceDefProblems({ ...def, every_secs: 1800, max_new: 5, keep_days: 7, comments: 0, check: { min_chars: 50 } }), []);
  one(sourceDefProblems({ ...def, schema: "x" }), "schema", "schema");
  one(sourceDefProblems({ ...def, id: "News" }), "id", "id");
  one(sourceDefProblems({ ...def, home_url: "news.example.com" }), "home_url", "home_url");
  one(sourceDefProblems({ ...def, match: [] }), "match", "match");
  one(sourceDefProblems({ ...def, kind: "video" }), "kind", "kind");
  one(sourceDefProblems({ ...def, needs: "vpn" }), "needs", "needs");
  one(sourceDefProblems({ ...def, every_secs: 60 }), "every_secs", "every_secs");
  one(sourceDefProblems({ ...def, check: { median_ratio: 2 } }), "check", "check");
});

Deno.test("events put copies title, source and time from the articles; an article that is not there is named", async () => {
  const stored = clone(ARTICLE) as unknown as Article;
  const supplement = { ...clone(ARTICLE) as unknown as Article, id: "01eabe90c23c8fe3", source: "_supplement", published_at: null, supplement: { event: "e-20261124-chip-export", adds: "x" } };
  supplement.title = { o: "Chip export rules, explained", t: "出口许可新规逐条解读" };
  const memory = memoryStore({ [articleKey(stored.id)]: JSON.stringify(stored), [articleKey(supplement.id)]: JSON.stringify(supplement) });
  await memory.seed();
  const bucket = new SiteBucket(memory.store, "b");
  const input = {
    schema: "buckyos.site.events/1",
    events: [{
      id: "e-20261124-chip-export",
      title: "半导体设备出口新规",
      summary: "",
      // What the writer gave for title and at is not kept: the article decides.
      articles: [{ id: stored.id, adds: "公布了细则", title: "made up", at: 1 }],
      supplements: [{ article: supplement.id, source_url: "https://example.org/blog/chip-export-rules-explained", adds: "逐条解释" }],
      updated_at: 1764001800000,
    }],
    updated_at: 1764001800000,
  };
  const events = await bucket.completeEvents(clone(input));
  assertEquals(events.events[0].articles, [{ id: stored.id, adds: "公布了细则", title: stored.title.t, source: "news-yahoo-co-jp", at: stored.published_at }]);
  assertEquals(events.events[0].supplements, [{
    article: supplement.id,
    source_url: "https://example.org/blog/chip-export-rules-explained",
    adds: "逐条解释",
    title: "出口许可新规逐条解读",
    at: supplement.fetched_at,
  }]);
  await bucket.writeEvents(events);
  assertEquals((memory.json(EVENTS_KEY) as Events).events[0].articles[0].title, stored.title.t);
  assertEquals(await bucket.events(), events);

  const gone = clone(input);
  gone.events[0].articles.push({ id: "81c5e462be246971", adds: "", title: "", at: 0 });
  gone.events[0].supplements[0].article = "8064031d77568dc9";
  const err = await assertFails(() => bucket.completeEvents(gone), "is not valid");
  assertEquals((err as unknown as { problems: string[] }).problems, [
    "events[0].articles[1]: the article 81c5e462be246971 is not in the bucket",
    "events[0].supplements[0]: the article 8064031d77568dc9 is not in the bucket",
  ]);
  await assertFails(() => bucket.completeEvents({ ...input, schema: "x" }), "schema");

  // What `site events get --out` writes goes back in through `site events put`: the stored document, and the
  // empty one of a site that has no events yet.
  assertEquals(await bucket.completeEvents(JSON.parse(JSON.stringify(await bucket.events()))), events);
  assertEquals([eventsProblems(emptyEvents()), await bucket.completeEvents(emptyEvents())], [[], emptyEvents()]);
});

Deno.test("what a source shows as is composed from site.json, its upgrades and its feed", () => {
  const site = clone(SITE);
  const [yahoo, hn, nikkei] = site.sources;
  assertEquals(displayState(site, yahoo, FEED), "ok");
  assertEquals(displayState(site, yahoo, null), "preparing");
  assertEquals(displayState(site, hn, FEED), "needs_upgrade");
  assertEquals(displayState(site, nikkei, FEED), "failed");
  hn.state = "paused";
  assertEquals(displayState(site, hn, FEED), "paused");
  site.upgrades[0].state = "dismissed";
  hn.state = "active";
  const login = clone(FEED);
  login.status.state = "needs_login";
  assertEquals(displayState(site, hn, login), "needs_login");
  yahoo.state = "preparing";
  assertEquals(displayState(site, yahoo, FEED), "preparing");

  assertEquals(sourceOfUrl(SITE, "https://news.yahoo.co.jp/articles/abc#x")?.id, "news-yahoo-co-jp");
  assertEquals(sourceOfUrl(SITE, "https://example.org/"), null);
  assertEquals(sourceOfUrl(SITE, "not a url"), null);
});

Deno.test("site.json is written with If-Match; a change made in between is read and changed again", async () => {
  const memory = memoryStore();
  const bucket = new SiteBucket(memory.store, "b");
  assertEquals(await bucket.site(), null);
  assertEquals(await bucket.createSite(clone(SITE)), true);
  assertEquals(await bucket.createSite(clone(SITE)), false, "a second create must not overwrite");

  // Somebody else changes the title once while we are writing ours.
  let interfered = 0;
  memory.hooks.beforePut = async (key) => {
    if (key !== SITE_KEY || interfered) return;
    interfered += 1;
    const theirs = memory.json(SITE_KEY) as Site;
    theirs.title = "theirs";
    await memory.write(SITE_KEY, JSON.stringify(theirs));
  };
  const site = await bucket.updateSite((draft) => draft.topics = ["一", "二"], "did:bns:jarvis.alice", 1764100000000);
  assertEquals([site.title, site.topics, site.updated_at, site.updated_by], ["theirs", ["一", "二"], 1764100000000, "did:bns:jarvis.alice"]);
  assertEquals((memory.json(SITE_KEY) as Site).title, "theirs");

  // A writer that keeps getting in the way: three tries, then the failure.
  memory.hooks.beforePut = async (key) => {
    if (key === SITE_KEY) await memory.write(SITE_KEY, JSON.stringify({ ...memory.json(SITE_KEY), updated_at: ++interfered }));
  };
  const before = interfered;
  const err = await assertFails(() => bucket.updateSite((draft) => draft.title = "mine", "me"), "3 times");
  assertEquals([(err as { code?: string }).code, interfered - before], ["precondition_failed", 3]);
  memory.hooks.beforePut = undefined;

  // A change that makes the site invalid is not written.
  const kept = memory.text(SITE_KEY);
  const invalid = await assertFails(() => bucket.updateSite((draft) => draft.sources[0].every_secs = 10, "me"), "every_secs");
  assert(invalid instanceof SiteError && invalid.kind === "invalid", "an invalid change is a SiteError");
  assertEquals(memory.text(SITE_KEY), kept);
});

Deno.test("images are named after their bytes, written once and served as immutable", async () => {
  const memory = memoryStore();
  const bucket = new SiteBucket(memory.store, "b");
  const bytes = new TextEncoder().encode("not really a picture");
  const T = 1764001700000;
  const first = await bucket.writeImage(bytes, "jpg", T);
  assert(/^_data\/img\/[0-9a-f]{64}\.jpg$/.test(first.key), first.key);
  assertEquals(memory.objects.get(first.key)!.headers, { "cache-control": IMMUTABLE });
  assertEquals([memory.objects.get(first.key)!.expiresAt, expiryOf(T) - T], [expiryOf(T), 181 * 86_400_000]);
  // An article fetched earlier refers to it again: nothing changes. A later one moves the expiry on.
  assertEquals(await bucket.writeImage(bytes, "jpg", T - 1000), first);
  assertEquals([memory.log.filter((entry) => entry.endsWith(first.key)), memory.objects.get(first.key)!.expiresAt], [[`put ${first.key}`, `bind ${first.key}`], expiryOf(T)]);
  assertEquals(await bucket.writeImage(bytes, "jpg", T + 86_400_000), first);
  assertEquals(memory.objects.get(first.key)!.expiresAt, expiryOf(T + 86_400_000));
  assertEquals(memory.objects.get(first.key)!.headers, { "cache-control": IMMUTABLE });
  assertEquals(memory.log.filter((entry) => entry.endsWith(first.key)), [`put ${first.key}`, `bind ${first.key}`, `bind ${first.key}`]);

  // A template file never expires and is left alone when it is there.
  await bucket.writeImmutable("_app/t1/app.js", "1");
  await bucket.writeImmutable("_app/t1/app.js", "1");
  assertEquals([memory.objects.get("_app/t1/app.js")!.expiresAt, memory.log.filter((entry) => entry.endsWith("app.js")).length], [undefined, 2]);

  assertEquals(await bucket.keys("_data/"), [first.key]);
  memory.objects.delete("_app/t1/app.js");
  for (let n = 0; n < 7; n += 1) await bucket.write(`_src/s/s${n + 1}/source.json`, "{}");
  assertEquals((await bucket.keys("_src/")).length, 7, "listing follows the pages");
  assertEquals(await bucket.folders("_src/s/"), ["s1", "s2", "s3", "s4", "s5", "s6", "s7"]);
  await bucket.remove("_data/none");
});

Deno.test("what is in the bucket is checked when it is read", async () => {
  const memory = memoryStore({ "_data/feed/news-yahoo-co-jp.json": '{"schema":"x"}', [SITE_KEY]: "not json" });
  await memory.seed();
  const bucket = new SiteBucket(memory.store, "b");
  await assertFails(() => bucket.feed("news-yahoo-co-jp"), "is not valid");
  await assertFails(() => bucket.site(), "is not JSON");
  assertEquals(await bucket.feed("other-source"), null);
  memory.hooks.readOnly = true;
  const err = await assertFails(() => bucket.writeFeed(clone(FEED)), "write on");
  assertEquals((err as { code?: string }).code, "permission_denied");
});
