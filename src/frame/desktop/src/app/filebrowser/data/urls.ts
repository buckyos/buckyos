/**
 * Location URLs — every pane/tab position is one canonical URL string.
 *
 *   dfs:///home/photos          folder (bare paths like `/home/photos` normalize here)
 *   view://recent               derived, read-only views
 *   view://topic/<id>           AI topic aggregation
 *   collection://<id>/<group?>  ordered reference collections
 *   device://<session>/<path>   the computer behind a Desktop Lite node; the
 *                               authority is the Files window's sudo session
 *                               (`/C:/Users` on Windows). Shown and copied as
 *                               a standard `file:///` URI, never as device://
 *
 * Internally everything stores the canonical form; `displayPath()` converts
 * back to the user-friendly form for breadcrumbs / the address bar.
 */

export const DFS_SCHEME = 'dfs://'
export const VIEW_SCHEME = 'view://'
export const COLLECTION_SCHEME = 'collection://'
export const DEVICE_SCHEME = 'device://'
export const FILE_URI_SCHEME = 'file://'

const SCHEME_RE = /^[a-z][a-z0-9+.-]*:\/\//i

export function urlScheme(url: string): string | null {
  const match = SCHEME_RE.exec(url)
  return match ? match[0].slice(0, -3).toLowerCase() : null
}

function stripTrailingSlash(value: string): string {
  return value.length > 1 && value.endsWith('/') ? value.replace(/\/+$/, '') : value
}

/** Normalize any user input (bare path or scheme URL) to canonical form. */
export function normalizeUrl(input: string): string {
  const raw = input.trim()
  if (!raw) return `${DFS_SCHEME}/`
  if (SCHEME_RE.test(raw)) {
    if (raw.startsWith(DEVICE_SCHEME)) {
      const parts = parseDeviceUrl(raw)
      return parts ? deviceUrl(parts.session, parts.path) : stripTrailingSlash(raw)
    }
    if (raw.startsWith(DFS_SCHEME)) {
      const path = raw.slice(DFS_SCHEME.length)
      return `${DFS_SCHEME}${normalizePath(path)}`
    }
    return stripTrailingSlash(raw)
  }
  return `${DFS_SCHEME}${normalizePath(raw)}`
}

function normalizePath(path: string): string {
  let value = path.startsWith('/') ? path : `/${path}`
  value = stripTrailingSlash(value)
  return value === '' ? '/' : value
}

/** The DFS path of a canonical url, or null when it is not a dfs location. */
export function dfsPathOf(url: string): string | null {
  return url.startsWith(DFS_SCHEME) ? url.slice(DFS_SCHEME.length) || '/' : null
}

/** User-friendly display form: dfs urls render as bare paths, computer folders as readable file URIs, others as-is. */
export function displayPath(url: string): string {
  const device = parseDeviceUrl(url)
  if (device) return `${FILE_URI_SCHEME}${device.path}`
  return dfsPathOf(url) ?? url
}

// ─── Computer (host realm) locations ───

export interface DeviceUrlParts {
  /** The Files window's sudo session key. */
  session: string
  /** Absolute path on the computer: `/` (root; on Windows the drive list), `/Users/a`, `/C:/Users/a`. */
  path: string
}

export function deviceUrl(session: string, path = '/'): string {
  const normalized = normalizePath(path)
  return normalized === '/' ? `${DEVICE_SCHEME}${session}` : `${DEVICE_SCHEME}${session}${normalized}`
}

export function parseDeviceUrl(url: string): DeviceUrlParts | null {
  if (!url.startsWith(DEVICE_SCHEME)) return null
  const rest = url.slice(DEVICE_SCHEME.length)
  const slash = rest.indexOf('/')
  const session = slash < 0 ? rest : rest.slice(0, slash)
  if (!session) return null
  const path = slash < 0 ? '/' : normalizePath(rest.slice(slash))
  return { session, path }
}

const WINDOWS_DRIVE_RE = /^\/[A-Za-z]:(\/|$)/
/** What Node's `pathToFileURL` leaves as is; everything else is UTF-8 percent-encoded. */
const FILE_URI_SAFE = /^[A-Za-z0-9!$&'()*+,\-.:;=@_]$/
const utf8 = new TextEncoder()

function encodeFileSegment(segment: string): string {
  let out = ''
  for (const ch of segment) {
    if (FILE_URI_SAFE.test(ch)) out += ch
    else for (const byte of utf8.encode(ch)) out += `%${byte.toString(16).toUpperCase().padStart(2, '0')}`
  }
  return out
}

/** RFC 8089 file URI of a computer path, percent-encoded like Node's `pathToFileURL`. */
export function fileUriOf(path: string): string {
  const segments = path.split('/').filter(Boolean)
  const drive = WINDOWS_DRIVE_RE.test(path)
  // A bare drive (`/C:`) keeps its trailing slash: `file:///C:/`.
  return `${FILE_URI_SCHEME}/${segments.map(encodeFileSegment).join('/')}${drive && segments.length === 1 ? '/' : ''}`
}

/**
 * The computer path a pasted `file:` URI names (encoded or not), or null.
 * Accepts `file:///p`, `file://localhost/p` and Windows `file:///C:/p`.
 */
export function pathOfFileUri(input: string): string | null {
  const raw = input.trim()
  if (!/^file:/i.test(raw)) return null
  let rest = raw.replace(/^file:/i, '')
  if (rest.startsWith('//')) {
    rest = rest.slice(2)
    const slash = rest.indexOf('/')
    const host = slash < 0 ? rest : rest.slice(0, slash)
    if (host && host.toLowerCase() !== 'localhost') return null
    rest = slash < 0 ? '/' : rest.slice(slash)
  }
  if (!rest.startsWith('/')) rest = `/${rest}`
  rest = rest.replace(/[?#].*$/, '')
  let decoded = rest
  try {
    decoded = decodeURIComponent(rest)
  } catch {
    // keep the undecodable text as typed
  }
  return normalizePath(decoded.replace(/\\/g, '/'))
}

/**
 * What "Copy path" puts on the clipboard: a DFS location (a bare path or a dfs url) as its Zone file URL
 * `cyfs:///<path>`, the form nfs-server's `canonical_path` and the previews use. Names stay readable; only `%`, `?`
 * and `#` are escaped so the text still parses as that URL. Any other location (an existing cyfs url,
 * collection://, view://) is copied as-is.
 */
export function copyPathText(pathOrUrl: string): string {
  const raw = pathOrUrl.trim()
  const device = parseDeviceUrl(raw)
  if (device) return fileUriOf(device.path)
  const path = dfsPathOf(raw) ?? (raw.startsWith('/') ? raw : null)
  if (path === null) return raw
  return `cyfs://${path.replace(/[%?#]/g, (c) => encodeURIComponent(c))}`
}

export interface CollectionUrlParts {
  collectionId: string
  /**
   * Group names from the collection root down to the open group. Segments are
   * percent-encoded in the URL and decoded here. Note this name-based path is
   * a display locator, not durable group identity — the NFSP adapter must
   * back it with entry_ref resolution (UI_DATAMODEL.md §2.2).
   */
  groupPath: string[]
}

function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment)
  } catch {
    return segment
  }
}

