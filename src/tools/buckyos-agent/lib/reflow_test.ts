import {
  contentTypeOf,
  getPackage,
  libraryBucketId,
  libraryState,
  listSites,
  manifestProblems,
  normalizeUrl,
  packageMatches,
  packageProblems,
  pathMatches,
  putPackage,
  putResult,
  readPreference,
  reflowAddress,
  ReflowError,
  removeSite,
  siteOfHost,
  viewOf,
} from "./reflow.ts";
import type { LibraryStore } from "./reflow.ts";

function assertEquals(actual: unknown, expected: unknown, what = ""): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`${what} expected ${expectedJson}, got ${actualJson}`);
  }
}

async function assertRejects(fn: () => Promise<unknown>, includes: string): Promise<ReflowError> {
  try {
    await fn();
  } catch (err) {
    if (!(err instanceof ReflowError) || !err.message.includes(includes)) {
      throw new Error(`expected a ReflowError with ${JSON.stringify(includes)}, got ${err}`);
    }
    return err;
  }
  throw new Error(`expected a ReflowError with ${JSON.stringify(includes)}`);
}

// The vectors the Agent Service (Rust) is checked against as well.
const VECTORS = JSON.parse(
  await Deno.readTextFile(
    new URL("../../../../doc/opendan/protocol/fixtures/reflow_vectors.json", import.meta.url),
  ),
) as {
  normalize: { url: string; normalized: string; site: string; page: string; view: string }[];
  invalid: string[];
  match: { manifest_match: { host: string; path?: string }[]; url: string; matches: boolean }[];
};

Deno.test("site, normalized URL, page and view follow the shared vectors", async () => {
  for (const vector of VECTORS.normalize) {
    const address = await reflowAddress(vector.url);
    assertEquals(
      [address.url, address.site, address.page, viewOf(address, { result: true, fallback: false })],
      [vector.normalized, vector.site, vector.page, vector.view],
      vector.url,
    );
  }
  for (const url of VECTORS.invalid) {
    await assertRejects(() => reflowAddress(url), "");
  }
});

Deno.test("package matching follows the shared vectors", async () => {
  for (const vector of VECTORS.match) {
    assertEquals(
      packageMatches(vector.manifest_match, await reflowAddress(vector.url)),
      vector.matches,
      `${JSON.stringify(vector.manifest_match)} ${vector.url}`,
    );
  }
});

Deno.test("a site is a host name with dashes; one without a dash gets -host", () => {
  assertEquals(siteOfHost("www.facebook.com"), "www-facebook-com");
  assertEquals(siteOfHost("localhost"), "localhost-host");
  assertEquals(siteOfHost("WWW.Example.com."), "www-example-com");
  assertEquals(normalizeUrl("https://www.example.com:443/a#b").url, "https://www.example.com/a");
  assertEquals(pathMatches("/news/*", "/news/2026/a"), true);
  assertEquals(pathMatches("/news/*", "/news"), false);
  for (const host of ["[::1]", "a/b", "a b", ""]) {
    try {
      siteOfHost(host);
    } catch (err) {
      if (err instanceof ReflowError) continue;
    }
    throw new Error(`${host} must not be a site`);
  }
});

const SITE = "www-example-com";
const manifest = (patch: Record<string, unknown> = {}) => ({
  schema: "buckyos.reflow/1",
  site: SITE,
  match: [{ host: "www.example.com", path: "/news/*" }],
  preference: "只看标题、日期和正文，手机单栏",
  author: { kind: "agent", id: "did:bns:jarvis.alice" },
  updated_at: 1794600000000,
  refresh: { script: "refresh.mjs", timeout_s: 60 },
  ...patch,
});

