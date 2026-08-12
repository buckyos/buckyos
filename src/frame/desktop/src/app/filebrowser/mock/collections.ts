/**
 * In-memory collection store + CollectionReader implementation.
 *
 * The collection *data model* is mock (persistence belongs to the backend —
 * "Shares" always was a backend concept), but member operations mutate this
 * store for real and broadcast watch invalidations, so the interaction loop
 * is fully experienceable in-session. No localStorage: collections are
 * backend data, the UI does not self-persist. Refresh = back to seeds.
 */

import type { FileEntry, SortDir, SortKey } from '../types'
import type { CollectionNode, CollectionReader } from '../data/CollectionModel'
import type {
  FileItem,
  FileItemPage,
  ListQuery,
  LocationCapabilities,
} from '../data/FolderReader'
import { registerReaderProvider } from '../data/readerRegistry'
import { resolveTarget } from '../data/targetResolver'
import { compareEntries } from '../data/sortEntries'
import { collectionUrl, dfsPathOf, parseCollectionUrl } from '../data/urls'
import { mockDelay } from '../data/mockReader'

export interface CollectionSummary {
  id: string
  title: string
  /** Reference count across all nested groups. */
  refCount: number
}

interface CollectionDef {
  id: string
  title: string
  description?: string
  nodes: CollectionNode[]
}

let refKeyCounter = 0
function nextKey(prefix: 'ref' | 'group'): string {
  refKeyCounter += 1
  return `${prefix}-${refKeyCounter}`
}

function ref(targetUrl: string): CollectionNode {
  return { type: 'ref', key: nextKey('ref'), targetUrl }
}

function group(name: string, children: CollectionNode[]): CollectionNode {
  return { type: 'group', key: nextKey('group'), name, children }
}

// ─── Seed data ───
//
// `shares` absorbs the old /shared sidebar root: by the three-kind model the
// existing "Shares" product concept *is* a collection. `reading-list` is the
// custom seed exercising: nested group, a ref to a real folder, a broken ref,
// and the same file referenced twice.
function seedCollections(): Map<string, CollectionDef> {
  const map = new Map<string, CollectionDef>()
  map.set('shares', {
    id: 'shares',
    title: 'Shares',
    description: 'Files and folders shared with you.',
    nodes: [ref('dfs:///shared/incoming')],
  })
  map.set('reading-list', {
    id: 'reading-list',
    title: 'Reading List',
    description: 'Hand-curated queue — order drives the default preview flow.',
    nodes: [
      ref('dfs:///home/Documents/Kyoto Trip Plan.md'),
      ref('dfs:///home/Downloads/market-report-2026.pdf'),
      // Same file on purpose: list identity is the ref key, not the file id.
      ref('dfs:///home/Downloads/market-report-2026.pdf'),
      // A real folder by reference — opening it navigates to its dfs:// home.
      ref('dfs:///home/Pictures/Trips/Kyoto'),
      // Dangling on purpose: the target never existed.
      ref('dfs:///home/Documents/archived-roadmap.pdf'),
      group('papers', [
        ref('dfs:///public/resume.pdf'),
        ref('dfs:///public/talks/personal-server-talk.pdf'),
      ]),
    ],
  })
  return map
}

const collections = seedCollections()

// ─── Store: queries, mutations, watch ───

const collectionListeners = new Map<string, Set<() => void>>()
const listListeners = new Set<() => void>()
let summariesCache: CollectionSummary[] | null = null
let listSnapshot = 0

function emit(id: string) {
  summariesCache = null
  listSnapshot += 1
  for (const listener of collectionListeners.get(id) ?? []) listener()
  for (const listener of listListeners) listener()
}

function countRefs(nodes: CollectionNode[]): number {
  let count = 0
  for (const node of nodes) {
    if (node.type === 'ref') count += 1
    else count += countRefs(node.children)
  }
  return count
}

function mustGet(id: string): CollectionDef {
  const def = collections.get(id)
  if (!def) throw new Error(`collection "${id}" not found`)
  return def
}

/** Children array of the group at `groupPath` (names), or null when missing. */
function nodesAt(def: CollectionDef, groupPath: string[]): CollectionNode[] | null {
  let nodes = def.nodes
  for (const name of groupPath) {
    const next = nodes.find(
      (node): node is Extract<CollectionNode, { type: 'group' }> =>
        node.type === 'group' && node.name === name,
    )
    if (!next) return null
    nodes = next.children
  }
  return nodes
}

