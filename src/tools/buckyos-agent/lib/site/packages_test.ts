import {
  getSource,
  getTemplate,
  listTemplates,
  putSource,
  putTemplate,
  readSourcePackage,
  removeSource,
  useTemplate,
} from "./packages.ts";
import { IMMUTABLE, SiteBucket } from "./store.ts";
import { assertEquals, assertFails, fixture, memoryStore, tempDir } from "./testing.ts";
import { feedKey, SITE_KEY } from "./types.ts";
import type { Site } from "./types.ts";

const SITE = await fixture("site/site.json") as Site;
const T = 1764100000000;

const SOURCE = {
  schema: "buckyos.site.source/1",
  id: "www-itmedia-co-jp",
  title: "ITmedia",
  home_url: "https://www.itmedia.co.jp/",
  match: [{ host: "www.itmedia.co.jp" }],
  lang: "ja",
  kind: "blog",
  needs: "http",
  comments: 0,
};

function pkg(patch: Record<string, unknown> = {}, files: Record<string, string> = {}) {
  return tempDir({
    "source.json": JSON.stringify({ ...SOURCE, ...patch }),
    "extract.ts": "export default async () => ({ list: [], items: [] })",
    "fixtures/index.json": "{}",
    "NOTES.md": "notes",
    ".git/config": "hidden",
    ...files,
  });
}

async function site() {
  const memory = memoryStore({ [SITE_KEY]: JSON.stringify(SITE), [feedKey("news-yahoo-co-jp")]: "{}" });
  await memory.seed();
  return { memory, bucket: new SiteBucket(memory.store, "b") };
}

Deno.test("a source package is checked before anything is stored", async () => {
  const { memory, bucket } = await site();
  const bad = await tempDir({ "source.json": JSON.stringify({ ...SOURCE, id: "ITmedia", needs: "vpn" }), "readme.txt": "x" });
  const notJson = await tempDir({ "source.json": "{", "extract.ts": "" });
  try {
    const err = await assertFails(() => putSource(bucket, bad, { by: "me", addedBy: "me" }), "is not valid");
    assertEquals((err as unknown as { problems: string[] }).problems.length, 3, JSON.stringify(err.message));
    await assertFails(() => readSourcePackage(notJson), "source.json is not JSON");
    await assertFails(() => readSourcePackage(`${bad}/nowhere`), "");
    assertEquals(memory.log, []);
  } finally {
    await Deno.remove(bad, { recursive: true });
    await Deno.remove(notJson, { recursive: true });
  }
});

