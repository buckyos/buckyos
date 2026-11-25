/* The user's reflow bucket `user/control-panel/<user>/reflow` as the App addresses it: the rules for an address
 * (Agent Browser Protocol §5.1: normalized URL, site, page), the match rules a source of the personal site states
 * (Personal Site Protocol §3.1), and the page tidied for one address — a fallback page, `<site>/fallback/<page>.html`
 * with its `.json`. The address and match rules are the ones of the `reflow` and `site` commands and of the Agent
 * Service; all of them are checked against doc/opendan/protocol/fixtures/reflow_vectors.json. */

export const REFLOW_APP = 'control-panel'
export const REFLOW_BUCKET = 'reflow'

export class ReflowError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ReflowError'
  }
}

// ── §5.1: site, normalized URL, page ──

export interface ReflowAddress {
  /** The normalized URL. */
  url: string
  /** Its host name: lower case, no port, no trailing dot. */
  host: string
  /** Its path, as the normalized URL spells it (percent-encoded). */
  path: string
  site: string
  page: string
}

/** `www.example.com` → `www-example-com`; a name without a dash gets `-host`. */
export function siteOfHost(host: string): string {
  const name = host.toLowerCase().replace(/\.$/, '')
  if (!/^[a-z0-9._-]+$/.test(name)) throw new ReflowError(`unsupported host name: ${host}`)
  const site = name.replaceAll('.', '-')
  return site.includes('-') ? site : `${site}-host`
}

/** WHATWG serialization without fragment and credentials; http(s) only. */
export function normalizeUrl(input: string): { url: string; host: string; path: string } {
  let url: URL
  try {
    url = new URL(input.trim())
  } catch {
    throw new ReflowError(`not a URL: ${input}`)
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new ReflowError(`only http(s) pages are reflowed: ${input}`)
  url.hash = ''
  url.username = ''
  url.password = ''
  return { url: url.href, host: url.hostname.replace(/\.$/, ''), path: url.pathname }
}

/** The first 16 hex characters of the SHA-256 of the normalized URL. */
export async function pageOf(normalizedUrl: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(normalizedUrl))
  return Array.from(new Uint8Array(digest).slice(0, 8), (byte) => byte.toString(16).padStart(2, '0')).join('')
}

export async function reflowAddress(input: string): Promise<ReflowAddress> {
  const { url, host, path } = normalizeUrl(input)
  return { url, host, path, site: siteOfHost(host), page: await pageOf(url) }
}

// ── Match rules ──

export interface MatchRule {
  host: string
  path?: string
}

/** `*` matches any run of characters, `/` included; the rest is literal. */
export function pathMatches(pattern: string, path: string): boolean {
  const parts = pattern.split('*')
  if (parts.length === 1) return pattern === path
  if (!path.startsWith(parts[0])) return false
  let at = parts[0].length
  for (const part of parts.slice(1, -1)) {
    const found = path.indexOf(part, at)
    if (found < 0) return false
    at = found + part.length
  }
  const last = parts[parts.length - 1]
  return path.length - at >= last.length && path.endsWith(last)
}

export function rulesMatch(rules: MatchRule[], address: { host: string; path: string }): boolean {
  return rules.some((rule) => rule.host === address.host && pathMatches(rule.path ?? '/*', address.path))
}

// ── The bucket ──

/** The part of the websdk `BucketClient` the App uses. */
export interface LibraryStore {
  listObjects(bucketId: string, req: { prefix?: string; delimiter?: string; start_after?: string }): Promise<{
    items: { key: string; size: number }[]
    common_prefixes: string[]
    next_start_after?: string
  }>
  fetchObject(bucketId: string, key: string): Promise<Response>
  putBytes(bucketId: string, key: string, body: Uint8Array | string, options?: { contentType?: string; if_match?: string }): Promise<unknown>
  delete(bucketId: string, key: string): Promise<unknown>
}

/** A user's reflow bucket: where it is and who is looking at it. */
export interface Library {
  store: LibraryStore
  bucketId: string
  userId: string
}

// ── The page tidied for one address ──

export interface Made {
  made_at: number
  made_by: string
  source_url: string
}

/** `browser.reflow_state` (§5.3): what the bucket has for one URL. */
export interface ReflowState {
  url: string
  site: string
  page: string
  fallback: Made | null
  /** The page to show, relative to `https://reflow.<zone>/`. */
  view: string | null
}

function madeOf(value: unknown): Made | null {
  const made = value as Partial<Made> | null
  if (!made || typeof made !== 'object' || typeof made.made_at !== 'number' || typeof made.made_by !== 'string' || typeof made.source_url !== 'string') return null
  return { made_at: made.made_at, made_by: made.made_by, source_url: made.source_url }
}

/** The same answer read from the bucket itself, for when the Agent's service does not give it. */
export async function libraryState({ store, bucketId }: Library, address: ReflowAddress): Promise<ReflowState> {
  const { url, site, page } = address
  let fallback: Made | null = null
  try {
    fallback = madeOf(JSON.parse(await (await store.fetchObject(bucketId, `${site}/fallback/${page}.json`)).text()))
  } catch (error) {
    if (!(error instanceof SyntaxError) && (error as { code?: unknown } | null)?.code !== 'no_such_key') throw error
  }
  return { url, site, page, fallback, view: fallback ? `${site}/fallback/${page}.html` : null }
}
