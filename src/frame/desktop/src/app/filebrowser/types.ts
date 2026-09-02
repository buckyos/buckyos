/**
 * File browser UI data model.
 *
 * The shapes here are the formal UI-facing data contract described in
 * `./UI_DATAMODEL.md` (§2). They drive the mock fixtures in `./mock/` and are
 * the boundary a future NFSP adapter maps onto — NFSP wire types never leak
 * into components.
 */

// ─── Semantic aliases (§2.1) ───
// They document meaning without branded-string runtime wrappers.

/** Target identity, mapped from a stable backend Ref — never a path. */
export type FileEntryId = string
/** Listing-occurrence identity; unique within one listing only. */
export type ListItemKey = string
export type CollectionId = string
export type TopicId = string
export type DeviceId = string
/** Canonical location URL (dfs:// | view:// | collection://). */
export type LocationUrl = string
/** A DFS display/locator path — not persistent identity. */
export type DfsPath = string
/** ISO-8601 timestamp string. */
export type ISODateTime = string

export type LocationKind = 'folder' | 'view' | 'collection'

/**
 * Presentation classification, not backend NodeKind. Unknown/unsupported
 * backend types render as `other`.
 */
export type FileKind =
  | 'folder'
  | 'image'
  | 'document'
  | 'video'
  | 'audio'
  | 'archive'
  | 'code'
  | 'other'

export type ViewMode = 'list' | 'icon'

/**
 * Toolbar sort model — pushed down to the FolderReader as part of the
 * ListQuery; the UI never sorts items itself. `manual` is the collection
 * member order (sortDir is ignored there).
 */
export type SortKey = 'manual' | 'name' | 'size' | 'modified' | 'kind'
export type SortDir = 'asc' | 'desc'

// ─── File entity and attached context (§2.3) ───

export interface FileExif {
  camera?: string
  takenAt?: ISODateTime | string
  location?: string
  lens?: string
}

export type FileSourceType =
  | 'local'
  | 'telegram'
  | 'shared'
  | 'friend-upload'
  | 'system'

export interface FileSource {
  type: FileSourceType
  /** Human-readable provenance; never used as identity. */
  label: string
}

export interface FileLink {
  /** Canonical target locator. Persistence MUST use the resolved target Ref. */
  targetUrl: LocationUrl
  /** True when the target is stale, missing, or inaccessible. */
  broken?: boolean
}

export interface StoryEntry {
  id: string
  kind: 'chat' | 'share' | 'session' | 'note'
  title: string
  excerpt: string
  occurredAt: ISODateTime
  source?: string
}

/**
 * UI projection for one underlying file/folder-like target.
 *
 * Optional enrichment fields stay absent when unknown — adapters must not
 * fabricate empty strings, zero sizes, or false metadata to satisfy rendering.
 */
export interface FileEntry {
  id: FileEntryId
  name: string
  kind: FileKind
  /** Current original DFS/display location, not persistent identity. */
  path: DfsPath | LocationUrl
  /** Prototype-only device anchor until device:// or referral integration exists. */
  devicePath?: string
  /** Externally usable URL supplied by access metadata. */
  publicUrl?: string
  /** Omitted for folders and when the source did not request base attributes. */
  sizeBytes?: number
  modifiedAt: ISODateTime
  /** Sidebar/tree convenience only; listing contents come from a reader. */
  children?: FileEntry[]
  /** Derived summary of applicable trigger policy. */
  triggersActive?: boolean
  tags?: string[]
  topicIds?: TopicId[]
  summary?: string
  exif?: FileExif
  source?: FileSource
  story?: StoryEntry[]
  /** Item-level soft link; independent of Collection membership. */
  link?: FileLink
}

// ─── Sidebar projections (§2.4) ───
// Separate sources, each with its own loading/error state — not one combined
// bootstrap DTO (see FileBrowserSidebarState in data/state.ts).

export interface DfsNode {
  id: string
  name: string
  path: DfsPath | LocationUrl
  icon?: string
  kind: 'home' | 'public' | 'shared' | 'privacy' | 'generic'
  children?: DfsNode[]
}

export interface DeviceRoot {
  path: string
  label: string
}

export interface DeviceNode {
  id: DeviceId
  name: string
  host: string
  status: 'online' | 'offline' | 'syncing'
  roots: DeviceRoot[]
}

export interface TopicGroup {
  id: string
  label: string
  axis: 'source' | 'location' | 'kind' | 'people' | 'time'
  fileIds: FileEntryId[]
}

export interface Topic {
  id: TopicId
  title: string
  description: string
  /** User-visible explanation of why the grouping exists. */
  reason: string
  coverageCount: number
  updatedAt: ISODateTime
  groups: TopicGroup[]
}

export interface CollectionSummary {
  id: CollectionId
  title: string
  /** Recursive reference count; Collection groups are not counted. */
  refCount: number
}

// ─── Search result model (§2.9) ───
// The rendered shape: full projected entry + explanation. Reasons are an
// extensible registry — unknown reasons render in a generic section, never
// dropped.

export type SearchReason =
  | 'filename'
  | 'folder'
  | 'fulltext'
  | 'ai_semantic'
  | 'ai_topic'
  | (string & {})

export interface SearchResultItem {
  entry: FileEntry
  reason: SearchReason
  /** User-visible evidence/explanation, already safe to display. */
  detail: string
  score?: number
}

export interface SearchSourceStatus {
  mode: string
  state: 'ok' | 'degraded' | 'error' | (string & {})
  tookMs?: number
  reason?: string
}

export interface SearchResultPage {
  items: SearchResultItem[]
  partial: boolean
  sources: SearchSourceStatus[]
  nextCursor?: string
}

// ─── Browser runtime state (§2.11) ───
// Session-local; never persisted as-is (Sets/Maps/pending promises would need
// normalization first). Recently closed tabs retain at most ten items.

export interface BrowserTab {
  id: string
  title: string
  /** Canonical location URL after adoption into a pane. */
  path: LocationUrl
}

export interface HistoryState {
  back: LocationUrl[]
  forward: LocationUrl[]
}

export interface ClipboardState {
  entries: FileEntry[]
  mode: 'cut' | 'copy'
}
