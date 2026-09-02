# File Browser UI DataModel

> Status: v1.2 — v1 extracted from the converged mock-first prototype; v1.1
> recorded the frontend-side deltas of §9 (items 1–5 and 11); v1.2 records the
> NFSP backend-integration stage: the adapter in <code>data/nfsp/</code> is
> implemented (§9 items 6–9 done, 10/12 still gated) and installation switches
> by runtime (<code>data/install.ts</code>)  
> Scope: <code>src/frame/desktop/src/app/filebrowser/</code>  
> Last reviewed: 2026-09-02

## 1. Overview

This document is the formal UI-facing data contract for BuckyOS File Browser. It records the
model proven by the current desktop/mobile prototype and makes the boundary explicit for a later
NFSP backend integration.

Sources, in order of authority:

1. The behavior implemented in <code>FileBrowserView.tsx</code>, <code>MainContent.tsx</code>,
   <code>PreviewPanel.tsx</code>, and <code>Sidebar.tsx</code>.
2. The data-source abstractions in <code>data/</code> and the executable fixtures in
   <code>mock/</code>.
3. The product intent in <code>product/bucky_file/filebrowser_PRD.md</code>.
4. The backend boundary described by <code>product/bucky_file/nfs_server.md</code>,
   <code>src/frame/desktop/src/api/nfsp_client.ts</code>, and
   <code>src/frame/desktop/src/api/nfs_browser_client.ts</code>.

The prototype currently installs mock readers only. The existing NFSP clients are protocol
references for the integration stage; their wire types are not the UI DataModel.

### 1.1 Covered views

- DFS logical folders, including the special public subtree.
- Read-only derived views: Recent and AI Topic.
- User-managed ordered Collections and nested Collection groups.
- Advanced device navigation placeholder.
- List, icon-grid, desktop split-pane, tabs, and mobile layouts.
- Search results grouped by traditional and AI-derived reasons.
- Preview, Meta, Story, AI-trigger explanation, status bar, and public URL.
- Desktop context menu and mobile action sheet built from the same menu model.
- Multi-selection, clipboard intent, recently closed tabs, and navigation history.

### 1.2 Model layers

~~~
Browser session state
  └─ Pane state: tabs, location, sort, selection, search, view mode
       └─ FileItemList: sparse loaded window for virtualized rendering
            └─ FolderReader: one reader for any browsable location
                 ├─ mock readers today
                 └─ NFSP adapter during backend integration
~~~

The layers intentionally keep four identities separate:

| Concept | Meaning | Identity |
|---|---|---|
| Location | A folder, view, Collection, or Collection group that a pane can open | Canonical location URL |
| File entity | The underlying file/folder-like target | <code>FileEntry.id</code>, mapped from a stable backend Ref |
| Listing item | One occurrence inside one listing | <code>FileItem.key</code> |
| Reference binding | A link/member occurrence pointing at a target | Entry/binding identity, never the target path |

The same file may occur in multiple Collections or more than once in one Collection. Therefore
<code>FileItem.key</code> and <code>FileEntry.id</code> MUST NOT be treated as interchangeable.

### 1.3 Normative rules

- UI components consume <code>FileItemList</code>/<code>FolderReader</code>, not NFSP responses.
- A path or URL is a locator and display value, not the persistent identity of a file or
  Collection member.
- <code>entry.path</code> always means the target's current original location.
- <code>item.ref.refPath</code> means the occurrence's Collection-side path.
- Folder, View, and Collection behavior is capability-driven; components MUST NOT branch on URL
  schemes to decide mutation permissions.
- Sorting and filtering happen in the reader/backend. The virtualized UI MUST NOT sort a partial
  client-side window.
- A Collection stores references and groups, consumes no file-content storage, and removing a
  member never deletes the target.
- A View is derived and read-only. Search is modeled as a query result view; the prototype
  routes it through the async <code>SearchProvider</code>/<code>SearchViewState</code> pair in
  <code>data/search.ts</code> (a View-compatible reader remains a later refinement).
- New enum values are expected. Consumers MUST provide a safe fallback for unknown kinds,
  sources, search reasons, metadata namespaces, and menu commands.

## 2. DataModel definitions

### 2.1 Semantic aliases and enums

The aliases document meaning without requiring branded-string runtime wrappers.

~~~ts
export type FileEntryId = string
export type ListItemKey = string
export type CollectionId = string
export type TopicId = string
export type DeviceId = string
export type LocationUrl = string
export type DfsPath = string
export type ISODateTime = string

export type LocationKind = 'folder' | 'view' | 'collection'

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
export type SortKey = 'manual' | 'name' | 'size' | 'modified' | 'kind'
export type SortDir = 'asc' | 'desc'
~~~

<code>FileKind</code> is a presentation classification, not NFSP
<code>NodeKind</code>. A backend file node is classified using its MIME type/name. Unknown and
unsupported types render as <code>other</code>.

### 2.2 Canonical location URLs

Every tab and pane stores a canonical URL:

| Form | Kind | Example |
|---|---|---|
| <code>dfs://</code> | Folder | <code>dfs:///home/Pictures</code> |
| <code>view://</code> | View | <code>view://recent</code>, <code>view://topic/topic-kyoto</code> |
| <code>collection://</code> | Collection/group | <code>collection://reading-list/papers</code> |

Bare paths normalize to <code>dfs://</code>. Legacy <code>topic://id</code> input normalizes to
<code>view://topic/id</code>. DFS locations are displayed as bare paths; other locations retain
their scheme. Trailing slashes are removed except for the root.

~~~ts
export interface CollectionUrlParts {
  collectionId: CollectionId
  groupPath: string[]
}

export interface ViewUrlParts {
  viewType: string
  rest: string[]
}

export interface UrlCrumb {
  label: string
  url: LocationUrl
}
~~~

Group names used in URLs MUST be percent-encoded. The URL helper
(<code>data/urls.ts</code>) now encodes segments on build and decodes them on parse. In addition,
NFSP resolves a Collection root by URI but identifies a nested group by its
<code>entry_ref</code>. The current name-based <code>groupPath</code> is therefore a display
locator, not durable group identity. Before backend integration, the adapter must either maintain
an entry-ref path behind this URL or replace the nested encoding with an identity-bearing,
deep-link-safe form.

### 2.3 File entity and attached context

~~~ts
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
~~~

Optional enrichment fields MUST remain absent when unknown. The adapter MUST NOT fabricate empty
strings, zero sizes, or false metadata merely to satisfy rendering.

### 2.4 Sidebar projections

