import { SiteBucket } from "./store.ts";
import { assert, assertEquals, assertFails, fixture, memoryStore, tempDir } from "./testing.ts";
import { feedKey, SITE_KEY } from "./types.ts";
import type { Feed, Site } from "./types.ts";
import type { SyncReport } from "./pipeline.ts";
import { configParent, configValue, decideUpgrade, HELP, siteStatus, syncOutcome } from "../../commands/site.ts";

const SITE = await fixture("site/site.json") as Site;
const FEED = await fixture("site/feed.json") as Feed;
const TOOL = new URL("../../aicc-tool.ts", import.meta.url).pathname;
const CONFIG = new URL("../../deno.json", import.meta.url).pathname;
const SOURCES = new URL("../../site-sources/", import.meta.url).pathname;

interface Ran {
  code: number;
  lines: string[];
  // deno-lint-ignore no-explicit-any
  result: any;
  stderr: string;
}

async function site(args: string[], env: Record<string, string> = {}, cwd?: string): Promise<Ran> {
  const out = await new Deno.Command(Deno.execPath(), {
    args: ["run", "--config", CONFIG, "--allow-net", "--allow-read", "--allow-write", "--allow-env", TOOL, "site", ...args],
    env: { ...env, NO_COLOR: "1" },
    cwd,
    stdout: "piped",
    stderr: "piped",
  }).output();
  const lines = new TextDecoder().decode(out.stdout).trimEnd().split("\n").filter(Boolean);
  let result = null;
  try {
    result = JSON.parse(lines.at(-1) ?? "");
  } catch {
    // not JSON: the caller looks at the lines
  }
  return { code: out.code, lines, result, stderr: new TextDecoder().decode(out.stderr) };
}

Deno.test("site --help lists every subcommand of the contract; bad arguments exit with 1", async () => {
  const help = await site(["--help"]);
  assertEquals([help.code, help.lines, help.stderr.trim()], [0, [], HELP]);
  for (
    const form of [
      "site init", "site status", "site match <url>", "site config get", "site config set", "site source put",
      "site source list", "site source get", "site source rm", "site try <dir>", "site sync <source>",
      "site template put", "site template list", "site template use", "site template get <version> <dir>",
      "site preview <dir>", "site upgrade open", "site upgrade close <id>", "site upgrade dismiss <id>",
      "site upgrade list", "site events get [--out <file>]", "site events put", "site supplement put",
      "site visits [--days 7] [--json]", "pending, pending_reason",
      "site reading [--days 7]",
      "site reading --compact", "Exit codes:", "OPENDAN_JOB_RUN_ID",
    ]
  ) assert(HELP.includes(form), `the help does not mention: ${form}`);

  const bad: [string[], string][] = [
    [[], "a subcommand is required"],
    [["publish"], "unknown subcommand: publish"],
    [["sync"], "expected: site sync"],
    [["sync", "a", "b"], "expected: site sync"],
    [["status", "--verbose"], "unsupported option: --verbose"],
    [["try", "x", "--fixtures", "--record"], "exclude each other"],
    [["try", "x", "--limit", "0"], "--limit must be at least 1"],
    [["config", "set", "title", "not json"], "must be JSON"],
    [["source", "put", "x", "--state", "paused"], "--state must be"],
    [["upgrade", "open", "u1", "--title", "t"], "--summary is required"],
    [["template", "rollback"], "expected: site template"],
    [["template", "get", "t1"], "expected: site template"],
    [["upgrade", "dismiss"], "expected: site upgrade"],
    [["events", "get", "extra"], "expected: site events get [--out <file>]"],
    [["visits", "today"], "expected: site visits"],
    [["visits", "--days", "0"], "--days must be at least 1"],
  ];
  for (const [args, text] of bad) {
    const ran = await site(args);
    assertEquals([ran.code, ran.lines.length, ran.result?.status, ran.result?.title], [1, 1, "error", "site => arg_error"], args.join(" "));
    assert(ran.result.summary.includes(text), `${args.join(" ")}: ${ran.result.summary}`);
  }
});

