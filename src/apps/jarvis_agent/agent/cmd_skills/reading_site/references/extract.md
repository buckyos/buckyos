# Writing a source package

A package is the directory `sources/<source>/`: `source.json` (what the source is), `extract.ts` (how its articles are fetched), `NOTES.md` (for whoever repairs it), `fixtures/` (real pages to test against). Everything after fetching — checking, pictures, translation, publishing, clean-up — is the `site` command's and the same for all sources. `site --help` has every field and the whole `ctx`.

Start from the nearest sample in `/opt/buckyos/bin/opendan/buckyos-agent/site-sources/` (on a node `$BUCKYOS_ROOT/libexec/buckyos-agent/site-sources/`), and read its `NOTES.md` as the model for yours:

| Sample | Shows |
| --- | --- |
| `news-yahoo-co-jp` | a feed, then article pages that embed their data as JSON; articles over several pages; comments on a page of their own |
| `news-ycombinator-com` | a forum: a feed, the discussion page for the comments, the linked page's text through `ctx.blocks`; skipping what is not text and remembering it in `state` |
| `news-web-nhk` | a site that gives every reader the lead only: a list-only source with `check.min_chars` lowered |

#### source.json: what to decide

- `id`: the host with dots as dashes (`news.example.com` → `news-example-com`), plus a word for one section of a site. It names the job (`src-<id>`), the feed and the directory: never change it later.
- `match`: the addresses that belong to this source — exact lower-case hosts (the apex and `www.` are different hosts) and an optional `path` pattern with `*`. A matching address the owner opens shows the site's article.
- `lang` (BCP 47 of the originals), `kind` (`news` 30 minutes, `blog` 6 hours, `forum` 1 hour between syncs), `needs` (`http`; `browser`; `login`, 6 hours).
- `every_secs`, `max_new` (20), `keep_days` (30), `comments` (10): leave them out for the defaults. `comments: 0` when the site has none or the owner does not want them. Never sync more often than the kind's default unless the owner asked.
- `check`: only when the defaults reject what is right for this source, with the reason in `NOTES.md`.

#### extract.ts

```ts
// Example News, the front page. How it is read and what breaks: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

const FEED = "https://news.example.com/feed.xml";

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const feed = await ctx.fetch(FEED, { conditional: true });
  if (feed.notModified) return { list: [], items: [], state: ctx.state };
  if (feed.status !== 200) throw new SiteError("program", `${FEED}: HTTP ${feed.status}`);

  const entries = ctx.feed(feed.text);
  // deno-lint-ignore no-explicit-any
  const list = entries.map((entry: any) => ({ url: entry.url, title: entry.title, published_at: entry.published_at }));
  const items = [];
  for (const entry of entries) {
    if (items.length >= ctx.limit) break;
    if (ctx.known(entry.url)) continue;
    const page = await ctx.fetch(entry.url);
    if (page.status !== 200) throw new SiteError("program", `${entry.url}: HTTP ${page.status}`);
    const doc = ctx.html(page.text);
    const body = doc.querySelector("article .article-body");
    if (!body) throw new SiteError("program", `${entry.url}: no "article .article-body" in the page`);
    const { blocks, links } = ctx.blocks(body, page.url);
    items.push({
      url: entry.url,
      title: doc.querySelector("article h1")?.textContent.trim() || entry.title,
      published_at: entry.published_at,
      byline: doc.querySelector('meta[name="author"]')?.getAttribute("content") ?? "",
      summary: entry.summary,
      blocks,
      links: links.slice(0, 20),
    });
  }
  return { list, items, state: null };
}
```

