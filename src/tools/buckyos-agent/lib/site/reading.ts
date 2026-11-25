// The reading summary: Personal Site Protocol §3.6. `_read/<yyyymmdd>/<ulid>.json`
// are the batches the pages reported; `site reading --compact` folds the new
// ones into `_data/reading.json`.

import { isReadRecord, SiteBucket } from "./store.ts";
import { articleKey, READ_PREFIX, READING_SCHEMA } from "./types.ts";
import type { Reading } from "./types.ts";

const DAY_MS = 86_400_000;
const KEEP_MS = 90 * DAY_MS;
const RECENT_MS = 7 * DAY_MS;
const PROFILE_TOP = 3;
const STATE_FILE = "reading_state.json";

/**
 * What the compacting job keeps in its `data/`: how far it got, every open of
 * the last 90 days (article → `opened_at` → [dwell_ms, max_scroll]) and the
 * tags of the articles it met. reading.json is made of the opens anew each
 * time, so a page that reports the same open again only adds to it.
 */
interface CompactState {
  after: string;
  opens: Record<string, { source: string; at: Record<string, [number, number]> }>;
  tags: Record<string, string[]>;
}

/** The `metrics` of the job's result line (Personal Site Protocol §5). */
export interface ReadingMetrics {
  records: number;
  opens_7d: number;
  dwell_ms_7d: number;
  articles_7d: number;
  /** `tags:a,b,c;sources:x,y,z`: the three most read of the last 7 days each, by time spent. */
  profile: string;
}

export interface CompactReport {
  batches: number;
  articles: number;
  metrics: ReadingMetrics;
}

// Most time first; the same time in the order of the names, so that the same reading gives the same line.
function top(dwell: Map<string, number>): string[] {
  return [...dwell].filter(([, ms]) => ms > 0)
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
    .slice(0, PROFILE_TOP).map(([name]) => name);
}

export async function compactReading(bucket: SiteBucket, dataDir: string, now: number = Date.now()): Promise<CompactReport> {
  const statePath = `${dataDir}/${STATE_FILE}`;
  // A job that starts over knows of no open: it folds every batch again.
  let state: CompactState = { after: "", opens: {}, tags: {} };
  try {
    const kept = JSON.parse(await Deno.readTextFile(statePath)) as CompactState;
    if (typeof kept.after === "string" && kept.opens && kept.tags) state = kept;
  } catch {
    // no state yet
  }

  const batches = (await bucket.keys(READ_PREFIX)).filter((key) => key > state.after).sort();
  let records = 0;
  for (const key of batches) {
    const found = await bucket.readJson(key).catch(() => null);
    for (const record of Array.isArray(found?.value) ? found.value : []) {
      if (!isReadRecord(record)) continue;
      records += 1;
      const article = state.opens[record.article] ??= { source: record.source, at: {} };
      const open = article.at[record.opened_at] ??= [0, 0];
      open[0] += record.dwell_ms;
      open[1] = Math.max(open[1], record.max_scroll);
    }
    state.after = key;
  }

  const since = now - KEEP_MS;
  const recent = now - RECENT_MS;
  const reading: Reading = { schema: READING_SCHEMA, updated_at: now, since, articles: {}, sources: {}, tags: {} };
  const metrics: ReadingMetrics = { records, opens_7d: 0, dwell_ms_7d: 0, articles_7d: 0, profile: "" };
  const tagTime = new Map<string, number>();
  const sourceTime = new Map<string, number>();
  for (const [id, article] of Object.entries(state.opens)) {
    for (const at of Object.keys(article.at)) {
      if (Number(at) < since) delete article.at[at];
    }
    const opens = Object.entries(article.at);
    if (!opens.length) {
      delete state.opens[id];
      delete state.tags[id];
      continue;
    }
    if (!state.tags[id]) {
      const stored = await bucket.readJson(articleKey(id)).catch(() => null);
      const tags = (stored?.value as { tags?: unknown } | undefined)?.tags;
      state.tags[id] = Array.isArray(tags) ? tags.filter((tag) => typeof tag === "string") : [];
    }
    const entry = reading.articles[id] = { source: article.source, opens: 0, dwell_ms: 0, max_scroll: 0, last_opened_at: 0 };
    let lately = 0;
    for (const [at, [dwell, scroll]] of opens) {
      entry.opens += 1;
      entry.dwell_ms += dwell;
      entry.max_scroll = Math.max(entry.max_scroll, scroll);
      entry.last_opened_at = Math.max(entry.last_opened_at, Number(at));
      if (Number(at) < recent) continue;
      lately += 1;
      metrics.opens_7d += 1;
      metrics.dwell_ms_7d += dwell;
      sourceTime.set(article.source, (sourceTime.get(article.source) ?? 0) + dwell);
      for (const tag of state.tags[id]) tagTime.set(tag, (tagTime.get(tag) ?? 0) + dwell);
    }
    if (lately) metrics.articles_7d += 1;
    const source = reading.sources[article.source] ??= { opens: 0, dwell_ms: 0, last_opened_at: 0 };
    source.opens += entry.opens;
    source.dwell_ms += entry.dwell_ms;
    source.last_opened_at = Math.max(source.last_opened_at, entry.last_opened_at);
    for (const name of state.tags[id]) {
      const tag = reading.tags[name] ??= { opens: 0, dwell_ms: 0 };
      tag.opens += entry.opens;
      tag.dwell_ms += entry.dwell_ms;
    }
  }
  metrics.profile = `tags:${top(tagTime).join(",")};sources:${top(sourceTime).join(",")}`;

  await bucket.writeReading(reading);
  await Deno.mkdir(dataDir, { recursive: true });
  await Deno.writeTextFile(statePath, JSON.stringify(state));
  return { batches: batches.length, articles: Object.keys(reading.articles).length, metrics };
}

/** The articles opened in the last `days`, latest first, with what the summary has for sources and tags. */
export function recentReading(reading: Reading, days: number, now: number = Date.now()) {
  const since = now - days * DAY_MS;
  const articles = Object.entries(reading.articles)
    .filter(([, entry]) => entry.last_opened_at >= since)
    .sort((a, b) => b[1].last_opened_at - a[1].last_opened_at)
    .map(([id, entry]) => ({ id, ...entry }));
  return { days, updated_at: reading.updated_at, articles, sources: reading.sources, tags: reading.tags };
}
