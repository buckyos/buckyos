/* The personal site as the App reads it: `_data/site.json`, a source's feed, an article and the reading summary of
 * the user's reflow bucket (Personal Site Protocol §2–3), read with the owner's session through the bucket client.
 * What the App writes is the owner's own: into `site.json` (with `If-Match`) the answer to an upgrade request, a
 * source paused or resumed, the reading records switched on or off; and the reading records cleared.
 * Samples: doc/opendan/protocol/fixtures/site/. */

import { normalizeUrl, pageOf, rulesMatch, type Library, type MatchRule } from '../reflow/library'

export const SITE_KEY = '_data/site.json'
export const READING_KEY = '_data/reading.json'
/** What the site's pages reported, before the hourly job sums it up. */
export const READ_PREFIX = '_read/'
export const feedKey = (source: string) => `_data/feed/${source}.json`
export const articleKey = (article: string) => `_data/a/${article}.json`

export type SourceState = 'preparing' | 'active' | 'paused' | 'failed'
export type FeedState = 'preparing' | 'ok' | 'failed' | 'needs_login'
/** What a source shows as, composed from `site.json`, its upgrade requests and its feed (§3.1). */
export type DisplayState = FeedState | 'paused' | 'needs_upgrade'

export interface SiteSource {
  id: string
  title: string
  home_url: string
  match: MatchRule[]
  /** The Goal job that syncs it (`src-<source>`). */
  job: string
  state: SourceState
  note: string | null
}

export interface Upgrade {
  id: string
  source?: string
  title: string
  summary: string
  /** The path of the request's write-up in the maintainer's workspace. */
  file: string
  goal_id: string
  opened_at: number
  state: 'open' | 'accepted' | 'dismissed' | 'closed'
  decided_at?: number
}

/** `site.json`, as far as the App looks at it; the rest is kept as it is when the App writes the file. */
export interface Site {
  title: string
  owner_did: string
  sources: SiteSource[]
  upgrades: Upgrade[]
  /** Whether the site's pages report what is read (§3.6). */
  reading: boolean
  maintainer: { agent: string; goal_id: string } | null
}

export interface FeedStatus {
  state: FeedState
  /** The last check that passed, new articles or not. */
  checked_at: number | null
  error: { kind: string; message: string; at: number } | null
  /** Articles that were fetched and are not published yet. */
  pending: { count: number; reason: string; message: string } | null
  login_url: string | null
}

export interface Feed {
  status: FeedStatus
  updated_at: number
}

/** What the shell shows about the article being read. */
export interface ArticleRef {
  id: string
  source: string
  url: string
}

