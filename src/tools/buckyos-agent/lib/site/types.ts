// The data of a personal site, as doc/opendan/protocol/Personal Site Protocol.md
// states it. Samples: doc/opendan/protocol/fixtures/site/.

export const SITE_SCHEMA = "buckyos.site/1";
export const FEED_SCHEMA = "buckyos.site.feed/1";
export const ARTICLE_SCHEMA = "buckyos.site.article/1";
export const EVENTS_SCHEMA = "buckyos.site.events/1";
export const READING_SCHEMA = "buckyos.site.reading/1";
export const SOURCE_SCHEMA = "buckyos.site.source/1";
export const TEMPLATE_SCHEMA = "buckyos.site.template/1";

export const SITE_KEY = "_data/site.json";
export const EVENTS_KEY = "_data/events.json";
export const READING_KEY = "_data/reading.json";
export const INDEX_KEY = "index.html";
export const feedKey = (source: string) => `_data/feed/${source}.json`;
export const articleKey = (article: string) => `_data/a/${article}.json`;
export const markdownKey = (article: string) => `_data/md/${article}.md`;
export const imageKey = (sha256: string, ext: string) => `_data/img/${sha256}.${ext}`;
export const templatePrefix = (version: string) => `_app/${version}/`;
export const sourcePrefix = (source: string, version?: string) =>
  version ? `_src/${source}/${version}/` : `_src/${source}/`;
export const READ_PREFIX = "_read/";
/** The source id of supplement articles (§3.5). */
export const SUPPLEMENT_SOURCE = "_supplement";

export type FailureKind = "auth" | "transient" | "program";

/** What an extractor, or a step of the pipeline, throws to say how it failed. */
export class SiteError extends Error {
  constructor(readonly kind: FailureKind | "config" | "budget" | "invalid", message: string) {
    super(message);
    this.name = "SiteError";
  }
}

// ── §3.1 site.json ─────────────────────────────────────────────────────────

export interface MatchRule {
  host: string;
  path?: string;
}

export type SourceKind = "news" | "blog" | "forum";
export type SourceNeeds = "http" | "browser" | "login";
export type SourceState = "preparing" | "active" | "paused" | "failed";

export interface SourceEntry {
  id: string;
  title: string;
  home_url: string;
  match: MatchRule[];
  lang: string;
  kind: SourceKind;
  needs: SourceNeeds;
  job: string;
  version: string | null;
  every_secs: number;
  max_new: number;
  keep_days: number;
  comments: number;
  state: SourceState;
  note: string | null;
  added_at: number;
  added_by: string;
}

export interface Upgrade {
  id: string;
  source?: string;
  title: string;
  summary: string;
  file: string;
  goal_id: string;
  opened_at: number;
  state: "open" | "accepted" | "dismissed" | "closed";
  decided_at?: number;
}

export interface TemplateParams {
  home_order?: "time" | "source" | "events";
  per_column?: number;
  hide_topics?: string[];
  collapsed?: string[];
  [name: string]: unknown;
}

export interface Site {
  schema: typeof SITE_SCHEMA;
  title: string;
  language: string;
  owner_did: string;
  topics: string[];
  template: { version: string; params: TemplateParams };
  columns: { id: string; title: string; sources: string[] }[];
  sources: SourceEntry[];
  upgrades: Upgrade[];
  reading: { enabled: boolean };
  limits: { translate_tokens_per_day: number };
  maintainer: { agent: string; goal_id: string } | null;
  updated_at: number;
  updated_by: string;
}

// ── §3.2 feed ──────────────────────────────────────────────────────────────

export type FeedState = "preparing" | "ok" | "failed" | "needs_login";
/** What a source shows as, composed from site.json, its upgrades and its feed (§3.1). */
export type DisplayState = FeedState | "paused" | "needs_upgrade";

export interface FeedStatus {
  state: FeedState;
  checked_at: number | null;
  published_at: number | null;
  error: { kind: FailureKind | "config"; message: string; at: number } | null;
  pending: {
    count: number;
    reason: "translate_budget" | "translate_config" | "translate_failed" | "time" | "images";
    message: string;
  } | null;
  login_url: string | null;
}

export interface FeedItem {
  id: string;
  object_id: string;
  url: string;
  title: string;
  summary: string;
  image: string | null;
  tags: string[];
  published_at: number | null;
  fetched_at: number;
  rev: number;
}

export interface Feed {
  schema: typeof FEED_SCHEMA;
  source: string;
  status: FeedStatus;
  items: FeedItem[];
  updated_at: number;
}

// ── §3.3 article ───────────────────────────────────────────────────────────

/** Original and translation; equal when the source is in the site's language. */
export interface Bi {
  o: string;
  t: string;
}

export type TextBlockType = "p" | "h2" | "h3" | "quote" | "li" | "code";

export type Block =
  | { type: TextBlockType; text: Bi }
  | {
    type: "img";
    src: string | null;
    object: string | null;
    alt: Bi;
    w: number;
    h: number;
    missing?: true;
  }
  | { type: "table"; rows: Bi[][] };

