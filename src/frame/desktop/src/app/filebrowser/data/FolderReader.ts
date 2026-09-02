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
 *
 * Readers own query execution, paging, item-key construction, error
 * normalization, and invalidation (UI_DATAMODEL.md §2.6).
 */

import type {
  FileEntry,
  ListItemKey,
  LocationKind,
  LocationUrl,
  SortDir,
  SortKey,
} from '../types'

export type { LocationKind }

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

/** Display meta for the location itself (view condition, collection title…). */
export interface LocationMeta {
  title: string
  description?: string
}

export interface ListQuery {
  sortKey: SortKey
  /** Ignored when sortKey = 'manual' (always the collection-defined order). */
  sortDir: SortDir
  /** Folders/groups first — pushed down to the reader, the UI never re-sorts. */
  foldersFirst: boolean
  /** UI window offset; an NFSP adapter translates it through a cursor chain. */
  offset: number
  limit: number
}

/** Collection occurrence context — the "collection side" of the dual path. */
export interface FileItemReference {
  /** Owning Collection/group location. */
  collectionUrl: LocationUrl
  /** Path of this occurrence inside the Collection. */
  refPath: LocationUrl
  /** Zero-based manual order within the current group. */
  orderIndex: number
  broken?: boolean
}

/**
 * A list element. NOT the same as a file entity: the same file can appear in
 * several collections (or twice in one), so list identity is `key`, never
 * `entry.id`.
 */
export interface FileItem {
  /** Unique within the listing: folder/view = entry.id, collection = ref key. */
  key: ListItemKey
  entry: FileEntry
  /** Present for Collection occurrences, including Collection groups. */
  ref?: FileItemReference
  // entry.path is always the original path (path 2) — it never changes just
  // because the file shows up inside a collection or view.
}

export interface FileItemPage {
  items: FileItem[]
  /** Optional because cursor pages do not promise a total. */
  totalCount?: number
  /** Required so unknown totals degrade the UI to a load-more mode. */
  hasMore: boolean
}

export interface FolderReader {
  readonly url: LocationUrl
  readonly capabilities: LocationCapabilities
  readonly meta?: LocationMeta
  list(query: ListQuery): Promise<FileItemPage>
  /** Single item by key (PreviewPanel / selection restore). */
  getItem(key: ListItemKey): Promise<FileItem | null>
  /**
   * Invalidation signal (upload finished, collection mutated in another pane,
   * refresh). Mock folders may no-op; collections must fire for real — the
   * dual-pane consistency depends on it.
   */
  watch(onInvalidate: () => void): () => void
  dispose(): void
}
