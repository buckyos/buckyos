import {
  decodeText,
  domBlocks,
  extractContext,
  extractedProblems,
  failureOf,
  feedEntries,
  loadDom,
  Net,
  runExtract,
  USER_AGENT,
} from "./extract.ts";
import type { Validators } from "./extract.ts";
import { assert, assertEquals, assertFails, cannedFetch, tempDir } from "./testing.ts";
import { SiteError } from "./types.ts";
import type { SourceDef } from "./types.ts";

const DEF: SourceDef = {
  schema: "buckyos.site.source/1",
  id: "news-example-com",
  title: "Example",
  home_url: "https://news.example.com/",
  match: [{ host: "news.example.com" }],
  lang: "ja",
  kind: "news",
  needs: "http",
};

// A clock that only moves when the code under test sleeps.
function clock(start = 1_000_000) {
  let now = start;
  const sleeps: number[] = [];
  return {
    now: () => now,
    sleep: (ms: number) => {
      sleeps.push(ms);
      now += ms;
      return Promise.resolve();
    },
    advance: (ms: number) => now += ms,
    sleeps,
  };
}

Deno.test("requests carry one User-Agent and keep a second between two to the same host", async () => {
  const time = clock();
  const { fetcher, requests } = cannedFetch({
    "https://a.example/1": "one",
    "https://a.example/2": "two",
    "https://b.example/1": "other",
  });
  const net = new Net({ fetcher, now: time.now, sleep: time.sleep });
  await net.get("https://a.example/1");
  await net.get("https://b.example/1");
  await net.get("https://a.example/2");
  assertEquals(time.sleeps, [1000], "only the second request to a.example waits");
  time.advance(5000);
  await net.get("https://a.example/1");
  assertEquals(time.sleeps, [1000], "a host left alone for a while is asked at once");
  assertEquals(requests.map((request) => request.headers["user-agent"]), Array(4).fill(USER_AGENT));
  assertEquals([net.requests, net.bytes], [4, 3 + 5 + 3 + 3]);
});

Deno.test("network failures, 429 and 5xx are transient; 401, 403 and 404 are the extractor's to judge", async () => {
  const { fetcher } = cannedFetch({
    "https://a.example/busy": { status: 503 },
    "https://a.example/slow": { status: 429 },
    "https://a.example/login": { status: 401, body: "sign in" },
    "https://a.example/big": "x".repeat(2000),
  });
  const net = new Net({ fetcher, sleep: () => Promise.resolve() });
  for (const path of ["busy", "slow"]) {
    const err = await assertFails(() => net.get(`https://a.example/${path}`), "HTTP");
    assertEquals((err as SiteError).kind, "transient");
  }
  assertEquals((await net.get("https://a.example/login")).status, 401);
  assertEquals((await net.get("https://a.example/none")).status, 404);
  const down = new Net({ fetcher: () => Promise.reject(new TypeError("connection refused")), sleep: () => Promise.resolve() });
  assertEquals((await assertFails(() => down.get("https://a.example/"), "connection refused") as SiteError).kind, "transient");
  assertEquals((await assertFails(() => net.get("https://a.example/big", { maxBytes: 1000 }), "2000 bytes") as SiteError).kind, "program");
  assertEquals((await assertFails(() => net.get("file:///etc/passwd"), "http(s)") as SiteError).kind, "program");
});

Deno.test("a conditional fetch sends the validators kept from last time and reports 304", async () => {
  let modified = true;
  const { fetcher, requests } = cannedFetch({
    "https://news.example.com/rss": () =>
      modified
        ? { body: "<rss/>", headers: { etag: 'W/"v1"', "last-modified": "Wed, 25 Nov 2026 10:00:00 GMT" } }
        : { status: 304 },
  });
  const net = new Net({ fetcher, sleep: () => Promise.resolve() });
  const validators: Record<string, Validators> = {};
  const ctx = await extractContext({
    source: DEF, dir: ".", state: null, validators, limit: 5, known: () => false, net, log: () => {},
  });
  const first = await ctx.fetch("https://news.example.com/rss", { conditional: true });
  assertEquals([first.status, first.text, first.notModified], [200, "<rss/>", false]);
  assertEquals(validators, {
    "https://news.example.com/rss": { etag: 'W/"v1"', last_modified: "Wed, 25 Nov 2026 10:00:00 GMT" },
  });
  modified = false;
  const second = await ctx.fetch("https://news.example.com/rss", { conditional: true });
  assertEquals([second.status, second.text, second.notModified, net.notModified], [304, "", true, 1]);
  assertEquals(
    [requests[1].headers["if-none-match"], requests[1].headers["if-modified-since"]],
    ['W/"v1"', "Wed, 25 Nov 2026 10:00:00 GMT"],
  );
  await ctx.fetch("https://news.example.com/rss");
  assertEquals(requests[2].headers["if-none-match"], undefined, "an unconditional fetch sends none");
});