~~~ts
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
~~~

<code>DfsNode</code>, <code>DeviceNode</code>, <code>Topic</code>, and
<code>CollectionSummary</code> are sidebar projections, not one combined bootstrap DTO. Each
source has its own loading/error state in the formal state model.

The prototype's <code>TriggerRule</code> and <code>FileBrowserSnapshot</code> are Mock fixture
shapes. No rendered view consumes the trigger list or the snapshot indexes directly, so neither is
a stable integration contract. The rendered stable field is currently
<code>FileEntry.triggersActive</code>; detailed trigger-policy UI requires a later projection.

### 2.5 Location capabilities

~~~ts
export interface LocationCapabilities {
  kind: LocationKind
  /** Real content can be uploaded/created/pasted here. */
  acceptsContent: boolean
  /** Existing targets can be added as references here. */
  acceptsReferences: boolean
  /** Destructive meaning of the visible remove action. */
  removal: 'destroy' | 'remove-ref' | null
  canReorder: boolean
  sortKeys: SortKey[]
  /** Directions the reader can honor; absent = both (NFSP v1: ['asc']). */
  sortDirs?: SortDir[]
  defaultSortKey: SortKey
}

export interface LocationMeta {
  title: string
  description?: string
}
~~~

The required capability matrix is:

| Kind | acceptsContent | acceptsReferences | removal | canReorder | Default sort |
|---|---:|---:|---|---:|---|
| Folder | true | false | <code>destroy</code> | false | <code>name</code> |
| View | false | false | <code>null</code> | false | Reader-defined; Recent uses <code>modified</code> |
| Collection | false | true | <code>remove-ref</code> | true | <code>manual</code> |

<code>sortKeys</code> is capability-negotiated. The UI only shows keys present in the active
location. <code>sortDirs</code> extends the same negotiation to directions: the toolbar
disables directions the reader does not advertise, and <code>FileItemList.setQuery</code>
clamps an unsupported direction at the reader boundary rather than letting a reader silently
return differently ordered pages.

### 2.6 Listing item, page, and reader

~~~ts
export interface FileItemReference {
  /** Owning Collection/group location. */
  collectionUrl: LocationUrl
  /** Path of this occurrence inside the Collection. */
  refPath: LocationUrl
  /** Zero-based manual order within the current group. */
  orderIndex: number
  broken?: boolean
}

export interface FileItem {
  /** Unique and stable within the current listing. */
  key: ListItemKey
  entry: FileEntry
  /** Present for Collection occurrences, including Collection groups. */
  ref?: FileItemReference
}

export interface ListQuery {
  sortKey: SortKey
  /** Ignored for manual order. */
  sortDir: SortDir
  /** Ignored for manual order. */
  foldersFirst: boolean
  /** UI window offset; an NFSP adapter translates it through a cursor chain. */
  offset: number
  limit: number
}

export interface FileItemPage {
  items: FileItem[]
  /** Optional because NFSP cursor pages do not promise a total. */
  totalCount?: number
  hasMore: boolean
}

export interface FolderReader {
  readonly url: LocationUrl
  readonly capabilities: LocationCapabilities
  readonly meta?: LocationMeta
  list(query: ListQuery): Promise<FileItemPage>
  getItem(key: ListItemKey): Promise<FileItem | null>
  watch(onInvalidate: () => void): () => void
  dispose(): void
}

export interface ReaderProvider {
  /** URL scheme without ://, for example dfs, view, or collection. */
  scheme: string
  create(url: LocationUrl): FolderReader
}

export type TargetResolver = (targetUrl: LocationUrl) => Promise<FileEntry | null>
~~~

Reader implementations own query execution, paging, item-key construction, error normalization,
and invalidation. A reader returns a new logical result after invalidation; UI components never
merge backend wire entries themselves.

<code>ReaderProvider</code> is the location extension point. <code>TargetResolver</code> is the
reference-target extension point and returns <code>null</code> for missing, inaccessible, failed,
or unsupported targets so the owning reader can preserve a privacy-safe broken occurrence.

### 2.7 Loaded-window view model

~~~ts
export type FileItemListStatus = 'idle' | 'loading' | 'ready' | 'error'

export interface FileItemList {
  readonly url: LocationUrl
  /** Monotonic render notification token; not domain data. */
  readonly snapshot: number
  readonly capabilities: LocationCapabilities
  readonly meta?: LocationMeta
  readonly totalCount: number | undefined
  readonly status: FileItemListStatus
  readonly error: Error | null
  itemAt(index: number): FileItem | undefined
  ensureRange(start: number, end: number): void
  loadedItemByKey(key: ListItemKey): FileItem | undefined
  loadedKeys(): ListItemKey[]
  reload(): void
}
~~~

<code>FileItemList</code> is a runtime controller/view model, not a serializable DTO. Its sparse
window and request-deduplication details are Volatile; the observable behavior is stable:

- unresolved positions return <code>undefined</code> and render skeleton rows/cells;
- stale responses after reload/query changes are ignored;
- changing sort restarts the logical result;
- invalidation reloads the last visible window;
- selection restoration uses listing keys, never a global mock index.

### 2.8 Collection model and operations

~~~ts
export type CollectionNode =
  | { type: 'ref'; key: ListItemKey; targetUrl: LocationUrl }
  | {
      type: 'group'
      key: ListItemKey
      name: string
      children: CollectionNode[]
    }

export interface CollectionDetail {
  id: CollectionId
  title: string
  description?: string
  nodes: CollectionNode[]
}

export interface CollectionReader extends FolderReader {
  addReferences(targets: LocationUrl[], position?: number): Promise<void>
  removeItems(itemKeys: ListItemKey[]): Promise<void>
  reorder(itemKeys: ListItemKey[], toIndex: number): Promise<void>
  createGroup(name: string, position?: number): Promise<void>
  renameGroup(itemKey: ListItemKey, name: string): Promise<void>
}
~~~

<code>CollectionNode</code> documents the current Mock store. It is not the backend persistence
shape: integration MUST resolve <code>targetUrl</code> to a stable NFSP Ref and map
<code>key</code> to <code>entry_ref</code>. The UI-facing Collection invariants are stable even
though this Mock representation is Volatile.

### 2.9 Search result model

The prototype-proven shape is the rendered object containing the full projected entry and an
explanation. It replaced the earlier unused <code>SearchHit</code> declaration, which has been
removed from <code>types.ts</code>.

~~~ts
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
~~~

Presentation grouping is derived:

