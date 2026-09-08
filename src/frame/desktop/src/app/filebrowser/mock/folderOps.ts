import type { FileEntry } from '../types'
import { registerFolderOps, runEntryBatch, operationError } from '../data/folderOps'
import { invalidateMockPath, mockDelay } from '../data/mockReader'
import { mockAddEntry, mockMoveEntry, mockNameExists, mockRemoveEntry, mockRenameEntry, mockEntryById, mockEntryByPath, mockEntriesAtPath } from './data'

const parentOf = (path: string) => path.slice(0, path.lastIndexOf('/')) || '/'
const failures = new Set<string>()
async function validate(entry: FileEntry, action: 'move' | 'rename' | 'delete') {
  const slow = new URLSearchParams(location.search).has('fbSlowOps')
  await mockDelay(slow ? 900 : 20, slow ? 900 : 60)
  const live = mockEntryById(entry.id)
  if (!live || live.path !== entry.path || live.name !== entry.name) throw operationError('STALE', 'This item moved or changed. Refresh before retrying.')
  if (live.operations?.[action] === 'denied' || entry.name.startsWith('denied-')) throw operationError('PERMISSION_DENIED', 'You do not have permission to change this item')
  if (entry.name.startsWith('fail-once-') && !failures.has(entry.id)) {
    failures.add(entry.id)
    throw operationError('NETWORK', 'The request was interrupted. Please retry.')
  }
}
function requireFolder(path: string) {
  if (mockEntriesAtPath(path) === undefined) throw operationError('NOT_FOUND', 'The destination folder no longer exists')
  if (path.startsWith('/readonly')) throw operationError('PERMISSION_DENIED', 'The destination is read-only')
}
export function registerMockFolderOps() {
  return registerFolderOps({
    supportsCopy: false,
    async nameExists(parent, name) { requireFolder(parent); return mockNameExists(parent, name) },
    async statEntry(parent, name) { requireFolder(parent); const item = mockEntryByPath(`${parent === '/' ? '' : parent}/${name}`); return item ? { ...item } : null },
    async createFolder(parent, name) {
      await mockDelay(20, 60)
      requireFolder(parent)
      if (mockNameExists(parent, name)) throw operationError('CONFLICT', 'This name already exists here')
      mockAddEntry({ id: crypto.randomUUID(), name, kind: 'folder', path: `${parent === '/' ? '' : parent}/${name}`, modifiedAt: new Date().toISOString() })
      invalidateMockPath(parent)
    },
    async renameEntry(entry, name) {
      await validate(entry, 'rename')
      const parent = parentOf(entry.path)
      if (name !== entry.name && mockNameExists(parent, name)) throw operationError('CONFLICT', 'This name already exists here')
      mockRenameEntry(entry.id, name)
      invalidateMockPath(parent)
    },
    deleteEntries(entries, options) {
      return runEntryBatch(entries, async (entry) => {
        await validate(entry, 'delete')
        options?.signal?.throwIfAborted()
        const parent = mockRemoveEntry(entry.id)
        if (parent) invalidateMockPath(parent)
      }, undefined, options)
    },
    moveEntries(entries, target, options) {
      return runEntryBatch(entries, async (entry, name) => {
        await validate(entry, 'move')
        options?.signal?.throwIfAborted()
        requireFolder(target)
        if (mockNameExists(target, name)) throw operationError('CONFLICT', 'This name already exists in the destination')
        const parent = parentOf(entry.path)
        mockMoveEntry(entry.id, target)
        if (name !== entry.name) mockRenameEntry(entry.id, name)
        invalidateMockPath(parent)
        invalidateMockPath(target)
      }, target, options)
    },
    downloadUrl() { return null },
  })
}