Deno.test("site try runs a package from its fixtures into a directory laid out like the bucket", async () => {
  const out = await tempDir();
  try {
    const ran = await site(["try", `${SOURCES}news-yahoo-co-jp`, "--fixtures", "--no-translate", "--limit", "2", "--out", out]);
    assertEquals([ran.code, ran.lines.length, ran.result.status, ran.result.title], [0, 1, "success", "site try => done"], ran.stderr);
    assert(ran.result.summary.startsWith("news-yahoo-co-jp: 3 listed, 2 new, 2 published"), ran.result.summary);
    assertEquals([ran.result.detail.ok, ran.result.detail.published.length, ran.result.detail.out], [true, 2, out]);
    const feed = JSON.parse(await Deno.readTextFile(`${out}/${feedKey("news-yahoo-co-jp")}`)) as Feed;
    assertEquals([feed.status.state, feed.items.length], ["ok", 2]);
    const article = JSON.parse(await Deno.readTextFile(`${out}/_data/a/${feed.items[0].id}.json`));
    assertEquals([article.title.o === article.title.t, article.comments.length, article.blocks.some((b: { missing?: boolean }) => b.missing)], [true, 10, true]);
    const made = JSON.parse(await Deno.readTextFile(`${out}/${SITE_KEY}`)) as Site;
    assertEquals([made.sources.map((source) => source.id), made.language], [["news-yahoo-co-jp"], "ja"]);
  } finally {
    await Deno.remove(out, { recursive: true });
  }
});