- traditional: <code>filename</code>, <code>folder</code>, <code>fulltext</code>;
- AI-enhanced: <code>ai_semantic</code>, <code>ai_topic</code>;
- unknown reasons: an additional generic section or the traditional section, never dropped.

Search results do not change <code>entry.path</code>. Selecting a folder leaves search and
navigates to its original location; selecting a file opens its preview context.

### 2.10 Menu extension model

Desktop and mobile render the same registry output. Renderer details such as icon choice are
extensible, while command context is stable.

~~~ts
export type FileMenuTarget = 'item' | 'selection' | 'view'

export interface FileMenuContext {
  target: FileMenuTarget
  items: FileItem[]
  entries: FileEntry[]
  currentUrl: LocationUrl
  viewMode: ViewMode
  capabilities: LocationCapabilities
  sortKey: SortKey
  collections: Pick<CollectionSummary, 'id' | 'title'>[]
  moveTargets: { label: string; path: DfsPath }[]
  pane: {
    canOpenInNewTab: boolean
    canOpenInRightPane: boolean
  }
}

export interface FileMenuLabel {
  key: string
  fallback: string
  vars?: Record<string, string | number>
}

export type FileMenuIcon =
  | 'open'
  | 'open-new-tab'
  | 'open-right'
  | 'preview'
  | 'copy'
  | 'link'
  | 'share'
  | 'download'
  | 'rename'
  | 'move'
  | 'trash'
  | 'new-folder'
  | 'upload'
  | 'refresh'
  | 'select-all'
  | 'view-list'
  | 'view-icon'
  | 'collection'
  | 'remove-ref'
  | 'jump'
  | 'broken'
  | 'move-up'
  | 'move-down'

export interface FileMenuAction {
  type: 'action'
  id: string
  command: string
  args?: Record<string, unknown>
  label: FileMenuLabel
  icon?: FileMenuIcon
  danger?: boolean
  disabled?: boolean
  shortcut?: string
}

export interface FileMenuSubmenu {
  type: 'submenu'
  id: string
  label: FileMenuLabel
  icon?: FileMenuIcon
  items: FileMenuAction[]
}

export type FileMenuEntryItem = FileMenuAction | FileMenuSubmenu
export type FileMenuSection = FileMenuEntryItem[]

export interface FileMenuProvider {
  id: string
  order: number
  when?: (context: FileMenuContext) => boolean
  build: (context: FileMenuContext) => FileMenuSection[]
}

export interface FileMenuConfig {
  hiddenProviders?: string[]
  hiddenItems?: string[]
  providerOrder?: Record<string, number>
}
~~~

Providers MUST decide visibility from <code>LocationCapabilities</code> and item/reference state.
Command IDs and argument payloads are host-local extension data until separately registered as a
cross-application command contract. Labels are i18n keys plus fallbacks; registry providers contain
no React or desktop/mobile renderer assumptions. Icon identifiers and provider/action IDs are
extensible registries rather than closed backend enums.

### 2.11 Browser runtime state

~~~ts
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

export interface BrowserSelectionState {
  selectedKeys: ReadonlySet<ListItemKey>
  /** Captured items preserve details after a virtual row leaves the DOM. */
  selectedItems: ReadonlyMap<ListItemKey, FileItem>
  anchorKey: ListItemKey | null
}

export interface BrowserPaneState {
  tabs: BrowserTab[]
  activeTabId: string
  historyByTabId: Record<string, HistoryState>
  viewMode: ViewMode
  sortKey: SortKey
  sortDir: SortDir
  searchQuery: string
  selection: BrowserSelectionState
}

export interface ClipboardState {
  entries: FileEntry[]
  mode: 'cut' | 'copy'
}

export interface FileBrowserUiState {
  left: BrowserPaneState
  right: BrowserPaneState
  focusedSide: 'left' | 'right'
  closedTabs: BrowserTab[]
  advancedMode: boolean
  previewCollapsed: boolean
  clipboard: ClipboardState | null
}
~~~

This state is session-local. The prototype does not persist it. If persistence is added, Sets,
Maps, transient menu anchors, toast text, pending promises, and captured <code>File</code> objects
MUST be excluded or normalized first. Recently closed tabs retain at most ten items.

### 2.12 Derived presentation rules

No duplicate persisted “preview DTO” is required. The following projections are derived:

- A preview target exists only when exactly one item is selected.
- Reference path = <code>item.ref.refPath</code> when it differs from
  <code>item.entry.path</code>.
- Original path = <code>entry.link.targetUrl</code> for a link, otherwise
  <code>entry.path</code>.
- Broken reference = <code>item.ref.broken || entry.link.broken</code>.
- Public context = current displayed DFS path equals <code>/public</code> or starts with
  <code>/public/</code>.
- Topic chips = <code>entry.topicIds</code> joined with known <code>Topic.id</code> values;
  unknown topic IDs are ignored without failing the preview.
- Multi-selection bytes = sum of known <code>sizeBytes</code>; unknown sizes contribute zero and
  MUST NOT be presented as proof of a complete total.
- “Select all” currently means all loaded keys. When selected count is less than a known total,
  status displays “selected N of total”.
- Collection reference count recursively counts ref nodes and excludes group nodes.
- Breadcrumbs derive from the canonical URL and optional reader title; breadcrumbs are not stored.

## 3. Input models and validation

These schemas define UI constraints. They are not mechanical copies of NFSP request objects.
Forms introduced during integration MUST use these schemas as the source of
<code>react-hook-form</code> types. They live in <code>data/schemas.ts</code>; the Collection,
group, rename, and new-folder flows now go through the schema-driven
<code>NamePromptDialog</code> (the <code>window.prompt()</code> paths are gone).

~~~ts
import { z } from 'zod'

const utf8Length = (value: string) => new TextEncoder().encode(value).length

export const entryNameSchema = z
  .string()
  .trim()
  .min(1, 'filebrowser.validation.nameRequired')
  .refine((value) => value !== '.' && value !== '..', {
    message: 'filebrowser.validation.reservedName',
  })
  .refine((value) => !value.includes('/') && !value.includes('\\') && !value.includes('\0'), {
    message: 'filebrowser.validation.invalidNameCharacter',
  })
  .refine((value) => utf8Length(value) <= 255, {
    message: 'filebrowser.validation.nameTooLong',
  })

export const collectionTitleSchema = z
  .string()
  .trim()
  .min(1, 'filebrowser.validation.collectionTitleRequired')
  .max(128, 'filebrowser.validation.collectionTitleTooLong')

