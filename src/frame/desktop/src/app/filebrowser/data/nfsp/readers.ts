/**
 * NFSP-backed FolderReaders for dfs:// folders and view:// locations
 * (UI_DATAMODEL.md §8).
 *
 * The reader translates the UI's offset-shaped window into the NFSP cursor
 * chain: pages are fetched sequentially, each response's `next_cursor` is
 * remembered at its offset, and no random access is ever invented from an
 * unknown cursor (§5.1). `totalCount` is never promised — FileItemList's
 * unknown-total mode (`loadedCount + hasMore` + sentinel row) carries the
 * rendering. A `revision_changed` cursor response discards the whole logical
 * result by firing invalidation (the list restarts from offset 0).
 */

import type { Listing, ListOptions, LocatorLike, WantGroup } from '../../../../api/nfsp_client'
import type {
  FileItem,
  FileItemPage,
  FolderReader,
  ListQuery,
  LocationCapabilities,
  LocationMeta,
} from '../FolderReader'
import { registerReaderProvider } from '../readerRegistry'
import { registerTargetResolver } from '../targetResolver'
import { dfsPathOf, parseViewUrl } from '../urls'
import { effectiveListLimit, ensureSession, nfspClient } from './client'
import { nfspToError } from './errors'
import { watchDfsPath } from './invalidation'
import {
  mapCapabilities,
  mapEntryToItem,
  mapLocationMeta,
  NFSP_ORDER_BY_SORT_KEY,
  NFSP_SORT_DIRS,
  refIdOf,
} from './mapping'

const LIST_WANT: WantGroup[] = ['base', 'access']

const PROVISIONAL_FOLDER: LocationCapabilities = {
  kind: 'folder',
  acceptsContent: true,
  acceptsReferences: false,
  removal: 'destroy',
  canReorder: false,
  sortKeys: ['name', 'size', 'modified'],
  sortDirs: NFSP_SORT_DIRS,
  defaultSortKey: 'name',
}

const PROVISIONAL_VIEW: LocationCapabilities = {
  kind: 'view',
  acceptsContent: false,
  acceptsReferences: false,
  removal: null,
  canReorder: false,
  sortKeys: ['manual', 'name', 'size', 'modified'],
  sortDirs: NFSP_SORT_DIRS,
  defaultSortKey: 'manual',
}

/**
 * Base reader over one NFSP container. Subclasses supply the locator (a dfs
 * path, a view/collection URI, or a walked group Ref) and the entry mapping.
 */
export abstract class NfspContainerReader implements FolderReader {
  readonly url: string

  protected caps: LocationCapabilities
  protected metaInfo?: LocationMeta
  /** DFS display path when this container is a dfs folder (entry path base). */
  protected dfsPath: string | null = null

  /** Cursor chain for the current (order) — cursor to pass at each offset. */
  private cursorByOffset = new Map<number, string>()
  private chainOrder: string | undefined
  private itemsByKey = new Map<string, FileItem>()

  /** Container identities seen in responses — watch events match on these. */
  protected containerRefIds = new Set<string>()
  private localListeners = new Set<() => void>()
  private clientUnsub: (() => void) | null = null
  private pathUnsub: (() => void) | null = null

  constructor(url: string, provisional: LocationCapabilities) {
    this.url = url
    this.caps = provisional
  }

  get capabilities(): LocationCapabilities {
    return this.caps
  }

  get meta(): LocationMeta | undefined {
    return this.metaInfo
  }

  /** Locator of the container (may resolve asynchronously for groups). */
  protected abstract locator(): Promise<LocatorLike>

  /**
   * Cache mode for list calls. Group containers bypass the cache: direct
   * collection_patch invalidation and watch events address the collection
   * root, so cached group pages could go stale otherwise.
   */
  protected cacheMode(): 'no-cache' | undefined {
    return undefined
  }

  /** Map one listing into FileItems (folder/view vs collection occurrences). */
  protected abstract mapListing(listing: Listing, offset: number): FileItem[]

  async list(query: ListQuery): Promise<FileItemPage> {
    await ensureSession()
    const at = await this.locator()
    const order = NFSP_ORDER_BY_SORT_KEY[query.sortKey]
    if (order !== this.chainOrder) {
      this.chainOrder = order
      this.cursorByOffset.clear()
    }
    const limit = effectiveListLimit(query.limit)
    const listing = await this.fetchAt(at, query.offset, order, limit)

    // Refine capabilities/meta from the authoritative container info.
    this.adoptContainer(listing)

    if (listing.revision_changed) {
      // A cursor page from another revision must never be merged (§5.1):
      // discard the accumulated window by restarting the logical result.
      this.cursorByOffset.clear()
      this.notifyLocal()
      throw nfspToError({ code: 'STALE', message: 'revision changed' })
    }

    const entryCount = listing.entries.length
    if (listing.next_cursor) {
      this.cursorByOffset.set(query.offset + entryCount, listing.next_cursor)
    }
    const items = this.mapListing(listing, query.offset)
    for (const item of items) this.itemsByKey.set(item.key, item)
    return {
      items,
      hasMore: listing.truncated === true,
    }
  }