export interface Article {
  schema: typeof ARTICLE_SCHEMA;
  id: string;
  source: string;
  url: string;
  lang: string;
  published_at: number | null;
  fetched_at: number;
  rev: number;
  object_id: string;
  object: FeedObject;
  title: Bi;
  summary: Bi;
  byline: string;
  tags: string[];
  blocks: Block[];
  comments: { author: string; at: number | null; text: Bi }[];
  links: { url: string; text: Bi; article?: string }[];
  images: Record<string, { key: string; w: number; h: number }>;
  thumb: string | null;
  supplement: { event: string; adds: string } | null;
}

// ── §3.4 the FeedObject shell ──────────────────────────────────────────────

export interface FeedObject {
  kind: "post";
  publisher: string;
  iat: number;
  content: {
    type: "article";
    title?: string;
    summary?: string;
    cover?: string;
    media?: { object: string; alt?: string }[];
  };
  wraps?: string;
  tags?: string[];
  link?: string;
  source?: {
    kind: "web";
    original_url: string;
    original_author?: string;
    captured_at_ms: number;
  };
  base_on?: string;
}

// ── §3.5 events, §3.6 reading ──────────────────────────────────────────────

export interface SiteEvent {
  id: string;
  title: string;
  summary: string;
  /** `title`, `source` and `at` are copied from the article by `site events put`. */
  articles: { id: string; adds: string; title: string; source: string; at: number }[];
  supplements: { article: string; source_url: string; adds: string; title: string; at: number }[];
  updated_at: number;
}

export interface Events {
  schema: typeof EVENTS_SCHEMA;
  events: SiteEvent[];
  updated_at: number;
}

export interface ReadRecord {
  article: string;
  source: string;
  opened_at: number;
  dwell_ms: number;
  max_scroll: number;
}

export interface Reading {
  schema: typeof READING_SCHEMA;
  updated_at: number;
  since: number;
  articles: Record<
    string,
    { source: string; opens: number; dwell_ms: number; max_scroll: number; last_opened_at: number }
  >;
  sources: Record<string, { opens: number; dwell_ms: number; last_opened_at: number }>;
  tags: Record<string, { opens: number; dwell_ms: number }>;
}

// ── §4 the source package ──────────────────────────────────────────────────

export interface SourceDef {
  schema: typeof SOURCE_SCHEMA;
  id: string;
  title: string;
  home_url: string;
  match: MatchRule[];
  lang: string;
  kind: SourceKind;
  needs: SourceNeeds;
  every_secs?: number;
  max_new?: number;
  keep_days?: number;
  comments?: number;
  check?: { min_chars?: number; median_ratio?: number; max_replacement?: number };
}

export type RawBlock =
  | { type: TextBlockType; text: string }
  | { type: "img"; url: string; alt?: string }
  | { type: "table"; rows: string[][] };

export interface RawLink {
  url: string;
  text: string;
}

export interface RawArticle {
  url: string;
  title: string;
  lang?: string;
  published_at?: number;
  byline?: string;
  summary?: string;
  blocks: RawBlock[];
  comments?: { author?: string; at?: number; text: string }[];
  links?: RawLink[];
}

export interface ListEntry {
  url: string;
  title: string;
  published_at?: number;
  summary?: string;
  image?: string;
}

export interface FeedEntry extends ListEntry {
  content_html?: string;
  /** RSS `<comments>`: the page where the entry is discussed. */
  comments_url?: string;
}

export interface Extracted {
  list: ListEntry[];
  items: RawArticle[];
  state?: unknown;
}

export interface FetchResult {
  status: number;
  url: string;
  text: string;
  notModified: boolean;
  headers: Headers;
}

export interface ExtractCtx {
  source: SourceDef;
  state: unknown;
  limit: number;
  known(url: string): boolean;
  log(message: string): void;
  fetch(
    url: string,
    init?: { conditional?: boolean; headers?: Record<string, string> },
  ): Promise<FetchResult>;
  html(text: string): Document;
  xml(text: string): Document;
  feed(text: string): FeedEntry[];
  blocks(root: Element, base: string): { blocks: RawBlock[]; links: RawLink[] };
  browser<T>(url: string | null, script: string, args?: unknown): Promise<T>;
}

export type Extract = (ctx: ExtractCtx) => Promise<Extracted>;

/** The last stdout line of `site sync` run as a goal job (Agent State Protocol §9.9). */
export interface JobResult {
  summary: string;
  kind?: "transient" | "auth" | "program" | "budget";
  metrics: {
    listed: number;
    new: number;
    published: number;
    rejected: number;
    images: number;
    tokens: number;
    ms: number;
    /** Articles that were fetched and are not published yet, and why (the feed's `pending.reason`). */
    pending: number;
    pending_reason: NonNullable<FeedStatus["pending"]>["reason"] | null;
  };
}
