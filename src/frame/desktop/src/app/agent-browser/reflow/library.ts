/* The reflow library as the App reads and writes it: the user's bucket `user/control-panel/<user>/reflow`
 * (Agent Browser Protocol §5.1–5.2). The address and package rules are the ones of the `reflow` command
 * (src/tools/buckyos-agent/lib/reflow.ts) and of the Agent Service; all of them are checked against
 * doc/opendan/protocol/fixtures/reflow_vectors.json. */

export const REFLOW_APP = 'control-panel'
export const REFLOW_BUCKET = 'reflow'
export const MANIFEST_SCHEMA = 'buckyos.reflow/1'
export const RESULT_SCHEMA = 'buckyos.reflow.result/1'
export const FALLBACK_SCHEMA = 'buckyos.reflow.fallback/1'
export const PREFERENCE_SCHEMA = 'buckyos.reflow.preference/1'
export const MANIFEST_FILE = 'manifest.json'
export const REFRESH_SCRIPT = 'refresh.mjs'
export const INDEX_FILE = 'index.html'
/** The user's preference for a site that has no package yet; a package carries it in its manifest. */
export const PREFERENCE_FILE = 'preference.json'
/** Kept by the library beside a package; never part of one. */
export const DATA_DIRS = ['results', 'fallback']
export const MAX_REFRESH_TIMEOUT_S = 600
/** `PUT /bucket/v1/...` takes one request body up to this size. */
export const MAX_FILE_BYTES = 32 * 1024 * 1024
export const MAX_PACKAGE_FILES = 500

