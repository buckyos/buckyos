/**
 * FileItemList — stateful loaded-window view model over one
 * (reader, sortKey, sortDir) combination.
 *
 * Holds a sparse map of loaded items so virtual scrolling can demand-load by
 * index; requests are page-aligned (PAGE_SIZE) and deduped in flight, and a
 * version token discards stale responses after reload / query changes.
 */

import type { SortDir, SortKey } from '../types'
import type { FileItem, FolderReader, LocationCapabilities } from './FolderReader'
import { resolveReader } from './readerRegistry'

export const PAGE_SIZE = 200

export type FileItemListStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface FileItemList {
  readonly url: string
  /** Monotonic change counter (useSyncExternalStore snapshot / effect dep). */
  readonly snapshot: number
  /** Pass-through of the reader's — the UI's single point of consumption. */
  readonly capabilities: LocationCapabilities
  readonly meta?: { title: string; description?: string }
  /** Unknown while no page answered yet — UI shows "…" / load-more mode. */
  readonly totalCount: number | undefined
  readonly status: FileItemListStatus
  readonly error: Error | null
  /** Undefined for not-yet-loaded positions → skeleton placeholder rows. */
  itemAt(index: number): FileItem | undefined
  /** Call on visible-range changes; merges/dedupes page requests internally. */
  ensureRange(start: number, end: number): void
  loadedItemByKey(key: string): FileItem | undefined
  /** Ordered keys of currently loaded items (shift range selection). */
  loadedKeys(): string[]
  reload(): void
}

export class FileItemListImpl implements FileItemList {
  readonly url: string

  private reader: FolderReader | null = null
  private unwatch: (() => void) | null = null
  private query: { sortKey: SortKey; sortDir: SortDir } | null = null

  private items = new Map<number, FileItem>()
  private byKey = new Map<string, FileItem>()
  private orderedKeysCache: string[] | null = null

  private versionToken = 0
  private inFlightPages = new Set<number>()
  private lastRange: [number, number] = [0, PAGE_SIZE - 1]

  private listeners = new Set<() => void>()
  /** Monotonic snapshot version for useSyncExternalStore. */
  snapshot = 0

  totalCount: number | undefined = undefined
  status: FileItemListStatus = 'idle'
  error: Error | null = null

  constructor(url: string) {
    this.url = url
  }

  private ensureReader(): FolderReader {
    if (!this.reader) {
      this.reader = resolveReader(this.url)
      this.unwatch = this.reader.watch(() => this.reload())
    }
    return this.reader
  }

  get capabilities(): LocationCapabilities {
    return this.ensureReader().capabilities
  }

  get meta(): { title: string; description?: string } | undefined {
    return this.ensureReader().meta
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  private emit() {
    this.snapshot += 1
    for (const listener of this.listeners) listener()
  }

  /** Set (or change) the sort query. Changes drop all loaded data and refetch. */
  setQuery(sortKey: SortKey, sortDir: SortDir) {
    const same = this.query && this.query.sortKey === sortKey && this.query.sortDir === sortDir
    if (same && this.status !== 'idle') return
    this.query = { sortKey, sortDir }
    this.restart()
  }

  reload() {
    if (!this.query) return
    this.restart()
  }

  /** Drop loaded data, invalidate in-flight responses, refetch the last window. */
  private restart() {
    this.versionToken += 1
    this.inFlightPages.clear()
    this.items.clear()
    this.byKey.clear()
    this.orderedKeysCache = null
    this.totalCount = undefined
    this.error = null
    this.status = 'loading'
    this.emit()
    this.fetchRange(this.lastRange[0], this.lastRange[1])
  }

  itemAt(index: number): FileItem | undefined {
    return this.items.get(index)
  }

  loadedItemByKey(key: string): FileItem | undefined {
    return this.byKey.get(key)
  }

  loadedKeys(): string[] {
    if (!this.orderedKeysCache) {
      this.orderedKeysCache = [...this.items.entries()]
        .sort((a, b) => a[0] - b[0])
        .map(([, item]) => item.key)
    }
    return this.orderedKeysCache
  }

  ensureRange(start: number, end: number) {
    const max = this.totalCount !== undefined ? this.totalCount - 1 : end
    const safeStart = Math.max(0, start)
    const safeEnd = Math.max(safeStart, Math.min(end, Math.max(max, 0)))
    this.lastRange = [safeStart, safeEnd]
    this.fetchRange(safeStart, safeEnd)
  }

  private fetchRange(start: number, end: number) {
    if (!this.query) return
    const firstPage = Math.floor(start / PAGE_SIZE)
    const lastPage = Math.floor(end / PAGE_SIZE)
    for (let page = firstPage; page <= lastPage; page += 1) {
      this.fetchPage(page)
    }
  }

  private pageLoaded(page: number): boolean {
    const offset = page * PAGE_SIZE
    const limit =
      this.totalCount !== undefined
        ? Math.min(PAGE_SIZE, Math.max(this.totalCount - offset, 0))
        : PAGE_SIZE
    if (limit === 0) return true
    for (let i = 0; i < limit; i += 1) {
      if (!this.items.has(offset + i)) return false
    }
    return true
  }

  private fetchPage(page: number) {
    if (this.inFlightPages.has(page) || this.pageLoaded(page)) return
    const token = this.versionToken
    const query = this.query
    if (!query) return
    this.inFlightPages.add(page)
    this.ensureReader()
      .list({
        sortKey: query.sortKey,
        sortDir: query.sortDir,
        foldersFirst: query.sortKey !== 'manual',
        offset: page * PAGE_SIZE,
        limit: PAGE_SIZE,
      })
      .then((result) => {
        if (token !== this.versionToken) return
        this.inFlightPages.delete(page)
        const offset = page * PAGE_SIZE
        result.items.forEach((item, i) => {
          this.items.set(offset + i, item)
          this.byKey.set(item.key, item)
        })
        if (result.totalCount !== undefined) {
          this.totalCount = result.totalCount
          // A shrunken total (e.g. members removed) invalidates tail indexes.
          for (const index of [...this.items.keys()]) {
            if (index >= result.totalCount) {
              const stale = this.items.get(index)
              this.items.delete(index)
              if (stale) this.byKey.delete(stale.key)
            }
          }
        } else if (!result.hasMore) {
          this.totalCount = offset + result.items.length
        }
        this.orderedKeysCache = null
        this.status = 'ready'
        this.error = null
        this.emit()
      })
      .catch((err: unknown) => {
        if (token !== this.versionToken) return
        this.inFlightPages.delete(page)
        this.status = 'error'
        this.error = err instanceof Error ? err : new Error(String(err))
        this.emit()
      })
  }

  /**
   * Release the reader + watch. The list revives lazily if used again
   * (StrictMode double-invokes effects), so this is safe to call from
   * effect cleanup.
   */
  dispose() {
    this.unwatch?.()
    this.unwatch = null
    this.reader?.dispose()
    this.reader = null
    this.versionToken += 1
    this.inFlightPages.clear()
    // Back to idle so a StrictMode re-mount's setQuery() restarts the fetch
    // (the same-query dedupe would otherwise leave it loading forever).
    this.status = 'idle'
  }
}