Deno.test("source put stores the next version and writes the entry; the owner's settings survive a new put", async () => {
  const { memory, bucket } = await site();
  const dir = await pkg();
  try {
    const first = await putSource(bucket, dir, { by: "did:bns:jarvis.alice", addedBy: "work:ws-1", now: T });
    assertEquals([first.id, first.version, first.files], ["www-itmedia-co-jp", "s1", ["NOTES.md", "extract.ts", "fixtures/index.json", "source.json"]]);
    assertEquals(memory.keys("_src/www-itmedia-co-jp/"), [
      "_src/www-itmedia-co-jp/s1/NOTES.md",
      "_src/www-itmedia-co-jp/s1/extract.ts",
      "_src/www-itmedia-co-jp/s1/fixtures/index.json",
      "_src/www-itmedia-co-jp/s1/source.json",
    ]);
    assertEquals(memory.log.at(-1), `put ${SITE_KEY}`, "site.json after the package");
    const stored = memory.json(SITE_KEY) as Site;
    assertEquals(stored.sources.at(-1), {
      id: "www-itmedia-co-jp",
      title: "ITmedia",
      home_url: "https://www.itmedia.co.jp/",
      match: [{ host: "www.itmedia.co.jp" }],
      lang: "ja",
      kind: "blog",
      needs: "http",
      job: "src-www-itmedia-co-jp",
      version: "s1",
      every_secs: 21600,
      max_new: 20,
      keep_days: 30,
      comments: 0,
      state: "preparing",
      note: null,
      added_at: T,
      added_by: "work:ws-1",
    });
    assertEquals([stored.updated_at, stored.updated_by, stored.sources.length], [T, "did:bns:jarvis.alice", 4]);

    // The owner renames it and slows it down; the next package changes the address rules.
    await bucket.updateSite((draft) => Object.assign(draft.sources.at(-1)!, { title: "IT 新闻", every_secs: 43200, keep_days: 7 }), "owner");
    await Deno.writeTextFile(`${dir}/source.json`, JSON.stringify({ ...SOURCE, title: "ITmedia NEWS", every_secs: 3600, match: [{ host: "www.itmedia.co.jp", path: "/news/*" }] }));
    const second = await putSource(bucket, dir, { state: "active", by: "did:bns:jarvis.alice", addedBy: "work:ws-2", now: T + 1 });
    const entry = (memory.json(SITE_KEY) as Site).sources.at(-1)!;
    assertEquals(
      [second.version, entry.version, entry.title, entry.every_secs, entry.keep_days, entry.match, entry.state, entry.added_at, entry.added_by],
      ["s2", "s2", "IT 新闻", 43200, 7, [{ host: "www.itmedia.co.jp", path: "/news/*" }], "active", T, "work:ws-1"],
    );

    // failed with the reason; three versions are kept.
    const failed = await putSource(bucket, dir, { state: "failed", note: "正文需要订阅", by: "a", addedBy: "a" });
    assertEquals([failed.entry.state, failed.entry.note, failed.version], ["failed", "正文需要订阅", "s3"]);
    const fourth = await putSource(bucket, dir, { state: "active", by: "a", addedBy: "a" });
    assertEquals([fourth.version, fourth.entry.note, await bucket.folders("_src/www-itmedia-co-jp/")], ["s4", null, ["s2", "s3", "s4"]]);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("source get writes the current version into an empty directory; source rm takes the source out", async () => {
  const { memory, bucket } = await site();
  const dir = await pkg();
  const out = await tempDir();
  try {
    await putSource(bucket, dir, { by: "a", addedBy: "a" });
    await putSource(bucket, dir, { by: "a", addedBy: "a" });
    const got = await getSource(bucket, memory.json(SITE_KEY) as Site, "www-itmedia-co-jp", `${out}/pkg`);
    assertEquals([got.version, got.files], ["s2", ["NOTES.md", "extract.ts", "fixtures/index.json", "source.json"]]);
    assertEquals((await readSourcePackage(`${out}/pkg`)).def.id, "www-itmedia-co-jp");
    await assertFails(() => getSource(bucket, memory.json(SITE_KEY) as Site, "www-itmedia-co-jp", `${out}/pkg`), "not empty");
    await assertFails(() => getSource(bucket, memory.json(SITE_KEY) as Site, "nothing-here", `${out}/x`), "no source");

    // Yahoo is in a column and has a feed.
    await memory.write("_src/news-yahoo-co-jp/s2/source.json", "{}");
    const removed = await removeSource(bucket, "news-yahoo-co-jp", "owner");
    const after = memory.json(SITE_KEY) as Site;
    assertEquals([removed, after.sources.some((source) => source.id === "news-yahoo-co-jp"), after.columns[0].sources], [2, false, []]);
    assertEquals([memory.objects.has(feedKey("news-yahoo-co-jp")), memory.keys("_src/news-yahoo-co-jp/")], [false, []]);
    await assertFails(() => removeSource(bucket, "news-yahoo-co-jp", "owner"), "no source");
  } finally {
    await Deno.remove(dir, { recursive: true });
    await Deno.remove(out, { recursive: true });
  }
});

Deno.test("a template is installed as a new immutable version beside the shared entry page", async () => {
  const { memory, bucket } = await site();
  const dir = await tempDir({
    "index.html": "<!doctype html><title>entry</title>",
    "app.js": "export default function start() {}",
    "app.css": "body{}",
    "i18n/zh-CN.js": "export default {}",
    "test/site.spec.ts": "// the template's own tests are not installed",
    "test/screenshots/home.png": "png",
  });
  const broken = await tempDir({ "app.css": "body{}" });
  try {
    const first = await putTemplate(bucket, dir, "did:bns:jarvis.alice", T);
    assertEquals([first.version, first.files], ["t1", ["app.css", "app.js", "i18n/zh-CN.js"]]);
    assertEquals(memory.keys("_app/"), ["_app/t1/app.css", "_app/t1/app.js", "_app/t1/i18n/zh-CN.js", "_app/t1/template.json"]);
    assertEquals(memory.objects.get("_app/t1/app.js")!.headers, { "cache-control": IMMUTABLE });
    assertEquals(memory.objects.get("index.html")!.headers, undefined, "the entry page is not cached for good");
    assertEquals(memory.json("_app/t1/template.json"), { schema: "buckyos.site.template/1", version: "t1", updated_at: T, updated_by: "did:bns:jarvis.alice" });
    assertEquals(memory.log.at(-1), "put index.html");
    assertEquals((memory.json(SITE_KEY) as Site).template.version, "t1", "site.json is the caller's to switch");

    await Deno.writeTextFile(`${dir}/app.js`, "export default function start() { /* v2 */ }");
    await Deno.remove(`${dir}/index.html`);
    const second = await putTemplate(bucket, dir, "a", T + 1);
    assertEquals([second.version, memory.text("_app/t1/app.js").includes("v2"), memory.text("_app/t2/app.js").includes("v2")], ["t2", false, true]);
    await useTemplate(bucket, "t2", "a");
    assertEquals((memory.json(SITE_KEY) as Site).template.version, "t2");
    assertEquals(await listTemplates(bucket), [
      { version: "t1", updated_at: T, updated_by: "did:bns:jarvis.alice" },
      { version: "t2", updated_at: T + 1, updated_by: "a" },
    ]);
    await useTemplate(bucket, "t1", "a");
    assertEquals((memory.json(SITE_KEY) as Site).template.version, "t1", "use is how to go back");

    // A new maintainer takes a version over: what it gets is what put takes again.
    const out = await tempDir();
    try {
      const got = await getTemplate(bucket, "t2", `${out}/tpl`);
      assertEquals(got, { version: "t2", files: ["app.css", "app.js", "i18n/zh-CN.js", "index.html"] });
      assertEquals(
        [await Deno.readTextFile(`${out}/tpl/app.js`), await Deno.readTextFile(`${out}/tpl/index.html`)],
        ["export default function start() { /* v2 */ }", "<!doctype html><title>entry</title>"],
      );
      assertEquals(await Deno.stat(`${out}/tpl/template.json`).catch(() => null), null, "the record of the install stays in the bucket");
      assertEquals((await putTemplate(bucket, `${out}/tpl`, "b", T + 2)).version, "t3");
      assertEquals(memory.text("_app/t3/app.js"), memory.text("_app/t2/app.js"));
      await assertFails(() => getTemplate(bucket, "t2", `${out}/tpl`), "is not empty");
      await assertFails(() => getTemplate(bucket, "t9", `${out}/none`), "no template version t9");
      await assertFails(() => getTemplate(bucket, "../_data", `${out}/none`), "no template version");
    } finally {
      await Deno.remove(out, { recursive: true });
    }
    await assertFails(() => useTemplate(bucket, "t9", "a"), "no template version t9");
    await assertFails(() => putTemplate(bucket, broken, "a"), "app.js is missing");

    const empty = new SiteBucket(memoryStore().store, "b");
    await Deno.remove(`${dir}/index.html`).catch(() => {});
    await assertFails(() => putTemplate(empty, dir, "a"), "index.html is missing");
  } finally {
    await Deno.remove(dir, { recursive: true });
    await Deno.remove(broken, { recursive: true });
  }
});