export const searchInputSchema = z.object({
  query: z
    .string()
    .trim()
    .min(1, 'filebrowser.validation.searchRequired')
    .max(256, 'filebrowser.validation.searchTooLong'),
  scope: z.string().trim().min(1).optional(),
})

export const locationInputSchema = z.object({
  raw: z
    .string()
    .trim()
    .min(1, 'filebrowser.validation.locationRequired')
    .max(2048, 'filebrowser.validation.locationTooLong'),
})

export const listQuerySchema = z
  .object({
    sortKey: z.enum(['manual', 'name', 'size', 'modified', 'kind']),
    sortDir: z.enum(['asc', 'desc']),
    foldersFirst: z.boolean().default(true),
    offset: z.number().int().min(0),
    limit: z.number().int().min(1).max(200).default(200),
  })
  .superRefine((value, context) => {
    if (value.sortKey === 'manual' && value.sortDir !== 'asc') {
      context.addIssue({
        code: 'custom',
        path: ['sortDir'],
        message: 'filebrowser.validation.manualDirectionIgnored',
      })
    }
  })

export const createCollectionInputSchema = z.object({
  title: collectionTitleSchema,
})

export const collectionGroupInputSchema = z.object({
  name: entryNameSchema,
})

export const renameEntryInputSchema = z.object({
  name: entryNameSchema,
})

export const reorderCollectionInputSchema = z.object({
  itemKeys: z.array(z.string().min(1)).min(1),
  toIndex: z.number().int().min(0),
})

export const uploadCandidateSchema = z.object({
  localId: z.string().min(1),
  name: entryNameSchema,
  sizeBytes: z.number().int().min(0),
  mimeType: z.string().max(255).optional(),
  relativePath: z.string().max(2048).optional(),
})

export type SearchInput = z.infer<typeof searchInputSchema>
export type LocationInput = z.infer<typeof locationInputSchema>
export type CreateCollectionInput = z.infer<typeof createCollectionInputSchema>
export type CollectionGroupInput = z.infer<typeof collectionGroupInputSchema>
export type RenameEntryInput = z.infer<typeof renameEntryInputSchema>
export type UploadCandidateInput = z.infer<typeof uploadCandidateSchema>
~~~

Validation notes:

- The 255-byte entry-name limit matches the current nfs-server name boundary; it is bytes, not
  JavaScript character count.
- Collection IDs are server-generated and not user-editable. The Mock slug derived from the title
  is not a stable contract.
- Address input is normalized only after validation. An unregistered or inaccessible scheme is a
  resolvable navigation error, not a string-format error.
- Search is inactive when the field is blank; blank input is not sent to the backend.
- A browser <code>File</code> object is held out-of-band and keyed by
  <code>UploadCandidateInput.localId</code>; it is never serialized into shared state.
- Rename, create-folder, and create-group share the path-segment schema. Collection titles do not
  have path-segment restrictions.

Defaults and examples:

| Input | Valid/default sample | Invalid sample | Expected error |
|---|---|---|---|
| Search | <code>{ query: "trip" }</code> | whitespace only | <code>searchRequired</code> |
| Address | <code>{ raw: "/home/Documents" }</code> | empty | <code>locationRequired</code> |
| Collection | <code>{ title: "Reading List" }</code> | empty | <code>collectionTitleRequired</code> |
| Group | <code>{ name: "papers" }</code> | <code>2026/reports</code> | <code>invalidNameCharacter</code> |
| Rename | <code>{ name: "Plan v2.md" }</code> | <code>..</code> | <code>reservedName</code> |
| List | name/asc, folders first, offset 0, limit 200 | negative offset | numeric range error |
| Upload | known name and non-negative size | name containing NUL | <code>invalidNameCharacter</code> |

Edit-state refill uses the current title/name exactly as displayed, then trims only on submit.

## 4. State definitions

### 4.1 Common state containers

~~~ts
export type LoadingState = 'idle' | 'loading' | 'success' | 'error'

export interface UiError {
  code: string
  messageKey: string
  fallback: string
  retryable: boolean
  details?: Record<string, unknown>
}

export interface DataState<T> {
  status: LoadingState
  data: T | null
  error: UiError | null
}

export type MutationStatus = 'idle' | 'submitting' | 'success' | 'error'

export interface MutationState<TResult = void> {
  status: MutationStatus
  result: TResult | null
  error: UiError | null
}
~~~

The existing <code>FileItemListStatus.ready</code> maps to common
<code>LoadingState.success</code>. <code>FileItemList</code> remains the optimized runtime
controller; <code>DataState</code> is the common state vocabulary for fetch points and
documentation.

### 4.2 Location listing state

~~~ts
export interface ListPageInfo {
  loadedCount: number
  totalCount?: number
  hasMore: boolean
  nextCursor?: string
}

export interface LocationListData {
  url: LocationUrl
  capabilities: LocationCapabilities
  meta?: LocationMeta
  pageInfo: ListPageInfo
}

export type LocationListState = DataState<LocationListData>
~~~

| State | UI treatment |
|---|---|
| Loading | Initial 12-row skeleton; no stale rows on first open |
| Normal | Virtualized list/grid; location banner for View/Collection |
| Empty | Kind-specific explanation; upload action only for content-accepting folders |
| Error | Localized message and Retry; navigation shell remains usable |
| Progress | Existing rows remain usable while later windows load; unresolved visible slots are skeletons |

Reload and sort change restart the logical result. Backend integration may retain stale rows during
revalidation, but it MUST distinguish them visually and MUST not combine pages from different
revisions.

### 4.3 Sidebar source states

The sidebar uses separate states so one unavailable source does not blank the entire browser:

~~~ts
export interface FileBrowserSidebarState {
  dfs: DataState<DfsNode[]>
  devices: DataState<DeviceNode[]>
  topics: DataState<Topic[]>
  collections: DataState<CollectionSummary[]>
}
~~~

| State | UI treatment |
|---|---|
| Loading | Skeleton rows inside the affected section |
| Normal | Nodes with counts/status indicators |
| Empty | Section-specific “none available”; DFS empty is a page-level access problem |
| Error | Inline error and retry for the affected source only |
| Progress | Refresh indicator on the affected section; keep the last successful projection |

The prototype now loads DFS/devices/topics through per-source async fetchers
(<code>data/sidebarSources.ts</code>) with exactly these states; the Collection store remains a
live in-memory projection. A deterministic per-source failure fixture is selectable via
<code>?fbFail=&lt;source&gt;</code>.

### 4.4 Search state

