// A personal site in its bucket `user/control-panel/<user>/reflow`: reading
// and writing every key of Personal Site Protocol §2, and what is wrong with a
// value that does not follow §3 / §4.1.

import { contentTypeOf, errorCode, normalizeUrl, pageOf, rulesMatch } from "../reflow.ts";
import { feedObjectProblems } from "./feed.ts";
import {
  ARTICLE_SCHEMA,
  articleKey,
  EVENTS_KEY,
  EVENTS_SCHEMA,
  FEED_SCHEMA,
  feedKey,
  imageKey,
  markdownKey,
  READING_KEY,
  SITE_KEY,
  SITE_SCHEMA,
  SiteError,
  SOURCE_SCHEMA,
  SUPPLEMENT_SOURCE,
} from "./types.ts";
import type {
  Article,
  DisplayState,
  Events,
  Feed,
  ReadRecord,
  Reading,
  Site,
  SourceEntry,
  SourceKind,
  SourceNeeds,
} from "./types.ts";

export const IMMUTABLE = "public, max-age=31536000, immutable";
export const MAX_TOPICS = 16;
export const MAX_FEED_ITEMS = 100;
export const MAX_UPGRADES = 50;
export const MAX_EVENTS = 200;
const SITE_WRITE_TRIES = 3;
const EXPIRY_MS = 181 * 86_400_000;

/**
 * When the bucket drops an article, its Markdown or an image by itself. The
 * job removes them earlier (keep_days); this is for what is left behind by a
 * source that was removed or a maintainer that changed.
 */
export const expiryOf = (fetchedAt: number) => fetchedAt + EXPIRY_MS;

/** The part of the websdk `BucketClient` a site uses. */
export interface SiteStore {
  listObjects(
    bucketId: string,
    req: { prefix?: string; delimiter?: string; start_after?: string },
  ): Promise<{ items: { key: string }[]; common_prefixes: string[]; next_start_after?: string }>;
  fetchObject(bucketId: string, key: string): Promise<Response>;
  putBytes(
    bucketId: string,
    key: string,
    body: Uint8Array | string,
    options?: { contentType?: string; if_match?: string; if_none_match?: string; expiresAt?: number },
  ): Promise<{ obj_id: string }>;
  /** Binds `key` to an object the store already has; the only way to give a key `http_headers`. */
  put(
    bucketId: string,
    req: {
      key: string;
      obj_id: string;
      content_type?: string;
      http_headers?: Record<string, string>;
      expires_at?: number;
    },
  ): Promise<unknown>;
  get(bucketId: string, key: string): Promise<{ obj_id?: string; expires_at?: number }>;
  delete(bucketId: string, key: string): Promise<unknown>;
}

// ── ids ────────────────────────────────────────────────────────────────────

export function isSourceId(value: unknown): value is string {
  return typeof value === "string" && /^[a-z0-9](?:[a-z0-9-]{0,46}[a-z0-9])?$/.test(value);
}

export function isArticleId(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{16}$/.test(value);
}

/** The normalized URL of an article and its id (the `<page>` of phase 1). */
export async function articleAddress(input: string): Promise<{ url: string; id: string }> {
  let url: string;
  try {
    url = normalizeUrl(input).url;
  } catch (err) {
    throw new SiteError("invalid", (err as Error).message);
  }
  return { url, id: await pageOf(url) };
}

/** The first source whose match rules take `url`. */
export function sourceOfUrl(site: Site, url: string): SourceEntry | null {
  let address;
  try {
    address = normalizeUrl(url);
  } catch {
    return null;
  }
  return site.sources.find((source) => rulesMatch(source.match, address)) ?? null;
}

/** What a source shows as (§3.1): composed from site.json, its upgrades and its feed. */
export function displayState(site: Site, source: SourceEntry, feed: Feed | null): DisplayState {
  if (source.state === "paused") return "paused";
  if (site.upgrades.some((upgrade) => upgrade.state === "open" && upgrade.source === source.id)) {
    return "needs_upgrade";
  }
  if (source.state === "preparing" || source.state === "failed") return source.state;
  return feed ? feed.status.state : "preparing";
}

/** The sync parameters a source of this kind gets when its package states none (§3.1). */
export function sourceDefaults(kind: SourceKind, needs: SourceNeeds) {
  const every = needs === "login" ? 21600 : { news: 1800, blog: 21600, forum: 3600 }[kind];
  return { every_secs: every, max_new: 20, keep_days: 30, comments: 10 };
}

