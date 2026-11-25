// One sync of one source: Personal Site Protocol §5.1. Extract, check, images,
// translation, FeedObject shell, article, feed, clean-up. What was fetched and
// not yet published waits in the job's `data/pending.json`; readers never see
// half of an article because the feed is written after everything it lists.

import { normalizeUrl } from "../reflow.ts";
import { bodyOf, checkBatch, nextLengths } from "./check.ts";
import type { CheckReport } from "./check.ts";
import { extractContext, extractedProblems, failureOf, Net, runExtract } from "./extract.ts";
import type { BrowserRun, Validators } from "./extract.ts";
import { articleMarkdown, buildFeedObject } from "./feed.ts";
import { downloadImage, NotAPicture, prepareImage } from "./images.ts";
import type { Resize } from "./images.ts";
import { articleAddress, invalid, MAX_FEED_ITEMS, SiteBucket } from "./store.ts";
import { ARTICLE_SCHEMA, articleKey, FEED_SCHEMA, markdownKey, SiteError, SUPPLEMENT_SOURCE } from "./types.ts";
import type {
  Article,
  Bi,
  Block,
  Feed,
  FeedItem,
  FeedStatus,
  JobResult,
  RawArticle,
  Site,
  SourceDef,
  SourceEntry,
} from "./types.ts";
import type { Translator } from "./translate.ts";

const DAY_MS = 86_400_000;
const KEEP_READ_DAYS = 180;
const REJECTED_MEMORY_MS = DAY_MS;
const FEED_EVERY = 10;
const MAX_ARTICLE_IMAGES = 20;
const MAX_DELETES = 500;
const TRANSLATE_TRIES = 3;
const STATE_SCHEMA = "buckyos.site.job/1";

/** `data/state.json` of a source's job. */
interface JobState {
  schema: string;
  /** What `extract` returned as its state. */
  extract: unknown;
  validators: Record<string, Validators>;
  /** Body lengths of the articles accepted lately (§5.3). */
  lengths: number[];
  /** How many entries the list had the last time it was read. */
  listed: number;
  /** Articles that did not pass the check, by URL, and when: not fetched again for a day. */
  rejected: Record<string, number>;
  cleaned_at: number;
}

/**
 * `data/index.json`: what this job published. An article that was cleaned up
 * stays as `gone` while the source still lists it, so it is not fetched anew.
 */
type Index = Record<string, { url: string; at: number; images: string[]; gone?: true }>;

interface StoredImage {
  key: string;
  object: string;
  w: number;
  h: number;
}

/** An article that was fetched, and what is stored of its images so far. */
export interface Fetched {
  raw: RawArticle;
  fetched_at: number;
  /**
   * By image URL: what was stored; `null` once it counts as missing (the
   * article shows a placeholder); `"left_out"` for what was fetched and is no
   * picture the site keeps (its block is dropped).
   */
  images: Record<string, StoredImage | null | "left_out">;
  image_tries: Record<string, number>;
  /** The small picture for lists: made of the first image that came. */
  thumb: string | null;
}

/** One entry of `data/pending.json`: an article that is not published yet. */
interface PendingItem extends Fetched {
  tries: number;
}

export interface SyncEnv {
  bucket: SiteBucket;
  site: Site;
  source: SourceEntry;
  def: SourceDef;
  /** The source package. */
  dir: string;
  /** The job's `data/`. */
  dataDir: string;
  net: Net;
  /** `null`: publish the original as the translation. */
  translator: Translator | null;
  resize: Resize | null;
  /** Model tokens used so far in this run. */
  tokens?: () => number;
  now?: () => number;
  timeLimitMs: number;
  /** New articles to fetch at most; the source's `max_new` when absent. */
  limit?: number;
  /** After how many failed tries an image is published as missing. */
  imageTries?: number;
  cleanup?: boolean;
  browser?: BrowserRun;
  log: (message: string) => void;
}