~~~ts
export type SearchViewState = DataState<SearchResultPage>
~~~

| State | UI treatment |
|---|---|
| Idle | Search field closed/blank; current location remains visible |
| Loading | Search-result skeleton, retaining the query and cancel/close action |
| Normal | Results grouped by reason with evidence text |
| Empty | “No results” guidance; query remains editable |
| Error | Search-specific error and retry; current folder state is not lost |
| Progress | Additional cursor page or slower sources loading; show completed sources and partial indicator |

When <code>partial</code> is true or any source is degraded, results remain usable and the UI shows
that coverage is incomplete.

### 4.5 Preview, Meta, and Story state

~~~ts
export interface PreviewData {
  item: FileItem
  topics: Topic[]
}

export type PreviewState = DataState<PreviewData>
~~~

| State | UI treatment |
|---|---|
| Idle/Empty selection | Teaching empty state asking the user to select one item |
| Loading | Selected identity/name remains visible; Meta/Story areas show skeletons |
| Normal | Base attributes, source, public URL, enrichment, Story, and AI policy tabs |
| Empty enrichment | Base attributes remain; individual Story/Meta sections show explicit empty states |
| Error | Preserve base attributes and show an error only in the failed enrichment section |
| Progress | Metadata namespaces may resolve independently; completed sections remain interactive |

Multiple selection intentionally has no preview target.

### 4.6 Collection mutation state

Each create/add/remove/reorder/rename operation owns a <code>MutationState</code>.

| State | UI treatment |
|---|---|
| Idle | Capability-derived actions available |
| Submitting | Disable duplicate submission; show action-local progress |
| Success | Reader invalidation/reload is the source of visible truth; optional toast |
| Error | Keep selection and inputs; show normalized error and Retry where safe |
| Conflict progress | On revision conflict, re-list before offering a replay; never silently overwrite order |

Optimistic reordering is optional. If used, it MUST roll back on failure and still reconcile
against the returned/watch revision.

### 4.7 Upload/transfer progress

~~~ts
export type TransferStatus =
  | 'queued'
  | 'hashing'
  | 'probing'
  | 'uploading'
  | 'committing'
  | 'success'
  | 'error'
  | 'cancelled'

export interface TransferTask {
  id: string
  targetUrl: LocationUrl
  candidate: UploadCandidateInput
  status: TransferStatus
  bytesSent: number
  totalBytes: number
  error: UiError | null
}
~~~

| State | UI treatment |
|---|---|
| Empty | No active or recent transfers |
| Queued/loading | Local placeholder exists outside the committed namespace |
| Progress | Stage plus determinate bytes when known |
| Success | Remove placeholder after the destination reader exposes the committed entry |
| Error | Keep retry/cancel context; collision and commit conflicts require a user decision |

The prototype implements this contract: uploads run through the transfer store
(<code>data/transfers.ts</code>) with a mock executor that walks the stages, supports
cancel/retry, surfaces name collisions as conflicts, and commits into the mock index so the
destination reader exposes the entry. The real <code>probe → upload → commit</code> integration
replaces the executor only.

## 5. Pagination, sorting, filtering, and aggregation

### 5.1 Location paging

- The prototype window is page-aligned with <code>PAGE_SIZE = 200</code>.
- <code>ensureRange(start, end)</code> requests every missing page intersecting the visible range.
- In-flight page numbers are deduplicated and responses carry a local version token.
- <code>totalCount</code> is optional. Known totals drive exact virtual extent and status text.
- With an unknown total, the formal model uses <code>loadedCount + hasMore</code>. The renderer
  MUST expose a load-more/sentinel row instead of remaining in its initial skeleton forever.
  Implemented: <code>FileItemList</code> loads unknown totals sequentially (no random access
  from an unknown cursor) and the views append one sentinel row that demand-loads the next page;
  the status bar shows <code>N+</code>. Fixture: <code>view://demo/unknown-total</code>.
- The effective backend page size is <code>min(200, hello.limits.max_list)</code>.
- NFSP is cursor-based. The adapter maintains a cursor chain per query/revision and satisfies the
  offset-shaped reader window sequentially. It MUST NOT invent random access from an unknown
  cursor.
- A cursor page reporting <code>revision_changed</code> invalidates the complete logical result;
  pages from different revisions are never merged.

The hard-coded value 200 is a performance default, not a Frozen product field.

### 5.2 Sorting

Current UI semantics:

| Sort | Direction | Tie-breaker/notes |
|---|---|---|
| Manual | Collection-defined only | Direction ignored |
| Name | Asc/desc | Numeric-aware name comparison |
| Size | Asc/desc | Unknown size behaves as zero in Mock |
| Modified | Asc/desc | ISO timestamp |
| Kind | Asc/desc | Name is stable tie-breaker |

Folders/groups appear first except under manual ordering. Recent defaults to modified descending;
Collections default to manual; other locations default to name ascending.

NFSP v1 currently exposes name/mtime/size/manual ascending order, but not direction, kind order, or
folders-first. Because client-sorting a partial window is invalid, the adapter resolves this gap
explicitly through capability negotiation (v1.2): NFSP readers advertise only the backend-honored
keys (no <code>kind</code>) and <code>sortDirs: ['asc']</code>, so the direction toggle is disabled
and never claims an order the pages do not have; <code>ListQuery.foldersFirst</code> is advisory
and NFSP listings render in backend order (folders not grouped first) until the protocol grows
these orders. Mock readers keep the richer prototype semantics via the same capability fields.

### 5.3 Filtering

The current location list exposes no filter control. NFSP <code>kind</code> and
<code>name_glob</code> filters therefore remain adapter capabilities, not active UI state.
Search is a separate scoped query and MUST NOT be emulated by filtering only the loaded window.

### 5.4 Aggregations and projections

- Recent Mock view: non-folder curated entries, modified descending, first 50 before requested
  reader sorting.
- Topic Mock view: de-duplicate all <code>TopicGroup.fileIds</code>, resolve known entries, then
  apply reader sorting.
- Collection count: recursive number of ref nodes, excluding groups.
- Collection list: manual order is the persisted sibling order; non-manual sorts are query
  projections and do not rewrite manual order.
- Search: traditional and AI groups are derived from <code>SearchResultItem.reason</code>.
- Status bar: total items, selection count, selected-known bytes, original/reference paths,
  public URL, and AI-active indicator.
- Preview: joins topic IDs against the topic projection; Story order follows the received array.
- Public URL column is shown from the current location context, while an individual URL is shown
  only when <code>entry.publicUrl</code> exists.