// ── validation ─────────────────────────────────────────────────────────────

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

const isText = (value: unknown): value is string => typeof value === "string";
const isName = (value: unknown): value is string => typeof value === "string" && value.trim() !== "";
const isMs = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;
const isMsOrNull = (value: unknown) => value === null || isMs(value);
const isBi = (value: unknown) => isRecord(value) && isText(value.o) && isText(value.t);
const isImageKey = (value: unknown) =>
  typeof value === "string" && /^_data\/img\/[0-9a-f]{64}\.[a-z0-9]+$/.test(value);
const intIn = (value: unknown, min: number, max: number) =>
  Number.isSafeInteger(value) && (value as number) >= min && (value as number) <= max;
const texts = (value: unknown, max = Infinity) =>
  Array.isArray(value) && value.length <= max && value.every(isName) &&
  new Set(value).size === value.length;

function isHttpUrl(value: unknown): value is string {
  try {
    const url = new URL(value as string);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname !== "";
  } catch {
    return false;
  }
}

function matchProblems(match: unknown, at: string): string[] {
  if (!Array.isArray(match) || match.length === 0) return [`${at}match must list at least one { host, path }`];
  const problems: string[] = [];
  match.forEach((rule, index) => {
    const where = `${at}match[${index}]`;
    if (
      !isRecord(rule) || !isName(rule.host) || rule.host !== rule.host.toLowerCase() ||
      !/^[a-z0-9._-]+$/.test(rule.host) || rule.host.endsWith(".")
    ) {
      problems.push(`${where}.host must be a lower-case host name without a port`);
    } else if (rule.path !== undefined && (!isText(rule.path) || !rule.path.startsWith("/"))) {
      problems.push(`${where}.path must be a path pattern starting with "/"`);
    }
  });
  return problems;
}

const KINDS = ["news", "blog", "forum"];
const NEEDS = ["http", "browser", "login"];

// The fields source.json and a site's source entry share.
function sourceFieldProblems(value: Record<string, unknown>, at: string, optionalParams: boolean): string[] {
  const problems: string[] = [];
  const param = (name: string, ok: boolean) => {
    if (!(optionalParams && value[name] === undefined) && !ok) problems.push(`${at}${name} is out of range`);
  };
  if (!isSourceId(value.id)) {
    problems.push(`${at}id must be 1-48 of a-z, 0-9 and "-", not starting or ending with "-"`);
  }
  if (!isName(value.title)) problems.push(`${at}title must be text`);
  if (!isHttpUrl(value.home_url)) problems.push(`${at}home_url must be an http(s) URL`);
  problems.push(...matchProblems(value.match, at));
  if (!isName(value.lang)) problems.push(`${at}lang must be a language tag`);
  if (!KINDS.includes(value.kind as string)) problems.push(`${at}kind must be ${KINDS.join(" | ")}`);
  if (!NEEDS.includes(value.needs as string)) problems.push(`${at}needs must be ${NEEDS.join(" | ")}`);
  param("every_secs", intIn(value.every_secs, 300, 30 * 86400));
  param("max_new", intIn(value.max_new, 1, 100));
  param("keep_days", intIn(value.keep_days, 1, 3650));
  param("comments", intIn(value.comments, 0, 10));
  return problems;
}

/** What is wrong with a `source.json`; empty when it is valid. */
export function sourceDefProblems(value: unknown): string[] {
  if (!isRecord(value)) return ["source.json is not a JSON object"];
  const problems: string[] = [];
  if (value.schema !== SOURCE_SCHEMA) problems.push(`schema must be "${SOURCE_SCHEMA}"`);
  problems.push(...sourceFieldProblems(value, "", true));
  if (value.check !== undefined) {
    const check = value.check;
    const ratio = (v: unknown) => v === undefined || (typeof v === "number" && v >= 0 && v <= 1);
    if (
      !isRecord(check) || !(check.min_chars === undefined || intIn(check.min_chars, 0, 1e6)) ||
      !ratio(check.median_ratio) || !ratio(check.max_replacement)
    ) {
      problems.push("check must be { min_chars, median_ratio (0-1), max_replacement (0-1) }, each optional");
    }
  }
  return problems;
}