export interface SyncReport {
  /** The run passed the check (§5.1: exit code 0). */
  ok: boolean;
  kind?: JobResult["kind"];
  summary: string;
  metrics: JobResult["metrics"];
  status: FeedStatus;
  published: {
    id: string;
    url: string;
    title: string;
    rev: number;
    chars: number;
    blocks: number;
    images: number;
    missing_images: number;
    comments: number;
    tags: string[];
    bytes: number;
  }[];
  rejected: CheckReport["rejected"];
  requests: number;
  not_modified: number;
  removed: number;
}

async function readLocal<T>(path: string): Promise<T | null> {
  try {
    return JSON.parse(await Deno.readTextFile(path)) as T;
  } catch (err) {
    if (err instanceof Deno.errors.NotFound || err instanceof SyntaxError) return null;
    throw err;
  }
}

async function writeLocal(path: string, value: unknown): Promise<void> {
  await Deno.writeTextFile(`${path}.tmp`, JSON.stringify(value));
  await Deno.rename(`${path}.tmp`, path);
}

function normalized(url: string): string | null {
  try {
    return normalizeUrl(url).url;
  } catch {
    return null;
  }
}

function sortItems(items: FeedItem[]): FeedItem[] {
  return items
    .sort((a, b) => (b.published_at ?? b.fetched_at) - (a.published_at ?? a.fetched_at))
    .slice(0, MAX_FEED_ITEMS);
}

// Every original text of an article, in the one order both sides of a translation use.
function textsOf(raw: RawArticle, comments: number): string[] {
  const texts = [raw.title, raw.summary ?? ""];
  for (const block of raw.blocks) {
    if (block.type === "img") texts.push(block.alt ?? "");
    else if (block.type === "table") texts.push(...block.rows.flat());
    else texts.push(block.text);
  }
  for (const comment of (raw.comments ?? []).slice(0, comments)) texts.push(comment.text);
  for (const link of raw.links ?? []) texts.push(link.text);
  return texts;
}

function sameOriginal(article: Article, raw: RawArticle): boolean {
  const cells = (blocks: (Block | RawArticle["blocks"][number])[]) =>
    blocks.map((block) => {
      if (block.type === "img") return "img";
      if (block.type === "table") {
        return (block.rows as (Bi | string)[][]).map((row) => row.map((c) => typeof c === "string" ? c : c.o));
      }
      return typeof block.text === "string" ? block.text : block.text.o;
    });
  return JSON.stringify([article.title.o, cells(article.blocks)]) === JSON.stringify([raw.title, cells(raw.blocks)]);
}

/**
 * Step 3: fetches, shrinks and stores the images of an article that are still
 * to do. One that could not be fetched `tries` times counts as missing from
 * then on, and `complete` is false while one is left to try again. What was
 * fetched and cannot be kept is settled at once: left out when it is no
 * picture of a kind the site keeps, missing when it is one that is too large
 * or does not convert.
 */
export async function storeImages(
  bucket: SiteBucket,
  net: Net,
  item: Fetched,
  options: { resize: Resize | null; tries: number; log: (message: string) => void },
): Promise<{ stored: number; complete: boolean }> {
  const referer = (await articleAddress(item.raw.url)).url;
  const urls = [...new Set(item.raw.blocks.flatMap((block) => block.type === "img" ? [block.url] : []))]
    .slice(0, MAX_ARTICLE_IMAGES);
  let stored = 0;
  let complete = true;
  for (const url of urls) {
    if (item.images[url] !== undefined) continue;
    if (/\.svg$/i.test(new URL(url).pathname)) {
      options.log(`image ${url}: an SVG is not a picture the site keeps — left out of the article`);
      item.images[url] = "left_out";
      continue;
    }
    let bytes;
    try {
      bytes = await downloadImage(net, url, referer);
    } catch (err) {
      // `program`: it came and is more than is taken; asking again brings the same.
      if (err instanceof SiteError && err.kind === "program") {
        options.log(`image ${url}: ${err.message} — published as missing`);
        item.images[url] = null;
        continue;
      }
      const tries = item.image_tries[url] = (item.image_tries[url] ?? 0) + 1;
      options.log(`image ${url}: ${(err as Error).message} (try ${tries} of ${options.tries})`);
      if (tries >= options.tries) item.images[url] = null;
      else complete = false;
      continue;
    }
    let prepared;
    try {
      prepared = await prepareImage(bytes, options.resize, item.thumb === null);
    } catch (err) {
      const leftOut = err instanceof NotAPicture;
      options.log(`image ${url}: ${(err as Error).message} — ${leftOut ? "left out of the article" : "published as missing"}`);
      item.images[url] = leftOut ? "left_out" : null;
      continue;
    }
    const full = await bucket.writeImage(prepared.full.bytes, prepared.full.ext, item.fetched_at);
    item.images[url] = { ...full, w: prepared.full.w, h: prepared.full.h };
    if (prepared.thumb) {
      item.thumb = prepared.thumb === prepared.full
        ? full.key
        : (await bucket.writeImage(prepared.thumb.bytes, prepared.thumb.ext, item.fetched_at)).key;
    }
    stored += 1;
  }
  return { stored, complete };
}

