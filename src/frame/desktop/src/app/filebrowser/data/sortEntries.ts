/**
 * Shared sort semantics for mock readers — folders first (when the query
 * asks), name as the stable tie-breaker with numeric compare. Real backends
 * sort server-side; the UI never sorts items itself.
 */

import type { FileEntry, SortDir, SortKey } from '../types'

export function compareEntries(a: FileEntry, b: FileEntry, key: SortKey, dir: SortDir): number {
  const factor = dir === 'asc' ? 1 : -1
  let cmp = 0
  if (key === 'size') cmp = (a.sizeBytes ?? 0) - (b.sizeBytes ?? 0)
  else if (key === 'modified') cmp = a.modifiedAt.localeCompare(b.modifiedAt)
  else if (key === 'kind') cmp = a.kind.localeCompare(b.kind)
  if (cmp === 0) cmp = a.name.localeCompare(b.name, undefined, { numeric: true })
  return cmp * factor
}

export function sortEntriesForQuery(
  entries: FileEntry[],
  key: SortKey,
  dir: SortDir,
  foldersFirst: boolean,
): FileEntry[] {
  return [...entries].sort((a, b) => {
    if (foldersFirst && (a.kind === 'folder') !== (b.kind === 'folder')) {
      return a.kind === 'folder' ? -1 : 1
    }
    return compareEntries(a, b, key, dir)
  })
}