function sourceEntryProblems(value: unknown, at: string): string[] {
  if (!isRecord(value)) return [`${at} must be an object`];
  const problems = sourceFieldProblems(value, `${at}.`, false);
  if (isSourceId(value.id) && value.job !== `src-${value.id}`) problems.push(`${at}.job must be "src-${value.id}"`);
  if (value.version !== null && !(isText(value.version) && /^s[1-9]\d*$/.test(value.version))) {
    problems.push(`${at}.version must be "s<n>" or null`);
  }
  if (!["preparing", "active", "paused", "failed"].includes(value.state as string)) {
    problems.push(`${at}.state must be preparing | active | paused | failed`);
  }
  if (value.note !== null && !isText(value.note)) problems.push(`${at}.note must be text or null`);
  if (!isMs(value.added_at)) problems.push(`${at}.added_at must be a millisecond timestamp`);
  if (!isName(value.added_by)) problems.push(`${at}.added_by must say who added it`);
  return problems;
}

function upgradeProblems(value: unknown, at: string): string[] {
  if (!isRecord(value)) return [`${at} must be an object`];
  const problems: string[] = [];
  if (!isSourceId(value.id)) problems.push(`${at}.id must be a short name of a-z, 0-9 and "-"`);
  if (value.source !== undefined && !isSourceId(value.source)) problems.push(`${at}.source must be a source id`);
  for (const field of ["title", "summary", "file", "goal_id"]) {
    if (!isName(value[field])) problems.push(`${at}.${field} must be text`);
  }
  if (!isMs(value.opened_at)) problems.push(`${at}.opened_at must be a millisecond timestamp`);
  if (!["open", "accepted", "dismissed", "closed"].includes(value.state as string)) {
    problems.push(`${at}.state must be open | accepted | dismissed | closed`);
  }
  if (value.decided_at !== undefined && !isMs(value.decided_at)) {
    problems.push(`${at}.decided_at must be a millisecond timestamp`);
  }
  return problems;
}

/** What is wrong with a `_data/site.json`; empty when it is valid. */
export function siteProblems(value: unknown): string[] {
  if (!isRecord(value)) return ["site.json is not a JSON object"];
  const problems: string[] = [];
  if (value.schema !== SITE_SCHEMA) problems.push(`schema must be "${SITE_SCHEMA}"`);
  if (!isName(value.title)) problems.push("title must be text");
  if (!isName(value.language)) problems.push("language must be a BCP 47 tag");
  if (!isText(value.owner_did) || !value.owner_did.startsWith("did:")) problems.push("owner_did must be a DID");
  if (!texts(value.topics, MAX_TOPICS)) problems.push(`topics must list at most ${MAX_TOPICS} distinct names`);

  const template = value.template;
  if (!isRecord(template) || !isText(template.version) || !/^t[1-9]\d*$/.test(template.version)) {
    problems.push('template.version must be "t<n>"');
  }
  const params = isRecord(template) ? template.params : undefined;
  if (!isRecord(params)) {
    problems.push("template.params must be an object");
  } else {
    if (params.home_order !== undefined && !["time", "source", "events"].includes(params.home_order as string)) {
      problems.push("template.params.home_order must be time | source | events");
    }
    if (params.per_column !== undefined && !intIn(params.per_column, 1, 100)) {
      problems.push("template.params.per_column must be 1 to 100");
    }
    for (const list of ["hide_topics", "collapsed"]) {
      if (params[list] !== undefined && !texts(params[list])) problems.push(`template.params.${list} must list names`);
    }
  }

  const ids = new Set<string>();
  if (!Array.isArray(value.sources)) {
    problems.push("sources must be a list");
  } else {
    value.sources.forEach((source, index) => {
      problems.push(...sourceEntryProblems(source, `sources[${index}]`));
      const id = isRecord(source) ? source.id : undefined;
      if (isText(id)) {
        if (ids.has(id)) problems.push(`sources[${index}].id "${id}" is listed twice`);
        ids.add(id);
      }
    });
  }
  if (!Array.isArray(value.columns)) {
    problems.push("columns must be a list");
  } else {
    value.columns.forEach((column, index) => {
      if (!isRecord(column) || !isName(column.id) || !isName(column.title) || !texts(column.sources)) {
        problems.push(`columns[${index}] must be { id, title, sources: [source ids] }`);
      }
    });
  }
  if (!Array.isArray(value.upgrades) || value.upgrades.length > MAX_UPGRADES) {
    problems.push(`upgrades must list at most ${MAX_UPGRADES} requests`);
  } else {
    value.upgrades.forEach((upgrade, index) => problems.push(...upgradeProblems(upgrade, `upgrades[${index}]`)));
  }
  if (!isRecord(value.reading) || typeof value.reading.enabled !== "boolean") {
    problems.push("reading.enabled must be true or false");
  }
  if (!isRecord(value.limits) || !isMs(value.limits.translate_tokens_per_day)) {
    problems.push("limits.translate_tokens_per_day must be a token count");
  }
  const maintainer = value.maintainer;
  if (maintainer !== null && !(isRecord(maintainer) && isName(maintainer.agent) && isName(maintainer.goal_id))) {
    problems.push("maintainer must be { agent, goal_id } or null");
  }
  if (!isMs(value.updated_at)) problems.push("updated_at must be a millisecond timestamp");
  if (!isName(value.updated_by)) problems.push("updated_by must say who wrote it");
  return problems;
}

