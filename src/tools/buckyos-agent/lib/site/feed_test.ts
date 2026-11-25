import { articleMarkdown, buildFeedObject, feedObjectId, feedObjectProblems } from "./feed.ts";
import { ARTICLE_SCHEMA, SiteError } from "./types.ts";
import type { Article, Block, Feed, FeedObject } from "./types.ts";

type ArticleBody = Omit<Article, "object" | "object_id">;

function assertEquals(actual: unknown, expected: unknown, what = ""): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`${what} expected ${expectedJson}, got ${actualJson}`);
  }
}

function assertInvalid(fn: () => unknown, includes: string): void {
  try {
    fn();
  } catch (err) {
    if (err instanceof SiteError && err.kind === "invalid" && err.message.includes(includes)) {
      return;
    }
    throw new Error(`expected an invalid SiteError with ${JSON.stringify(includes)}, got ${err}`);
  }
  throw new Error(`expected an invalid SiteError with ${JSON.stringify(includes)}`);
}

async function fixture<T>(name: string): Promise<T> {
  const url = new URL(
    `../../../../../doc/opendan/protocol/fixtures/site/${name}`,
    import.meta.url,
  );
  return JSON.parse(await Deno.readTextFile(url)) as T;
}

// The vectors HomeStation (Rust) is checked against as well.
const VECTORS = await fixture<{
  valid: { name: string; object: FeedObject; object_id: string }[];
  invalid: { name: string; object: unknown; problem: string }[];
}>("feed_object_vectors.json");
const vector = (name: string) => VECTORS.valid.find((entry) => entry.name === name)!;

const PAGE = "https://news.yahoo.co.jp/articles/0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
const OWNER = "did:bns:alice";
const file = (n: number) => `cyfile:${n.toString(16).padStart(2, "0").repeat(32)}`;
const image = (n: number, alt = ""): Block => ({
  type: "img",
  src: `_data/img/${n.toString(16).padStart(2, "0").repeat(32)}.jpg`,
  object: file(n),
  alt: { o: alt, t: alt },
  w: 1280,
  h: 720,
});
const MISSING: Block = {
  type: "img",
  src: null,
  object: null,
  alt: { o: "記者会見", t: "记者会现场" },
  w: 0,
  h: 0,
  missing: true,
};

function article(change: Partial<ArticleBody> = {}): ArticleBody {
  return {
    schema: ARTICLE_SCHEMA,
    id: "7eec8758d56a244a",
    source: "news-yahoo-co-jp",
    url: PAGE,
    lang: "ja",
    published_at: 1764000000000,
    fetched_at: 1764001700999,
    rev: 1,
    title: { o: "半導体の輸出新ルール", t: "半导体出口新规下月生效，日本设备厂商受影响" },
    summary: { o: "", t: "" },
    byline: "",
    tags: [],
    blocks: [{ type: "p", text: { o: "原文", t: "译文" } }],
    comments: [],
    links: [],
    images: {},
    thumb: null,
    supplement: null,
    ...change,
  };
}

Deno.test("validity and ObjId follow the shared vectors", () => {
  for (const { name, object, object_id } of VECTORS.valid) {
    assertEquals(feedObjectProblems(object), [], name);
    assertEquals(feedObjectId(object), object_id, name);
  }
  for (const { name, object, problem } of VECTORS.invalid) {
    const problems = feedObjectProblems(object);
    if (problems.length !== 1 || !problems[0].includes(problem)) {
      throw new Error(`${name}: expected ${JSON.stringify(problem)}, got ${JSON.stringify(problems)}`);
    }
  }
});

Deno.test("the sample article carries the shell built from it", async () => {
  const { object, object_id, ...body } = await fixture<Article>("article.json");
  assertEquals(
    buildFeedObject(body, { ownerDid: object.publisher, wraps: object.wraps!, baseOn: object.base_on }),
    { object, object_id },
  );
  assertEquals((await fixture<Feed>("feed.json")).items[0].object_id, object_id);
});

Deno.test("empty fields are left out, and iat is the fetch time in whole seconds", () => {
  const minimal = vector("minimal");
  assertEquals(
    buildFeedObject(article({ fetched_at: 1764001700000 }), { ownerDid: OWNER, wraps: file(0x33) }),
    { object: minimal.object, object_id: minimal.object_id },
  );
  const built = buildFeedObject(article({ blocks: [MISSING] }), { ownerDid: OWNER, wraps: file(0x33) });
  assertEquals([built.object.iat, built.object.source?.captured_at_ms], [1764001700, 1764001700999]);
  assertEquals(Object.keys(built.object), ["kind", "publisher", "iat", "content", "wraps", "link", "source"]);
  assertEquals(Object.keys(built.object.content), ["type", "title"]);
});

