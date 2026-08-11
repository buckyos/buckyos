/**
 * FolderReader — the data-source abstraction behind every browsable location.
 *
 * A pane URL resolves (via the reader registry) to one FolderReader whose
 * `kind` encodes the product's three location types:
 *
 *   folder      storage destination, writable, delete destroys data
 *   view        query-derived, strictly read-only
 *   collection  ordered set of references, members managed but never storage
 *
 * The UI consumes readers exclusively through FileItemList/useFolderList and
 * trims interactions by `LocationCapabilities` — nothing is hardcoded per
 * scheme. Swapping mocks for the real DFS backend means writing a new reader;
 * the UI does not change.
 */

import type { FileEntry, SortDir, SortKey } from '../types'

export type LocationKind = 'folder' | 'view' | 'collection'

export interface LocationCapabilities {
  kind: LocationKind
  /** Folder-only: real storage destination (upload / new / paste content). */
  acceptsContent: boolean
  /** Collection-only: existing files/folders can be added as references. */
  acceptsReferences: boolean
  /** Delete semantics: destroy data (folder), drop the reference (collection), or none (view). */
  removal: 'destroy' | 'remove-ref' | null
  /** Collection-only: manual ordering available (sortKeys contains 'manual'). */
  canReorder: boolean
  sortKeys: SortKey[]
  /** Sort key a pane should fall back to when entering this location. */
  defaultSortKey: SortKey
}

export interface ListQuery {
  sortKey: SortKey
  /** Ignored when sortKey = 'manual' (always the collection-defined order). */
  sortDir: SortDir
  /** Folders/groups first — pushed down to the reader, the UI never re-sorts. */
  foldersFirst: boolean
  offset: number
  limit: number
}

/**
 * A list element. NOT the same as a file entity: the same file can appear in
 * several collections (or twice in one), so list identity is `key`, never
 * `entry.id`.
 */
export interface FileItem {
  /** Unique within the listing: folder/view = entry.id, collection = ref key. */
  key: string
  entry: FileEntry
  /** Collection listings only — the "collection side" of the dual path. */
  ref?: {
    /** Owning collection url (including the group path). */
    collectionUrl: string
    /** Path 1: the path of this item as seen through the collection. */
    refPath: string
    /** Manual order inside its group. */
    orderIndex: number
    broken?: boolean
  }
  // entry.path is always the original path (path 2) — it never changes just
  // because the file shows up inside a collection or view.
}

export interface FileItemPage {
  items: FileItem[]
  /** Unknown totals degrade the UI to a "load more" mode. */
  totalCount?: number
  hasMore: boolean
}

export interface FolderReader {
  readonly url: string
  readonly capabilities: LocationCapabilities
  /** Display meta for the location itself (view condition, collection title…). */
  readonly meta?: { title: string; description?: string }
  list(query: ListQuery): Promise<FileItemPage>
  /** Single item by key (PreviewPanel / selection restore). */
  getItem(key: string): Promise<FileItem | null>
  /**
   * Invalidation signal (upload finished, collection mutated in another pane,
   * refresh). Mock folders may no-op; collections must fire for real — the
   * dual-pane consistency depends on it.
   */
  watch(onInvalidate: () => void): () => void
  dispose(): void
}