/** What is wrong with a `_data/feed/<source>.json`; empty when it is valid. */
export function feedProblems(value: unknown): string[] {
  if (!isRecord(value)) return ["a feed is a JSON object"];
  const problems: string[] = [];
  if (value.schema !== FEED_SCHEMA) problems.push(`schema must be "${FEED_SCHEMA}"`);
  if (!isSourceId(value.source)) problems.push("source must be a source id");
  const status = value.status;
  if (!isRecord(status)) {
    problems.push("status must be an object");
  } else {
    if (!["preparing", "ok", "failed", "needs_login"].includes(status.state as string)) {
      problems.push("status.state must be preparing | ok | failed | needs_login");
    }
    if (!isMsOrNull(status.checked_at)) problems.push("status.checked_at must be a timestamp or null");
    if (!isMsOrNull(status.published_at)) problems.push("status.published_at must be a timestamp or null");
    const error = status.error;
    if (
      error !== null && !(isRecord(error) && isText(error.message) && isMs(error.at) &&
        ["transient", "auth", "program", "config"].includes(error.kind as string))
    ) {
      problems.push("status.error must be { kind: transient | auth | program | config, message, at } or null");
    }
    const pending = status.pending;
    if (
      pending !== null && !(isRecord(pending) && intIn(pending.count, 1, 1e6) && isText(pending.message) &&
        ["translate_budget", "translate_config", "translate_failed", "time", "images"].includes(
          pending.reason as string,
        ))
    ) {
      problems.push(
        "status.pending must be { count, reason: translate_budget | translate_config | translate_failed | time | images, message } or null",
      );
    }
    if (status.login_url !== null && !isHttpUrl(status.login_url)) {
      problems.push("status.login_url must be an http(s) URL or null");
    }
    if ((status.state === "needs_login") !== (status.login_url !== null)) {
      problems.push("status.login_url goes with the state needs_login");
    }
  }
  if (!Array.isArray(value.items) || value.items.length > MAX_FEED_ITEMS) {
    problems.push(`items must list at most ${MAX_FEED_ITEMS} entries`);
  } else {
    value.items.forEach((item, index) => {
      const at = `items[${index}]`;
      if (!isRecord(item)) {
        problems.push(`${at} must be an object`);
        return;
      }
      if (!isArticleId(item.id)) problems.push(`${at}.id must be 16 hex characters`);
      if (!isText(item.object_id) || !item.object_id.startsWith("cyfeed:")) problems.push(`${at}.object_id must be a cyfeed id`);
      if (!isHttpUrl(item.url)) problems.push(`${at}.url must be an http(s) URL`);
      if (!isText(item.title) || !isText(item.summary)) problems.push(`${at}.title and summary must be text`);
      if (item.image !== null && !isImageKey(item.image)) problems.push(`${at}.image must be a _data/img/ key or null`);
      if (!texts(item.tags, 3)) problems.push(`${at}.tags must list at most 3 topics`);
      if (!isMsOrNull(item.published_at)) problems.push(`${at}.published_at must be a timestamp or null`);
      if (!isMs(item.fetched_at)) problems.push(`${at}.fetched_at must be a timestamp`);
      if (!intIn(item.rev, 1, 1e6)) problems.push(`${at}.rev must start at 1`);
    });
  }
  if (!isMs(value.updated_at)) problems.push("updated_at must be a millisecond timestamp");
  return problems;
}