/**
 * Steps 4–5 for one article whose images are done: translation and tags, the
 * Markdown, the FeedObject shell, the article. `previous` is the revision it
 * corrects; `has` tells whether a link's target is an article of the site.
 */
export async function writeArticle(
  bucket: SiteBucket,
  item: Fetched,
  options: {
    site: Site;
    source: string;
    lang: string;
    comments: number;
    translator: Translator | null;
    previous: Article | null;
    has: (id: string) => boolean;
    supplement?: Article["supplement"];
  },
): Promise<Article> {
  const { raw } = item;
  const { site } = options;
  const { url, id } = await articleAddress(raw.url);
  const originals = textsOf(raw, options.comments);
  const translation = options.translator
    ? await options.translator.translate({
      id,
      from: options.lang,
      to: site.language,
      topics: site.topics,
      texts: originals,
      summary: raw.summary ?? "",
    })
    : { texts: originals, tags: [], summary: "" };
  let at = 0;
  const bi = (): Bi => ({ o: originals[at], t: translation.texts[at++] });

  const title = bi();
  const summary = bi();
  if (!summary.o.trim()) summary.t = translation.summary;
  const images: Article["images"] = {};
  const blocks: Block[] = [];
  for (const block of raw.blocks) {
    if (block.type === "img") {
      const alt = bi();
      const stored = item.images[block.url];
      if (stored === undefined || stored === "left_out") continue;
      if (stored === null) {
        blocks.push({ type: "img", src: null, object: null, alt, w: 0, h: 0, missing: true });
      } else {
        blocks.push({ type: "img", src: stored.key, object: stored.object, alt, w: stored.w, h: stored.h });
        images[stored.object] = { key: stored.key, w: stored.w, h: stored.h };
      }
    } else if (block.type === "table") {
      blocks.push({ type: "table", rows: block.rows.map((row) => row.map(bi)) });
    } else {
      blocks.push({ type: block.type, text: bi() });
    }
  }
  const comments = (raw.comments ?? []).slice(0, options.comments)
    .map((comment) => ({ author: comment.author ?? "", at: comment.at ?? null, text: bi() }));
  const links: Article["links"] = [];
  for (const link of raw.links ?? []) {
    const text = bi();
    const target = await articleAddress(link.url).catch(() => null);
    if (target) links.push({ url: target.url, text, ...(options.has(target.id) ? { article: target.id } : {}) });
  }

  const body: Omit<Article, "object" | "object_id"> = {
    schema: ARTICLE_SCHEMA,
    id,
    source: options.source,
    url,
    lang: options.lang,
    published_at: raw.published_at ?? null,
    fetched_at: item.fetched_at,
    rev: options.previous ? options.previous.rev + 1 : 1,
    title,
    summary,
    byline: raw.byline ?? "",
    tags: translation.tags,
    blocks,
    comments,
    links,
    images,
    thumb: item.thumb,
    supplement: options.supplement ?? null,
  };
  const wraps = await bucket.writeMarkdown(id, articleMarkdown(body), item.fetched_at);
  const shell = buildFeedObject(body, {
    ownerDid: site.owner_did,
    wraps,
    ...(options.previous ? { baseOn: options.previous.object_id } : {}),
  });
  const article: Article = { ...body, ...shell };
  await bucket.writeArticle(article);
  return article;
}