## 6. Field stability classification

### 6.1 Stable and extensible fields

| Field/contract | Stability | Notes |
|---|---|---|
| Canonical location URL | Frozen | Pane/tab/reader routing identity |
| Three location kinds | Frozen | Product behavior boundary |
| <code>LocationCapabilities</code> semantics | Frozen | All mutation affordances derive from it |
| <code>FileEntry.id</code> | Frozen | Target identity; must map from Ref, never path |
| <code>FileEntry.name</code> | Frozen | Core display field |
| <code>FileEntry.path</code> meaning | Frozen | Current original locator/display path, not identity |
| <code>FileEntry.kind</code> | Extensible | Unknown values fall back to other |
| <code>FileEntry.sizeBytes</code> | Frozen | Optional byte count |
| <code>FileEntry.modifiedAt</code> | Frozen | ISO timestamp in UI |
| <code>FileItem.key</code> | Frozen | Listing occurrence identity |
| <code>FileItem.entry</code> | Frozen | Target projection |
| <code>FileItem.ref</code> dual-path meaning | Frozen | Collection occurrence context |
| <code>broken</code> semantics | Frozen | Missing, stale, or inaccessible target |
| <code>FileItemPage.hasMore</code> | Frozen | Required for unknown totals |
| <code>FileItemPage.totalCount</code> | Extensible | Optional optimization |
| Sort-key enum | Extensible | Reader capability controls exposure |
| Topic fields/groups | Extensible | New axes and AI metadata may appear |
| Story fields | Extensible | New story kinds render with generic fallback |
| Search reason/source | Extensible | Backend modes may grow |
| Menu providers/actions | Extensible | Registered extensions may add commands |
| <code>publicUrl</code> | Frozen | Optional externally usable access URL |
| Collection member order | Frozen | Determines manual order and future preview sequence |

### 6.2 Volatile implementation details

| Field/contract | Stability | Notes |
|---|---|---|
| <code>FileBrowserSnapshot</code> and its indexes | Volatile | Mock source, not an API DTO |
| Detailed <code>TriggerRule</code> Mock list | Volatile | Not consumed by a rendered view |
| <code>CollectionNode.targetUrl</code> persistence | Volatile | Backend stores Ref |
| Name-based Collection <code>groupPath</code> | Volatile | NFSP group identity is entry_ref |
| Mock slug-based Collection IDs | Volatile | Backend generates identity |
| <code>PAGE_SIZE = 200</code> | Volatile | Negotiated/performance setting |
| <code>FileItemList.snapshot</code> | Volatile | React notification mechanism |
| Sparse Map, in-flight Set, version token | Volatile | Controller internals |
| Runtime <code>Error</code> object | Volatile | Integration normalizes to <code>UiError</code> |
| Set/Map selection storage | Volatile | Runtime representation only |
| <code>Date.now()</code> tab IDs | Volatile | Session-local implementation |
| Device navigation string <code>DeviceName:/path</code> | Volatile | No registered device reader |
| Mock transfer executor stage timing | Volatile | Real executor drives NFSP probe/tus/commit |
| Human-readable source label | Volatile | Presentation/provenance copy |
| Toast duration and menu anchors | Volatile | Rendering behavior |

### 6.3 High-impact invariants

Any change to these requires coordinated UI, adapter, tests, and documentation updates:

1. Conflating listing keys with target IDs.
2. Treating paths as persistent reference identity.
3. Changing Collection removal from unlink to target deletion.
4. Allowing writes in a View or uploads into a Collection.
5. Sorting only the loaded client window.
6. Merging cursor pages across revisions or ignoring watch <code>resync</code>.
7. Hiding a stale/broken target by silently dropping its Collection entry.
8. Exposing target metadata when a reference is visible but the target is not authorized.

## 7. Mock data contract

### 7.1 Representative objects

~~~ts
export const mockFile: FileEntry = {
  id: 'pic-kyoto-temple',
  name: 'kyoto-temple-0412.jpg',
  kind: 'image',
  path: '/home/Pictures/Trips/Kyoto/kyoto-temple-0412.jpg',
  sizeBytes: 4_820_000,
  modifiedAt: '2026-04-12T07:12:00Z',
  tags: ['temple', 'kyoto', 'sunrise', 'architecture'],
  topicIds: ['topic-kyoto'],
  summary: 'Early-morning shot of Kiyomizu-dera.',
  exif: {
    camera: 'Fujifilm X-T5',
    takenAt: '2026-04-12 05:48',
    location: 'Kyoto, Japan',
    lens: 'XF 16-55mm F2.8',
  },
  source: { type: 'local', label: 'Imported from camera' },
}

export const mockCollectionItem: FileItem = {
  key: 'ref-1',
  entry: mockFile,
  ref: {
    collectionUrl: 'collection://reading-list',
    refPath: 'collection://reading-list/kyoto-temple-0412.jpg',
    orderIndex: 0,
  },
}

export const mockBrokenReference: FileItem = {
  key: 'ref-broken',
  entry: {
    id: 'reading-list:ref-broken',
    name: 'archived-roadmap.pdf',
    kind: 'other',
    path: '/home/Documents/archived-roadmap.pdf',
    modifiedAt: '2026-01-01T00:00:00Z',
    link: {
      targetUrl: 'dfs:///home/Documents/archived-roadmap.pdf',
      broken: true,
    },
  },
  ref: {
    collectionUrl: 'collection://reading-list',
    refPath: 'collection://reading-list/archived-roadmap.pdf',
    orderIndex: 4,
    broken: true,
  },
}

export const mockPage: FileItemPage = {
  items: [mockCollectionItem, mockBrokenReference],
  totalCount: 2,
  hasMore: false,
}

export const mockSearchPage: SearchResultPage = {
  items: [
    {
      entry: mockFile,
      reason: 'ai_semantic',
      detail: 'Tag match — #kyoto',
      score: 0.91,
    },
  ],
  partial: false,
  sources: [{ mode: 'semantic', state: 'ok', tookMs: 34 }],
}
~~~

### 7.2 Required fixture coverage