Deno.test("a correction is a new object based on the one before", () => {
  const first = buildFeedObject(article(), { ownerDid: OWNER, wraps: file(0x33) });
  const second = buildFeedObject(article({ rev: 2 }), {
    ownerDid: OWNER,
    wraps: file(0x34),
    baseOn: first.object_id,
  });
  assertEquals(second.object.base_on, first.object_id);
  assertEquals(second.object_id === first.object_id, false);
  assertEquals(buildFeedObject(article(), { ownerDid: OWNER, wraps: file(0x33) }), first);
});

Deno.test("the cover is the first image and media holds at most 9 different ones", () => {
  const blocks = [MISSING, image(1, "第一张"), image(1, "重复"), image(2)];
  for (let n = 3; n <= 12; n++) blocks.push(image(n, `图 ${n}`));
  const { content } = buildFeedObject(article({ blocks }), { ownerDid: OWNER, wraps: file(0x33) }).object;
  assertEquals(content.cover, file(1));
  assertEquals(content.media?.map((part) => part.object), [1, 2, 3, 4, 5, 6, 7, 8, 9].map(file));
  assertEquals(content.media?.slice(0, 3), [
    { object: file(1), alt: "第一张" },
    { object: file(2) },
    { object: file(3), alt: "图 3" },
  ]);
});

Deno.test("the summary is cut so that title and summary hold 8000 characters", () => {
  const title = { o: "", t: "标题".repeat(50) };
  const fits = { o: "", t: "𠮷".repeat(7900) };
  assertEquals(
    buildFeedObject(article({ title, summary: fits }), { ownerDid: OWNER, wraps: file(0x33) }).object
      .content.summary,
    fits.t,
  );
  const { summary } = buildFeedObject(
    article({ title, summary: { o: "", t: "𠮷".repeat(7901) } }),
    { ownerDid: OWNER, wraps: file(0x33) },
  ).object.content;
  assertEquals(summary, `${"𠮷".repeat(7899)}…`);
  assertInvalid(
    () =>
      buildFeedObject(article({ title: { o: "", t: "题".repeat(8001) }, summary: fits }), {
        ownerDid: OWNER,
        wraps: file(0x33),
      }),
    "inline content exceeds 8000 characters",
  );
});

Deno.test("an article whose shell does not validate is refused", () => {
  const opts = { ownerDid: OWNER, wraps: file(0x33) };
  assertInvalid(
    () => buildFeedObject(article(), { ...opts, wraps: "https://reflow.example.org/_data/md/a.md" }),
    "wraps must be an ObjId, not a URL",
  );
  assertInvalid(() => buildFeedObject(article(), { ...opts, wraps: "" }), "wraps: invalid ObjId");
  assertInvalid(() => buildFeedObject(article(), { ...opts, ownerDid: "alice" }), "publisher must be a DID");
  assertInvalid(
    () => buildFeedObject(article({ url: "about:blank" }), opts),
    "source.original_url must be an http(s) URL",
  );
  assertInvalid(
    () => buildFeedObject(article({ fetched_at: 1764001700000.5 }), opts),
    "source.captured_at_ms must be a non-negative integer",
  );
  assertInvalid(
    () => buildFeedObject(article({ tags: Array.from({ length: 17 }, (_, i) => `t${i}`) }), opts),
    "at most 16 tags",
  );
  assertInvalid(
    () => buildFeedObject(article({ byline: "x".repeat(70000) }), opts),
    "object too large",
  );
});

