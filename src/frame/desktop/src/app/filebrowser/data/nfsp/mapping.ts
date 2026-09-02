/**
 * NFSP wire → UI DataModel projections (UI_DATAMODEL.md §8.2).
 *
 * Components never see these wire types; readers/providers in this directory
 * are the only importers. Optional enrichment fields stay absent when unknown
 * — nothing is fabricated to satisfy rendering (§2.3).
 */

import type {
  Capabilities,
  Entry,
  MetaRecord,
  NodeInfo,
  NodeKind,
  WireRef,
} from '../../../../api/nfsp_client'
import type {
  FileEntry,
  FileExif,
  FileSource,
  SortDir,
  SortKey,
  StoryEntry,
} from '../../types'
import type { FileItem, LocationCapabilities, LocationMeta } from '../FolderReader'
import { classifyFileKind } from '../fileKinds'
import { DFS_SCHEME } from '../urls'
import { nfspBaseUrl } from './client'

/** Stable serialized Ref identity — never a canonical path (§8.2). */
export function refIdOf(ref: WireRef): string {
  if (ref.type === 'live') return ref.node_id
  return ref.inner_path ? `${ref.obj_id}:${ref.inner_path}` : ref.obj_id
}

/** `cyfs:///home/x` (current zone) → UI DFS display path `/home/x`. */
export function cyfsToDfsPath(canonical: string): string | null {
  if (!canonical.startsWith('cyfs:///')) return null
  const rest = canonical.slice('cyfs://'.length)
  return rest === '' ? '/' : rest
}

export function unixToIso(seconds: number | undefined): string {
  return new Date((seconds ?? 0) * 1000).toISOString()
}

/** UI sort key → NFSP list order. `kind` has no backend order and is not advertised. */
export const NFSP_ORDER_BY_SORT_KEY: Partial<Record<SortKey, string>> = {
  name: 'name',
  size: 'size',
  modified: 'mtime',
  manual: 'manual',
}

/** NFSP v1 serves ascending order only — advertised, not silently ignored (§5.2). */
export const NFSP_SORT_DIRS: SortDir[] = ['asc']

const CONTAINER_SORT_KEYS: SortKey[] = ['name', 'size', 'modified']

/**
 * Capability mapping (§8.2): snake_case → camelCase, unlink → remove-ref.
 * The frozen §2.5 matrix wins where the wire shape is broader: folders never
 * acceptReferences in the UI even though the server allows bind_ref into dirs.
 */
export function mapCapabilities(kind: NodeKind, caps: Capabilities): LocationCapabilities {
  if (kind === 'collection' || kind === 'group') {
    return {
      kind: 'collection',
      acceptsContent: false,
      acceptsReferences: caps.accepts_references,
      removal: 'remove-ref',
      canReorder: caps.ordered,
      sortKeys: ['manual', ...CONTAINER_SORT_KEYS],
      sortDirs: NFSP_SORT_DIRS,
      defaultSortKey: 'manual',
    }
  }
  if (kind === 'view') {
    return {
      kind: 'view',
      acceptsContent: false,
      acceptsReferences: false,
      removal: null,
      canReorder: false,
      sortKeys: ['manual', ...CONTAINER_SORT_KEYS],
      sortDirs: NFSP_SORT_DIRS,
      defaultSortKey: 'manual',
    }
  }
  return {
    kind: 'folder',
    acceptsContent: caps.accepts_content,
    acceptsReferences: false,
    removal: caps.remove_semantics === 'none' ? null : 'destroy',
    canReorder: false,
    sortKeys: CONTAINER_SORT_KEYS,
    sortDirs: NFSP_SORT_DIRS,
    defaultSortKey: 'name',
  }
}

export function mapLocationMeta(info: NodeInfo): LocationMeta | undefined {
  if (info.title) return { title: info.title }
  return undefined
}

