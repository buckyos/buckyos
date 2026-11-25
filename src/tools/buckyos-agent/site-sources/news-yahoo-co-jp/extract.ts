// Yahoo! JAPAN News, the main topics. How it is read and what breaks: NOTES.md.

declare class SiteError extends Error {
  constructor(kind: "auth" | "transient" | "program", message: string);
}

const FEED = "https://news.yahoo.co.jp/rss/topics/top-picks.xml";
const ARTICLE = /https:\/\/news\.yahoo\.co\.jp\/articles\/[0-9a-f]+/;
const MAX_PAGES = 5;

// The body carries zero-width characters between words; they are not text.
const clean = (text: string) => text.replace(/[​-‍⁠﻿]/g, "");

// deno-lint-ignore no-explicit-any
type Json = any;

function preloadedState(html: string, url: string): Json {
  const found = /window\.__PRELOADED_STATE__\s*=\s*(\{.*?\})\s*<\/script>/s.exec(html);
  if (!found) throw new SiteError("program", `${url}: no __PRELOADED_STATE__ in the page`);
  try {
    return JSON.parse(found[1]);
  } catch (err) {
    throw new SiteError("program", `${url}: __PRELOADED_STATE__ is not JSON (${(err as Error).message})`);
  }
}

// deno-lint-ignore no-explicit-any
async function page(ctx: any, url: string): Promise<Json> {
  const response = await ctx.fetch(url);
  if (response.status !== 200) throw new SiteError("program", `${url}: HTTP ${response.status}`);
  return preloadedState(response.text, url);
}

// The article a topic points to: the feed names it in <comments>, the topic page links it.
// deno-lint-ignore no-explicit-any
async function articleUrl(ctx: any, entry: Json): Promise<string | null> {
  const fromFeed = ARTICLE.exec(entry.comments_url ?? "")?.[0];
  if (fromFeed) return fromFeed;
  const response = await ctx.fetch(entry.url);
  return response.status === 200 ? ARTICLE.exec(response.text)?.[0] ?? null : null;
}

// deno-lint-ignore no-explicit-any
export default async function extract(ctx: any) {
  const feed = await ctx.fetch(FEED, { conditional: true });
  if (feed.notModified) return { list: [], items: [], state: ctx.state };
  if (feed.status !== 200) throw new SiteError("program", `${FEED}: HTTP ${feed.status}`);

  const list = [];
  const items = [];
  for (const entry of ctx.feed(feed.text)) {
    const url = await articleUrl(ctx, entry);
    if (!url) {
      ctx.log(`${entry.url}: no article behind this topic`);
      continue;
    }
    list.push({ url, title: entry.title, published_at: entry.published_at });
    if (items.length >= ctx.limit || ctx.known(url)) continue;

    const first = (await page(ctx, url)).articleDetail;
    if (!first?.headline || !Array.isArray(first.paragraphs)) {
      throw new SiteError("program", `${url}: articleDetail has no headline and paragraphs`);
    }
    const paragraphs = [...first.paragraphs];
    for (let n = 2; n <= Math.min(first.maxPage ?? 1, MAX_PAGES); n += 1) {
      paragraphs.push(...((await page(ctx, `${url}?page=${n}`)).articleDetail?.paragraphs ?? []));
    }

    const blocks = [];
    const links = [];
    for (const paragraph of paragraphs) {
      for (const image of paragraph.objectItems ?? []) {
        const src = image.resizedViewUrl?.jpeg ?? image.thumb?.uri;
        if (src) blocks.push({ type: "img", url: src, alt: clean([image.caption, image.credit].filter(Boolean).join(" ")) });
      }
      for (const detail of paragraph.textDetails ?? []) {
        for (const item of detail.paragraphItems ?? []) {
          if (typeof item.text !== "string") continue;
          if (item.link?.url) links.push({ url: item.link.url, text: clean(item.text) });
          for (const text of clean(item.text).split(/\n{2,}/)) {
            if (text.trim()) blocks.push({ type: item.type === "heading" ? "h2" : "p", text: text.trim() });
          }
        }
      }
    }
    for (const related of first.relatedLink ?? []) {
      if (related.uri && related.title) links.push({ url: related.uri, text: clean(related.title) });
    }

    let comments: { author: string; text: string }[] = [];
    if (ctx.source.comments !== 0 && first.isCommentVisible) {
      const state = await page(ctx, `${url}/comments`);
      comments = (state.commentFull?.userCommentList ?? [])
        .filter((comment: Json) => typeof comment.text === "string" && comment.isVisible !== false)
        .map((comment: Json) => ({ author: comment.name ?? "", text: clean(comment.text) }));
    }

    const published = Date.parse(first.createDateTime ?? "");
    items.push({
      url,
      title: clean(first.headline),
      lang: "ja",
      published_at: Number.isFinite(published) ? published : entry.published_at,
      byline: first.media?.mediaName ?? "",
      blocks,
      comments,
      links,
    });
  }
  return { list, items, state: null };
}
