import { failureResult, HELP } from "../commands/reflow.ts";
import {
  contentTypeOf,
  fallbackKey,
  libraryBucketId,
  libraryState,
  listSites,
  normalizeUrl,
  pathMatches,
  putFallback,
  reflowAddress,
  ReflowError,
  removeSite,
  rulesMatch,
  siteOfHost,
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
  normalize: { url: string; normalized: string; site: string; page: string }[];
  invalid: string[];
  match: { manifest_match: { host: string; path?: string }[]; url: string; matches: boolean }[];
};

Deno.test("site, normalized URL and page follow the shared vectors", async () => {
  for (const vector of VECTORS.normalize) {
    const address = await reflowAddress(vector.url);
    assertEquals([address.url, address.site, address.page], [vector.normalized, vector.site, vector.page], vector.url);
    assertEquals(fallbackKey(address), `${vector.site}/fallback/${vector.page}.html`);
  }
  for (const url of VECTORS.invalid) {
    await assertRejects(() => reflowAddress(url), "");
  }
});

Deno.test("match rules follow the shared vectors", async () => {
  for (const vector of VECTORS.match) {
    assertEquals(
      rulesMatch(vector.manifest_match, await reflowAddress(vector.url)),
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

Deno.test("files are served with their web types", () => {
  assertEquals(contentTypeOf("index.html"), "text/html; charset=utf-8");
  assertEquals(contentTypeOf("fetch.mjs"), "text/javascript; charset=utf-8");
  assertEquals(contentTypeOf("a/b/style.CSS"), "text/css; charset=utf-8");
  assertEquals(contentTypeOf("site.json"), "application/json");
  assertEquals(contentTypeOf("blob"), "application/octet-stream");
});

const SITE = "www-example-com";
const BUCKET = libraryBucketId("alice");

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

async function fileDir(files: Record<string, string>): Promise<string> {
  const dir = await Deno.makeTempDir();
  for (const [name, text] of Object.entries(files)) await Deno.writeTextFile(`${dir}/${name}`, text);
  return dir;
}

Deno.test("what the library shows for a URL is the fallback page kept for it", async () => {
  const address = await reflowAddress("https://www.example.com/news/1?a=1#top");
  const other = await reflowAddress("https://www.example.com/about");
  const made = { made_at: 1794600001000, made_by: "did:bns:jarvis.alice", source_url: address.url };
  const { store, objects, puts, text } = fakeStore();
  assertEquals(await libraryState(store, BUCKET, address), {
    url: "https://www.example.com/news/1?a=1",
    site: SITE,
    page: address.page,
    fallback: null,
    view: null,
  });

  const dir = await fileDir({ "page.html": "<h1>T</h1>", "data.json": '{"title":"T"}', "notes.txt": "x" });
  try {
    const report = await putFallback(store, BUCKET, address, `${dir}/page.html`, made.made_by, made.made_at);
    const key = `${SITE}/fallback/${address.page}.html`;
    assertEquals(report, { key, state: { url: address.url, site: SITE, page: address.page, fallback: made, view: key } });
    assertEquals(puts, [key, `${SITE}/fallback/${address.page}.json`], "the page before what says it is there");
    assertEquals([text(key), objects.get(key)!.contentType], ["<h1>T</h1>", "text/html; charset=utf-8"]);
    assertEquals(JSON.parse(text(`${SITE}/fallback/${address.page}.json`)), { schema: "buckyos.reflow.fallback/1", source_url: address.url, made_at: made.made_at, made_by: made.made_by });
    assertEquals((await libraryState(store, BUCKET, other)).view, null, "another page of the site has none");

    // Only a page: data for a package to draw has nowhere to go any more.
    for (const file of ["data.json", "notes.txt"]) {
      await assertRejects(() => putFallback(store, BUCKET, other, `${dir}/${file}`, made.made_by), "a fallback page is a .html file");
    }
    // A file that is not there is the caller's local error, not the library's.
    try {
      await putFallback(store, BUCKET, other, `${dir}/missing.html`, made.made_by);
      throw new Error("expected NotFound");
    } catch (err) {
      assertEquals((err as Error).name, "NotFound");
      assertEquals(failureResult(err).exitCode, 5);
    }
    assertEquals(puts.length, 2);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("ls lists the sites that have fallback pages; rm removes everything of a site", async () => {
  const { store, keys } = fakeStore({
    "a-site/fallback/0123456789abcdef.html": "<p>",
    "a-site/fallback/0123456789abcdef.json": "{}",
    "a-site/fallback/fedcba9876543210.html": "<p>",
    "a-site/fallback/fedcba9876543210.json": "{}",
    "b-site/index.html": "left by a package of before",
    "c-site/fallback/0123456789abcdef.html": "<p>",
    // The reading site lives in the same bucket and is no site of the library.
    "_data/site.json": "{}",
    "_data/a/0123456789abcdef.json": "{}",
    "_app/t1/app.js": "1",
    "_src/news-example-com/s1/source.json": "{}",
    "index.html": "<!doctype html>",
  });
  assertEquals(await listSites(store, BUCKET), [{ site: "a-site", pages: 2 }, { site: "c-site", pages: 1 }]);
  assertEquals(await removeSite(store, BUCKET, "a-site"), 4);
  assertEquals(await removeSite(store, BUCKET, "a-site"), 0);
  assertEquals(await removeSite(store, BUCKET, "b-site"), 1);
  assertEquals(keys().filter((key) => !key.startsWith("_")), ["c-site/fallback/0123456789abcdef.html", "index.html"]);
  for (const name of ["_data", "localhost", "index.html", "A-Site"]) {
    await assertRejects(() => removeSite(store, BUCKET, name), "not a site name");
  }
  assertEquals(keys().filter((key) => key.startsWith("_")).length, 4, "nothing of the reading site was touched");
});

Deno.test("failures are told apart: bad input 1, the service 2, a local file 5", async () => {
  const of = (err: unknown, write = false) => {
    const { result, exitCode } = failureResult(err, { write });
    return [exitCode, result.title];
  };
  assertEquals(of(new ReflowError("not a URL: x")), [1, "reflow => invalid"]);
  assertEquals(of(new Deno.errors.NotFound("No such file or directory (os error 2): readfile 'page.html'")), [5, "reflow => io_failed"]);
  assertEquals(of(await Deno.readFile("/nonexistent/page.html").catch((err) => err)), [5, "reflow => io_failed"]);
  const coded = (code: string) => Object.assign(new Error(code), { code });
  assertEquals(of(coded("permission_denied"), true), [2, "reflow => permission_denied"]);
  assertEquals(of(coded("permission_denied")), [2, "reflow => failed"]);
  assertEquals(of(coded("bucket_not_found")), [2, "reflow => no_library"]);
  assertEquals(of(new Error("connection reset")), [2, "reflow => failed"]);

  const { store } = fakeStore({}, { readOnly: true });
  const dir = await fileDir({ "page.html": "<h1>T</h1>" });
  try {
    const address = await reflowAddress("https://www.example.com/news/1");
    const err = await putFallback(store, BUCKET, address, `${dir}/page.html`, "did:bns:jarvis.alice").catch((e) => e);
    assertEquals(of(err, true), [2, "reflow => permission_denied"]);
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
  return { code: output.code, result: stdout ? JSON.parse(stdout) : null, stderr: new TextDecoder().decode(output.stderr) };
}

Deno.test("reflow: bad arguments end before any service is called", async () => {
  const bad = [
    [], ["bogus"], ["match"], ["ls", "--force"], ["rm"], ["result", "get", "x", "y"], ["result", "put", "https://a.example/"],
    // Gone with the packages.
    ["put", SITE, "dir"], ["get", SITE, "dir"], ["refresh", "https://www.example.com/"], ["match", "https://a.example/", "--no-wait"],
  ];
  for (const args of bad) {
    const { code, result } = await reflow(args);
    assertEquals([code, result.status, result.tool, result.title], [1, "error", "reflow", "reflow => arg_error"], args.join(" "));
  }
});

Deno.test("reflow --help describes what is left and points to site for reading sites", async () => {
  const { code, result, stderr } = await reflow(["--help"]);
  assertEquals([code, result, stderr.trim()], [0, null, HELP]);
  for (const kept of ["reflow match <url>", "reflow ls", "reflow rm <site>", "reflow result put <url> <page.html>", "site --help", "Exit codes:"]) {
    if (!HELP.includes(kept)) throw new Error(`the help does not mention: ${kept}`);
  }
  for (const gone of ["manifest", "refresh", "package", "preference", "reflow put", "reflow get", "results/", "--timeout", "--no-wait"]) {
    if (HELP.includes(gone)) throw new Error(`the help still mentions: ${gone}`);
  }
});