  private async fetchAt(
    at: LocatorLike,
    offset: number,
    order: string | undefined,
    limit: number,
  ): Promise<Listing> {
    const client = nfspClient()
    const base: ListOptions = { limit }
    if (order) base.order = order as ListOptions['order']
    const opts = { cache: this.cacheMode() }
    if (offset === 0) {
      const listing = await client.list(at, base, LIST_WANT, opts)
      if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'listing unavailable' })
      return listing
    }
    const known = this.cursorByOffset.get(offset)
    if (known) {
      const listing = await client.list(at, { ...base, cursor: known }, LIST_WANT, opts)
      if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'listing unavailable' })
      return listing
    }
    // Chain lost (e.g. reader revived): rebuild sequentially from the start —
    // never invent random access from an unknown cursor (§5.1).
    let position = 0
    let listing = await client.list(at, base, LIST_WANT, opts)
    for (;;) {
      if (!listing) throw nfspToError({ code: 'NOT_FOUND', message: 'listing unavailable' })
      if (listing.revision_changed || position === offset) return listing
      const nextPosition = position + listing.entries.length
      if (listing.next_cursor) this.cursorByOffset.set(nextPosition, listing.next_cursor)
      if (!listing.next_cursor || nextPosition > offset) {
        // The result shrank below the requested offset: empty page ends it.
        return { ...listing, entries: [], truncated: false, next_cursor: undefined }
      }
      position = nextPosition
      listing = await client.list(at, { ...base, cursor: listing.next_cursor }, LIST_WANT, opts)
    }
  }

  protected adoptContainer(listing: Listing) {
    const container = listing.container
    this.containerRefIds.add(refIdOf(container.ref))
    this.caps = this.refineCapabilities(mapCapabilities(container.kind, container.capabilities))
    const meta = this.locationMetaOf(listing)
    if (meta) this.metaInfo = meta
  }

  /** Hook: subclasses may pin parts of the capability projection. */
  protected refineCapabilities(mapped: LocationCapabilities): LocationCapabilities {
    return mapped
  }

  protected locationMetaOf(listing: Listing): LocationMeta | undefined {
    return mapLocationMeta(listing.container)
  }

  async getItem(key: string): Promise<FileItem | null> {
    return this.itemsByKey.get(key) ?? null
  }

  protected notifyLocal() {
    for (const listener of this.localListeners) listener()
  }

  watch(onInvalidate: () => void): () => void {
    this.localListeners.add(onInvalidate)
    if (!this.clientUnsub) {
      // Cache-layer signal: watch SSE container_changed/resync and background
      // revalidation both land here (§8.4).
      this.clientUnsub = nfspClient().onInvalidate((ref) => {
        if (ref === null || this.containerRefIds.has(refIdOf(ref))) this.notifyLocal()
      })
    }
    if (!this.pathUnsub && this.dfsPath !== null) {
      // Same-client direct writes (folder ops / upload commits) notify here.
      this.pathUnsub = watchDfsPath(this.dfsPath, () => this.notifyLocal())
    }
    return () => {
      this.localListeners.delete(onInvalidate)
    }
  }

  dispose() {
    this.localListeners.clear()
    this.clientUnsub?.()
    this.clientUnsub = null
    this.pathUnsub?.()
    this.pathUnsub = null
    this.cursorByOffset.clear()
    this.itemsByKey.clear()
  }
}

/** dfs:// folder reader. */
class NfspDfsReader extends NfspContainerReader {
  constructor(url: string) {
    super(url, PROVISIONAL_FOLDER)
    this.dfsPath = dfsPathOf(url) ?? '/'
  }

  protected async locator(): Promise<LocatorLike> {
    return this.dfsPath ?? '/'
  }

  protected mapListing(listing: Listing): FileItem[] {
    return listing.entries.map((entry) =>
      mapEntryToItem(entry, { containerDfsPath: this.dfsPath ?? '/' }),
    )
  }
}

/** view://<type>[/<rest>] reader — resolves the server view id from the URL. */
class NfspViewReader extends NfspContainerReader {
  private readonly viewUri: string

  constructor(url: string, viewId: string) {
    super(url, PROVISIONAL_VIEW)
    this.viewUri = `view://${viewId}`
  }

  protected async locator(): Promise<LocatorLike> {
    return { uri: this.viewUri }
  }

  protected mapListing(listing: Listing): FileItem[] {
    return listing.entries.map((entry) => mapEntryToItem(entry, {}))
  }
}

/** UI view URL → server view id: `view://topic/<id>` targets the topic view id. */
function viewIdOf(parts: { viewType: string; rest: string[] }): string {
  if (parts.viewType === 'topic' && parts.rest[0]) return parts.rest[0]
  return [parts.viewType, ...parts.rest].join('/')
}

export function registerNfspReaders() {
  registerReaderProvider({
    scheme: 'dfs',
    create: (url) => new NfspDfsReader(url),
  })
  registerReaderProvider({
    scheme: 'view',
    create: (url) => {
      const parts = parseViewUrl(url)
      return new NfspViewReader(url, parts ? viewIdOf(parts) : url.slice('view://'.length))
    },
  })
  // Reference-target resolver: a dfs canonical URL resolves through stat.
  registerTargetResolver('dfs', async (targetUrl) => {
    const path = dfsPathOf(targetUrl)
    if (!path) return null
    await ensureSession()
    const info = await nfspClient().resolve(path, ['base', 'access'])
    if (!info) return null
    const name = path.split('/').filter(Boolean).pop() ?? path
    const item = mapEntryToItem(
      {
        name,
        binding: 'native',
        target: {
          ref: info.ref,
          kind: info.kind,
          attrs: {
            size: info.size,
            mtime: info.mtime,
            access_urls: info.access_urls,
          },
        },
        canonical_path: `cyfs://${path}`,
      },
      {},
    )
    return item.entry
  })
}
