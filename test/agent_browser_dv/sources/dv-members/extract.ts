// The members' column of the DV test site (test/agent_browser_dv/site.mjs, /members): plain pages behind a
// sign-in, read with the cookies of the Agent's browser. How and what breaks: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

// deno-lint-ignore no-explicit-any
type Any = any;

// A page as the signed-in user gets it. Signed out, fetch.mjs ends the run as "auth".
async function page(ctx: Any, url: string): Promise<Any> {
  const got = await ctx.browser(null, "fetch.mjs", { url });
  if (got.status !== 200) throw new SiteError("program", `${url}: HTTP ${got.status}`);
  return ctx.html(got.html);
}

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const home = ctx.source.home_url;
  const front = await page(ctx, home);
  const list = [...front.querySelectorAll(".posts .post")].map((post: Any) => {
    const link = post.querySelector("a.post-link");
    const at = Date.parse(post.querySelector("time")?.getAttribute("datetime") ?? "");
    return {
      url: new URL(link?.getAttribute("href") ?? "", home).href,
      title: link?.textContent.trim() ?? "",
      ...(Number.isFinite(at) ? { published_at: at } : {}),
    };
  });
  if (!list.length) {
    throw new SiteError("program", `${home}: the page has no ".posts .post": the site's markup changed`);
  }

  const items = [];
  for (const entry of list) {
    if (items.length >= ctx.limit) break;
    if (ctx.known(entry.url)) continue;
    const doc = await page(ctx, entry.url);
    const article = doc.querySelector("article.member-post");
    const body = article?.querySelector(".post-body");
    if (!article || !body) {
      throw new SiteError("program", `${entry.url}: the page has no "article.member-post .post-body"`);
    }
    const { blocks, links } = ctx.blocks(body, entry.url);
    items.push({
      url: entry.url,
      title: article.querySelector(".post-title")?.textContent.trim() || entry.title,
      published_at: entry.published_at,
      byline: article.querySelector(".post-meta .author")?.textContent.trim() ?? "",
      blocks,
      links,
    });
  }
  return { list, items, state: null };
}