Deno.test("a manifest is checked field by field", () => {
  assertEquals(manifestProblems(manifest(), SITE), []);
  assertEquals(manifestProblems(manifest({ refresh: undefined, preference: undefined }), SITE), []);
  const bad: [Record<string, unknown>, string][] = [
    [{ schema: "buckyos.reflow/2" }, "schema"],
    [{ site: "other-site" }, "site"],
    [{ match: [] }, "match"],
    [{ match: [{ host: "news.example.com" }] }, "match[0].host"],
    [{ match: [{ host: "WWW.example.com" }] }, "match[0].host"],
    [{ match: [{ host: "www.example.com:8080" }] }, "match[0].host"],
    [{ match: [{ host: "www.example.com", path: "news/*" }] }, "match[0].path"],
    [{ preference: 3 }, "preference"],
    [{ author: { kind: "robot", id: "x" } }, "author"],
    [{ author: undefined }, "author"],
    [{ updated_at: "today" }, "updated_at"],
    [{ refresh: { script: "other.mjs", timeout_s: 60 } }, "refresh.script"],
    [{ refresh: { script: "refresh.mjs", timeout_s: 0 } }, "refresh.timeout_s"],
  ];
  for (const [patch, field] of bad) {
    const problems = manifestProblems(manifest(patch), SITE);
    if (problems.length !== 1 || !problems[0].includes(field)) {
      throw new Error(`${JSON.stringify(patch)}: ${JSON.stringify(problems)}`);
    }
  }
  assertEquals(manifestProblems("x", SITE), ["manifest.json is not a JSON object"]);
});

Deno.test("a package has a manifest, refresh.mjs or index.html, and none of the library's data", () => {
  assertEquals(packageProblems(["index.html", "manifest.json", "refresh.mjs"], manifest(), SITE), []);
  assertEquals(
    packageProblems(["index.html", "manifest.json"], manifest({ refresh: undefined }), SITE),
    [],
  );
  const one = (files: string[], value: unknown, includes: string) => {
    const problems = packageProblems(files, value, SITE);
    if (problems.length !== 1 || !problems[0].includes(includes)) {
      throw new Error(`${files}: ${JSON.stringify(problems)}`);
    }
  };
  one(["index.html"], undefined, "manifest.json is missing");
  one(["manifest.json", "style.css"], manifest({ refresh: undefined }), "refresh.mjs, index.html or both");
  one(["index.html", "manifest.json"], manifest(), "refresh.mjs is missing");
  one(["index.html", "manifest.json", "results/a.json"], manifest({ refresh: undefined }), "results/");
  one(["fallback/a.html", "index.html", "manifest.json"], manifest({ refresh: undefined }), "fallback/");
  one(["index.html", "manifest.json", "preference.json"], manifest({ refresh: undefined }), "preference.json");
  // Only the one beside the manifest is the library's.
  assertEquals(
    packageProblems(["assets/preference.json", "index.html", "manifest.json"], manifest({ refresh: undefined }), SITE),
    [],
  );
});

Deno.test("package files are served with their web types", () => {
  assertEquals(contentTypeOf("index.html"), "text/html; charset=utf-8");
  assertEquals(contentTypeOf("refresh.mjs"), "text/javascript; charset=utf-8");
  assertEquals(contentTypeOf("a/b/style.CSS"), "text/css; charset=utf-8");
  assertEquals(contentTypeOf("manifest.json"), "application/json");
  assertEquals(contentTypeOf("blob"), "application/octet-stream");
});

const BUCKET = libraryBucketId("alice");

// `<site>/preference.json` as the App writes it for a site without a package.
const kept = (preference: string) => ({ schema: "buckyos.reflow.preference/1", preference, updated_at: 1794500000000 });

