// The news section of the DV test site (test/agent_browser_dv/site.mjs, /live): pages that are empty until
// their script ran, so they are read in the Agent's browser. How and what breaks: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

// deno-lint-ignore no-explicit-any
type Any = any;

// The page as its script drew it.
async function rendered(ctx: Any, url: string): Promise<Any> {
  const page = await ctx.browser(url, "render.mjs");
  return { url: page.url, doc: ctx.html(page.html) };
}

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const home = ctx.source.home_url;
  const front = await rendered(ctx, home);
  const list = [...front.doc.querySelectorAll(".stories .story")].map((story: Any) => {
    const link = story.querySelector("a.story-link");
    const at = Date.parse(story.querySelector("time")?.getAttribute("datetime") ?? "");
    return {
      url: new URL(link?.getAttribute("href") ?? "", front.url).href,
      title: link?.textContent.trim() ?? "",
      ...(Number.isFinite(at) ? { published_at: at } : {}),
    };
  });
  if (!list.length) {
    throw new SiteError("program", `${home}: the drawn page has no ".stories .story": the site's markup changed`);
  }

  const items = [];
  for (const entry of list) {
    if (items.length >= ctx.limit) break;
    if (ctx.known(entry.url)) continue;
    const page = await rendered(ctx, entry.url);
    const article = page.doc.querySelector("article.story-body");
    const body = article?.querySelector(".story-text");
    if (!article || !body) {
      throw new SiteError("program", `${entry.url}: the drawn page has no "article.story-body .story-text"`);
    }
    const { blocks, links } = ctx.blocks(body, page.url);
    items.push({
      url: entry.url,
      title: article.querySelector(".story-title")?.textContent.trim() || entry.title,
      published_at: entry.published_at,
      byline: article.querySelector(".byline .author")?.textContent.trim() ?? "",
      blocks,
      links,
    });
  }
  return { list, items, state: null };
}
