/* Zone items as BuckyOS standard content (剪贴板规范 §2.2): one conversion from a Files entry, shared by the system
 * file picker and every drag or copy source that shows Zone files. */
import { contentDescriptor, type ZoneContent } from 'buckyos/content'
import { normalizeCyfsPath } from '../components/preview/session'
import type { FileEntry } from '../app/filebrowser/types'

/** Whether an entry is a Zone file-system item (a DFS path), not a computer file or a collection-only row. */
export const isZoneEntry = (entry: Pick<FileEntry, 'path'>): boolean => entry.path.startsWith('/')

/** A Zone file or folder of the file browser as a content reference: its path, the object id the listing gave, and
 * the name, MIME type and size as hints. */
export function zoneContentOfEntry(entry: Pick<FileEntry, 'path' | 'name' | 'kind' | 'objId' | 'sizeBytes'>): ZoneContent {
  const source = { kind: 'cyfs-path' as const, path: normalizeCyfsPath(entry.path) }
  const folder = entry.kind === 'folder'
  return {
    origin: 'zone', kind: folder ? 'folder' : 'file', source,
    ...(entry.objId ? { objId: entry.objId } : {}),
    hints: folder ? { name: entry.name } : { name: entry.name, mime: contentDescriptor(source, { name: entry.name }).mime, ...(entry.sizeBytes !== undefined ? { size: entry.sizeBytes } : {}) },
  }
}

/** The DFS path (`/home/…`) a `cyfs-path` reference names; null for an object reference. */
export const dfsPathOfContent = (item: ZoneContent): string | null =>
  item.source.kind === 'cyfs-path' ? item.source.path.slice('cyfs://'.length) || '/' : null