- `list` is every entry the source lists now, the ones the site has included; `items` are the new ones only (`!ctx.known(url)`), at most `ctx.limit`. An article's `url` is its key: the address the owner would open, stable, without tracking parameters.
- Nothing new: `{ list: [], items: [], state: ctx.state }`, and only after a conditional request answered "not modified". An empty list otherwise fails the run, on purpose.
- Fetch only with `ctx.fetch` (one request a second per host, 30 seconds, one user agent; fixtures and recording go through it). Parse only with `ctx.html`, `ctx.xml`, `ctx.feed`, `ctx.blocks`. Import nothing but files of the package.
- `ctx.blocks(element, baseUrl)` turns an article's element into paragraphs, headings, quotes, list items, code, images and tables and collects its links; navigation, forms and hidden elements are left out. Give it the narrowest element that holds the whole article. When the page embeds its data, build the blocks from it yourself: `{ type: "p", text }`, `{ type: "img", url, alt }`.
- Text is text, never markup. Pictures by absolute address, the largest the page gives every reader. Comments are `{ author, at, text }`: the first `ctx.source.comments` in the site's own order.
- Say how it failed: `throw new SiteError("program", "<address>: <what was expected and is not there>")` names the selector or field, so a repair knows where to look. Network errors, 429 and 5xx are `transient` already. `auth` only when the owner signing in would fix it: 401 or 403 on a source without sign-in is `program` — the site began to refuse programs, which is not worked around.
- `state` (at most 64 KiB) comes back as `ctx.state`: what was skipped for good, a cursor. Conditional requests keep their ETag by themselves.
- An article over several pages or comments on another page mean more requests: write the number per article into `NOTES.md` and keep `max_new` low enough for a first sync.

#### A source that needs the browser

`ctx.browser(url | null, "<script>.mjs", args?)` runs a script of the package in a background tab of the Agent's browser: a tab of its own that disturbs neither the owner nor a session at work, one run at a time, 60 seconds. The script's default export gets `{ page, request, save, url, args }` and returns JSON. Keep the script dumb and every selector in `extract.ts`: then a changed page fails there as `program`, by name, and not as a timeout of the browser.

Pages their own script draws (`"needs": "browser"`):

```js
// render.mjs: the tab is at the page already; hand back what its script drew.
export default async ({ page }) => {
  await page.waitForLoadState('networkidle');
  return { url: page.url(), html: await page.content() };
};
```

```ts
const page = await ctx.browser(url, "render.mjs");
const doc = ctx.html(page.html);
```

Pages for signed-in readers (`"needs": "login"`): the browser's cookies, nothing rendered.

```js
// fetch.mjs: a page that sends the reader to sign in ends the run as "auth".
export default async ({ request, args }) => {
  const response = await request.get(args.url, { maxRedirects: 0 });
  const status = response.status();
  if (status === 401 || status === 403 || (status >= 300 && status < 400)) {
    throw Object.assign(new Error(`${args.url} asks to sign in (HTTP ${status})`), { code: 'auth' });
  }
  return { status, url: response.url(), html: await response.text() };
};
```

```ts
const got = await ctx.browser(null, "fetch.mjs", { url });
if (got.status !== 200) throw new SiteError("program", `${url}: HTTP ${got.status}`);
const doc = ctx.html(got.html);
```

- A site that answers a signed-out reader with 200 and a sign-in form: recognise the form in `extract.ts` and `throw new SiteError("auth", …)`.
- On `auth` the source shows "sign-in needed" with a link to `home_url`, the goal's check tells the owner once, and the job goes on trying and recovers by itself after the sign-in. So `home_url` must lead a signed-out reader to the sign-in page.
- A script uses the one tab it was given; several addresses are fetched with `request`.
- `site try` on such a source uses this runtime's browser (`agent-browser status` must not say unavailable), and `--fixtures` does not replay what the browser returned: say in `NOTES.md` that its test needs the live site.

#### NOTES.md

Whoever repairs this later has not seen the site. Give them, as the samples do:

- what the source is: which section, what one article is here, full text or list only or behind the owner's sign-in, and the day you read it;
- how it is read: every request in order, the fields and selectors taken, the requests one new article costs;
- what to check: the `site try` command and what it must show;
- what goes wrong: each failure you met or expect, what it means, and whether it is a repair, an upgrade or nothing to do;
- what the owner was told: list only, sign-in, comments left out;
- the fixtures: what they are and how they were cut.

#### Fixtures

`site try <dir> --record` writes every page it fetched to `fixtures/` and their addresses to `fixtures/index.json`. Keep the feed and three articles of different shape (short, long, with pictures or a table), and cut large pages down to the part the extractor reads: a package holds at most 200 files and 20 MiB and is copied into its job. After cutting, `site try <dir> --fixtures --no-translate` must still publish the same articles.