Deno.test("only the fields of a shell, with JSON values HomeStation reads, are valid", () => {
  const base = vector("minimal").object;
  const problemsOf = (change: Record<string, unknown>) => feedObjectProblems({ ...base, ...change });
  assertEquals(feedObjectProblems(null), ["a FeedObject is a JSON object"]);
  assertEquals(feedObjectProblems([base]), ["a FeedObject is a JSON object"]);
  assertEquals(problemsOf({ entry: "did:bns:x" }), ["entry is not a field of the shell"]);
  assertEquals(problemsOf({ nonce: "1" }), ["nonce is not a field of the shell"]);
  assertEquals(problemsOf({ kind: "comment" }), ['kind must be "post"']);
  assertEquals(problemsOf({ iat: -1 }), ["iat must be a non-negative integer"]);
  assertEquals(problemsOf({ iat: 2 ** 53 }), ["iat must be a non-negative integer"]);
  assertEquals(problemsOf({ iat: "1764001700" }), ["iat must be a non-negative integer"]);
  assertEquals(problemsOf({ tags: null }), ["tags must be a list of text"]);
  assertEquals(problemsOf({ tags: ["科技", 1] }), ["tags must be a list of text"]);
  assertEquals(problemsOf({ link: [PAGE] }), ["link must be an http(s) URL"]);
  assertEquals(problemsOf({ wraps: null }), ["wraps: invalid ObjId null"]);
  assertEquals(problemsOf({ base_on: "cyfeed:12" }), ['base_on: invalid ObjId "cyfeed:12"']);
  assertEquals(problemsOf({ content: { type: "text", text: "x" } }), [
    'content must be { "type": "article", … }',
  ]);
  assertEquals(problemsOf({ content: { type: "article", title: 1, text: "x", media: null } }), [
    "content.text is not a field of the shell",
    "content.title must be text",
    "content.media must be a list",
  ]);
  assertEquals(problemsOf({ content: { type: "article", media: [{ alt: "x", w: 1.5 }, "a"] } }), [
    "content.media.w is not a field of the shell",
    "content.media.object: invalid ObjId null",
    "content.media must list { object, alt }",
  ]);
  assertEquals(problemsOf({ content: { type: "article", title: "\ud83d" } }), [
    "content.title must be text",
  ]);
  assertEquals(problemsOf({ source: { kind: "rss", original_url: PAGE, captured_at_ms: 1 } }), [
    'source must be { "kind": "web", … }',
  ]);
  assertEquals(problemsOf({ source: { kind: "web", original_url: PAGE, captured_at_ms: 1.5 } }), [
    "source.captured_at_ms must be a non-negative integer",
  ]);
  assertEquals(problemsOf({ tags: [], content: { type: "article", media: [] } }), []);
});

Deno.test("the Markdown of the sample article", async () => {
  const body = await fixture<Article>("article.json");
  assertEquals(
    articleMarkdown(body),
    `# 半导体出口新规下月生效，日本设备厂商受影响

原文：${PAGE}

政府25日公布了先进半导体制造设备出口许可的细则。

![工厂里的光刻设备](cyfile:${"11".repeat(32)})

## 涉及的设备

涉及光刻、刻蚀、成膜三个领域，许可审查最长需要 90 天。

- 光刻设备
- 刻蚀设备

> 一家大型厂商的负责人说：“正在仔细评估对订单的影响。”

| 年度 | 出口额 |
| --- | --- |
| 2025 | 4.1 万亿日元 |
| 2026 | 4.6 万亿日元 |

![出口额走势图](cyfile:${"22".repeat(32)})

\`\`\`
HS 8486.20
\`\`\`

### 今后的日程

细则将于下月 1 日生效。
`,
  );
});

Deno.test("Markdown uses the translation only and keeps page text from acting as markup", () => {
  const bi = (t: string) => ({ o: "原文", t });
  const markdown = articleMarkdown(article({
    title: bi("<b>标题</b>\n第二行"),
    blocks: [
      { type: "p", text: bi("# 不是标题\n1. 不是列表\n- 也不是 *强调* 和 [链接](x)") },
      { type: "p", text: bi("   ") },
      { type: "quote", text: bi("第一行\n第二行") },
      { type: "li", text: bi("一\n续行") },
      { type: "li", text: bi("二") },
      { type: "code", text: bi("a ``` b\n<i>*</i>") },
      { type: "table", rows: [[bi("a|b"), bi("c\nd")], [bi("e")], []] },
      { type: "table", rows: [] },
      MISSING,
      image(1, "图[1]\n说明"),
    ],
  }));
  assertEquals(
    markdown,
    `# \\<b\\>标题\\</b\\> 第二行

原文：${PAGE}

\\# 不是标题
1\\. 不是列表
\\- 也不是 \\*强调\\* 和 \\[链接\\](x)

> 第一行
> 第二行

- 一
  续行
- 二

\`\`\`\`
a \`\`\` b
<i>*</i>
\`\`\`\`

| a\\|b | c d |
| --- | --- |
| e |  |
|  |  |

![图\\[1\\] 说明](cyfile:${"01".repeat(32)})
`,
  );
  assertEquals(markdown.includes("原文\n"), false);
});
