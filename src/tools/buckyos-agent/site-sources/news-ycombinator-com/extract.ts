// Hacker News, the front page. An article is a story: the text of the page it
// links to, and the top comments of its discussion. What that means and what
// is skipped: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

const FEED = "https://news.ycombinator.com/rss";
const MIN_TEXT = 400;
const MAX_LINKS = 20;
const MAX_SKIPPED = 300;

// deno-lint-ignore no-explicit-any
type Json = any;

const time = (age: Json): number | undefined => {
  const ms = Date.parse(`${(age?.getAttribute("title") ?? "").split(" ")[0]}Z`);
  return Number.isFinite(ms) ? ms : undefined;
};

// deno-lint-ignore no-explicit-any
function discussion(ctx: any, html: string, url: string, limit: number) {
  const page = ctx.html(html);
  const story = page.querySelector(".fatitem");
  const title = story?.querySelector(".titleline > a");
  if (!story || !title) throw new SiteError("program", `${url}: the item page has no story (.fatitem .titleline)`);
  const own = story.querySelector(".toptext");
  const comments = [];
  for (const row of page.querySelectorAll("tr.athing.comtr")) {
    if (comments.length >= limit) break;
    const body = row.querySelector(".commtext");
    if (row.querySelector("td.ind")?.getAttribute("indent") !== "0" || !body) continue;
    body.querySelector(".reply")?.remove();
    const text = ctx.blocks(body, url).blocks
      .flatMap((block: Json) => typeof block.text === "string" ? [block.text] : []).join("\n");
    if (text) comments.push({ author: row.querySelector('a[href^="user?id="]')?.textContent ?? "", at: time(row.querySelector(".age")), text });
  }
  return {
    title: title.textContent.trim(),
    link: new URL(title.getAttribute("href"), url).href,
    by: story.querySelector('a[href^="user?id="]')?.textContent ?? "",
    at: time(story.querySelector(".age")),
    own: own ? ctx.blocks(own, url) : { blocks: [], links: [] },
    comments,
  };
}

const chars = (blocks: Json[]) => blocks.reduce((sum, block) => sum + (typeof block.text === "string" ? block.text.length : 0), 0);

// The text of the page a story links to; null when it is not a page of text.
// deno-lint-ignore no-explicit-any
async function linked(ctx: any, url: string) {
  let response;
  try {
    response = await ctx.fetch(url);
  } catch (err) {
    ctx.log(`${url}: ${(err as Error).message}`);
    return null;
  }
  if (response.status !== 200 || !/html/.test(response.headers.get("content-type") ?? "")) return null;
  const page = ctx.html(response.text);
  // The longest <article>; a page without one that is long enough is read from <main>, then <body>.
  const longest = (selector: string) =>
    [...page.querySelectorAll(selector)].map((root: Json) => ctx.blocks(root, response.url))
      .sort((a: Json, b: Json) => chars(b.blocks) - chars(a.blocks))[0];
  const best = ["article", "main, [role=main]", "body"].map(longest)
    .find((found: Json) => found && chars(found.blocks) >= MIN_TEXT);
  if (!best) return null;
  return { ...best, lang: page.documentElement?.getAttribute("lang")?.trim() || "en" };
}

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const feed = await ctx.fetch(FEED);
  if (feed.status !== 200) throw new SiteError("program", `${FEED}: HTTP ${feed.status}`);
  const entries = ctx.feed(feed.text).filter((entry: Json) => entry.comments_url);
  const list = entries.map((entry: Json) => ({ url: entry.comments_url, title: entry.title, published_at: entry.published_at }));

  // Stories whose link is not a page of text (a video, a PDF, a repository): not asked for again.
  const skipped: string[] = Array.isArray(ctx.state?.skipped) ? ctx.state.skipped : [];
  const items = [];
  for (const entry of entries) {
    if (items.length >= ctx.limit) break;
    const url = entry.comments_url;
    if (ctx.known(url) || skipped.includes(url)) continue;
    const item = await ctx.fetch(url);
    if (item.status !== 200) throw new SiteError("program", `${url}: HTTP ${item.status}`);
    const story = discussion(ctx, item.text, url, ctx.source.comments ?? 10);
    const own = story.link.startsWith("https://news.ycombinator.com/item?");
    const body = own ? { ...story.own, lang: "en" } : await linked(ctx, story.link);
    if (!body || chars(body.blocks) < MIN_TEXT) {
      ctx.log(`${url}: ${story.link} is not a page of text; skipped`);
      skipped.push(url);
      continue;
    }
    items.push({
      url,
      title: story.title,
      lang: body.lang,
      published_at: story.at ?? entry.published_at,
      byline: own ? story.by : new URL(story.link).hostname.replace(/^www\./, ""),
      blocks: body.blocks,
      comments: story.comments,
      links: [
        ...(own ? [] : [{ url: story.link, text: story.title }]),
        ...body.links.slice(0, MAX_LINKS),
      ],
    });
  }
  return { list, items, state: { skipped: skipped.slice(-MAX_SKIPPED) } };
}
