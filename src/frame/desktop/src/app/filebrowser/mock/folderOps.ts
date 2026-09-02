/**
 * Mock folder write operations over the in-memory index (UI_DATAMODEL.md
 * §7.3): mutations are real in-session so the interaction loop is
 * experienceable; refresh resets to seeds because persistence belongs to the
 * backend. Downloads have no mock data plane and return null.
 */

import type { FileEntry } from '../types'
import { registerFolderOps } from '../data/folderOps'
import { invalidateMockPath, mockDelay } from '../data/mockReader'
import {
  mockAddEntry,
  mockMoveEntry,
  mockNameExists,
  mockRemoveEntry,
  mockRenameEntry,
} from './data'

function parentPathOf(path: string): string {
  return path.split('/').slice(0, -1).join('/') || '/'
}

export function registerMockFolderOps() {
  return registerFolderOps({
    async nameExists(parentPath, name) {
      return mockNameExists(parentPath, name)
    },

    async createFolder(parentPath, name) {
      await mockDelay(20, 60)
      mockAddEntry({
        id: `folder-${Date.now()}`,
        name,
        kind: 'folder',
        path: parentPath === '/' ? `/${name}` : `${parentPath}/${name}`,
        modifiedAt: new Date().toISOString(),
      })
      invalidateMockPath(parentPath)
    },

    async renameEntry(entry, name) {
      await mockDelay(20, 60)
      if (mockRenameEntry(entry.id, name)) {
        invalidateMockPath(parentPathOf(entry.path))
      }
    },

    async deleteEntries(entries: FileEntry[]) {
      await mockDelay(20, 60)
      const touched = new Set<string>()
      for (const entry of entries) {
        const parent = mockRemoveEntry(entry.id)
        if (parent) touched.add(parent)
      }
      for (const path of touched) invalidateMockPath(path)
    },

    async moveEntries(entries: FileEntry[], toParentPath) {
      await mockDelay(20, 60)
      const touched = new Set<string>()
      for (const entry of entries) {
        const from = mockMoveEntry(entry.id, toParentPath)
        if (from) {
          touched.add(from)
          touched.add(toParentPath)
        }
      }
      for (const path of touched) invalidateMockPath(path)
    },

    downloadUrl() {
      return null
    },
  })
}