Deno.test("a page is decoded with the charset it names", () => {
  const sjis = Uint8Array.from([0x93, 0xfa, 0x96, 0x7b]);
  assertEquals(decodeText(sjis, "text/html; charset=Shift_JIS"), "日本");
  const meta = new Uint8Array([...new TextEncoder().encode('<meta charset="shift_jis">'), ...sjis]);
  assertEquals(decodeText(meta, "text/html").endsWith("日本"), true);
  assertEquals(decodeText(new TextEncoder().encode("日本"), null), "日本");
  assertEquals(decodeText(sjis, "text/html").includes("�"), true, "a wrong guess shows as U+FFFD");
});

const RSS2 = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>Example</title><link>https://news.example.com/</link>
<item><title><![CDATA[日本語の見出し & more]]></title><link>https://news.example.com/a/1</link>
<pubDate>Wed, 25 Nov 2026 15:17:03 GMT</pubDate><comments>https://news.example.com/a/1/comments</comments>
<description><![CDATA[<p>Lead <b>text</b>.</p><img src="https://img.example.com/1.jpg">]]></description>
<content:encoded><![CDATA[<p>Full text.</p>]]></content:encoded></item>
<item><title>Second</title><guid>https://news.example.com/a/2</guid><media:thumbnail url="https://img.example.com/2.jpg"/></item>
<item><title>No address</title></item>
</channel></rss>`;

const RSS1 = `<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
<channel rdf:about="https://example.org/"><title>RDF</title></channel>
<item rdf:about="https://example.org/1"><title>RDF item</title><link>https://example.org/1</link>
<dc:date>2026-11-25T09:00:00+09:00</dc:date><description>Plain summary</description></item>
</rdf:RDF>`;

const ATOM = `<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom</title>
<entry><title>Atom entry</title><link rel="self" href="https://example.net/self"/><link rel="alternate" href="https://example.net/post"/>
<updated>2026-11-25T00:00:00Z</updated><summary type="html">&lt;p&gt;Summary&lt;/p&gt;</summary>
<content type="html">&lt;p&gt;Body&lt;/p&gt;</content></entry>
<entry><title>Bare</title><link href="https://example.net/bare"/></entry>
</feed>`;

Deno.test("RSS 2.0, RSS 1.0 and Atom give the same entries", async () => {
  const dom = await loadDom();
  assertEquals(feedEntries(dom.xml(RSS2), dom), [
    {
      url: "https://news.example.com/a/1",
      title: "日本語の見出し & more",
      published_at: Date.UTC(2026, 10, 25, 15, 17, 3),
      summary: "Lead text.",
      image: "https://img.example.com/1.jpg",
      content_html: "<p>Full text.</p>",
      comments_url: "https://news.example.com/a/1/comments",
    },
    { url: "https://news.example.com/a/2", title: "Second", image: "https://img.example.com/2.jpg" },
  ]);
  assertEquals(feedEntries(dom.xml(RSS1), dom), [
    { url: "https://example.org/1", title: "RDF item", published_at: Date.UTC(2026, 10, 25, 0, 0, 0), summary: "Plain summary" },
  ]);
  assertEquals(feedEntries(dom.xml(ATOM), dom), [
    {
      url: "https://example.net/post",
      title: "Atom entry",
      published_at: Date.UTC(2026, 10, 25),
      summary: "Summary",
      content_html: "<p>Body</p>",
    },
    { url: "https://example.net/bare", title: "Bare" },
  ]);
  assertEquals(feedEntries(dom.xml("<html><body>not a feed</body></html>"), dom), []);
});

const ARTICLE_HTML = `<html><body><nav><a href="/">Home</a></nav>
<article>
  <h1>Headline</h1>
  <p>First   paragraph with a <a href="/related?x=1">related <b>story</b></a> and
     a line<br>break.</p>
  <div>Loose text<br><br>after two breaks</div>
  <h2>Section</h2><h4>Deeper</h4>
  <blockquote><p>Quoted words.</p></blockquote>
  <ul><li>one</li><li>two <a href="https://other.example/x">link</a></li></ul>
  <pre>code  line 1
  line 2</pre>
  <figure><img src="/img/a.jpg" alt=""><figcaption>The caption</figcaption></figure>
  <p><img data-src="https://cdn.example.com/lazy.png" src="data:image/gif;base64,R0lGOD" alt="Lazy"></p>
  <img src="/pixel.gif" width="1" height="1">
  <img src="/icons/LinkIcons/svg/github.svg" alt="GitHub"><img src="https://cdn.example.com/Logo.SVG?v=3">
  <img src="/img/placeholder.svg" data-src="/img/real.jpg" alt="Lazy too">
  <img src="data:image/svg+xml;base64,PHN2Zy8+" alt="inline icon">
  <table><tr><th>Year</th><th>Value</th></tr><tr><td>2026</td><td>4.6</td></tr></table>
  <table><tr><td><p>Layout table text.</p></td></tr></table>
  <p hidden>Hidden.</p><p style="display:none">Also hidden.</p>
  <script>var x = "<p>not text</p>";</script><form><input value="x"><button>Send</button></form>
  <p><a href="javascript:void(0)">Not a link</a>, <a href="#top">nor this</a>.</p>