| Contract/state | Required fixture |
|---|---|
| Normal folder | Mixed folders and files with optional enrichment |
| Empty folder | Writable folder with zero items and upload action |
| Empty View | Read-only zero-result explanation |
| Empty Collection | Zero references and add-reference guidance |
| Loading | Reader latency before first page and unloaded virtual slots |
| Error | Deterministic failing reader with retry success/failure controls |
| Large listing | Deterministic 10,000-item folder with mixed kinds/sizes/dates |
| Public subtree | Entries with and without public URL |
| File soft link | A link inside a normal folder |
| Collection structure | Nested group and reference to a real folder |
| Duplicate target | Same target referenced twice with different item keys |
| Broken reference | Preserved occurrence marked broken |
| Device states | Online, syncing, and offline |
| Topic | Multiple grouping axes and overlapping file IDs |
| Preview enrichment | Full metadata, missing metadata, Story present/empty |
| Search | Traditional, AI, empty, partial/degraded, error, cursor continuation |
| Unknown total | At least two cursor pages without <code>totalCount</code> |
| Mutation | Success, validation error, revision conflict, permission error |
| Upload | Queued through success, retryable failure, conflict, cancellation |

The Mock implements the normal/boundary rows above (10k folder, public URL, soft link,
nested/duplicate/broken Collection references, all device statuses, topics, empty folder
buckets) plus the deterministic scenarios: fetch errors and retry recovery
(<code>view://demo/error</code>, <code>view://demo/flaky</code>, reachable from the advanced-mode
Diagnostics section), unknown-total paging (<code>view://demo/unknown-total</code>), partial and
failing search (<code>partial:</code>/<code>error:</code>/<code>unknown:</code> query prefixes),
sidebar source failure (<code>?fbFail=</code>), schema validation failures (the form dialogs),
and upload progress including retryable failure (a name containing <code>fail</code>) and
name-collision conflict. Revision-conflict and permission-error mutation fixtures remain for the
backend-integration stage.

### 7.3 Mock behavior rules

- Default reader delay is 50–150ms; Collection reads use 30–80ms and mutations 20–60ms.
- Generated stress data is deterministic across reloads.
- Collection mutations change the in-memory store and broadcast invalidation in-session.
- Refresh resets Collections to seed data because persistence belongs to the backend.
- Mock folder write actions may remain non-persistent only during prototype mode and must be
  visibly identified as Mock behavior.
- Mock UI code MUST consume readers rather than reading <code>entriesByPath</code> or
  <code>entriesById</code> directly.
- Error/partial scenarios MUST be selectable without fixed Playwright timeouts.

## 8. NFSP / backend mapping notes

### 8.1 Integration boundary

The real implementation (v1.2, <code>data/nfsp/</code>) adds NFSP-backed
<code>FolderReader</code>s and a target resolver, using <code>NfsBrowserClient</code> for cached
reads, writes, and watch invalidation. UI components and <code>FileItemList</code>'s observable
API are unchanged; NFSP readers never promise <code>totalCount</code>, so all NFSP listings run
in the unknown-total sequential mode (§5.1). Direct writes additionally notify a small local
path bus (<code>data/nfsp/invalidation.ts</code>) because the cache layer only fires listeners
from watch events and revalidation — same-client reloads must not wait for the SSE round-trip.

~~~
FileBrowser UI
  → FolderReader / CollectionReader
    → NFSP adapter and error mapper
      → NfsBrowserClient
        → NfspClient
          → /kapi/nfs-server/nfs/v1/*
~~~

### 8.2 Read mapping

| UI field | NFSP source | Transform |
|---|---|---|
| Location URL | Locator/path/URI | Bare DFS path ↔ realm <code>dfs</code>; View/Collection root URI direct |
| Location kind | <code>NodeInfo.kind</code> | dir → folder; view → view; collection/group → collection |
| Capabilities | <code>NodeInfo.capabilities</code> | snake_case to UI camelCase; unlink → remove-ref; none → null |
| Location title/description | Node title + metadata | Normalize into <code>LocationMeta</code> |
| <code>FileItem.key</code> | <code>Entry.entry_ref</code> | Required for Collection/member/reference occurrences |
| <code>FileEntry.id</code> | <code>Entry.target.ref</code> | Stable serialized Ref identity; never canonical path |
| Name | <code>Entry.name</code> | Direct |
| File kind | target kind + MIME/name | dir/group → folder; classify file; unknown → other |
| Original path | <code>Entry.canonical_path</code> | Convert current-Zone <code>cyfs:///</code> to UI DFS display path |
| Size | <code>target.attrs.size</code> | Direct byte count, optional |
| Modified | <code>target.attrs.mtime</code> | Unix seconds → ISO string |
| Public URL | <code>target.attrs.access_urls</code> | Choose authorized external/public URL by kind/policy |
| Broken | <code>target.target_state</code> | stale/missing/inaccessible → true with privacy-safe placeholder |
| Ref context | binding + entry_ref + container | Build Collection URL/refPath/orderIndex |
| Meta/Story/Tags/Topic | <code>get_meta.records</code> | Namespace/key-specific projection; unknown records ignored |
| Search reason | <code>match_source</code> | Map name and future modes; retain unknown source |
| Search detail | <code>explain</code> | Produce localized, privacy-safe evidence text |
| Search source state | <code>sources[]</code> | snake_case to <code>SearchSourceStatus</code> |

NFSP <code>entry_ref</code> is present in the current server listings, including native entries.
For any Collection occurrence it is mandatory for correct independent selection/removal. Missing
Collection <code>entry_ref</code> is an adapter/protocol error, not permission to use target ID.
Opening a nested View/Collection group resolves the root container and walks the selected group
<code>entry_ref</code>; the adapter must not send the prototype's slash-joined group names as a
Collection ID.

### 8.3 Write mapping

| UI action | NFSP operation | Notes |
|---|---|---|
| New folder | <code>mkdir</code> | Parent Ref + validated name + expected revision |
| Rename/move | <code>move</code> | Preserve from/to revisions; cross-device failure is visible |
| Delete native item | <code>delete</code> | Never call for reference/member binding |
| Remove folder link | <code>unlink</code> | Deletes binding only |
| Upload file | <code>probe/open_write/tus/commit_file</code> | Local placeholder until commit |
| Read/download | data-plane <code>readUrl/readFile</code> | Range/ETag supported |
| Create Collection | <code>create_collection</code> | Server owns Collection ID |
| Add Collection refs | <code>collection_patch.add_ref</code> | Resolve locator to target Ref first |
| Remove Collection item | <code>collection_patch.remove_entry</code> | Use item <code>entry_ref</code> |
| Reorder Collection | <code>collection_patch.move_entries</code> | Use sibling entry refs and expected revision |
| Create/rename group | Collection patch | Use validated name; rename uses group entry_ref |
| Read/write metadata | <code>get_meta/set_meta</code> | Only authorized writable namespaces |
| Search | <code>search</code> | Cursor-based; v1 name mode, response remains extensible |
| Share | <code>grant/revoke</code> | Do not expose as complete until data-plane cap enforcement exists |

### 8.4 Revision, cache, and watch rules

- Revision is an opaque equality token. It is never numerically ordered or compared across
  containers.
- The adapter retains the container Ref, revision, cursor chain, and watch token outside the
  display model.
- A successful write invalidates the affected reader immediately; watch is not required for
  same-client correctness.
- <code>container_changed</code> invalidates that container. <code>resync</code> invalidates all
  interested readers and forces trusted re-resolution/re-listing.
- Watch is lossy. Disconnect/reconnect cannot be interpreted as proof that cached rows are
  current.
- <code>revision_changed</code> on a cursor response discards the entire accumulated window.
- Cached stale-while-revalidate data may be displayed, but background failures must not erase the
  last successful projection without an explicit state transition.

### 8.5 Error normalization

NFSP errors map to stable UI categories rather than exposing raw server messages:

| Backend code/category | UI behavior |
|---|---|
| NOT_FOUND / STALE | Re-resolve once; then show missing/stale state |
| PERMISSION_DENIED | Access-denied state; do not leak target metadata |
| NAMESPACE_CONFLICT | Preserve both facts and offer an explicit repair flow |
| REVISION/TARGET mismatch | Conflict state; re-list before retry |
| NOT_EMPTY | Confirmation or non-recursive-delete guidance |
| NEED_PULL | Continue upload/acquisition flow, not a generic failure |
| REFERRAL | Hand to a registered remote/device reader; never follow silently |
| UNSUPPORTED | Hide unsupported capability after negotiation or explain it |
| Network/session | Retryable connection state; session refresh may replay only safe client calls |
| INTERNAL/unknown | Generic localized failure with a diagnostic code |

Raw paths, refs, server messages, and authorization details MUST be filtered before entering
<code>UiError.details</code> or user-visible copy.

## 9. Prototype-to-formal-model delta

These are required integration or follow-up changes, not reasons to weaken the formal contract.
Items 1–5 and 11 are DONE in the prototype; items 6–9 are DONE in the NFSP integration
(2026-09-02, <code>data/nfsp/</code>); 10 and 12 remain gated:

1. ~~Replace the unused <code>types.ts SearchHit</code> and inline component type with
   <code>SearchResultItem</code>.~~ Done — <code>types.ts</code> now defines the §2 model.
2. ~~Route search through an async state/provider~~ Done via
   <code>data/search.ts</code> + <code>mock/search.ts</code>; a View-compatible search reader
   remains a later refinement.
3. ~~Replace Collection/group <code>window.prompt()</code> and rename/create forms with the
   Zod schemas and <code>react-hook-form</code>.~~ Done — <code>dialogs/NamePromptDialog</code>.
4. ~~Add deterministic File Browser error, partial, validation, unknown-total, and transfer Mock
   scenarios plus Playwright coverage.~~ Done — see §7.2; covered by
   <code>tests/e2e/pages/filebrowser.spec.ts</code>. Revision-conflict/permission fixtures wait
   for backend semantics.
5. ~~Fix unknown-total rendering so <code>hasMore</code> listings leave initial loading and can
   request subsequent cursor pages.~~ Done — sentinel-row load-more in the virtualized views.
6. ~~Implement the NFSP reader/target-resolver adapter and switch data installation by
   environment, without importing protocol DTOs into components.~~ Done — <code>data/nfsp/</code>
   (readers, collection reader, collection directory, sidebar sources, search provider, transfer
   executor, folder ops, error mapper); <code>data/install.ts</code> switches on
   <code>isMockRuntime()</code> with a <code>?fbData=mock|nfsp</code> diagnostic override and a
   <code>VITE_NFS_PROXY</code> dev proxy. New registries extracted so components stay
   backend-blind: <code>data/folderOps.ts</code> (mkdir/rename/delete/move/download) and
   <code>data/collectionDirectory.ts</code> (collection list/create). NFSP v1 has no
   list-collections method, so the NFSP directory keeps known ids in localStorage and
   revalidates them against the server — a documented stopgap until the protocol grows
   enumeration.
7. ~~Resolve the NFSP sort gap for descending, kind, and folders-first before claiming identical
   backend behavior.~~ Done — capability-negotiated <code>sortDirs</code> plus reduced
   <code>sortKeys</code>; see §5.2.
8. ~~Replace or back the name-based Collection group URL with an entry-ref-aware deep-link
   model.~~ Done — the URL keeps its percent-encoded name form as a display locator, and the
   NFSP collection reader walks each group segment through the parent listing to its
   <code>entry_ref</code>/target Ref (slash-joined names are never sent as an id). A fully
   identity-bearing URL form remains a possible later refinement.
9. ~~Define metadata namespace/key mappings for summary, tags, topics, EXIF, source, Story, and
   trigger-policy status.~~ Done — <code>data/nfsp/mapping.ts applyMetaRecords</code> maps the
   <code>user</code>/<code>ai</code> namespaces (keys <code>summary/tags/topics/exif/source/
   story/triggers_active</code>) into the entry projection via the preview enricher
   (<code>data/usePreview.ts</code>); v1 nfs-server only persists the <code>user</code>
   namespace, so AI enrichment stays absent until its pipeline exists.
10. Replace the current device navigation placeholder with a registered device/referral reader.
    The NFSP sidebar source returns an empty device list (explicit empty state, no fabricated
    data) until referral readers exist.
11. ~~Add explicit async states for sidebar sources and partial Preview metadata.~~ Done —
    <code>data/sidebarSources.ts</code>, <code>data/usePreview.ts</code>.
12. Keep share UI feature-gated until authorization and data-plane cap enforcement are complete.
    Unchanged: share remains an informational stub in both modes.

## 10. Stage-three completion checklist

- [x] All prototype data entities and runtime view models are documented or explicitly classified
  as Mock/Volatile.
- [x] Display/query models are separated from editable input models.
- [x] Input schemas, defaults, limits, and invalid examples are defined.
- [x] Normal, empty, loading, error, and progress behavior is defined for every data view.
- [x] Paging, sorting, filtering, aggregation, and unknown-total behavior are defined.
- [x] Stable, extensible, and volatile fields are classified.
- [x] Mock fixtures and required edge scenarios are specified.
- [x] NFSP mapping, revision/watch semantics, errors, and known protocol gaps are recorded.
- [x] The document can be handed to the backend-integration stage without treating NFSP DTOs as
  UI models.