/** The children array that directly contains `key`, searched recursively. */
function containerOf(nodes: CollectionNode[], key: string): CollectionNode[] | null {
  if (nodes.some((node) => node.key === key)) return nodes
  for (const node of nodes) {
    if (node.type === 'group') {
      const found = containerOf(node.children, key)
      if (found) return found
    }
  }
  return null
}

export const collectionStore = {
  subscribeList(listener: () => void): () => void {
    listListeners.add(listener)
    return () => listListeners.delete(listener)
  },

  subscribe(id: string, listener: () => void): () => void {
    let set = collectionListeners.get(id)
    if (!set) {
      set = new Set()
      collectionListeners.set(id, set)
    }
    set.add(listener)
    return () => {
      set.delete(listener)
      if (set.size === 0) collectionListeners.delete(id)
    }
  },

  /** Stable-identity snapshot for useSyncExternalStore. */
  listSnapshot(): number {
    return listSnapshot
  },

  list(): CollectionSummary[] {
    if (!summariesCache) {
      summariesCache = [...collections.values()].map((def) => ({
        id: def.id,
        title: def.title,
        refCount: countRefs(def.nodes),
      }))
    }
    return summariesCache
  },

  get(id: string): { title: string; description?: string } | undefined {
    const def = collections.get(id)
    return def ? { title: def.title, description: def.description } : undefined
  },

  create(title: string): string {
    const base =
      title
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '') || 'collection'
    let id = base
    for (let i = 2; collections.has(id); i += 1) id = `${base}-${i}`
    collections.set(id, { id, title, nodes: [] })
    emit(id)
    return id
  },

  addReferences(
    id: string,
    targets: string[],
    options?: { groupPath?: string[]; position?: number },
  ) {
    const def = mustGet(id)
    const nodes = nodesAt(def, options?.groupPath ?? [])
    if (!nodes) throw new Error(`group ${options?.groupPath?.join('/')} not found in "${id}"`)
    const refs = targets.map((target) => ref(target))
    const at = options?.position ?? nodes.length
    nodes.splice(Math.max(0, Math.min(at, nodes.length)), 0, ...refs)
    emit(id)
  },

  removeItems(id: string, itemKeys: string[]) {
    const def = mustGet(id)
    const keys = new Set(itemKeys)
    const prune = (nodes: CollectionNode[]): CollectionNode[] =>
      nodes
        .filter((node) => !keys.has(node.key))
        .map((node) =>
          node.type === 'group' ? { ...node, children: prune(node.children) } : node,
        )
    def.nodes = prune(def.nodes)
    emit(id)
  },

  /** Move `itemKeys` (siblings) as a block to `toIndex` within their group. */
  reorder(id: string, itemKeys: string[], toIndex: number) {
    if (!itemKeys.length) return
    const def = mustGet(id)
    const container = containerOf(def.nodes, itemKeys[0])
    if (!container) return
    const keys = new Set(itemKeys)
    const moved = container.filter((node) => keys.has(node.key))
    if (!moved.length) return
    const rest = container.filter((node) => !keys.has(node.key))
    const at = Math.max(0, Math.min(toIndex, rest.length))
    container.splice(0, container.length, ...rest.slice(0, at), ...moved, ...rest.slice(at))
    emit(id)
  },

  createGroup(id: string, name: string, options?: { groupPath?: string[]; position?: number }) {
    const def = mustGet(id)
    const nodes = nodesAt(def, options?.groupPath ?? [])
    if (!nodes) throw new Error(`group ${options?.groupPath?.join('/')} not found in "${id}"`)
    const at = options?.position ?? nodes.length
    nodes.splice(Math.max(0, Math.min(at, nodes.length)), 0, group(name, []))
    emit(id)
  },

  renameGroup(id: string, itemKey: string, name: string) {
    const def = mustGet(id)
    const container = containerOf(def.nodes, itemKey)
    const node = container?.find((item) => item.key === itemKey)
    if (node && node.type === 'group') {
      node.name = name
      emit(id)
    }
  },
}

// ─── CollectionReader ───

const COLLECTION_CAPABILITIES: LocationCapabilities = {
  kind: 'collection',
  acceptsContent: false,
  acceptsReferences: true,
  removal: 'remove-ref',
  canReorder: true,
  sortKeys: ['manual', 'name', 'size', 'modified', 'kind'],
  defaultSortKey: 'manual',
}

const GROUP_MTIME = '2026-01-01T00:00:00Z'

function basenameOf(targetUrl: string): string {
  const path = dfsPathOf(targetUrl) ?? targetUrl
  return path.split('/').filter(Boolean).pop() ?? targetUrl
}