// A bucket service of its own: lists two entries at a time, like a server page.
function fakeStore(initial: Record<string, string> = {}, options: { readOnly?: boolean } = {}) {
  const objects = new Map<string, { body: Uint8Array; contentType?: string }>();
  const encode = (text: string) => new TextEncoder().encode(text);
  for (const [key, text] of Object.entries(initial)) objects.set(key, { body: encode(text) });
  const puts: string[] = [];
  const check = (bucketId: string) => assertEquals(bucketId, BUCKET, "bucket");
  const denied = () => Object.assign(new Error("write on user/control-panel/alice/reflow"), { code: "permission_denied" });
  const store: LibraryStore = {
    listObjects(bucketId, req) {
      check(bucketId);
      const prefix = req.prefix ?? "";
      const after = req.start_after ?? "";
      const out = { items: [] as { key: string }[], common_prefixes: [] as string[], next_start_after: undefined as string | undefined };
      let last = "";
      for (const key of [...objects.keys()].sort()) {
        if (!key.startsWith(prefix) || key <= after) continue;
        if (req.delimiter && after.endsWith(req.delimiter) && key.startsWith(after)) continue;
        const cut = req.delimiter ? key.indexOf(req.delimiter, prefix.length) : -1;
        const common = cut >= 0 ? key.slice(0, cut + 1) : undefined;
        if (common && out.common_prefixes.includes(common)) continue;
        if (out.items.length + out.common_prefixes.length === 2) {
          out.next_start_after = last;
          break;
        }
        last = common ?? key;
        if (common) out.common_prefixes.push(common);
        else out.items.push({ key });
      }
      return Promise.resolve(out);
    },
    fetchObject(bucketId, key) {
      check(bucketId);
      const object = objects.get(key);
      if (!object) throw Object.assign(new Error(`no such key: ${key}`), { code: "no_such_key" });
      return Promise.resolve(new Response(object.body as BodyInit));
    },
    putBytes(bucketId, key, body, putOptions) {
      check(bucketId);
      if (options.readOnly) throw denied();
      objects.set(key, {
        body: typeof body === "string" ? encode(body) : body,
        contentType: putOptions?.contentType,
      });
      puts.push(key);
      return Promise.resolve({});
    },
    delete(bucketId, key) {
      check(bucketId);
      if (options.readOnly) throw denied();
      objects.delete(key);
      return Promise.resolve({});
    },
  };
  const text = (key: string) => new TextDecoder().decode(objects.get(key)!.body);
  return { store, objects, puts, text, keys: () => [...objects.keys()].sort() };
}

async function packageDir(files: Record<string, string>): Promise<string> {
  const dir = await Deno.makeTempDir();
  for (const [name, text] of Object.entries(files)) {
    const path = `${dir}/${name}`;
    await Deno.mkdir(path.slice(0, path.lastIndexOf("/")), { recursive: true });
    await Deno.writeTextFile(path, text);
  }
  return dir;
}