/**
 * A supplement of an event (§3.5): one article the maintainer found elsewhere,
 * stored like any other with the source `_supplement`, and entered in the
 * event's `supplements`.
 */
export async function putSupplement(env: {
  bucket: SiteBucket;
  site: Site;
  raw: RawArticle;
  event: string;
  adds: string;
  net: Net;
  translator: Translator | null;
  resize: Resize | null;
  now?: number;
  log: (message: string) => void;
}): Promise<{ article: Article; images: number }> {
  const { bucket, raw } = env;
  const now = env.now ?? Date.now();
  const problems = extractedProblems({ list: [], items: [raw] });
  if (!problems.length && !raw.title.trim()) problems.push("items[0].title is empty");
  if (problems.length) throw invalid("the supplement", problems.map((problem) => problem.replace("items[0]", "the article")));
  const events = await bucket.events();
  const event = events?.events.find((entry) => entry.id === env.event);
  if (!events || !event) throw new SiteError("invalid", `events.json has no event ${env.event}: site events put adds it first`);
  const { url, id } = await articleAddress(raw.url);
  const previous = await bucket.article(id);
  if (previous && previous.source !== SUPPLEMENT_SOURCE) {
    throw new SiteError(
      "invalid",
      `${url} is the article ${id} of ${previous.source} already: list it in the event's articles instead`,
    );
  }

  const item: Fetched = { raw, fetched_at: now, images: {}, image_tries: {}, thumb: null };
  const pictures = await storeImages(bucket, env.net, item, { resize: env.resize, tries: 1, log: env.log });
  const article = await writeArticle(bucket, item, {
    site: env.site,
    source: SUPPLEMENT_SOURCE,
    lang: raw.lang ?? "und",
    comments: raw.comments?.length ?? 0,
    translator: env.translator,
    previous,
    has: () => false,
    supplement: { event: env.event, adds: env.adds },
  });

  event.supplements = [
    ...event.supplements.filter((entry) => entry.article !== id),
    { article: id, source_url: url, adds: env.adds, title: article.title.t, at: article.published_at ?? article.fetched_at },
  ];
  event.updated_at = now;
  events.updated_at = now;
  await bucket.writeEvents(events);
  return { article, images: pictures.stored };
}