export interface SiteFile {
  site: Site
  etag: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

const text = (value: unknown): string => (typeof value === 'string' ? value : '')
const moment = (value: unknown): number | null => (typeof value === 'number' && Number.isFinite(value) ? value : null)

function parseSite(value: unknown): Site | null {
  if (!isRecord(value) || value.schema !== 'buckyos.site/1') return null
  const sources = (Array.isArray(value.sources) ? value.sources : []).filter(isRecord).filter((item) => typeof item.id === 'string' && item.id).map((item): SiteSource => ({
    id: item.id as string,
    title: text(item.title) || (item.id as string),
    home_url: text(item.home_url),
    match: (Array.isArray(item.match) ? item.match : []).filter(isRecord).filter((rule) => typeof rule.host === 'string').map((rule) => ({ host: rule.host as string, ...(typeof rule.path === 'string' ? { path: rule.path } : {}) })),
    job: text(item.job),
    state: item.state === 'preparing' || item.state === 'paused' || item.state === 'failed' ? item.state : 'active',
    note: text(item.note) || null,
  }))
  const upgrades = (Array.isArray(value.upgrades) ? value.upgrades : []).filter(isRecord).filter((item) => typeof item.id === 'string' && item.id).map((item): Upgrade => ({
    id: item.id as string,
    ...(typeof item.source === 'string' && item.source ? { source: item.source } : {}),
    title: text(item.title),
    summary: text(item.summary),
    file: text(item.file),
    goal_id: text(item.goal_id),
    opened_at: moment(item.opened_at) ?? 0,
    state: item.state === 'accepted' || item.state === 'dismissed' || item.state === 'closed' ? item.state : 'open',
    ...(moment(item.decided_at) === null ? {} : { decided_at: item.decided_at as number }),
  }))
  const maintainer = isRecord(value.maintainer) && typeof value.maintainer.agent === 'string' ? { agent: value.maintainer.agent, goal_id: text(value.maintainer.goal_id) } : null
  const reading = !isRecord(value.reading) || value.reading.enabled !== false
  return { title: text(value.title), owner_did: text(value.owner_did), sources, upgrades, reading, maintainer }
}

function parseFeed(value: unknown): Feed | null {
  if (!isRecord(value) || !isRecord(value.status)) return null
  const status = value.status
  const error = isRecord(status.error) ? { kind: text(status.error.kind), message: text(status.error.message), at: moment(status.error.at) ?? 0 } : null
  const pending = isRecord(status.pending) && typeof status.pending.count === 'number' ? { count: status.pending.count, reason: text(status.pending.reason), message: text(status.pending.message) } : null
  return {
    status: {
      state: status.state === 'ok' || status.state === 'failed' || status.state === 'needs_login' ? status.state : 'preparing',
      checked_at: moment(status.checked_at),
      error,
      pending,
      login_url: text(status.login_url) || null,
    },
    updated_at: moment(value.updated_at) ?? 0,
  }
}

const errorCode = (error: unknown): unknown => (error as { code?: unknown } | null)?.code

/** The object at `key` with its ETag; undefined when there is none (or no bucket yet: a Zone activated before it was declared). */
async function fetchJson({ store, bucketId }: Library, key: string): Promise<{ value: unknown; etag: string } | undefined> {
  let response: Response
  try {
    response = await store.fetchObject(bucketId, key)
  } catch (error) {
    if (errorCode(error) === 'no_such_key' || errorCode(error) === 'bucket_not_found') return undefined
    throw error
  }
  const body = await response.text()
  try {
    return { value: JSON.parse(body), etag: response.headers.get('ETag') ?? '' }
  } catch {
    return { value: undefined, etag: response.headers.get('ETag') ?? '' }
  }
}

/** `site.json`; null when the user has no site yet (or what is there is not one). */
export async function readSite(library: Library): Promise<SiteFile | null> {
  const file = await fetchJson(library, SITE_KEY)
  const site = file && parseSite(file.value)
  return file && site ? { site, etag: file.etag } : null
}

/** The feed of a source; null before its first sync wrote one. */
export async function readFeed(library: Library, source: string): Promise<Feed | null> {
  return parseFeed((await fetchJson(library, feedKey(source)))?.value)
}

/** The article with this id, if it was collected. */
export async function readArticle(library: Library, id: string): Promise<ArticleRef | null> {
  if (!/^[0-9a-f]{16}$/.test(id)) return null
  const value = (await fetchJson(library, articleKey(id)))?.value
  return isRecord(value) && typeof value.url === 'string' ? { id, source: text(value.source), url: value.url } : null
}

/** The source an address belongs to: the first whose `match` covers it (§3.1). */
export function sourceOf(site: Site, url: string): SiteSource | null {
  try {
    const address = normalizeUrl(url)
    return site.sources.find((source) => rulesMatch(source.match, address)) ?? null
  } catch {
    return null
  }
}

/** The page of the site that shows an address: its article when it was collected, else the page of its source. */
export async function locate(library: Library, site: Site, url: string): Promise<string | null> {
  let normalized: string
  try {
    normalized = normalizeUrl(url).url
  } catch {
    return null
  }
  const article = await readArticle(library, await pageOf(normalized))
  if (article) return `a=${article.id}`
  const source = sourceOf(site, normalized)
  return source ? `s=${source.id}` : null
}

export const openUpgrades = (site: Site, source: string | null): Upgrade[] =>
  site.upgrades.filter((upgrade) => upgrade.state === 'open' && (upgrade.source ?? null) === source)

/** §3.1: paused, then an open upgrade request, then `preparing` / `failed` of `site.json`, then the feed's own state. */
export function displayState(site: Site, source: SiteSource, feed: Feed | null): DisplayState {
  if (source.state === 'paused') return 'paused'
  if (openUpgrades(site, source.id).length > 0) return 'needs_upgrade'
  if (source.state === 'preparing' || source.state === 'failed') return source.state
  return feed?.status.state ?? 'preparing'
}

const WRITE_ATTEMPTS = 3

/**
 * Changes `site.json` as it is now: `change` edits the file's own object and says whether there is anything to write.
 * Written with `If-Match`; read again when somebody else wrote in between. False when nothing was written.
 */
async function updateSite(library: Library, change: (site: Record<string, unknown>) => boolean, now: number = Date.now()): Promise<boolean> {
  const { store, bucketId } = library
  for (let attempt = 1; ; attempt++) {
    const file = await fetchJson(library, SITE_KEY)
    if (!file || !isRecord(file.value) || !change(file.value)) return false
    const next = { ...file.value, updated_at: now, updated_by: text(file.value.owner_did) || file.value.updated_by }
    try {
      await store.putBytes(bucketId, SITE_KEY, JSON.stringify(next, null, 2), { contentType: 'application/json', if_match: file.etag })
      return true
    } catch (error) {
      if (errorCode(error) !== 'precondition_failed' || attempt === WRITE_ATTEMPTS) throw error
    }
  }
}

const entryOf = (list: unknown, id: string): Record<string, unknown> | undefined =>
  (Array.isArray(list) ? list : []).find((item: unknown): item is Record<string, unknown> => isRecord(item) && item.id === id)

/** The owner's answer to an upgrade request, with the time. False when the request is not open any more. */
export function decideUpgrade(library: Library, id: string, decision: 'accepted' | 'dismissed', now: number = Date.now()): Promise<boolean> {
  return updateSite(library, (site) => {
    const upgrade = entryOf(site.upgrades, id)
    if (upgrade?.state !== 'open') return false
    upgrade.state = decision
    upgrade.decided_at = now
    return true
  }, now)
}

/** A source paused by the owner, or resumed. False when it is not there, or cannot change that way (being prepared, not added). */
export function setSourcePaused(library: Library, id: string, paused: boolean): Promise<boolean> {
  return updateSite(library, (site) => {
    const source = entryOf(site.sources, id)
    if (source?.state !== (paused ? 'active' : 'paused')) return false
    source.state = paused ? 'paused' : 'active'
    return true
  })
}

/** The reading records switched on or off (§3.1 `reading.enabled`). */
export function setReadingEnabled(library: Library, enabled: boolean): Promise<boolean> {
  return updateSite(library, (site) => {
    site.reading = { ...(isRecord(site.reading) ? site.reading : {}), enabled }
    return true
  })
}

// ── The reading records (§3.6) ──

export interface ReadingTotal {
  /** A source's id, or a tag. */
  name: string
  opens: number
  dwell_ms: number
}

/** `_data/reading.json`, as far as the App shows it: what was read most, by source and by tag. */
export interface ReadingSummary {
  updated_at: number
  since: number
  articles: number
  sources: ReadingTotal[]
  tags: ReadingTotal[]
}

function totals(value: unknown): ReadingTotal[] {
  if (!isRecord(value)) return []
  return Object.entries(value).filter((entry): entry is [string, Record<string, unknown>] => isRecord(entry[1]))
    .map(([name, total]) => ({ name, opens: moment(total.opens) ?? 0, dwell_ms: moment(total.dwell_ms) ?? 0 }))
    .sort((a, b) => b.dwell_ms - a.dwell_ms || b.opens - a.opens)
}

/** The reading summary; null before the hourly job wrote one. */
export async function readReading(library: Library): Promise<ReadingSummary | null> {
  const value = (await fetchJson(library, READING_KEY))?.value
  if (!isRecord(value)) return null
  return { updated_at: moment(value.updated_at) ?? 0, since: moment(value.since) ?? 0, articles: isRecord(value.articles) ? Object.keys(value.articles).length : 0, sources: totals(value.sources), tags: totals(value.tags) }
}

/** Deletes every reading record: what the pages reported (`_read/**`) and the summary. Resolves to how many objects went. */
export async function clearReading({ store, bucketId }: Library): Promise<number> {
  const keys: string[] = []
  let startAfter: string | undefined
  do {
    const page = await store.listObjects(bucketId, { prefix: READ_PREFIX, ...(startAfter ? { start_after: startAfter } : {}) })
    keys.push(...page.items.map((item) => item.key))
    startAfter = page.next_start_after
  } while (startAfter)
  for (const key of [...keys, READING_KEY]) {
    try {
      await store.delete(bucketId, key)
    } catch (error) {
      if (errorCode(error) !== 'no_such_key') throw error
    }
  }
  return keys.length
}