export function parseCollectionUrl(url: string): CollectionUrlParts | null {
  if (!url.startsWith(COLLECTION_SCHEME)) return null
  const rest = url.slice(COLLECTION_SCHEME.length)
  const segments = rest.split('/').filter(Boolean)
  if (!segments.length) return null
  return {
    collectionId: decodeSegment(segments[0]),
    groupPath: segments.slice(1).map(decodeSegment),
  }
}

export function collectionUrl(collectionId: string, groupPath: string[] = []): string {
  return stripTrailingSlash(
    `${COLLECTION_SCHEME}${[collectionId, ...groupPath]
      .map((segment) => encodeURIComponent(segment))
      .join('/')}`,
  )
}

export interface ViewUrlParts {
  viewType: string
  rest: string[]
}

export function parseViewUrl(url: string): ViewUrlParts | null {
  if (!url.startsWith(VIEW_SCHEME)) return null
  const segments = url.slice(VIEW_SCHEME.length).split('/').filter(Boolean)
  if (!segments.length) return null
  return { viewType: segments[0], rest: segments.slice(1) }
}

/** Parent location for the "up" navigation; null when there is none. */
export function parentUrl(url: string): string | null {
  const device = parseDeviceUrl(url)
  if (device) {
    if (device.path === '/') return null
    return deviceUrl(device.session, device.path.split('/').slice(0, -1).join('/') || '/')
  }
  const path = dfsPathOf(url)
  if (path !== null) {
    if (path === '/') return null
    const parent = path.split('/').slice(0, -1).join('/') || '/'
    return `${DFS_SCHEME}${parent}`
  }
  const collection = parseCollectionUrl(url)
  if (collection && collection.groupPath.length) {
    return collectionUrl(collection.collectionId, collection.groupPath.slice(0, -1))
  }
  return null
}

/** Fallback tab/crumb title derived from the url alone (readers refine via meta). */
export function fallbackTitle(url: string): string {
  const device = parseDeviceUrl(url)
  if (device) return device.path.split('/').filter(Boolean).pop() ?? ''
  const path = dfsPathOf(url)
  if (path !== null) {
    return path.split('/').filter(Boolean).pop() ?? 'root'
  }
  const segments = url.split('/').filter((seg) => seg && !seg.endsWith(':'))
  const last = segments.pop()
  return last ? decodeSegment(last) : url
}

export interface UrlCrumb {
  label: string
  url: string
}

/**
 * Breadcrumbs for any location. `rootLabel` overrides the first crumb's
 * label (e.g. a collection/view title from reader meta).
 */
export function crumbsForUrl(url: string, rootLabel?: string, computerLabel?: string): UrlCrumb[] {
  const device = parseDeviceUrl(url)
  if (device) {
    const crumbs: UrlCrumb[] = [{ label: computerLabel || 'Computer', url: deviceUrl(device.session) }]
    let running = ''
    for (const segment of device.path.split('/').filter(Boolean)) {
      running += `/${segment}`
      crumbs.push({ label: segment, url: deviceUrl(device.session, running) })
    }
    return crumbs
  }
  const path = dfsPathOf(url)
  if (path !== null) {
    const crumbs: UrlCrumb[] = [{ label: 'root', url: `${DFS_SCHEME}/` }]
    let running = ''
    for (const segment of path.split('/').filter(Boolean)) {
      running += `/${segment}`
      crumbs.push({ label: segment, url: `${DFS_SCHEME}${running}` })
    }
    return crumbs
  }
  const collection = parseCollectionUrl(url)
  if (collection) {
    const crumbs: UrlCrumb[] = [
      {
        label: rootLabel ?? collection.collectionId,
        url: collectionUrl(collection.collectionId),
      },
    ]
    for (let i = 0; i < collection.groupPath.length; i += 1) {
      crumbs.push({
        label: collection.groupPath[i],
        url: collectionUrl(collection.collectionId, collection.groupPath.slice(0, i + 1)),
      })
    }
    return crumbs
  }
  return [{ label: rootLabel ?? fallbackTitle(url), url }]
}