export async function syncSource(env: SyncEnv): Promise<SyncReport> {
  const now = env.now ?? Date.now;
  const started = now();
  const { bucket, source, site, net } = env;
  const maxImageTries = env.imageTries ?? 2;
  const path = (name: string) => `${env.dataDir}/${name}`;
  await Deno.mkdir(env.dataDir, { recursive: true });

  const state: JobState = {
    schema: STATE_SCHEMA,
    extract: null,
    validators: {},
    lengths: [],
    listed: 0,
    rejected: {},
    cleaned_at: 0,
    ...await readLocal<JobState>(path("state.json")),
  };
  const index = await readLocal<Index>(path("index.json")) ?? {};
  let queue = (await readLocal<{ items: PendingItem[] }>(path("pending.json")))?.items ?? [];
  const feed: Feed = await bucket.feed(source.id) ?? {
    schema: FEED_SCHEMA,
    source: source.id,
    status: { state: "preparing", checked_at: null, published_at: null, error: null, pending: null, login_url: null },
    items: [],
    updated_at: started,
  };
  // Articles a run that died had written and not yet listed.
  let unlisted = await readLocal<FeedItem[]>(path("unlisted.json")) ?? [];
  for (const item of unlisted) {
    feed.items = feed.items.filter((entry) => entry.id !== item.id || entry.rev > item.rev);
    if (!feed.items.some((entry) => entry.id === item.id)) feed.items.push(item);
  }
  for (const item of feed.items) index[item.id] ??= { url: item.url, at: item.fetched_at, images: [] };
  for (const [url, at] of Object.entries(state.rejected)) {
    if (started - at > REJECTED_MEMORY_MS) delete state.rejected[url];
  }

  const metrics: JobResult["metrics"] = {
    listed: 0,
    new: 0,
    published: 0,
    rejected: 0,
    images: 0,
    tokens: 0,
    ms: 0,
    pending: 0,
    pending_reason: null,
  };
  const report: SyncReport = {
    ok: false,
    summary: "",
    metrics,
    status: feed.status,
    published: [],
    rejected: [],
    requests: 0,
    not_modified: 0,
    removed: 0,
  };
  const finish = (summary: string): SyncReport => {
    metrics.tokens = env.tokens?.() ?? 0;
    metrics.ms = now() - started;
    // As the feed says it: a run that failed leaves what the run before it left.
    metrics.pending = feed.status.pending?.count ?? 0;
    metrics.pending_reason = feed.status.pending?.reason ?? null;
    report.requests = net.requests;
    report.not_modified = net.notModified;
    report.summary = `${source.id}: ${summary}`;
    report.status = feed.status;
    return report;
  };
  const writeFeed = async () => {
    feed.items = sortItems(feed.items);
    feed.updated_at = now();
    await bucket.writeFeed(feed);
    if (unlisted.length) {
      unlisted = [];
      await writeLocal(path("unlisted.json"), unlisted);
    }
  };

  // ── 1–2: extract and check ───────────────────────────────────────────────
  const knownUrls = new Set([
    ...Object.values(index).map((entry) => entry.url),
    ...Object.keys(state.rejected),
    ...queue.flatMap((item) => normalized(item.raw.url) ?? []),
  ]);
  const validators = structuredClone(state.validators);
  let checked: CheckReport;
  let nextExtractState: unknown;
  try {
    const ctx = await extractContext({
      source: {
        ...env.def,
        every_secs: source.every_secs,
        max_new: source.max_new,
        keep_days: source.keep_days,
        comments: source.comments,
      },
      dir: env.dir,
      state: state.extract,
      validators,
      limit: Math.max(0, (env.limit ?? source.max_new) - queue.length),
      known: (url) => knownUrls.has(normalized(url) ?? ""),
      net,
      log: env.log,
      browser: env.browser,
    });
    const extracted = await runExtract(env.dir, ctx);
    metrics.listed = extracted.list.length;
    metrics.new = extracted.items.length;
    checked = checkBatch({
      list: extracted.list,
      items: extracted.items,
      listedBefore: state.listed,
      notModified: net.notModified > 0,
      lengths: state.lengths,
      check: env.def.check,
    });
    report.rejected = checked.rejected;
    metrics.rejected = checked.rejected.length;
    if (checked.failure) throw new SiteError("program", checked.failure);
    nextExtractState = extracted.state ?? null;
    if (extracted.list.length) {
      state.listed = extracted.list.length;
      const listed = new Set(extracted.list.map((entry) => normalized(entry.url)));
      for (const [id, entry] of Object.entries(index)) {
        if (entry.gone && !listed.has(entry.url)) delete index[id];
      }
    }
  } catch (err) {
    const failure = failureOf(err);
    const kind = failure.kind === "auth" || failure.kind === "transient" ? failure.kind : "program";
    feed.status.error = { kind, message: failure.message, at: now() };
    if (kind === "auth") {
      feed.status.state = "needs_login";
      feed.status.login_url = source.home_url;
    } else if (kind === "program") {
      feed.status.state = "failed";
      feed.status.login_url = null;
    }
    await writeFeed();
    report.kind = kind;
    return finish(`${kind}: ${failure.message}`);
  }

  // What passed is kept on disk before anything slow starts.
  const fetchedAt = now();
  for (const raw of checked.accepted) {
    const url = normalized(raw.url);
    if (!url || queue.some((item) => normalized(item.raw.url) === url)) continue;
    queue.push({ raw, fetched_at: fetchedAt, images: {}, image_tries: {}, thumb: null, tries: 0 });
  }
  for (const entry of checked.rejected) {
    const url = normalized(entry.url);
    if (url) state.rejected[url] = fetchedAt;
  }
  state.extract = nextExtractState;
  state.validators = validators;
  state.lengths = nextLengths(state.lengths, checked.accepted);
  await writeLocal(path("pending.json"), { items: queue });
  await writeLocal(path("state.json"), state);

  // ── 3–5: images, translation, shell, article ─────────────────────────────
  const deadline = started + env.timeLimitMs;
  let stop: { reason: NonNullable<FeedStatus["pending"]>["reason"]; message: string } | null = null;
  let configError: string | null = null;
  let deferred = 0;
  let flushed = 0;
  let failuresInARow = 0;

  const publish = async (item: PendingItem): Promise<"published" | "images" | "unchanged"> => {
    const { raw } = item;
    const { url, id } = await articleAddress(raw.url);
    const previous = index[id] && !index[id].gone ? await bucket.article(id) : null;
    if (previous && sameOriginal(previous, raw)) return "unchanged";

    const pictures = await storeImages(bucket, net, item, { resize: env.resize, tries: maxImageTries, log: env.log });
    metrics.images += pictures.stored;
    if (!pictures.complete) return "images";

    const article = await writeArticle(bucket, item, {
      site,
      source: source.id,
      lang: raw.lang ?? env.def.lang,
      comments: source.comments,
      translator: env.translator,
      previous,
      has: (target) => !!index[target] && !index[target].gone,
    });

    const stored = Object.values(item.images).flatMap((image) => image && image !== "left_out" ? [image.key] : []);
    index[id] = { url, at: item.fetched_at, images: [...new Set([...stored, ...(item.thumb ? [item.thumb] : [])])] };
    const listed: FeedItem = {
      id,
      object_id: article.object_id,
      url,
      title: article.title.t,
      summary: article.summary.t,
      image: item.thumb,
      tags: article.tags,
      published_at: article.published_at,
      fetched_at: article.fetched_at,
      rev: article.rev,
    };
    feed.items = feed.items.filter((entry) => entry.id !== id);
    feed.items.push(listed);
    unlisted = [...unlisted.filter((entry) => entry.id !== id), listed];
    report.published.push({
      id,
      url,
      title: article.title.o,
      rev: article.rev,
      chars: bodyOf(raw).length,
      blocks: article.blocks.length,
      images: stored.length,
      missing_images: Object.values(item.images).filter((image) => image === null).length,
      comments: article.comments.length,
      tags: article.tags,
      bytes: new TextEncoder().encode(JSON.stringify(article)).length,
    });
    return "published";
  };

  const left: PendingItem[] = [];
  const interimStatus = (remaining: number) => {
    feed.status = {
      state: "ok",
      checked_at: fetchedAt,
      published_at: metrics.published ? now() : feed.status.published_at,
      error: null,
      pending: remaining
        ? { count: remaining, reason: "time", message: `${remaining} more article(s) of this sync are on their way` }
        : null,
      login_url: null,
    };
  };
  while (queue.length) {
    const item = queue[0];
    if (!stop && now() >= deadline) {
      stop = { reason: "time", message: "the sync ran out of time; the rest is published by the next one" };
    }
    if (stop) {
      left.push(...queue);
      queue = [];
      break;
    }
    try {
      const outcome = await publish(item);
      failuresInARow = 0;
      if (outcome === "images") {
        left.push(item);
      } else if (outcome === "published") {
        metrics.published += 1;
      }
    } catch (err) {
      if (!(err instanceof SiteError)) throw err;
      if (err.kind === "budget") {
        stop = { reason: "translate_budget", message: err.message };
        continue;
      }
      if (err.kind === "config") {
        configError = err.message;
        stop = { reason: "translate_config", message: err.message };
        continue;
      }
      item.tries += 1;
      const dropped = err.kind === "invalid" || item.tries >= TRANSLATE_TRIES;
      env.log(`${item.raw.url}: ${err.message}${dropped ? " — not published" : " — left for the next sync"}`);
      if (dropped) {
        report.rejected.push({ url: item.raw.url, title: item.raw.title, reason: err.message });
        metrics.rejected += 1;
        const url = normalized(item.raw.url);
        if (url) state.rejected[url] = now();
      } else {
        deferred += 1;
        left.push(item);
      }
      failuresInARow += 1;
      if (failuresInARow >= TRANSLATE_TRIES) {
        stop = {
          reason: "translate_failed",
          message: `translation keeps failing (${err.message}); the rest waits for the next sync`,
        };
      }
    }
    queue.shift();
    await writeLocal(path("index.json"), index);
    await writeLocal(path("unlisted.json"), unlisted);
    await writeLocal(path("pending.json"), { items: [...left, ...queue] });
    if (metrics.published - flushed >= FEED_EVERY && queue.length) {
      flushed = metrics.published;
      interimStatus(left.length + queue.length);
      await writeFeed();
    }
  }
  await writeLocal(path("pending.json"), { items: left });

  // ── 6: clean-up, at most once a day ──────────────────────────────────────
  const doomed: { id: string; keys: string[] }[] = [];
  if (env.cleanup !== false && started - state.cleaned_at >= DAY_MS) {
    try {
      const kept = new Set<string>(Object.keys((await bucket.reading())?.articles ?? {}));
      for (const event of (await bucket.events())?.events ?? []) {
        for (const entry of event.articles) kept.add(entry.id);
        for (const entry of event.supplements) kept.add(entry.article);
      }
      const expired = Object.entries(index)
        .filter(([id, entry]) =>
          !entry.gone && started - entry.at > (kept.has(id) ? KEEP_READ_DAYS : source.keep_days) * DAY_MS
        )
        .sort((a, b) => a[1].at - b[1].at);
      const plan: typeof expired = [];
      let count = 0;
      for (const candidate of expired) {
        count += 2 + candidate[1].images.length;
        if (count > MAX_DELETES) break;
        plan.push(candidate);
      }
      const planned = new Set(plan.map(([id]) => id));
      const inUse = new Set(Object.entries(index).flatMap(([id, entry]) => planned.has(id) ? [] : entry.images));
      for (const [id, entry] of plan) {
        const images = entry.images.filter((key) => !inUse.has(key));
        for (const key of images) inUse.add(key);
        doomed.push({ id, keys: [articleKey(id), markdownKey(id), ...images] });
      }
      state.cleaned_at = started;
    } catch (err) {
      env.log(`clean-up skipped: ${(err as Error).message}`);
    }
  }
  const doomedIds = new Set(doomed.map((entry) => entry.id));
  feed.items = feed.items.filter((item) => !doomedIds.has(item.id));

  // ── the feed, last ───────────────────────────────────────────────────────
  interimStatus(0);
  if (left.length) {
    const reason = stop?.reason ?? (deferred ? "translate_failed" : "images");
    feed.status.pending = {
      count: left.length,
      reason,
      message: stop?.message ??
        (deferred
          ? "the translation of some articles failed; they are tried again by the next sync"
          : "some images could not be fetched; they are tried once more by the next sync"),
    };
  }
  feed.status.checked_at = now();
  if (configError) feed.status.error = { kind: "config", message: configError, at: now() };
  await writeFeed();

  for (const { id, keys } of doomed) {
    for (const key of keys) await bucket.remove(key);
    index[id] = { url: index[id].url, at: index[id].at, images: [], gone: true };
    await Deno.remove(path(`tr/${id}.json`)).catch(() => {});
    report.removed += keys.length;
  }
  await writeLocal(path("index.json"), index);
  await writeLocal(path("state.json"), state);

  report.ok = true;
  if (stop?.reason === "translate_budget" || stop?.reason === "translate_config") report.kind = "budget";
  const parts = [`${metrics.listed} listed`, `${metrics.new} new`, `${metrics.published} published`];
  if (metrics.rejected) parts.push(`${metrics.rejected} rejected`);
  if (left.length) parts.push(`${left.length} waiting (${feed.status.pending!.reason}: ${feed.status.pending!.message})`);
  if (report.removed) parts.push(`${report.removed} old object(s) removed`);
  return finish(parts.join(", "));
}