export class ReflowError extends Error {
  readonly problems: string[]
  constructor(message: string, problems: string[] = []) {
    super(message)
    this.name = 'ReflowError'
    this.problems = problems
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

export function isSite(value: string): boolean {
  return /^[a-z0-9_-]+$/.test(value) && value.includes('-')
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

// ── §5.2: the package ──

export interface MatchRule {
  host: string
  path?: string
}

export interface Author {
  kind: 'agent' | 'user'
  id: string
}

export interface Manifest {
  schema: string
  site: string
  match: MatchRule[]
  preference?: string
  author: Author
  updated_at: number
  refresh?: { script: string; timeout_s: number }
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

export function packageMatches(rules: MatchRule[], address: { host: string; path: string }): boolean {
  return rules.some((rule) => rule.host === address.host && pathMatches(rule.path ?? '/*', address.path))
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

/** What is wrong with a manifest kept at `<site>/manifest.json`; empty when it is valid. */
export function manifestProblems(value: unknown, site: string): string[] {
  if (!isRecord(value)) return ['manifest.json is not a JSON object']
  const problems: string[] = []
  if (value.schema !== MANIFEST_SCHEMA) problems.push(`schema must be "${MANIFEST_SCHEMA}"`)
  if (value.site !== site) problems.push(`site must be "${site}"`)
  if (!Array.isArray(value.match) || value.match.length === 0) {
    problems.push('match must list at least one { host, path }')
  } else {
    value.match.forEach((rule: unknown, index: number) => {
      const at = `match[${index}]`
      if (!isRecord(rule) || typeof rule.host !== 'string') {
        problems.push(`${at}.host must be a host name`)
        return
      }
      let ruleSite: string | undefined
      try {
        ruleSite = siteOfHost(rule.host)
      } catch {
        // reported below
      }
      if (ruleSite !== site || rule.host !== rule.host.toLowerCase() || rule.host.endsWith('.')) {
        problems.push(`${at}.host "${rule.host}" is not the lower-case host name of site ${site}`)
      }
      if (rule.path !== undefined && (typeof rule.path !== 'string' || !rule.path.startsWith('/'))) {
        problems.push(`${at}.path must be a path pattern starting with "/"`)
      }
    })
  }
  if (value.preference !== undefined && typeof value.preference !== 'string') problems.push('preference must be text')
  const author = value.author
  if (!isRecord(author) || (author.kind !== 'agent' && author.kind !== 'user') || typeof author.id !== 'string' || !author.id) {
    problems.push('author must be { "kind": "agent" | "user", "id": "<who>" }')
  }
  if (!Number.isSafeInteger(value.updated_at) || (value.updated_at as number) <= 0) problems.push('updated_at must be a millisecond timestamp')
  if (value.refresh !== undefined) {
    const refresh = value.refresh
    if (!isRecord(refresh) || refresh.script !== REFRESH_SCRIPT) problems.push(`refresh.script must be "${REFRESH_SCRIPT}"`)
    const timeout = isRecord(refresh) ? refresh.timeout_s : undefined
    if (!Number.isSafeInteger(timeout) || (timeout as number) < 1 || (timeout as number) > MAX_REFRESH_TIMEOUT_S) {
      problems.push(`refresh.timeout_s must be 1 to ${MAX_REFRESH_TIMEOUT_S} seconds`)
    }
  }
  return problems
}

/** What is wrong with a package (its files, relative to its directory, and its manifest). */
export function packageProblems(files: string[], manifest: unknown, site: string): string[] {
  const problems: string[] = []
  if (!files.includes(MANIFEST_FILE)) problems.push(`${MANIFEST_FILE} is missing`)
  else problems.push(...manifestProblems(manifest, site))
  if (!files.includes(REFRESH_SCRIPT) && !files.includes(INDEX_FILE)) problems.push(`a package has ${REFRESH_SCRIPT}, ${INDEX_FILE} or both`)
  if (isRecord(manifest) && manifest.refresh !== undefined && !files.includes(REFRESH_SCRIPT)) {
    problems.push(`the manifest declares refresh but ${REFRESH_SCRIPT} is missing`)
  }
  for (const dir of DATA_DIRS) {
    if (files.some((file) => file.startsWith(`${dir}/`))) problems.push(`${dir}/ holds the library's data and is not part of a package`)
  }
  if (files.includes(PREFERENCE_FILE)) {
    problems.push(`${PREFERENCE_FILE} is where the library keeps the user's preference for a site without a package and is not part of one: state the preference in ${MANIFEST_FILE}`)
  }
  if (files.length > MAX_PACKAGE_FILES) problems.push(`a package has at most ${MAX_PACKAGE_FILES} files, this one has ${files.length}`)
  return problems
}

/** The type a package file is served with on `reflow.<zone>`. */
export function contentTypeOf(name: string): string {
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : ''
  switch (ext) {
    case 'html':
    case 'htm':
      return 'text/html; charset=utf-8'
    case 'js':
    case 'mjs':
      return 'text/javascript; charset=utf-8'
    case 'css':
      return 'text/css; charset=utf-8'
    case 'json':
    case 'map':
      return 'application/json'
    case 'txt':
    case 'md':
      return 'text/plain; charset=utf-8'
    case 'svg':
      return 'image/svg+xml'
    case 'png':
      return 'image/png'
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg'
    case 'gif':
      return 'image/gif'
    case 'webp':
      return 'image/webp'
    case 'avif':
      return 'image/avif'
    case 'ico':
      return 'image/x-icon'
    case 'woff':
      return 'font/woff'
    case 'woff2':
      return 'font/woff2'
    case 'ttf':
      return 'font/ttf'
    case 'wasm':
      return 'application/wasm'
    default:
      return 'application/octet-stream'
  }
}

// ── The library in the bucket ──

/** The part of the websdk `BucketClient` the library uses. */
export interface LibraryStore {
  listObjects(bucketId: string, req: { prefix?: string; delimiter?: string; start_after?: string }): Promise<{
    items: { key: string; size: number }[]
    common_prefixes: string[]
    next_start_after?: string
  }>
  fetchObject(bucketId: string, key: string): Promise<Response>
  putBytes(bucketId: string, key: string, body: Uint8Array | string, options?: { contentType?: string }): Promise<unknown>
  delete(bucketId: string, key: string): Promise<unknown>
}

/** A user's reflow library: where it is and who is looking at it. */
export interface Library {
  store: LibraryStore
  bucketId: string
  userId: string
}

async function readText({ store, bucketId }: Library, key: string): Promise<string | undefined> {
  try {
    return await (await store.fetchObject(bucketId, key)).text()
  } catch (error) {
    if ((error as { code?: unknown } | null)?.code === 'no_such_key') return undefined
    throw error
  }
}

async function readJson(library: Library, key: string): Promise<unknown> {
  const text = await readText(library, key)
  if (text === undefined) return undefined
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

async function listObjects({ store, bucketId }: Library, prefix: string): Promise<{ key: string; size: number }[]> {
  const objects: { key: string; size: number }[] = []
  let startAfter: string | undefined
  do {
    const page = await store.listObjects(bucketId, { prefix, ...(startAfter ? { start_after: startAfter } : {}) })
    objects.push(...page.items.map((item) => ({ key: item.key, size: item.size })))
    startAfter = page.next_start_after
  } while (startAfter)
  return objects
}

function isDataKey(site: string, key: string): boolean {
  return DATA_DIRS.some((dir) => key.startsWith(`${site}/${dir}/`))
}

export interface Made {
  made_at: number
  made_by: string
  source_url: string
}

/** What the library has for one URL: `browser.reflow_state` without `refresh`. */
export interface LibraryState {
  url: string
  site: string
  page: string
  package: { updated_at: number; author: Author; preference: string } | null
  result: Made | null
  fallback: Made | null
  /** The page to show, relative to `https://reflow.<zone>/`. */
  view: string | null
}

/** `browser.reflow_state` (§5.3). */
export interface ReflowState extends LibraryState {
  refresh: { status: 'idle' | 'running' | 'failed'; error: string | null; at_ms: number | null }
}

function madeOf(value: unknown): Made | null {
  if (!isRecord(value) || typeof value.made_at !== 'number' || typeof value.made_by !== 'string' || typeof value.source_url !== 'string') return null
  return { made_at: value.made_at, made_by: value.made_by, source_url: value.source_url }
}

async function validManifest(library: Library, site: string): Promise<Manifest | null> {
  const manifest = await readJson(library, `${site}/${MANIFEST_FILE}`)
  return manifestProblems(manifest, site).length === 0 ? manifest as Manifest : null
}

export function viewOf(address: ReflowAddress, shown: { result: boolean; fallback: boolean }): string | null {
  if (shown.result) return `${address.site}/?url=${encodeURIComponent(address.url)}&page=${address.page}`
  return shown.fallback ? `${address.site}/fallback/${address.page}.html` : null
}

export async function libraryState(library: Library, address: ReflowAddress): Promise<LibraryState> {
  const { site, page } = address
  const valid = await validManifest(library, site)
  const manifest = valid && packageMatches(valid.match, address) ? valid : null
  const result = madeOf(await readJson(library, `${site}/results/${page}.json`))
  const fallback = madeOf(await readJson(library, `${site}/fallback/${page}.json`))
  return {
    url: address.url,
    site,
    page,
    package: manifest && { updated_at: manifest.updated_at, author: manifest.author, preference: manifest.preference ?? '' },
    result,
    fallback,
    view: viewOf(address, { result: !!manifest && !!result, fallback: !!fallback }),
  }
}

/** The result the shown page was made from: the package's when it has one, else the fallback page's. */
export function shownMade(state: LibraryState): Made | null {
  return state.package && state.result ? state.result : state.fallback
}

// ── Preference ──

/** The user's preference for a site: the package's when the site has one, else the one kept beside it. */
export async function readPreference(library: Library, site: string): Promise<string> {
  const manifest = await validManifest(library, site)
  if (manifest) return manifest.preference ?? ''
  const kept = await readJson(library, `${site}/${PREFERENCE_FILE}`)
  return isRecord(kept) && typeof kept.preference === 'string' ? kept.preference : ''
}

/** Keeps the preference where `readPreference` finds it: in the package's manifest, or in `<site>/preference.json`. */
export async function writePreference(library: Library, site: string, preference: string, now: number = Date.now()): Promise<void> {
  if (!isSite(site)) throw new ReflowError(`not a site name: ${site}`)
  const { store, bucketId } = library
  const manifest = await validManifest(library, site)
  if (manifest) {
    const next: Manifest = { ...manifest }
    if (preference) next.preference = preference
    else delete next.preference
    await store.putBytes(bucketId, `${site}/${MANIFEST_FILE}`, JSON.stringify(next, null, 2), { contentType: contentTypeOf(MANIFEST_FILE) })
    return
  }
  const key = `${site}/${PREFERENCE_FILE}`
  if (preference) {
    await store.putBytes(bucketId, key, JSON.stringify({ schema: PREFERENCE_SCHEMA, preference, updated_at: now }), { contentType: 'application/json' })
  } else if (await readText(library, key) !== undefined) {
    await store.delete(bucketId, key)
  }
}

// ── The sites of the library ──

export interface SiteEntry {
  site: string
  /** `null`: the site has results, fallback pages or a preference but no valid package. */
  package: { match: MatchRule[]; updated_at: number; author: Author } | null
  preference: string
  /** The package's files, relative to the site, with their sizes. */
  files: { name: string; size: number }[]
  results: number
  fallbacks: number
}

async function siteEntry(library: Library, site: string): Promise<SiteEntry> {
  const objects = await listObjects(library, `${site}/`)
  const files: SiteEntry['files'] = []
  let results = 0
  let fallbacks = 0
  let keptPreference = false
  for (const { key, size } of objects) {
    const name = key.slice(site.length + 1)
    if (name.startsWith('results/')) { if (name.endsWith('.json')) results += 1 }
    else if (name.startsWith('fallback/')) { if (name.endsWith('.html')) fallbacks += 1 }
    else if (name === PREFERENCE_FILE) keptPreference = true
    else files.push({ name, size })
  }
  const manifest = files.some((file) => file.name === MANIFEST_FILE) ? await validManifest(library, site) : null
  let preference = manifest?.preference ?? ''
  if (!manifest && keptPreference) {
    const kept = await readJson(library, `${site}/${PREFERENCE_FILE}`)
    if (isRecord(kept) && typeof kept.preference === 'string') preference = kept.preference
  }
  return {
    site,
    package: manifest && { match: manifest.match, updated_at: manifest.updated_at, author: manifest.author },
    preference,
    files,
    results,
    fallbacks,
  }
}

export async function listSites(library: Library): Promise<SiteEntry[]> {
  const { store, bucketId } = library
  const sites: string[] = []
  let startAfter: string | undefined
  do {
    const page = await store.listObjects(bucketId, { delimiter: '/', ...(startAfter ? { start_after: startAfter } : {}) })
    sites.push(...page.common_prefixes.map((prefix) => prefix.slice(0, -1)))
    startAfter = page.next_start_after
  } while (startAfter)
  return Promise.all(sites.map((site) => siteEntry(library, site)))
}

/** `<site>/manifest.json` as it is stored, for showing; undefined when the site has none. */
export function readManifestText(library: Library, site: string): Promise<string | undefined> {
  return readText(library, `${site}/${MANIFEST_FILE}`)
}

/** Removes everything the library keeps for `site`: package, preference, results and fallback pages. */
export async function removeSite(library: Library, site: string): Promise<number> {
  if (!isSite(site)) throw new ReflowError(`not a site name: ${site}`)
  const objects = await listObjects(library, `${site}/`)
  for (const { key } of objects) await library.store.delete(library.bucketId, key)
  return objects.length
}

export interface ImportReport {
  site: string
  files: string[]
  /** Files of the package this one replaced that it no longer has. */
  removed: string[]
  manifest: Manifest
}

/**
 * Checks a package (its files by their path inside it) the way `reflow put` does and makes it the package of the
 * site its manifest names: every file goes to `<site>/<path>`, the manifest last (stamped with `updated_at`, with
 * `author` when it names none, and with the preference kept for the site when it states none); files of the
 * replaced package that are gone are deleted. Results and fallback pages stay.
 */
export async function importPackage(library: Library, files: ReadonlyMap<string, Uint8Array>, options: { author: Author; now?: number }): Promise<ImportReport> {
  const { store, bucketId } = library
  const names = [...files.keys()].sort()
  let manifest: unknown
  const manifestBytes = files.get(MANIFEST_FILE)
  if (manifestBytes) {
    try {
      manifest = JSON.parse(new TextDecoder().decode(manifestBytes))
    } catch (error) {
      throw new ReflowError('the package is not valid', [`${MANIFEST_FILE} is not JSON: ${(error as Error).message}`])
    }
    if (isRecord(manifest)) {
      manifest.updated_at = options.now ?? Date.now()
      if (manifest.author === undefined) manifest.author = options.author
    }
  }
  const named = isRecord(manifest) && typeof manifest.site === 'string' ? manifest.site : ''
  const site = isSite(named) ? named : ''
  const problems = isRecord(manifest) && !site ? [`site "${named}" is not a site name (the host name with dots as dashes)`] : packageProblems(names, manifest, site)
  for (const [name, bytes] of files) {
    if (bytes.byteLength > MAX_FILE_BYTES) problems.push(`${name} is ${bytes.byteLength} bytes; a package file is at most ${MAX_FILE_BYTES}`)
  }
  if (problems.length > 0) throw new ReflowError('the package is not valid', problems)

  const next = manifest as Manifest
  if (next.preference === undefined) {
    const kept = await readPreference(library, site)
    if (kept) next.preference = kept
  }
  const before = (await listObjects(library, `${site}/`)).map((object) => object.key).filter((key) => !isDataKey(site, key))
  for (const name of names) {
    if (name !== MANIFEST_FILE) await store.putBytes(bucketId, `${site}/${name}`, files.get(name)!, { contentType: contentTypeOf(name) })
  }
  await store.putBytes(bucketId, `${site}/${MANIFEST_FILE}`, JSON.stringify(next, null, 2), { contentType: contentTypeOf(MANIFEST_FILE) })
  const removed: string[] = []
  for (const key of before) {
    const name = key.slice(site.length + 1)
    if (!files.has(name)) {
      await store.delete(bucketId, key)
      removed.push(name)
    }
  }
  return { site, files: names, removed, manifest: next }
}
