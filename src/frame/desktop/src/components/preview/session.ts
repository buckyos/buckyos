/**
 * Session Context → stable Session Items (PRD §11).
 *
 * Source answers "what is open now"; Session Context answers "which browsing
 * set it belongs to". Everything about previous / next — counts, bounds,
 * wrap — is derived here and never guessed from a path.
 */

import {
  isBlobRef,
  isCyfsPathRef,
  isObjectIdRef,
  type ContentRef,
  type NavigationMode,
  type PreviewProvider,
  type PreviewSessionContext,
  type PreviewSessionItemInput,
} from './types'

export interface SessionItem {
  id: string
  source: ContentRef
  title?: string
}

export interface ResolvedSession {
  /** Identity of the browsing set — equal keys never re-enumerate. */
  key: string
  kind: PreviewSessionContext['kind']
  sessionId?: string
  items: SessionItem[]
  index: number
  navigation: NavigationMode
  container?: ContentRef
}

const CYFS_PREFIX = 'cyfs://'

/** `/home/x`, `cyfs:///home/x`, `dfs:///home/x` → `cyfs:///home/x`. */
export function normalizeCyfsPath(input: string): string {
  let path = input.trim()
  if (path.startsWith('dfs://')) path = path.slice('dfs://'.length)
  else if (path.startsWith(CYFS_PREFIX)) path = path.slice(CYFS_PREFIX.length)
  if (!path.startsWith('/')) path = `/${path}`
  path = path.replace(/\/{2,}/g, '/')
  if (path.length > 1 && path.endsWith('/')) path = path.replace(/\/+$/, '')
  return `${CYFS_PREFIX}${path}`
}

/** The bare `/home/x` part of a canonical cyfs path (current zone). */
export function cyfsPathToLocal(canonical: string): string {
  const normalized = normalizeCyfsPath(canonical)
  return normalized.slice(CYFS_PREFIX.length) || '/'
}

export function parentCyfsPath(canonical: string): string | null {
  const local = cyfsPathToLocal(canonical)
  if (local === '/') return null
  const parent = local.split('/').slice(0, -1).join('/') || '/'
  return normalizeCyfsPath(parent)
}

export function lastSegment(path: string): string {
  return cyfsPathToLocal(path).split('/').filter(Boolean).pop() ?? ''
}

/** Stable identity string for matching items, windows and cache keys. */
export function refIdentity(ref: ContentRef): string {
  if (isCyfsPathRef(ref)) return `path:${normalizeCyfsPath(ref.path)}${ref.version ? `@${ref.version}` : ''}`
  if (isObjectIdRef(ref)) return `obj:${ref.objectId}${ref.version ? `@${ref.version}` : ''}`
  if (isBlobRef(ref)) {
    const { blob, name } = ref.value
    return `blob:${name ?? ''}#${blob.size}#${blob.type}`
  }
  let value = ''
  try {
    value = JSON.stringify((ref as { value?: unknown }).value ?? null)
  } catch {
    value = String((ref as { value?: unknown }).value)
  }
  return `ext:${ref.kind}:${value}`
}

export function sameRef(a: ContentRef | undefined, b: ContentRef | undefined): boolean {
  if (!a || !b) return false
  return refIdentity(a) === refIdentity(b)
}

export function refDisplayName(ref: ContentRef): string {
  if (isCyfsPathRef(ref)) return lastSegment(ref.path) || cyfsPathToLocal(ref.path)
  if (isObjectIdRef(ref)) {
    const id = ref.objectId
    return id.length > 20 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id
  }
  if (isBlobRef(ref)) return ref.value.name ?? 'Untitled'
  return ref.kind
}

/** Container a source naturally belongs to (paths only — objects have none). */
export function implicitContainerOf(ref: ContentRef): ContentRef | undefined {
  if (isCyfsPathRef(ref)) {
    const parent = parentCyfsPath(ref.path)
    return parent ? { kind: 'cyfs-path', path: parent } : undefined
  }
  return undefined
}

function itemId(input: PreviewSessionItemInput): string {
  return input.id ?? refIdentity(input.source)
}

