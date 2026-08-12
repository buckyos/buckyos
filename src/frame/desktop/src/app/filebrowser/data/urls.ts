/**
 * Location URLs — every pane/tab position is one canonical URL string.
 *
 *   dfs:///home/photos          folder (bare paths like `/home/photos` normalize here)
 *   view://recent               derived, read-only views
 *   view://topic/<id>           AI topic aggregation (legacy `topic://<id>` rewrites here)
 *   collection://<id>/<group?>  ordered reference collections
 *
 * Internally everything stores the canonical form; `displayPath()` converts
 * back to the user-friendly form for breadcrumbs / the address bar.
 */

export const DFS_SCHEME = 'dfs://'
export const VIEW_SCHEME = 'view://'
export const COLLECTION_SCHEME = 'collection://'

const SCHEME_RE = /^[a-z][a-z0-9+.-]*:\/\//i

export function urlScheme(url: string): string | null {
  const match = SCHEME_RE.exec(url)
  return match ? match[0].slice(0, -3).toLowerCase() : null
}

function stripTrailingSlash(value: string): string {
  return value.length > 1 && value.endsWith('/') ? value.replace(/\/+$/, '') : value
}

/** Normalize any user/legacy input (bare path, `topic://`, scheme URL) to canonical form. */
export function normalizeUrl(input: string): string {
  const raw = input.trim()
  if (!raw) return `${DFS_SCHEME}/`
  // Legacy topic scheme → view://topic/<id>
  if (raw.startsWith('topic://')) {
    return `${VIEW_SCHEME}topic/${raw.slice('topic://'.length)}`
  }
  if (SCHEME_RE.test(raw)) {
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

/** User-friendly display form: dfs urls render as bare paths, others as-is. */
export function displayPath(url: string): string {
  return dfsPathOf(url) ?? url
}

export interface CollectionUrlParts {
  collectionId: string
  /** Group names from the collection root down to the open group. */
  groupPath: string[]
}

export function parseCollectionUrl(url: string): CollectionUrlParts | null {
  if (!url.startsWith(COLLECTION_SCHEME)) return null
  const rest = url.slice(COLLECTION_SCHEME.length)
  const segments = rest.split('/').filter(Boolean)
  if (!segments.length) return null
  return { collectionId: segments[0], groupPath: segments.slice(1) }
}

export function collectionUrl(collectionId: string, groupPath: string[] = []): string {
  return stripTrailingSlash(
    `${COLLECTION_SCHEME}${[collectionId, ...groupPath].join('/')}`,
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
  const path = dfsPathOf(url)
  if (path !== null) {
    return path.split('/').filter(Boolean).pop() ?? 'root'
  }
  const segments = url.split('/').filter((seg) => seg && !seg.endsWith(':'))
  return segments.pop() ?? url
}

export interface UrlCrumb {
  label: string
  url: string
}

/**
 * Breadcrumbs for any location. `rootLabel` overrides the first crumb's
 * label (e.g. a collection/view title from reader meta).
 */
export function crumbsForUrl(url: string, rootLabel?: string): UrlCrumb[] {
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