</article><footer>Footer</footer></body></html>`;

Deno.test("an article's DOM becomes blocks in document order, with its links", async () => {
  const dom = await loadDom();
  const article = dom.html(ARTICLE_HTML).querySelector("article")!;
  const notes: string[] = [];
  const { blocks, links } = domBlocks(article, "https://news.example.com/a/1", (message) => notes.push(message));
  // An SVG is no picture to keep: it is skipped here, before anything asks for it, with a line that says so.
  assertEquals(notes, [
    "image https://news.example.com/icons/LinkIcons/svg/github.svg: an SVG is not a picture the site keeps — left out of the article",
    "image https://cdn.example.com/Logo.SVG?v=3: an SVG is not a picture the site keeps — left out of the article",
  ]);
  assertEquals(blocks, [
    { type: "h2", text: "Headline" },
    { type: "p", text: "First paragraph with a related story and a line\nbreak." },
    { type: "p", text: "Loose text" },
    { type: "p", text: "after two breaks" },
    { type: "h2", text: "Section" },
    { type: "h3", text: "Deeper" },
    { type: "quote", text: "Quoted words." },
    { type: "li", text: "one" },
    { type: "li", text: "two link" },
    { type: "code", text: "code  line 1\n  line 2" },
    { type: "img", url: "https://news.example.com/img/a.jpg", alt: "The caption" },
    { type: "img", url: "https://cdn.example.com/lazy.png", alt: "Lazy" },
    { type: "img", url: "https://news.example.com/img/real.jpg", alt: "Lazy too" },
    { type: "table", rows: [["Year", "Value"], ["2026", "4.6"]] },
    { type: "p", text: "Layout table text." },
    { type: "p", text: "Not a link, nor this." },
  ]);
  assertEquals(links, [
    { url: "https://news.example.com/related?x=1", text: "related story" },
    { url: "https://other.example/x", text: "link" },
  ]);
});

Deno.test("ctx.browser is for sources that need it, runs a script of the package, and maps an auth failure", async () => {
  const dir = await tempDir({ "fetch.mjs": "export default async () => 1" });
  try {
    const calls: unknown[] = [];
    const base = { dir, state: null, validators: {}, limit: 1, known: () => false, net: new Net(), log: () => {} };
    const http = await extractContext({ ...base, source: DEF, browser: () => Promise.resolve(1) });
    assertEquals((await assertFails(() => http.browser(null, "fetch.mjs"), "needs") as SiteError).kind, "program");

    const login = await extractContext({
      ...base,
      source: { ...DEF, needs: "login" },
      browser: (params) => {
        calls.push(params);
        return Promise.resolve({ items: 3 });
      },
    });
    assertEquals(await login.browser("https://news.example.com/my", "fetch.mjs", { page: 2 }), { items: 3 });
    assertEquals(calls, [{
      script_path: `${await Deno.realPath(dir)}/fetch.mjs`,
      url: "https://news.example.com/my",
      args: { page: 2 },
    }]);
    for (const script of ["fetch.js", "/etc/x.mjs", "../x.mjs"]) {
      assertEquals((await assertFails(() => login.browser(null, script), ".mjs file of the package") as SiteError).kind, "program");
    }
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("background_run through the Agent Service: an error that starts with 'auth: ' is auth", async () => {
  const answers: unknown[] = [
    { result: { data: { ok: true } } },
    { error: JSON.stringify({ code: "other", message: "auth: the session has expired" }) },
    { error: JSON.stringify({ code: "browser_unavailable", message: "the browser does not start" }) },
    { error: JSON.stringify({ code: "other", message: "TypeError: x is undefined" }) },
    { error: JSON.stringify({ code: "other", message: "agent-browser: background run failed: page.waitForSelector: Timeout 15000ms exceeded." }) },
    { error: JSON.stringify({ code: "other", message: "agent-browser: background run failed: page.goto: net::ERR_CONNECTION_REFUSED at http://news.example.com/" }) },
    { error: JSON.stringify({ code: "other", message: "agent-browser: background run failed: timed out after 60 seconds" }) },
  ];
  const seen: { method: string; params: unknown; token: string }[] = [];
  const server = Deno.serve({ hostname: "127.0.0.1", port: 0, onListen: () => {} }, async (request) => {
    const body = await request.json();
    seen.push({ method: body.method, params: body.params, token: body.sys[1] });
    return Response.json(answers.shift());
  });
  const dir = await tempDir({ "fetch.mjs": "export default async () => 1" });
  Deno.env.set("OPENDAN_BROWSER_RPC", `http://127.0.0.1:${server.addr.port}/kapi/agent`);
  Deno.env.set("BUCKYOS_APPCLIENT_SESSION_TOKEN", "token-1");
  try {
    const ctx = await extractContext({
      source: { ...DEF, needs: "browser" }, dir, state: null, validators: {}, limit: 1, known: () => false,
      net: new Net(), log: () => {},
    });
    assertEquals(await ctx.browser(null, "fetch.mjs"), { ok: true });
    assertEquals(seen[0], {
      method: "browser.background_run",
      params: { script_path: `${await Deno.realPath(dir)}/fetch.mjs` },
      token: "token-1",
    });
    const auth = await assertFails(() => ctx.browser(null, "fetch.mjs"), "the session has expired") as SiteError;
    assertEquals([auth.kind, auth.message], ["auth", "the session has expired"]);
    assertEquals((await assertFails(() => ctx.browser(null, "fetch.mjs"), "does not start") as SiteError).kind, "transient");
    assertEquals((await assertFails(() => ctx.browser(null, "fetch.mjs"), "TypeError") as SiteError).kind, "program");
    // Something the page no longer has is the program's to fix; a site that is down or slow is not.
    assertEquals((await assertFails(() => ctx.browser(null, "fetch.mjs"), "waitForSelector") as SiteError).kind, "program");
    assertEquals((await assertFails(() => ctx.browser(null, "fetch.mjs"), "ERR_CONNECTION_REFUSED") as SiteError).kind, "transient");
    assertEquals((await assertFails(() => ctx.browser(null, "fetch.mjs"), "timed out after 60 seconds") as SiteError).kind, "transient");
  } finally {
    Deno.env.delete("OPENDAN_BROWSER_RPC");
    Deno.env.delete("BUCKYOS_APPCLIENT_SESSION_TOKEN");
    await server.shutdown();
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("an extractor is loaded from its package, says how it failed, and what it returns is checked", async () => {
  const dir = await tempDir({
    "ok/extract.ts": `import { title } from "./lib.ts";
      export default async function extract(ctx) {
        const page = await ctx.fetch("https://news.example.com/a/1");
        const body = ctx.blocks(ctx.html(page.text).querySelector("article"), page.url);
        return { list: [{ url: page.url, title }], items: [{ url: page.url, title, blocks: body.blocks }, { url: page.url + "?2", title, blocks: [] }], state: { seen: ctx.state?.seen ?? 0 } };
      }`,
    "ok/lib.ts": `export const title = "From the package";`,
    "auth/extract.ts": `export default async () => { throw new SiteError("auth", "sign in again"); }`,
    "duck/extract.ts": `export default async () => { throw Object.assign(new Error("try later"), { kind: "transient" }); }`,
    "bug/extract.ts": `export default async () => { return undefined.list; }`,
    "shape/extract.ts": `export default async () => ({ list: [], items: [{ url: "/relative", title: "x", blocks: [{ type: "video" }] }] })`,
    "nofn/extract.ts": `export const extract = 1;`,
  });
  try {
    const { fetcher } = cannedFetch({ "https://news.example.com/a/1": "<article><p>Body text.</p></article>" });
    const ctx = await extractContext({
      source: DEF, dir, state: { seen: 7 }, validators: {}, limit: 1, known: () => false,
      net: new Net({ fetcher, sleep: () => Promise.resolve() }), log: () => {},
    });
    const out = await runExtract(`${dir}/ok`, ctx);
    assertEquals(out, {
      list: [{ url: "https://news.example.com/a/1", title: "From the package" }],
      items: [{ url: "https://news.example.com/a/1", title: "From the package", blocks: [{ type: "p", text: "Body text." }] }],
      state: { seen: 7 },
    });
    const kinds: [string, string, string][] = [
      ["auth", "auth", "sign in again"],
      ["duck", "transient", "try later"],
      ["bug", "program", "undefined"],
      ["shape", "program", "items[0]"],
      ["nofn", "program", "default"],
      ["missing", "program", "does not load"],
    ];
    for (const [name, kind, text] of kinds) {
      const err = await assertFails(() => runExtract(`${dir}/${name}`, ctx), text);
      assert(err instanceof SiteError && err.kind === kind, `${name}: ${(err as SiteError).kind}`);
    }
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
  assertEquals(extractedProblems({ list: [], items: [] }), []);
  assertEquals(extractedProblems({ list: [], items: [], state: "x".repeat(70_000) }).length, 1);
  assertEquals(failureOf(new SiteError("config", "x")).kind, "config");
});
