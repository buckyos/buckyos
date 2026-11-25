// The FeedObject shell of an article and the Markdown it wraps: Personal Site
// Protocol §3.4. A shell is valid when HomeStation's `validate_feed_object`
// (src/frame/homestation/src/protocol.rs) accepts it; both sides are checked
// against doc/opendan/protocol/fixtures/site/feed_object_vectors.json.

import { ndn } from "buckyos";
import { SiteError } from "./types.ts";
import type { Article, Block, FeedObject } from "./types.ts";

const MAX_OBJECT_BYTES = 65536;
const MAX_INLINE_TEXT = 8000;
const MAX_MEDIA_PARTS = 9;
const MAX_TAGS = 16;

type ArticleBody = Omit<Article, "object" | "object_id">;

// HomeStation counts Unicode scalar values, not UTF-16 units.
const chars = (text: string) => Array.from(text);
const oneLine = (text: string) => text.replace(/\s*\n\s*/g, " ").trim();

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

// ── Markdown ───────────────────────────────────────────────────────────────

// The text comes from a web page: nothing in it may act as Markdown or HTML.
function escapeText(text: string): string {
  return text
    .replace(/^[ \t]+/gm, "")
    .replace(/[\\`*_[\]<>&|~]/g, "\\$&")
    .replace(/^[#+=-]/gm, "\\$&")
    .replace(/^(\d+)([.)])(?=\s|$)/gm, "$1\\$2");
}

function blockMarkdown(block: Block): string | null {
  switch (block.type) {
    case "img":
      return block.object ? `![${escapeText(oneLine(block.alt.t))}](${block.object})` : null;
    case "table": {
      const width = Math.max(0, ...block.rows.map((row) => row.length));
      if (!width) return null;
      const line = (cells: string[]) =>
        `| ${Array.from({ length: width }, (_, i) => cells[i] ?? "").join(" | ")} |`;
      const lines = block.rows.map((row) => line(row.map((cell) => escapeText(oneLine(cell.t)))));
      lines.splice(1, 0, line(Array(width).fill("---")));
      return lines.join("\n");
    }
  }
  const text = block.text.t;
  if (!text.trim()) return null;
  switch (block.type) {
    case "h2":
      return `## ${escapeText(oneLine(text))}`;
    case "h3":
      return `### ${escapeText(oneLine(text))}`;
    case "quote":
      return escapeText(text).replace(/^/gm, "> ");
    case "li":
      return `- ${escapeText(text).replace(/\n/g, "\n  ")}`;
    case "code": {
      const fence = "`".repeat(
        Math.max(3, ...Array.from(text.matchAll(/`+/g), (run) => run[0].length + 1)),
      );
      return `${fence}\n${text}\n${fence}`;
    }
    default:
      return escapeText(text);
  }
}

/** The translation of an article as the Markdown file its FeedObject wraps. */
export function articleMarkdown(article: ArticleBody): string {
  let out = `# ${escapeText(oneLine(article.title.t))}\n\n原文：${article.url}\n`;
  let previous: Block["type"] | undefined;
  for (const block of article.blocks) {
    const text = blockMarkdown(block);
    if (text === null) continue;
    out += `${previous === "li" && block.type === "li" ? "" : "\n"}${text}\n`;
    previous = block.type;
  }
  return out;
}

// ── FeedObject ─────────────────────────────────────────────────────────────

const isText = (value: unknown): value is string =>
  typeof value === "string" && value.isWellFormed();
const isCount = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

function isHttpUrl(value: unknown): boolean {
  if (typeof value !== "string") return false;
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname !== "";
  } catch {
    return false;
  }
}

// HomeStation also takes the base32 form; a shell only holds the `type:hex` one the bucket returns.
function refProblem(field: string, value: unknown): string | null {
  if (typeof value === "string" && value.includes("://")) {
    return `${field} must be an ObjId, not a URL`;
  }
  if (typeof value === "string" && /^[^:]+:[0-9a-fA-F]{64}$/.test(value)) return null;
  return `${field}: invalid ObjId ${JSON.stringify(value)}`;
}

/**
 * What is wrong with a FeedObject shell; empty when it is valid. The rules of
 * HomeStation's `validate_feed_object`, for the fields a shell has.
 */
export function feedObjectProblems(object: unknown): string[] {
  if (!isRecord(object)) return ["a FeedObject is a JSON object"];
  const problems: string[] = [];
  const only = (value: Record<string, unknown>, at: string, fields: string[]) => {
    for (const key of Object.keys(value)) {
      if (!fields.includes(key)) problems.push(`${at}${key} is not a field of the shell`);
    }
  };
  const ref = (field: string, value: unknown) => {
    const problem = value === undefined ? null : refProblem(field, value);
    if (problem) problems.push(problem);
  };
  const text = (field: string, value: unknown) => {
    if (value !== undefined && !isText(value)) problems.push(`${field} must be text`);
  };

  only(object, "", [
    "kind",
    "publisher",
    "iat",
    "content",
    "wraps",
    "tags",
    "link",
    "source",
    "base_on",
  ]);
  if (object.kind !== "post") problems.push('kind must be "post"');
  if (!isText(object.publisher) || !object.publisher.startsWith("did:")) {
    problems.push("publisher must be a DID");
  }
  if (!isCount(object.iat)) problems.push("iat must be a non-negative integer");
  ref("wraps", object.wraps);
  ref("base_on", object.base_on);

  const content = object.content;
  if (!isRecord(content) || content.type !== "article") {
    problems.push('content must be { "type": "article", … }');
  } else {
    only(content, "content.", ["type", "title", "summary", "cover", "media"]);
    text("content.title", content.title);
    text("content.summary", content.summary);
    ref("content.cover", content.cover);
    const media = content.media === undefined ? [] : content.media;
    if (!Array.isArray(media)) {
      problems.push("content.media must be a list");
    } else {
      if (media.length > MAX_MEDIA_PARTS) problems.push(`at most ${MAX_MEDIA_PARTS} media parts`);
      for (const part of media) {
        if (!isRecord(part)) {
          problems.push("content.media must list { object, alt }");
          continue;
        }
        only(part, "content.media.", ["object", "alt"]);
        ref("content.media.object", part.object ?? null);
        text("content.media.alt", part.alt);
      }
    }
    const inline = [content.title, content.summary]
      .reduce((sum: number, value) => sum + (isText(value) ? chars(value).length : 0), 0);
    if (inline > MAX_INLINE_TEXT) {
      problems.push(`inline content exceeds ${MAX_INLINE_TEXT} characters`);
    }
  }

  if (object.link !== undefined && !isHttpUrl(object.link)) {
    problems.push("link must be an http(s) URL");
  }
  const source = object.source;
  if (source !== undefined) {
    if (!isRecord(source) || source.kind !== "web") {
      problems.push('source must be { "kind": "web", … }');
    } else {
      only(source, "source.", ["kind", "original_url", "original_author", "captured_at_ms"]);
      if (!isHttpUrl(source.original_url)) {
        problems.push("source.original_url must be an http(s) URL");
      }
      text("source.original_author", source.original_author);
      if (!isCount(source.captured_at_ms)) {
        problems.push("source.captured_at_ms must be a non-negative integer");
      }
    }
  }
  const tags = object.tags === undefined ? [] : object.tags;
  if (!Array.isArray(tags) || !tags.every(isText)) {
    problems.push("tags must be a list of text");
  } else if (tags.length > MAX_TAGS) {
    problems.push(`at most ${MAX_TAGS} tags`);
  }

  if (
    !problems.length && new TextEncoder().encode(JSON.stringify(object)).length > MAX_OBJECT_BYTES
  ) {
    problems.push("object too large");
  }
  return problems;
}

/** The ObjId of a FeedObject, `cyfeed:<hex>`. */
export function feedObjectId(object: FeedObject): string {
  return ndn.buildNamedObjectByJson("cyfeed", object)[0].toString();
}

function withoutEmpty<T extends object>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, field]) =>
      field != null && field !== "" && !(Array.isArray(field) && !field.length)
    ),
  ) as T;
}

/** The shell of an article whose Markdown is stored as `wraps`; `baseOn` is the one it corrects. */
export function buildFeedObject(
  article: ArticleBody,
  opts: { ownerDid: string; wraps: string; baseOn?: string },
): { object: FeedObject; object_id: string } {
  const media: NonNullable<FeedObject["content"]["media"]> = [];
  for (const block of article.blocks) {
    if (block.type !== "img" || !block.object) continue;
    if (media.some((part) => part.object === block.object)) continue;
    if (media.length === MAX_MEDIA_PARTS) break;
    media.push(withoutEmpty({ object: block.object, alt: block.alt.t }));
  }
  const title = article.title.t;
  let summary = article.summary.t;
  const room = MAX_INLINE_TEXT - chars(title).length;
  if (chars(summary).length > room) {
    summary = room > 0 ? `${chars(summary).slice(0, room - 1).join("")}…` : "";
  }
  const object: FeedObject = {
    kind: "post",
    publisher: opts.ownerDid,
    iat: Math.floor(article.fetched_at / 1000),
    content: withoutEmpty({ type: "article", title, summary, cover: media[0]?.object, media }),
    wraps: opts.wraps,
    ...withoutEmpty({ tags: article.tags, link: article.url }),
    source: withoutEmpty({
      kind: "web" as const,
      original_url: article.url,
      original_author: article.byline,
      captured_at_ms: article.fetched_at,
    }),
    ...withoutEmpty({ base_on: opts.baseOn }),
  };
  const problems = feedObjectProblems(object);
  if (problems.length) throw new SiteError("invalid", problems.join("; "));
  return { object, object_id: feedObjectId(object) };
}