Deno.test("put checks the package, uploads the manifest last and replaces the old package", async () => {
  const { store, puts, text, keys, objects } = fakeStore({
    [`${SITE}/manifest.json`]: JSON.stringify(manifest()),
    [`${SITE}/index.html`]: "old",
    [`${SITE}/old.css`]: "old",
    [`${SITE}/results/0123456789abcdef.json`]: "{}",
    [`${SITE}/fallback/0123456789abcdef.html`]: "<p>",
    "other-site/index.html": "other",
  });
  const local = manifest({ author: undefined, updated_at: undefined });
  const dir = await packageDir({
    "manifest.json": JSON.stringify(local),
    "refresh.mjs": "export default async () => ({})",
    "index.html": "<!doctype html>",
    "assets/app.js": "1",
    ".DS_Store": "x",
  });
  try {
    const report = await putPackage(store, BUCKET, SITE, dir, { agentDid: "did:bns:jarvis.alice", now: 1794700000000 });
    assertEquals(report.files, ["assets/app.js", "index.html", "manifest.json", "refresh.mjs"]);
    assertEquals(report.removed, ["old.css"]);
    assertEquals(puts.at(-1), `${SITE}/manifest.json`);
    assertEquals(keys(), [
      "other-site/index.html",
      `${SITE}/assets/app.js`,
      `${SITE}/fallback/0123456789abcdef.html`,
      `${SITE}/index.html`,
      `${SITE}/manifest.json`,
      `${SITE}/refresh.mjs`,
      `${SITE}/results/0123456789abcdef.json`,
    ]);
    const stored = JSON.parse(text(`${SITE}/manifest.json`));
    assertEquals([stored.updated_at, stored.author], [1794700000000, { kind: "agent", id: "did:bns:jarvis.alice" }]);
    assertEquals(objects.get(`${SITE}/index.html`)!.contentType, "text/html; charset=utf-8");
    assertEquals(objects.get(`${SITE}/refresh.mjs`)!.contentType, "text/javascript; charset=utf-8");
    assertEquals(manifestProblems(stored, SITE), []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("put uploads nothing of an invalid package and names every problem", async () => {
  const { store, puts } = fakeStore();
  const dir = await packageDir({
    "manifest.json": JSON.stringify(manifest({ site: "elsewhere-com", match: [{ host: "elsewhere.com" }] })),
    "style.css": "x",
  });
  try {
    const err = await assertRejects(() => putPackage(store, BUCKET, SITE, dir), "is not valid");
    assertEquals(err.problems.length, 4, JSON.stringify(err.problems));
    await assertRejects(() => putPackage(store, BUCKET, "localhost", dir), "not a site name");
    assertEquals(puts, []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("a store that refuses the write is reported with its code", async () => {
  const { store } = fakeStore({}, { readOnly: true });
  const dir = await packageDir({ "manifest.json": JSON.stringify(manifest({ refresh: undefined })), "index.html": "x" });
  try {
    await putPackage(store, BUCKET, SITE, dir);
  } catch (err) {
    assertEquals((err as { code?: string }).code, "permission_denied");
    return;
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
  throw new Error("expected permission_denied");
});

Deno.test("get writes the package without the library's data; rm removes the site", async () => {
  const { store, keys } = fakeStore({
    [`${SITE}/manifest.json`]: JSON.stringify(manifest()),
    [`${SITE}/assets/app.js`]: "1",
    [`${SITE}/index.html`]: "<!doctype html>",
    [`${SITE}/preference.json`]: JSON.stringify(kept("Text only")),
    [`${SITE}/refresh.mjs`]: "export default async () => ({})",
    [`${SITE}/results/0123456789abcdef.json`]: "{}",
    "other-site/index.html": "other",
    "wish-site/preference.json": JSON.stringify(kept("Large text")),
  });
  const dir = await Deno.makeTempDir();
  try {
    const files = await getPackage(store, BUCKET, SITE, `${dir}/pkg`);
    assertEquals(files, ["assets/app.js", "index.html", "manifest.json", "refresh.mjs"]);
    assertEquals(await Deno.readTextFile(`${dir}/pkg/assets/app.js`), "1");
    await assertRejects(() => getPackage(store, BUCKET, SITE, `${dir}/pkg`), "not empty");
    await assertRejects(() => getPackage(store, BUCKET, "nothing-here", `${dir}/none`), "no package");
    // A preference alone is no package to get; rm takes it with the site.
    await assertRejects(() => getPackage(store, BUCKET, "wish-site", `${dir}/wish`), "no package");
    assertEquals(await removeSite(store, BUCKET, SITE), 6);
    assertEquals(keys(), ["other-site/index.html", "wish-site/preference.json"]);
    assertEquals(await removeSite(store, BUCKET, SITE), 0);
    assertEquals(await removeSite(store, BUCKET, "wish-site"), 1);
    assertEquals(keys(), ["other-site/index.html"]);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("ls lists every site, with its package when it has a valid one, and the user's preference", async () => {
  const { store } = fakeStore({
    "a-site/manifest.json": JSON.stringify(manifest({ site: "a-site", match: [{ host: "a.site" }], preference: undefined })),
    "a-site/preference.json": JSON.stringify(kept("Stale: the package is what counts")),
    "b-site/fallback/0123456789abcdef.html": "<p>",
    "c-site/manifest.json": "{ broken",
    "c-site/preference.json": JSON.stringify(kept("Text only")),
    "d-site/preference.json": JSON.stringify(kept("Large text")),
    "e-site/preference.json": "{ broken",
    [`${SITE}/manifest.json`]: JSON.stringify(manifest()),
  });
  const sites = await listSites(store, BUCKET);
  assertEquals(sites.map((entry) => [entry.site, !!entry.package, entry.preference]), [
    ["a-site", true, ""],
    ["b-site", false, ""],
    ["c-site", false, "Text only"],
    ["d-site", false, "Large text"],
    ["e-site", false, ""],
    [SITE, true, "只看标题、日期和正文，手机单栏"],
  ]);
  assertEquals(sites[5].package!.match, [{ host: "www.example.com", path: "/news/*" }]);
  assertEquals(await readPreference(store, BUCKET, "c-site"), "Text only");
  assertEquals(await readPreference(store, BUCKET, SITE), "只看标题、日期和正文，手机单栏");
  assertEquals(await readPreference(store, BUCKET, "no-site"), "");
});

// The same rules as the App's import (Agent Browser Protocol §5.2).
Deno.test("put keeps the user's preference: the manifest's own, else the site's, and preference.json gives way", async () => {
  const files = (preference?: string) => ({
    "manifest.json": JSON.stringify(manifest({ refresh: undefined, author: undefined, updated_at: undefined, preference })),
    "index.html": "<!doctype html>",
  });
  const put = async (initial: Record<string, string>, preference?: string) => {
    const library = fakeStore(initial);
    const dir = await packageDir(files(preference));
    try {
      const report = await putPackage(library.store, BUCKET, SITE, dir, { agentDid: "did:bns:jarvis.alice", now: 1794700000000 });
      const stored = JSON.parse(library.text(`${SITE}/manifest.json`));
      assertEquals(stored.preference, report.manifest.preference);
      assertEquals(manifestProblems(stored, SITE), []);
      return { preference: stored.preference, removed: report.removed, keys: library.keys(), puts: library.puts };
    } finally {
      await Deno.remove(dir, { recursive: true });
    }
  };
  const keptFile = { [`${SITE}/preference.json`]: JSON.stringify(kept("Text only")) };
  const packageKeys = [`${SITE}/index.html`, `${SITE}/manifest.json`];

  // A site without a package: what the user asked for in the App goes into the manifest.
  let done = await put(keptFile);
  assertEquals([done.preference, done.removed, done.keys], ["Text only", ["preference.json"], packageKeys]);
  assertEquals(done.puts.at(-1), `${SITE}/manifest.json`, "the preference is in the manifest before its file goes");

  // The manifest states one: that one counts.
  done = await put(keptFile, "Title and text");
  assertEquals([done.preference, done.removed, done.keys], ["Title and text", ["preference.json"], packageKeys]);
  done = await put(keptFile, "");
  assertEquals([done.preference, done.keys], ["", packageKeys]);

  // A package replacing a package: the old one's preference stays unless the new one states its own.
  const old = { [`${SITE}/manifest.json`]: JSON.stringify(manifest()), [`${SITE}/old.css`]: "old" };
  done = await put(old);
  assertEquals([done.preference, done.removed], ["只看标题、日期和正文，手机单栏", ["old.css"]]);
  done = await put(old, "Title and text");
  assertEquals(done.preference, "Title and text");

  // Beside a valid package the manifest is where the preference is: a preference.json left there is not read.
  done = await put({ ...keptFile, [`${SITE}/manifest.json`]: JSON.stringify(manifest({ preference: undefined })) });
  assertEquals([done.preference, done.removed, done.keys], [undefined, ["preference.json"], packageKeys]);
  // Beside a broken one it is.
  done = await put({ ...keptFile, [`${SITE}/manifest.json`]: "{ broken" });
  assertEquals([done.preference, done.keys], ["Text only", packageKeys]);

  // Nothing kept, nothing stated: no preference.
  done = await put({});
  assertEquals([done.preference, done.removed], [undefined, []]);
  done = await put({ [`${SITE}/preference.json`]: "{ broken" });
  assertEquals([done.preference, done.keys], [undefined, packageKeys]);
});

Deno.test("a package cannot bring a preference.json; the library's stays until a valid package is in", async () => {
  const library = fakeStore({ [`${SITE}/preference.json`]: JSON.stringify(kept("Text only")) });
  const dir = await packageDir({
    "manifest.json": JSON.stringify(manifest({ refresh: undefined })),
    "index.html": "<!doctype html>",
    "preference.json": JSON.stringify(kept("From the package")),
  });
  try {
    const err = await assertRejects(() => putPackage(library.store, BUCKET, SITE, dir), "is not valid");
    assertEquals(err.problems.length, 1);
    if (!err.problems[0].includes("state the preference in manifest.json")) throw new Error(err.problems[0]);
    assertEquals(library.puts, []);
    assertEquals(library.keys(), [`${SITE}/preference.json`]);
    assertEquals(await readPreference(library.store, BUCKET, SITE), "Text only");
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("what the library shows for a URL: a package's result, else a fallback page", async () => {
  const address = await reflowAddress("https://www.example.com/news/1?a=1#top");
  const other = await reflowAddress("https://www.example.com/about");
  const made = { made_at: 1794600001000, made_by: "did:bns:jarvis.alice", source_url: address.url };
  const { store } = fakeStore({ [`${SITE}/manifest.json`]: JSON.stringify(manifest()) });
  let state = await libraryState(store, BUCKET, address);
  assertEquals([state.package?.updated_at, state.result, state.fallback, state.view], [1794600000000, null, null, null]);
  assertEquals((await libraryState(store, BUCKET, other)).package, null, "the path does not match");

  const dir = await packageDir({ "data.json": '{"title":"T"}', "page.html": "<h1>T</h1>", "notes.txt": "x" });
  try {
    const result = await putResult(store, BUCKET, address, `${dir}/data.json`, made.made_by, made.made_at);
    assertEquals([result.kind, result.key], ["result", `${SITE}/results/${address.page}.json`]);
    assertEquals(result.state.result, made);
    assertEquals(
      result.state.view,
      `${SITE}/?url=${encodeURIComponent("https://www.example.com/news/1?a=1")}&page=${address.page}`,
    );
    const stored = JSON.parse(await (await store.fetchObject(BUCKET, result.key)).text());
    assertEquals(stored, { schema: "buckyos.reflow.result/1", source_url: address.url, made_at: made.made_at, made_by: made.made_by, data: { title: "T" } });

    // A page no package covers: data has nowhere to be drawn, a fallback page shows by itself.
    await assertRejects(() => putResult(store, BUCKET, other, `${dir}/data.json`, made.made_by), "no package");
    await assertRejects(() => putResult(store, BUCKET, other, `${dir}/notes.txt`, made.made_by), ".json");
    const fallback = await putResult(store, BUCKET, other, `${dir}/page.html`, made.made_by, made.made_at);
    assertEquals([fallback.kind, fallback.key], ["fallback", `${SITE}/fallback/${other.page}.html`]);
    assertEquals(fallback.state.view, `${SITE}/fallback/${other.page}.html`);
    assertEquals(fallback.state.fallback, { ...made, source_url: other.url });
    state = await libraryState(store, BUCKET, other);
    assertEquals([state.package, state.result], [null, null]);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

// ── The command itself, as the Agent runs it ───────────────────────────────

const TOOL_DIR = new URL("..", import.meta.url).pathname;

async function reflow(args: string[], env: Record<string, string> = {}) {
  const output = await new Deno.Command(Deno.execPath(), {
    args: ["run", "-A", "--config", `${TOOL_DIR}deno.json`, `${TOOL_DIR}aicc-tool.ts`, "reflow", ...args],
    env,
    clearEnv: true,
    stdout: "piped",
    stderr: "piped",
  }).output();
  const stdout = new TextDecoder().decode(output.stdout).trim();
  return { code: output.code, result: stdout ? JSON.parse(stdout) : null };
}

// An Agent Service answering `browser.reflow_*` with the states given, in order.
function fakeAgentService(answers: (Record<string, unknown> | string)[]) {
  const calls: { method: string; params: unknown; token: unknown }[] = [];
  const server = Deno.serve({ port: 0, hostname: "127.0.0.1", onListen: () => {} }, async (req) => {
    const body = await req.json();
    calls.push({ method: body.method, params: body.params, token: body.sys[1] });
    const answer = answers[Math.min(calls.length, answers.length) - 1];
    return Response.json(typeof answer === "string" ? { error: answer, sys: [body.sys[0]] } : { result: answer, sys: [body.sys[0]] });
  });
  const { port } = server.addr as Deno.NetAddr;
  return {
    calls,
    env: { OPENDAN_BROWSER_RPC: `http://127.0.0.1:${port}/kapi/jarvis.alice`, BUCKYOS_APPCLIENT_SESSION_TOKEN: "session-token" },
    close: () => server.shutdown(),
  };
}

const URL_1 = "https://www.example.com/news/1";
const state = (status: string, extra: Record<string, unknown> = {}) => ({
  url: URL_1,
  site: SITE,
  page: "0123456789abcdef",
  package: { updated_at: 1794600000000, author: { kind: "agent", id: "did:bns:jarvis.alice" }, preference: "" },
  result: null,
  fallback: null,
  refresh: { status, error: null, at_ms: null },
  view: null,
  ...extra,
});

Deno.test("reflow: bad arguments end before any service is called", async () => {
  for (const args of [[], ["bogus"], ["match"], ["put", SITE], ["result", "get", "x", "y"], ["ls", "--force"]]) {
    const { code, result } = await reflow(args);
    assertEquals([code, result.status, result.tool], [1, "error", "reflow"], args.join(" "));
  }
});

Deno.test("reflow refresh asks the Agent Service and waits for the result", async () => {
  const made = { made_at: 1794600002000, made_by: "did:bns:jarvis.alice", source_url: URL_1 };
  const view = `${SITE}/?url=${encodeURIComponent(URL_1)}&page=0123456789abcdef`;
  const service = fakeAgentService([state("running"), state("running"), state("idle", { result: made, view })]);
  try {
    const { code, result } = await reflow(["refresh", URL_1], service.env);
    assertEquals([code, result.status, result.detail.result, result.detail.view], [0, "success", made, view]);
    assertEquals(service.calls.map((call) => call.method), [
      "browser.reflow_refresh",
      "browser.reflow_state",
      "browser.reflow_state",
    ]);
    assertEquals([service.calls[0].params, service.calls[0].token], [{ url: URL_1 }, "session-token"]);
  } finally {
    await service.close();
  }
});

Deno.test("reflow refresh reports a failed script, a missing package and a missing browser", async () => {
  const failing = fakeAgentService([state("running"), state("failed", { refresh: { status: "failed", error: "TypeError: x is not a function", at_ms: 1 } })]);
  try {
    const { code, result } = await reflow(["refresh", URL_1], failing.env);
    assertEquals([code, result.status, result.title], [2, "error", "reflow refresh => refresh_failed"]);
    if (!result.summary.includes("TypeError: x is not a function")) throw new Error(result.summary);
  } finally {
    await failing.close();
  }

  const none = fakeAgentService(['browser: {"kind":"invalid","code":"reflow_no_package","message":"no package matches"}']);
  try {
    const { code, result } = await reflow(["refresh", URL_1], none.env);
    assertEquals([code, result.title, result.detail.error_code], [2, "reflow refresh => reflow_no_package", "reflow_no_package"]);
    if (!result.summary.includes("reflow put")) throw new Error(result.summary);
  } finally {
    await none.close();
  }

  const started = fakeAgentService([state("running")]);
  try {
    const noWait = await reflow(["refresh", URL_1, "--no-wait"], started.env);
    assertEquals([noWait.code, noWait.result.status, started.calls.length], [0, "success", 1]);
    const timedOut = await reflow(["refresh", URL_1, "--timeout", "1"], started.env);
    assertEquals([timedOut.code, timedOut.result.title], [4, "reflow refresh => wait_timeout"]);
  } finally {
    await started.close();
  }

  const { code, result } = await reflow(["refresh", URL_1]);
  assertEquals([code, result.title], [2, "reflow refresh => no_browser"]);
});