const TEXT_BLOCKS = ["p", "h2", "h3", "quote", "li", "code"];

function blockProblem(block: unknown): string | null {
  if (!isRecord(block)) return "must be an object";
  if (TEXT_BLOCKS.includes(block.type as string)) return isBi(block.text) ? null : "text must be { o, t }";
  if (block.type === "table") {
    const ok = Array.isArray(block.rows) && block.rows.every((row) => Array.isArray(row) && row.every(isBi));
    return ok ? null : "rows must be rows of { o, t }";
  }
  if (block.type !== "img") return `unknown type ${JSON.stringify(block.type)}`;
  if (!isBi(block.alt) || !isMs(block.w) || !isMs(block.h)) return "an img has alt { o, t }, w and h";
  if (block.missing === true) return block.src === null && block.object === null ? null : "a missing img has no src and object";
  if (block.missing !== undefined) return "missing is true or absent";
  return isImageKey(block.src) && isText(block.object) && block.object.startsWith("cyfile:")
    ? null
    : "an img has src (a _data/img/ key) and object (a cyfile id)";
}

/** What is wrong with a `_data/a/<article>.json`; empty when it is valid. */
export function articleProblems(value: unknown): string[] {
  if (!isRecord(value)) return ["an article is a JSON object"];
  const problems: string[] = [];
  if (value.schema !== ARTICLE_SCHEMA) problems.push(`schema must be "${ARTICLE_SCHEMA}"`);
  if (!isArticleId(value.id)) problems.push("id must be 16 hex characters");
  if (value.source !== SUPPLEMENT_SOURCE && !isSourceId(value.source)) problems.push("source must be a source id");
  if (!isHttpUrl(value.url)) problems.push("url must be an http(s) URL");
  if (!isName(value.lang)) problems.push("lang must be a language tag");
  if (!isMsOrNull(value.published_at)) problems.push("published_at must be a timestamp or null");
  if (!isMs(value.fetched_at)) problems.push("fetched_at must be a timestamp");
  if (!intIn(value.rev, 1, 1e6)) problems.push("rev must start at 1");
  if (!isText(value.object_id) || !value.object_id.startsWith("cyfeed:")) problems.push("object_id must be a cyfeed id");
  problems.push(...feedObjectProblems(value.object).map((problem) => `object: ${problem}`));
  if (!isBi(value.title) || !isBi(value.summary)) problems.push("title and summary must be { o, t }");
  if (!isText(value.byline)) problems.push("byline must be text");
  if (!texts(value.tags, 3)) problems.push("tags must list at most 3 topics");
  if (!Array.isArray(value.blocks)) {
    problems.push("blocks must be a list");
  } else {
    value.blocks.forEach((block, index) => {
      const problem = blockProblem(block);
      if (problem) problems.push(`blocks[${index}]: ${problem}`);
    });
  }
  const comments = value.comments;
  if (
    !Array.isArray(comments) ||
    !comments.every((c) => isRecord(c) && isText(c.author) && isMsOrNull(c.at) && isBi(c.text))
  ) {
    problems.push("comments must list { author, at, text: { o, t } }");
  }
  const links = value.links;
  if (
    !Array.isArray(links) ||
    !links.every((l) =>
      isRecord(l) && isHttpUrl(l.url) && isBi(l.text) && (l.article === undefined || isArticleId(l.article))
    )
  ) {
    problems.push("links must list { url, text: { o, t }, article? }");
  }
  const images = value.images;
  if (
    !isRecord(images) ||
    !Object.entries(images).every(([id, image]) =>
      id.startsWith("cyfile:") && isRecord(image) && isImageKey(image.key) && isMs(image.w) && isMs(image.h)
    )
  ) {
    problems.push("images must map cyfile ids to { key, w, h }");
  }
  if (value.thumb !== null && !isImageKey(value.thumb)) problems.push("thumb must be a _data/img/ key or null");
  const supplement = value.supplement;
  if (supplement !== null && !(isRecord(supplement) && isEventId(supplement.event) && isText(supplement.adds))) {
    problems.push("supplement must be { event, adds } or null");
  }
  if ((value.source === SUPPLEMENT_SOURCE) !== (supplement !== null)) {
    problems.push(`a supplement article, and only one, has the source "${SUPPLEMENT_SOURCE}"`);
  }
  return problems;
}