Deno.test("site try says what the check found and exits with 2 when the run fails it; a bad package is 1", async () => {
  const source = (id: string) =>
    JSON.stringify({
      schema: "buckyos.site.source/1", id, title: "T", home_url: "https://t.example/", match: [{ host: "t.example" }],
      lang: "en", kind: "blog", needs: "http",
    });
  const item = (n: number, text: string) => `{ url: "https://t.example/${n}", title: "T${n}", blocks: [{ type: "p", text: ${JSON.stringify(text)} }] }`;
  const dir = await tempDir({
    "short/source.json": source("short-example"),
    "short/extract.ts": `export default async () => ({ list: [{ url: "https://t.example/1", title: "T1" }], items: [${item(1, "too short")}, ${item(2, "short too")}, ${item(3, "x".repeat(300))}] })`,
    "login/source.json": source("login-example"),
    "login/extract.ts": `export default async () => { throw new SiteError("auth", "the page asks to sign in"); }`,
    "nosource/extract.ts": "export default async () => ({ list: [], items: [] })",
  });
  try {
    const short = await site(["try", `${dir}/short`, "--no-translate", "--out", `${dir}/out`]);
    assertEquals([short.code, short.result.status, short.result.title], [2, "error", "site try => program"], short.stderr);
    assert(short.result.summary.includes("2 of 3 articles did not pass the check"), short.result.summary);
    assert(short.result.summary.includes("rejected https://t.example/1: the body has 9 characters, fewer than 200"), short.result.summary);
    const login = await site(["try", `${dir}/login`, "--no-translate", "--out", `${dir}/out2`]);
    assertEquals([login.code, login.result.title, login.result.detail.status.state], [2, "site try => auth", "needs_login"]);
    const none = await site(["try", `${dir}/nosource`, "--no-translate"]);
    assertEquals([none.code, none.result.title, none.result.detail.problems], [1, "site => invalid", ["source.json is missing"]]);
    const missing = await site(["try", `${dir}/nowhere`, "--no-translate"]);
    assertEquals([missing.code, missing.result.title], [5, "site => io_failed"]);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("site preview serves a template over the output of site try", async () => {
  const dir = await tempDir({
    "tpl/index.html": "<!doctype html><title>entry</title>",
    "tpl/app.js": "export default function start() {}",
    "data/_data/site.json": JSON.stringify(SITE),
    "data/_data/feed/news-yahoo-co-jp.json": JSON.stringify(FEED),
  });
  const child = new Deno.Command(Deno.execPath(), {
    args: ["run", "--config", CONFIG, "--allow-net", "--allow-read", "--allow-write", "--allow-env", TOOL, "site", "preview", `${dir}/tpl`, "--data", `${dir}/data`],
    stdout: "piped",
    stderr: "null",
  }).spawn();
  try {
    const reader = child.stdout.getReader();
    let text = "";
    while (!text.includes("\n")) {
      const { value, done } = await reader.read();
      if (done) break;
      text += new TextDecoder().decode(value);
    }
    reader.releaseLock();
    const started = JSON.parse(text.split("\n")[0]);
    assertEquals([started.status, started.title], ["success", "site preview => serving"]);
    const url = started.detail.url as string;
    assert(/^http:\/\/127\.0\.0\.1:\d+\/$/.test(url), url);
    const get = async (path: string, init?: RequestInit) => {
      const response = await fetch(`${url}${path}`, init);
      return [response.status, response.headers.get("content-type"), await response.text()];
    };
    assertEquals(await get(""), [200, "text/html; charset=utf-8", "<!doctype html><title>entry</title>"]);
    assertEquals((await get("_app/t7/app.js")).slice(0, 2), [200, "text/javascript; charset=utf-8"]);
    assertEquals(JSON.parse((await get("_data/site.json"))[2] as string).title, SITE.title);
    assertEquals((await get("_data/feed/news-yahoo-co-jp.json"))[0], 200);
    assertEquals((await get("_data/a/0000000000000000.json"))[0], 404);
    assertEquals((await get("_app/t1/..%2F..%2Fdata%2F_data%2Fsite.json"))[0], 404);
    assertEquals((await get("_read", { method: "POST", body: "[]" }))[0], 204);
    assertEquals((await get("_data/site.json", { method: "PUT", body: "{}" }))[0], 405);
  } finally {
    child.kill();
    await child.status;
    await child.stdout.cancel().catch(() => {});
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("as a Goal job a failed start still ends with the job's result line", async () => {
  const cwd = await tempDir();
  try {
    const env = {
      OPENDAN_JOB_RUN_ID: "run-1",
      BUCKYOS_APPCLIENT_SESSION_TOKEN: "not-a-token",
      BUCKYOS_HOST_GATEWAY: "127.0.0.1",
      BUCKYOS_NODE_GATEWAY_PORT: "9",
      HOME: cwd,
    };
    for (const args of [["sync", "news-example-com"], ["reading", "--compact"]]) {
      const ran = await site(args, env, cwd);
      assertEquals([ran.code, Object.keys(ran.result).sort(), ran.result.kind], [2, ["kind", "summary"], "transient"], ran.stderr);
      assert(typeof ran.result.summary === "string" && ran.result.summary.length > 0, "a summary");
    }
    // Outside a job the same failure is one AgentToolResult.
    const plain = await site(["sync", "news-example-com"], { ...env, OPENDAN_JOB_RUN_ID: "" }, cwd);
    assertEquals([plain.code, plain.result.agent_tool_protocol, plain.result.status], [2, "1", "error"]);
  } finally {
    await Deno.remove(cwd, { recursive: true });
  }
});

const report = (patch: Partial<SyncReport> = {}): SyncReport => ({
  ok: true,
  summary: "news-example-com: 8 listed, 2 new, 2 published",
  metrics: { listed: 8, new: 2, published: 2, rejected: 0, images: 3, tokens: 4200, ms: 61000, pending: 0, pending_reason: null },
  status: { state: "ok", checked_at: 1, published_at: 1, error: null, pending: null, login_url: null },
  published: [],
  rejected: [],
  requests: 7,
  not_modified: 0,
  removed: 0,
  ...patch,
});

Deno.test("site sync: the job's result line as a Goal job, an AgentToolResult elsewhere; exit code by the check", () => {
  assertEquals(syncOutcome(report(), true, "mock-mt"), {
    result: {
      summary: "news-example-com: 8 listed, 2 new, 2 published",
      metrics: { listed: 8, new: 2, published: 2, rejected: 0, images: 3, tokens: 4200, ms: 61000, pending: 0, pending_reason: null },
    },
    exitCode: 0,
  });
  // The host drops `kind` of a run that exits 0: what a check script reads of a refused translation is in the metrics.
  const waiting = { ...report().metrics, published: 0, pending: 2, pending_reason: "translate_budget" as const };
  const refused = syncOutcome(report({ kind: "budget", metrics: waiting }), true, null);
  assertEquals(
    [(refused.result as { kind: string }).kind, (refused.result as { metrics: unknown }).metrics, refused.exitCode],
    ["budget", waiting, 0],
    "a refused translation is not a failed run",
  );
  for (const kind of ["auth", "program", "transient"] as const) {
    const failed = syncOutcome(report({ ok: false, kind, summary: `news-example-com: ${kind}: why` }), true, null);
    assertEquals([failed.result, failed.exitCode], [{ summary: `news-example-com: ${kind}: why`, kind, metrics: report().metrics }, 2]);
    // deno-lint-ignore no-explicit-any
    const tool = syncOutcome(report({ ok: false, kind }), false, null) as any;
    assertEquals([tool.result.status, tool.result.title, tool.exitCode], ["error", `site sync => ${kind}`, 2]);
  }
  // deno-lint-ignore no-explicit-any
  const done = syncOutcome(report(), false, "mock-mt") as any;
  assertEquals([done.result.status, done.result.title, done.result.detail.model, done.result.detail.metrics.tokens, done.exitCode], ["success", "site sync => done", "mock-mt", 4200, 0]);
  assert(done.result.summary.includes("7 request(s), 4200 model token(s), 61s"), done.result.summary);
});

Deno.test("site config reads any field and changes only the ones that are the user's or the maintainer's", async () => {
  const site = structuredClone(SITE);
  assertEquals(configValue(site, "title"), "我的阅读");
  assertEquals(configValue(site, "template.params.per_column"), 20);
  assertEquals(configValue(site, "sources.news-yahoo-co-jp.every_secs"), 1800);
  assertEquals(configValue(site, "maintainer.goal_id"), "g-20261123T020304-9c8d7e6f");
  await assertFails(() => configValue(site, "sources.nope.title"), "no sources.nope.title");

  const set = (path: string, value: unknown) => {
    const { parent, field } = configParent(site, path);
    parent[field] = value;
  };
  set("title", "新标题");
  set("topics", ["国际"]);
  set("template.params.hide_topics", ["体育", "娱乐"]);
  set("template.params.accent", "teal");
  set("sources.news-ycombinator-com.state", "paused");
  set("sources.news-ycombinator-com.every_secs", 7200);
  set("reading.enabled", false);
  set("limits.translate_tokens_per_day", 500000);
  set("maintainer", null);
  assertEquals(
    [site.title, site.topics, site.template.params.hide_topics, site.template.params.accent, site.sources[1].state, site.sources[1].every_secs, site.reading.enabled, site.limits.translate_tokens_per_day, site.maintainer],
    ["新标题", ["国际"], ["体育", "娱乐"], "teal", "paused", 7200, false, 500000, null],
  );
  for (const path of ["owner_did", "schema", "template.version", "sources", "sources.news-yahoo-co-jp.id", "sources.news-yahoo-co-jp.version", "sources.news-yahoo-co-jp.match", "upgrades", "updated_at", "reading"]) {
    await assertFails(() => configParent(site, path), "is not a field site config changes");
  }
  await assertFails(() => configParent(site, "sources.nope.title"), "no source nope");
});

Deno.test("site status shows each source as the page will", async () => {
  const memory = memoryStore({ [SITE_KEY]: JSON.stringify(SITE), [feedKey("news-yahoo-co-jp")]: JSON.stringify(FEED) });
  await memory.seed();
  const status = await siteStatus(new SiteBucket(memory.store, "b"), SITE);
  assertEquals(status.detail.sources.map((source) => [source.id, source.state, source.items]), [
    ["news-yahoo-co-jp", "ok", 3],
    ["news-ycombinator-com", "needs_upgrade", 0],
    ["www-nikkei-com", "failed", 0],
  ]);
  assertEquals(status.detail.upgrades.map((upgrade) => upgrade.id), ["hn-comments-layout"]);
  const lines = status.summary.split("\n");
  assert(lines[0].startsWith("我的阅读 (zh-CN), template t1, kept by did:bns:jarvis.alice"), lines[0]);
  assert(lines[1].includes("waiting: 2 (translate_budget)"), lines[1]);
  assert(lines[3].includes("note: 正文需要订阅"), lines[3]);
  assert(lines[4].startsWith("upgrade hn-comments-layout (news-ycombinator-com)"), lines[4]);
});

Deno.test("the help tells to give the reading job a directory with a file in it", () => {
  assert(!HELP.includes("--dir <empty dir>"), "a job is not installed from an empty directory");
  assert(HELP.includes("a directory with one file in it"), "what the directory has to be");
});

Deno.test("an upgrade request is closed when the upgrade was made and dismissed when the user declined", async () => {
  const site = structuredClone(SITE);
  const dismissed = decideUpgrade(site, "hn-comments-layout", "dismissed", 1764100000000);
  assertEquals([dismissed.state, dismissed.decided_at, site.upgrades[0].state], ["dismissed", 1764100000000, "dismissed"]);
  assertEquals(decideUpgrade(site, "hn-comments-layout", "closed", 1764100000001).state, "closed");
  await assertFails(() => decideUpgrade(site, "no-such", "dismissed"), "there is no upgrade request no-such");
});

Deno.test("site visits asks this Agent's browser service and says when recording is off or the service is out of reach", async () => {
  const DAY = 86_400_000;
  const calls: { method: string; params: { since_ms: number }; token: string }[] = [];
  const answers: unknown[] = [
    {
      result: {
        enabled: true,
        visits: [
          { url: "https://www.itmedia.co.jp/news/articles/1.html", title: "ITmedia の記事", first_at_ms: 1764000000000, last_at_ms: 1764003600000, dwell_ms: 95000, count: 2 },
          { url: "https://example.org/blog/a", title: "", first_at_ms: 1763990000000, last_at_ms: 1763990060000, dwell_ms: 61000, count: 1 },
          { url: "https://www.itmedia.co.jp/", title: "ITmedia", first_at_ms: 1763980000000, last_at_ms: 1763980005000, dwell_ms: 4000, count: 3 },
        ],
      },
    },
    { result: { enabled: false, visits: [] } },
    { error: JSON.stringify({ kind: "no_permission", code: "no_permission", message: "browser.visits is the owner's and this Agent's" }) },
  ];
  const server = Deno.serve({ hostname: "127.0.0.1", port: 0, onListen: () => {} }, async (request) => {
    const body = await request.json();
    calls.push({ method: body.method, params: body.params, token: body.sys[1] });
    return Response.json(answers.shift());
  });
  const env = { OPENDAN_BROWSER_RPC: `http://127.0.0.1:${server.addr.port}/kapi/agent`, BUCKYOS_APPCLIENT_SESSION_TOKEN: "token-1" };
  try {
    const before = Date.now();
    const shown = await site(["visits", "--days", "3"], env);
    assertEquals([shown.code, shown.result.status, shown.result.title], [0, "success", "site visits => done"], shown.stderr);
    assertEquals([calls[0].method, calls[0].token], ["browser.visits", "token-1"]);
    assert(Math.abs(calls[0].params.since_ms - (before - 3 * DAY)) < 60_000, `since_ms ${calls[0].params.since_ms}`);
    const at = (ms: number) => new Date(ms).toISOString();
    assertEquals(shown.result.summary.split("\n"), [
      "recording is on; 3 page(s) on 2 host(s) in the last 3 day(s)",
      `host www.itmedia.co.jp  2 page(s), 5 visit(s), 99s, last ${at(1764003600000)}`,
      `host example.org  1 page(s), 1 visit(s), 61s, last ${at(1763990060000)}`,
      `https://www.itmedia.co.jp/news/articles/1.html  "ITmedia の記事"  2 visit(s), 95s, last ${at(1764003600000)}`,
      `https://example.org/blog/a  (no title)  1 visit(s), 61s, last ${at(1763990060000)}`,
      `https://www.itmedia.co.jp/  "ITmedia"  3 visit(s), 4s, last ${at(1763980005000)}`,
    ]);
    assertEquals(at(1764003600000), "2025-11-24T17:00:00.000Z");
    assertEquals([shown.result.detail.enabled, shown.result.detail.days, shown.result.detail.visits.length, shown.result.detail.hosts[0]], [
      true,
      3,
      3,
      { host: "www.itmedia.co.jp", pages: 2, count: 5, dwell_ms: 99000, last_at_ms: 1764003600000 },
    ]);

    const off = await site(["visits", "--json"], env);
    assertEquals([off.code, JSON.parse(off.result.summary), off.result.detail.enabled], [0, { days: 7, enabled: false, hosts: [], visits: [] }, false]);
    const plain = (await import("./visits.ts")).describeVisits({ enabled: false, visits: [] }, 7).summary;
    assert(plain.startsWith("recording is switched off (the owner turned it off)") && plain.endsWith("nothing was recorded in the last 7 day(s)"), plain);

    const refused = await site(["visits"], env);
    assertEquals([refused.code, refused.result.title, refused.result.detail.error_code], [2, "site visits => no_permission", "no_permission"]);
  } finally {
    await server.shutdown();
  }

  // Not on the Agent's own runtime, or the service does not answer: the same, plain answer.
  const nowhere = await site(["visits"], { OPENDAN_BROWSER_RPC: "", BUCKYOS_APPCLIENT_SESSION_TOKEN: "" });
  assertEquals([nowhere.code, nowhere.result.status, nowhere.result.title], [2, "error", "site visits => no_browser"]);
  assert(nowhere.result.summary.includes("not reachable from this runtime") && nowhere.result.summary.includes("$OPENDAN_BROWSER_RPC"), nowhere.result.summary);
  const down = await site(["visits"], { OPENDAN_BROWSER_RPC: `http://127.0.0.1:${server.addr.port}/kapi/agent`, BUCKYOS_APPCLIENT_SESSION_TOKEN: "token-1" });
  assertEquals([down.code, down.result.title], [2, "site visits => no_browser"]);
  assert(down.result.summary.includes("cannot reach the Agent Service"), down.result.summary);
});