function absoluteUrl(url: string): string {
  return /^[a-z][a-z0-9+.-]*:/i.test(url) ? url : `${nfspBaseUrl()}${url}`
}

/** Pick the externally usable URL from access metadata (`kind: "read"`). */
function publicUrlOf(attrs: Entry['target']['attrs']): string | undefined {
  const urls = attrs?.access_urls
  if (!Array.isArray(urls)) return undefined
  const read = urls.find((u) => u.kind === 'read')
  return read ? absoluteUrl(read.url) : undefined
}

export interface EntryMapContext {
  /** DFS display path of the listed container, when it is a dfs folder. */
  containerDfsPath?: string
}

/**
 * Listing entry → FileItem for folder/view containers. Collection occurrences
 * (groups, references) get their own mapping in the collection reader because
 * they need the collection URL context.
 */
export function mapEntryToItem(entry: Entry, context: EntryMapContext): FileItem {
  const target = entry.target
  const broken = target.target_state !== undefined && target.target_state !== 'ok'
  const canonicalDfs = entry.canonical_path ? cyfsToDfsPath(entry.canonical_path) : null
  const path =
    canonicalDfs ??
    (context.containerDfsPath !== undefined
      ? context.containerDfsPath === '/'
        ? `/${entry.name}`
        : `${context.containerDfsPath}/${entry.name}`
      : entry.name)

  const isContainer =
    target.kind === 'dir' || target.kind === 'collection' || target.kind === 'group'
  const fileEntry: FileEntry = {
    id: refIdOf(target.ref),
    name: entry.name,
    kind: isContainer ? 'folder' : target.kind === 'file' ? classifyFileKind(entry.name) : 'other',
    path,
    modifiedAt: unixToIso(target.attrs?.mtime),
  }
  if (typeof target.attrs?.size === 'number' && target.kind === 'file') {
    fileEntry.sizeBytes = target.attrs.size
  }
  const publicUrl = publicUrlOf(target.attrs)
  if (publicUrl) fileEntry.publicUrl = publicUrl
  if (broken) {
    fileEntry.link = { targetUrl: `${DFS_SCHEME}${path}`, broken: true }
  }
  return {
    key: entry.entry_ref ?? refIdOf(target.ref),
    entry: fileEntry,
  }
}

// ─── Meta namespace/key mapping (§8.2 get_meta.records → enrichment, §9 item 9) ───
//
// v1 writable namespace is `user`; `ai` is reserved for the AI pipeline stage.
// Unknown namespaces/keys/shapes are ignored without failing the preview.

const ENRICHMENT_NAMESPACES = new Set(['user', 'ai'])

function asStringArray(value: unknown): string[] | undefined {
  return Array.isArray(value) && value.every((v) => typeof v === 'string')
    ? (value as string[])
    : undefined
}

export function applyMetaRecords(entry: FileEntry, records: MetaRecord[]): FileEntry {
  const enriched: FileEntry = { ...entry }
  for (const record of records) {
    if (!ENRICHMENT_NAMESPACES.has(record.ns)) continue
    const value = record.value
    switch (record.key) {
      case 'summary':
        if (typeof value === 'string') enriched.summary = value
        break
      case 'tags': {
        const tags = asStringArray(value)
        if (tags) enriched.tags = tags
        break
      }
      case 'topics': {
        const topics = asStringArray(value)
        if (topics) enriched.topicIds = topics
        break
      }
      case 'exif':
        if (value && typeof value === 'object') enriched.exif = value as FileExif
        break
      case 'source':
        if (
          value &&
          typeof value === 'object' &&
          typeof (value as FileSource).label === 'string'
        ) {
          enriched.source = value as FileSource
        }
        break
      case 'story':
        if (Array.isArray(value)) enriched.story = value as StoryEntry[]
        break
      case 'triggers_active':
        if (typeof value === 'boolean') enriched.triggersActive = value
        break
      default:
        break // unknown keys are expected (§1.3)
    }
  }
  return enriched
}
