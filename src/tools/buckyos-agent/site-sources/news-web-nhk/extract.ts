// NHK News. Without the viewer's confirmation the site gives the lead of an
// article only, so this source lists headlines with their lead: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

const FEED = "https://news.web.nhk/n-data/conf/na/rss/cat0.xml";

// deno-lint-ignore no-explicit-any
type Json = any;

// The NewsArticle of the page's JSON-LD.
// deno-lint-ignore no-explicit-any
function newsArticle(ctx: any, html: string): Json | null {
  for (const script of ctx.html(html).querySelectorAll('script[type="application/ld+json"]')) {
    try {
      const data = JSON.parse(script.textContent ?? "");
      if (data?.["@type"] === "NewsArticle") return data;
    } catch {
      // another script's JSON
    }
  }
  return null;
}

const text = (value: Json): string => typeof value === "string" ? value : value?.["@value"] ?? "";

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const feed = await ctx.fetch(FEED, { conditional: true });
  if (feed.notModified) return { list: [], items: [], state: ctx.state };
  if (feed.status !== 200) throw new SiteError("program", `${FEED}: HTTP ${feed.status}`);

  const entries = ctx.feed(feed.text);
  const list = entries.map((entry: Json) => ({
    url: entry.url,
    title: entry.title,
    published_at: entry.published_at,
    summary: entry.summary,
  }));
  const items = [];
  for (const entry of entries) {
    if (items.length >= ctx.limit) break;
    if (ctx.known(entry.url)) continue;
    const response = await ctx.fetch(entry.url);
    if (response.status !== 200) throw new SiteError("program", `${entry.url}: HTTP ${response.status}`);
    const article = newsArticle(ctx, response.text);
    if (!article) throw new SiteError("program", `${entry.url}: no NewsArticle in the page's JSON-LD`);

    // The lead as the feed has it: whole paragraphs, then the beginning of the next one, cut with "…".
    const lead = (entry.summary ?? text(article.description)).split(/\n+/)
      .map((part: string) => part.trim()).filter(Boolean);
    if (lead.length > 1 && lead.at(-1).endsWith("…")) lead.pop();
    const image = article.image?.[0]?.url;
    const published = Date.parse(article.datePublished ?? "");
    items.push({
      url: response.url,
      title: text(article.headline) || entry.title,
      lang: "ja",
      published_at: Number.isFinite(published) ? published : entry.published_at,
      byline: "NHK",
      summary: lead[0] ?? "",
      blocks: [
        ...(image ? [{ type: "img", url: image }] : []),
        ...lead.map((part: string) => ({ type: "p", text: part })),
      ],
    });
  }
  return { list, items, state: null };
}