function toSessionItems(inputs: PreviewSessionItemInput[]): SessionItem[] {
  const seen = new Set<string>()
  const items: SessionItem[] = []
  for (const input of inputs) {
    let id = itemId(input)
    // The same file may appear twice in an explicit list — keep both, disambiguate.
    while (seen.has(id)) id = `${id}#${items.length}`
    seen.add(id)
    items.push({ id, source: input.source, title: input.title })
  }
  return items
}

function hashString(input: string): string {
  let h = 2166136261
  for (let i = 0; i < input.length; i += 1) {
    h ^= input.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return (h >>> 0).toString(36)
}

/** Identity of a session context — equal keys share the enumerated items. */
export function sessionKeyOf(context: PreviewSessionContext | undefined, source: ContentRef): string {
  if (!context || context.kind === 'single') return `single:${refIdentity(source)}`
  if (context.kind === 'container') {
    let extra = ''
    try {
      extra = JSON.stringify({ sort: context.sort ?? null, filter: context.filter ?? null })
    } catch {
      extra = ''
    }
    return `container:${refIdentity(context.container)}:${hashString(extra)}`
  }
  if (context.kind === 'list') {
    const identity = context.sessionId ?? hashString(context.items.map(itemId).join('\n'))
    return `list:${identity}:${context.version ?? ''}`
  }
  return `provider:${context.sessionId}`
}

/** PRD §22 defaults: explicit lists wrap, containers stop at the edge. */
export function defaultNavigation(context: PreviewSessionContext | undefined): NavigationMode {
  if (!context || context.kind === 'single') return 'bounded'
  if (context.navigation) return context.navigation
  return context.kind === 'list' ? 'wrap' : 'bounded'
}

export function locateIndex(items: SessionItem[], source: ContentRef, preferredId?: string): number {
  if (preferredId) {
    const byId = items.findIndex((item) => item.id === preferredId)
    if (byId >= 0) return byId
  }
  const identity = refIdentity(source)
  return items.findIndex((item) => refIdentity(item.source) === identity)
}

export async function resolveSessionContext(
  source: ContentRef,
  context: PreviewSessionContext | undefined,
  provider: PreviewProvider,
  signal?: AbortSignal,
): Promise<ResolvedSession> {
  const key = sessionKeyOf(context, source)
  const navigation = defaultNavigation(context)

  if (!context || context.kind === 'single') {
    return {
      key,
      kind: 'single',
      items: toSessionItems([{ source, title: refDisplayName(source) }]),
      index: 0,
      navigation,
    }
  }

  if (context.kind === 'list') {
    const items = toSessionItems(context.items)
    let index = Math.min(Math.max(context.currentIndex, 0), Math.max(items.length - 1, 0))
    // The host's index wins unless it clearly disagrees with the source.
    if (items[index] && !sameRef(items[index].source, source)) {
      const located = locateIndex(items, source)
      if (located >= 0) index = located
    }
    if (items.length === 0) {
      items.push(...toSessionItems([{ source, title: refDisplayName(source) }]))
      index = 0
    }
    return { key, kind: 'list', sessionId: context.sessionId, items, index, navigation }
  }

  if (context.kind === 'container') {
    const enumerated = await provider.enumerateContainer(context.container, {
      signal,
      sort: context.sort,
      filter: context.filter,
    })
    const items = toSessionItems(enumerated)
    let index = locateIndex(items, context.current)
    if (index < 0) index = locateIndex(items, source)
    if (index < 0) {
      // Defensive: the current item is not (yet) visible in the container —
      // keep it browsable instead of failing the whole session.
      items.unshift(...toSessionItems([{ source, title: refDisplayName(source) }]))
      index = 0
    }
    return { key, kind: 'container', items, index, navigation, container: context.container }
  }

  // provider (P1 contract, minimal support): one-shot listing.
  const listed = await context.provider.listItems()
  const items = toSessionItems(listed)
  let index = locateIndex(items, source, context.currentItemId)
  if (index < 0) {
    items.unshift(...toSessionItems([{ id: context.currentItemId, source, title: refDisplayName(source) }]))
    index = 0
  }
  return { key, kind: 'provider', sessionId: context.sessionId, items, index, navigation }
}

export function neighborIndex(session: ResolvedSession, delta: number, from = session.index): number | null {
  const count = session.items.length
  if (count <= 1) return null
  const next = from + delta
  if (next >= 0 && next < count) return next
  if (session.navigation === 'wrap') return ((next % count) + count) % count
  return null
}