class MockCollectionReader implements CollectionReader {
  readonly capabilities = COLLECTION_CAPABILITIES
  readonly url: string
  private readonly collectionId: string
  private readonly groupPath: string[]

  constructor(url: string) {
    this.url = url
    const parts = parseCollectionUrl(url)
    if (!parts) throw new Error(`bad collection url: ${url}`)
    this.collectionId = parts.collectionId
    this.groupPath = parts.groupPath
  }

  get meta(): { title: string; description?: string } {
    const def = collectionStore.get(this.collectionId)
    const title = def?.title ?? this.collectionId
    return {
      title: this.groupPath.length ? `${title} › ${this.groupPath.join(' › ')}` : title,
      description: def?.description,
    }
  }

  private async resolveItems(): Promise<FileItem[]> {
    const def = collections.get(this.collectionId)
    const nodes = def ? nodesAt(def, this.groupPath) : null
    if (!nodes) return []
    return Promise.all(
      nodes.map(async (node, index): Promise<FileItem> => {
        if (node.type === 'group') {
          const path = collectionUrl(this.collectionId, [...this.groupPath, node.name])
          const entry: FileEntry = {
            id: `${this.collectionId}:${node.key}`,
            name: node.name,
            kind: 'folder',
            // Groups are collection structure, not real folders: their path
            // *is* the collection URL, so "open" naturally navigates there.
            path,
            modifiedAt: GROUP_MTIME,
          }
          return {
            key: node.key,
            entry,
            ref: { collectionUrl: this.url, refPath: path, orderIndex: index },
          }
        }
        const resolved = await resolveTarget(node.targetUrl)
        const broken = !resolved
        const entry: FileEntry =
          resolved ??
          ({
            id: `${this.collectionId}:${node.key}`,
            name: basenameOf(node.targetUrl),
            kind: 'other',
            path: dfsPathOf(node.targetUrl) ?? node.targetUrl,
            modifiedAt: GROUP_MTIME,
            link: { targetUrl: node.targetUrl, broken: true },
          } satisfies FileEntry)
        return {
          key: node.key,
          entry,
          ref: {
            collectionUrl: this.url,
            refPath: `${this.url}/${entry.name}`,
            orderIndex: index,
            broken,
          },
        }
      }),
    )
  }

  private sortItems(items: FileItem[], query: ListQuery): FileItem[] {
    if (query.sortKey === 'manual') return items
    return [...items].sort((a, b) => {
      if (query.foldersFirst && (a.entry.kind === 'folder') !== (b.entry.kind === 'folder')) {
        return a.entry.kind === 'folder' ? -1 : 1
      }
      return compareEntries(
        a.entry,
        b.entry,
        query.sortKey as SortKey,
        query.sortDir as SortDir,
      )
    })
  }

  async list(query: ListQuery): Promise<FileItemPage> {
    await mockDelay(30, 80)
    const items = this.sortItems(await this.resolveItems(), query)
    const slice = items.slice(query.offset, query.offset + query.limit)
    return {
      items: slice,
      totalCount: items.length,
      hasMore: query.offset + slice.length < items.length,
    }
  }

  async getItem(key: string): Promise<FileItem | null> {
    const items = await this.resolveItems()
    return items.find((item) => item.key === key) ?? null
  }

  watch(onInvalidate: () => void): () => void {
    return collectionStore.subscribe(this.collectionId, onInvalidate)
  }

  dispose() {}

  // ─── Member management — mutates the mock store for real ───

  async addReferences(targets: string[], position?: number): Promise<void> {
    await mockDelay(20, 60)
    collectionStore.addReferences(this.collectionId, targets, {
      groupPath: this.groupPath,
      position,
    })
  }

  async removeItems(itemKeys: string[]): Promise<void> {
    await mockDelay(20, 60)
    collectionStore.removeItems(this.collectionId, itemKeys)
  }

  async reorder(itemKeys: string[], toIndex: number): Promise<void> {
    await mockDelay(20, 60)
    collectionStore.reorder(this.collectionId, itemKeys, toIndex)
  }

  async createGroup(name: string, position?: number): Promise<void> {
    await mockDelay(20, 60)
    collectionStore.createGroup(this.collectionId, name, {
      groupPath: this.groupPath,
      position,
    })
  }

  async renameGroup(itemKey: string, name: string): Promise<void> {
    await mockDelay(20, 60)
    collectionStore.renameGroup(this.collectionId, itemKey, name)
  }
}

export function registerCollectionReader() {
  registerReaderProvider({
    scheme: 'collection',
    create: (url) => new MockCollectionReader(url),
  })
}
