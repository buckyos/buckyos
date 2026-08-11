/**
 * Mock FolderReader implementations over the in-memory snapshot.
 *
 * They simulate the asynchronous, paged shape of the real DFS RPC (including
 * 50–150ms artificial latency, surfacing loading states and out-of-order
 * responses) — swapping in the real backend means registering different
 * providers, the UI stays unchanged.
 */

import type { FileEntry, Topic } from '../types'
import {
  fileBrowserSnapshot,
  mockEntriesAtPath,
  mockEntryById,
  mockEntryByPath,
  mockLibraryEntries,
} from '../mock/data'
import type {
  FileItem,
  FileItemPage,
  FolderReader,
  ListQuery,
  LocationCapabilities,
} from './FolderReader'
import { registerReaderProvider } from './readerRegistry'
import { registerTargetResolver } from './targetResolver'
import { sortEntriesForQuery } from './sortEntries'
import { dfsPathOf, parseViewUrl } from './urls'

export function mockDelay(min = 50, max = 150): Promise<void> {
  const ms = min + Math.random() * (max - min)
  return new Promise((resolve) => setTimeout(resolve, ms))
}

const FOLDER_CAPABILITIES: LocationCapabilities = {
  kind: 'folder',
  acceptsContent: true,
  acceptsReferences: false,
  removal: 'destroy',
  canReorder: false,
  sortKeys: ['name', 'size', 'modified', 'kind'],
  defaultSortKey: 'name',
}

const VIEW_CAPABILITIES: LocationCapabilities = {
  kind: 'view',
  acceptsContent: false,
  acceptsReferences: false,
  removal: null,
  canReorder: false,
  sortKeys: ['name', 'size', 'modified', 'kind'],
  defaultSortKey: 'name',
}

/**
 * Invalidation bus for mock folder data. Folder writes are still toast mocks,
 * but the refresh path is real: anything that mutates folder content should
 * call `invalidateMockPath` and watching readers reload.
 */
const fsListeners = new Map<string, Set<() => void>>()

export function invalidateMockPath(path: string) {
  for (const listener of fsListeners.get(path) ?? []) listener()
}

function watchPath(path: string, onInvalidate: () => void): () => void {
  let set = fsListeners.get(path)
  if (!set) {
    set = new Set()
    fsListeners.set(path, set)
  }
  set.add(onInvalidate)
  return () => {
    set.delete(onInvalidate)
    if (set.size === 0) fsListeners.delete(path)
  }
}

function pageOf(sorted: FileEntry[], query: ListQuery): FileItemPage {
  const slice = sorted.slice(query.offset, query.offset + query.limit)
  return {
    items: slice.map((entry): FileItem => ({ key: entry.id, entry })),
    totalCount: sorted.length,
    hasMore: query.offset + slice.length < sorted.length,
  }
}

class MockDfsFolderReader implements FolderReader {
  readonly capabilities = FOLDER_CAPABILITIES
  readonly url: string
  private readonly path: string

  constructor(url: string) {
    this.url = url
    this.path = dfsPathOf(url) ?? '/'
  }

  async list(query: ListQuery): Promise<FileItemPage> {
    await mockDelay()
    const entries = mockEntriesAtPath(this.path) ?? []
    const sorted = sortEntriesForQuery(entries, query.sortKey, query.sortDir, query.foldersFirst)
    return pageOf(sorted, query)
  }

  async getItem(key: string): Promise<FileItem | null> {
    const entry = mockEntryById(key)
    return entry ? { key, entry } : null
  }

  watch(onInvalidate: () => void): () => void {
    return watchPath(this.path, onInvalidate)
  }

  dispose() {}
}

/** `view://recent` — newest N files of the curated library, modifiedAt desc. */
class MockRecentViewReader implements FolderReader {
  readonly capabilities: LocationCapabilities = {
    ...VIEW_CAPABILITIES,
    defaultSortKey: 'modified',
  }
  readonly meta = {
    title: 'Recent',
    description: 'Files touched most recently across your library.',
  }
  readonly url: string

  constructor(url: string) {
    this.url = url
  }

  private entries(): FileEntry[] {
    return mockLibraryEntries()
      .filter((entry) => entry.kind !== 'folder')
      .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt))
      .slice(0, 50)
  }

  async list(query: ListQuery): Promise<FileItemPage> {
    await mockDelay()
    const sorted = sortEntriesForQuery(
      this.entries(),
      query.sortKey,
      query.sortDir,
      query.foldersFirst,
    )
    return pageOf(sorted, query)
  }

  async getItem(key: string): Promise<FileItem | null> {
    const entry = mockEntryById(key)
    return entry ? { key, entry } : null
  }

  watch(): () => void {
    return () => {}
  }

  dispose() {}
}

/** `view://topic/<id>` — the old topic:// aggregation, now a regular view reader. */
class MockTopicViewReader implements FolderReader {
  readonly capabilities = VIEW_CAPABILITIES
  readonly meta: { title: string; description?: string }
  readonly url: string
  private readonly topic: Topic | null

  constructor(url: string, topicId: string) {
    this.url = url
    this.topic = fileBrowserSnapshot.topics.find((topic) => topic.id === topicId) ?? null
    this.meta = this.topic
      ? { title: this.topic.title, description: this.topic.reason }
      : { title: topicId, description: 'Unknown topic' }
  }

  private entries(): FileEntry[] {
    if (!this.topic) return []
    const ids = new Set(this.topic.groups.flatMap((group) => group.fileIds))
    return [...ids]
      .map((id) => mockEntryById(id))
      .filter((entry): entry is FileEntry => !!entry)
  }

  async list(query: ListQuery): Promise<FileItemPage> {
    await mockDelay()
    const sorted = sortEntriesForQuery(
      this.entries(),
      query.sortKey,
      query.sortDir,
      query.foldersFirst,
    )
    return pageOf(sorted, query)
  }

  async getItem(key: string): Promise<FileItem | null> {
    const entry = mockEntryById(key)
    return entry ? { key, entry } : null
  }

  watch(): () => void {
    return () => {}
  }

  dispose() {}
}

/** Stub for unknown view types — empty, read-only. */
class EmptyViewReader implements FolderReader {
  readonly capabilities = VIEW_CAPABILITIES
  readonly meta = { title: 'Unknown view' }
  readonly url: string

  constructor(url: string) {
    this.url = url
  }

  async list(): Promise<FileItemPage> {
    return { items: [], totalCount: 0, hasMore: false }
  }

  async getItem(): Promise<FileItem | null> {
    return null
  }

  watch(): () => void {
    return () => {}
  }

  dispose() {}
}

export function registerMockReaders() {
  registerReaderProvider({
    scheme: 'dfs',
    create: (url) => new MockDfsFolderReader(url),
  })
  registerReaderProvider({
    scheme: 'view',
    create: (url) => {
      const parts = parseViewUrl(url)
      if (parts?.viewType === 'recent') return new MockRecentViewReader(url)
      if (parts?.viewType === 'topic' && parts.rest[0]) {
        return new MockTopicViewReader(url, parts.rest[0])
      }
      return new EmptyViewReader(url)
    },
  })
  // Reference targets: dfs:// resolves against the same mock source.
  registerTargetResolver('dfs', async (targetUrl) => {
    const path = dfsPathOf(targetUrl)
    if (!path) return null
    return mockEntryByPath(path) ?? null
  })
}