/** events.json of a site that has none yet. */
export function emptyEvents(): Events {
  return { schema: EVENTS_SCHEMA, events: [], updated_at: 0 };
}

export function isEventId(value: unknown): value is string {
  return typeof value === "string" && value.length <= 48 && /^e-[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(value);
}

/**
 * What is wrong with a `_data/events.json`; empty when it is valid. `filled`
 * is false for what a writer hands to `site events put`: the `title`,
 * `source` and `at` of the entries are not asked of it.
 */
export function eventsProblems(value: unknown, filled = true): string[] {
  if (!isRecord(value)) return ["events.json is not a JSON object"];
  const problems: string[] = [];
  if (value.schema !== EVENTS_SCHEMA) problems.push(`schema must be "${EVENTS_SCHEMA}"`);
  if (!Array.isArray(value.events) || value.events.length > MAX_EVENTS) {
    problems.push(`events must list at most ${MAX_EVENTS} events`);
  } else {
    const ids = new Set<unknown>();
    value.events.forEach((event, index) => {
      const at = `events[${index}]`;
      if (!isRecord(event)) {
        problems.push(`${at} must be an object`);
        return;
      }
      if (!isEventId(event.id)) problems.push(`${at}.id must be "e-" and a short name of a-z, 0-9 and "-", at most 48 characters`);
      if (ids.has(event.id)) problems.push(`${at}.id "${event.id}" is listed twice`);
      ids.add(event.id);
      if (!isName(event.title) || !isText(event.summary)) problems.push(`${at}.title and summary must be text`);
      const copied = (entry: Record<string, unknown>, source: boolean) =>
        !filled ||
        (isText(entry.title) && isMs(entry.at) &&
          (!source || entry.source === SUPPLEMENT_SOURCE || isSourceId(entry.source)));
      const articles = event.articles;
      if (
        !Array.isArray(articles) ||
        !articles.every((a) => isRecord(a) && isArticleId(a.id) && isText(a.adds) && copied(a, true))
      ) {
        problems.push(`${at}.articles must list { id, adds${filled ? ", title, source, at" : ""} }`);
      }
      const supplements = event.supplements;
      if (
        !Array.isArray(supplements) ||
        !supplements.every((s) =>
          isRecord(s) && isArticleId(s.article) && isHttpUrl(s.source_url) && isText(s.adds) && copied(s, false)
        )
      ) {
        problems.push(`${at}.supplements must list { article, source_url, adds${filled ? ", title, at" : ""} }`);
      }
      if (!isMs(event.updated_at)) problems.push(`${at}.updated_at must be a millisecond timestamp`);
    });
  }
  if (!isMs(value.updated_at)) problems.push("updated_at must be a millisecond timestamp");
  return problems;
}

/** One record a page reported to `POST /_read` (§3.6). */
export function isReadRecord(value: unknown): value is ReadRecord {
  return isRecord(value) && isArticleId(value.article) &&
    (value.source === SUPPLEMENT_SOURCE || isSourceId(value.source)) && isMs(value.opened_at) &&
    isMs(value.dwell_ms) && intIn(value.max_scroll, 0, 100);
}

// ── the bucket ─────────────────────────────────────────────────────────────

export function invalid(what: string, problems: string[]): SiteError {
  return Object.assign(new SiteError("invalid", `${what} is not valid: ${problems.join("; ")}`), { problems });
}

const json = (value: unknown) => JSON.stringify(value, null, 2);

/** One site: a store and the bucket the site is in. */
export class SiteBucket {
  constructor(readonly store: SiteStore, readonly bucketId: string) {}

  /** The bytes of a key and its ETag; `null` when the key does not exist. */
  async read(key: string): Promise<{ bytes: Uint8Array; etag: string | null } | null> {
    let response: Response;
    try {
      response = await this.store.fetchObject(this.bucketId, key);
    } catch (err) {
      if (errorCode(err) === "no_such_key") return null;
      throw err;
    }
    return { bytes: new Uint8Array(await response.arrayBuffer()), etag: response.headers.get("etag") };
  }

  async readJson(key: string): Promise<{ value: unknown; etag: string | null } | null> {
    const found = await this.read(key);
    if (!found) return null;
    try {
      return { value: JSON.parse(new TextDecoder().decode(found.bytes)), etag: found.etag };
    } catch {
      throw new SiteError("invalid", `${key} in the bucket is not JSON`);
    }
  }

  private async checked<T>(key: string, problemsOf: (value: unknown) => string[]): Promise<T | null> {
    const found = await this.readJson(key);
    if (!found) return null;
    const problems = problemsOf(found.value);
    if (problems.length) throw invalid(`${key} in the bucket`, problems);
    return found.value as T;
  }

  async write(
    key: string,
    body: Uint8Array | string,
    options: { if_match?: string; if_none_match?: string; expiresAt?: number } = {},
  ): Promise<string> {
    const put = await this.store.putBytes(this.bucketId, key, body, { contentType: contentTypeOf(key), ...options });
    return put.obj_id;
  }

  /**
   * Writes a key that never changes: kept when it is there already, served as
   * `immutable`. With `expiresAt` the key expires then, and one that is there
   * already and would expire earlier is given the later time.
   */
  async writeImmutable(key: string, body: Uint8Array | string, expiresAt?: number): Promise<string> {
    const expiry = expiresAt === undefined ? {} : { expires_at: expiresAt };
    let objId: string;
    try {
      objId = await this.write(key, body, { if_none_match: "*", ...(expiresAt === undefined ? {} : { expiresAt }) });
    } catch (err) {
      if (errorCode(err) !== "precondition_failed") throw err;
      const existing = await this.store.get(this.bucketId, key);
      if (!existing.obj_id) throw new Error(`${key} binds no object`);
      if (expiresAt === undefined || (existing.expires_at ?? 0) >= expiresAt) return existing.obj_id;
      objId = existing.obj_id;
    }
    await this.store.put(this.bucketId, {
      key,
      obj_id: objId,
      content_type: contentTypeOf(key),
      http_headers: { "cache-control": IMMUTABLE },
      ...expiry,
    });
    return objId;
  }

  async keys(prefix: string): Promise<string[]> {
    const keys: string[] = [];
    let startAfter: string | undefined;
    do {
      const page = await this.store.listObjects(this.bucketId, {
        prefix,
        ...(startAfter ? { start_after: startAfter } : {}),
      });
      keys.push(...page.items.map((item) => item.key));
      startAfter = page.next_start_after;
    } while (startAfter);
    return keys;
  }

  /** The names directly under `prefix` that have keys below them. */
  async folders(prefix: string): Promise<string[]> {
    const names: string[] = [];
    let startAfter: string | undefined;
    do {
      const page = await this.store.listObjects(this.bucketId, {
        prefix,
        delimiter: "/",
        ...(startAfter ? { start_after: startAfter } : {}),
      });
      names.push(...page.common_prefixes.map((folder) => folder.slice(prefix.length, -1)));
      startAfter = page.next_start_after;
    } while (startAfter);
    return names;
  }

  async remove(key: string): Promise<void> {
    try {
      await this.store.delete(this.bucketId, key);
    } catch (err) {
      if (errorCode(err) !== "no_such_key") throw err;
    }
  }

  // ── site.json ────────────────────────────────────────────────────────────

  async site(): Promise<{ site: Site; etag: string | null } | null> {
    const found = await this.readJson(SITE_KEY);
    if (!found) return null;
    const problems = siteProblems(found.value);
    if (problems.length) throw invalid(`${SITE_KEY} in the bucket`, problems);
    return { site: found.value as Site, etag: found.etag };
  }

  /** The site, or the failure that tells to make one. */
  async requireSite(): Promise<Site> {
    const found = await this.site();
    if (!found) throw new SiteError("invalid", "there is no site yet: run `site init` first");
    return found.site;
  }

  /** Writes the first site.json; `false` when the bucket has one already. */
  async createSite(site: Site): Promise<boolean> {
    const problems = siteProblems(site);
    if (problems.length) throw invalid("the site", problems);
    try {
      await this.write(SITE_KEY, json(site), { if_none_match: "*" });
      return true;
    } catch (err) {
      if (errorCode(err) === "precondition_failed") return false;
      throw err;
    }
  }

  /**
   * Changes site.json: `change` edits the site it is given, the result is
   * checked and written with If-Match; when somebody else wrote in between it
   * is read again and changed again, three times at most.
   */
  async updateSite(change: (site: Site) => void, by: string, now: number = Date.now()): Promise<Site> {
    for (let attempt = 1;; attempt += 1) {
      const found = await this.site();
      if (!found) throw new SiteError("invalid", "there is no site yet: run `site init` first");
      const site = found.site;
      change(site);
      site.updated_at = now;
      site.updated_by = by;
      const problems = siteProblems(site);
      if (problems.length) throw invalid("the changed site.json", problems);
      try {
        await this.write(SITE_KEY, json(site), found.etag ? { if_match: found.etag } : {});
        return site;
      } catch (err) {
        if (errorCode(err) !== "precondition_failed") throw err;
        if (attempt === SITE_WRITE_TRIES) {
          throw Object.assign(
            new Error(`${SITE_KEY} was changed by somebody else ${SITE_WRITE_TRIES} times in a row; try again`),
            { code: "precondition_failed" },
          );
        }
      }
    }
  }

  // ── feed, articles, images ───────────────────────────────────────────────

  feed(source: string): Promise<Feed | null> {
    return this.checked<Feed>(feedKey(source), feedProblems);
  }

  async writeFeed(feed: Feed): Promise<void> {
    const problems = feedProblems(feed);
    if (problems.length) throw invalid(`the feed of ${feed.source}`, problems);
    await this.write(feedKey(feed.source), json(feed));
  }

  article(id: string): Promise<Article | null> {
    return this.checked<Article>(articleKey(id), articleProblems);
  }

  async writeArticle(article: Article): Promise<void> {
    const problems = articleProblems(article);
    if (problems.length) throw invalid(`the article ${article.url}`, problems);
    await this.write(articleKey(article.id), json(article), { expiresAt: expiryOf(article.fetched_at) });
  }

  /** Stores the Markdown a FeedObject wraps; its `cyfile` id. */
  writeMarkdown(id: string, text: string, fetchedAt: number): Promise<string> {
    return this.write(markdownKey(id), text, { expiresAt: expiryOf(fetchedAt) });
  }

  /**
   * Stores an image of an article fetched at `fetchedAt` under the SHA-256 of
   * its bytes; its key and `cyfile` id.
   */
  async writeImage(bytes: Uint8Array, ext: string, fetchedAt: number): Promise<{ key: string; object: string }> {
    const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
    const hex = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
    const key = imageKey(hex, ext);
    return { key, object: await this.writeImmutable(key, bytes, expiryOf(fetchedAt)) };
  }

  events(): Promise<Events | null> {
    return this.checked<Events>(EVENTS_KEY, eventsProblems);
  }

  async writeEvents(events: Events): Promise<void> {
    const problems = eventsProblems(events);
    if (problems.length) throw invalid("events.json", problems);
    await this.write(EVENTS_KEY, json(events));
  }

  /**
   * Checks what a writer gives as events.json and copies into every entry the
   * `title`, `source` and `at` of its article: the event page orders and
   * shows the entries by them without fetching each article. An entry whose
   * article is not in the bucket is a problem of the input.
   */
  async completeEvents(value: unknown): Promise<Events> {
    const problems = eventsProblems(value, false);
    if (problems.length) throw invalid("events.json", problems);
    const events = value as Events;
    const articles = new Map<string, Article | null>();
    const lookUp = async (id: string, at: string) => {
      if (!articles.has(id)) articles.set(id, await this.article(id));
      const article = articles.get(id);
      if (!article) problems.push(`${at}: the article ${id} is not in the bucket`);
      return article;
    };
    for (const [e, event] of events.events.entries()) {
      for (const [n, entry] of event.articles.entries()) {
        const article = await lookUp(entry.id, `events[${e}].articles[${n}]`);
        if (!article) continue;
        event.articles[n] = {
          id: entry.id,
          adds: entry.adds,
          title: article.title.t,
          source: article.source,
          at: article.published_at ?? article.fetched_at,
        };
      }
      for (const [n, entry] of event.supplements.entries()) {
        const article = await lookUp(entry.article, `events[${e}].supplements[${n}]`);
        if (!article) continue;
        event.supplements[n] = {
          article: entry.article,
          source_url: entry.source_url,
          adds: entry.adds,
          title: article.title.t,
          at: article.published_at ?? article.fetched_at,
        };
      }
    }
    if (problems.length) throw invalid("events.json", problems);
    return events;
  }

  async reading(): Promise<Reading | null> {
    const found = await this.readJson(READING_KEY);
    return found ? found.value as Reading : null;
  }

  async writeReading(reading: Reading): Promise<void> {
    await this.write(READING_KEY, json(reading));
  }
}
