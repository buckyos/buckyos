/**
 * NFSP folder write operations (UI_DATAMODEL.md §8.3): mkdir for new
 * folders, move for rename and relocation, delete for native destruction,
 * and the data-plane read URL for downloads. Every successful write
 * notifies the local invalidation bus so same-client readers reload without
 * waiting for the watch round-trip (§8.4).
 */

import type { WireRef } from '../../../../api/nfsp_client'
import type { FileEntry } from '../../types'
import { registerFolderOps } from '../folderOps'
import { ensureSession, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { notifyDfsPath } from './invalidation'

function parentPathOf(path: string): string {
  return path.split('/').slice(0, -1).join('/') || '/'
}

async function resolveDirRef(path: string): Promise<WireRef> {
  const info = await nfspClient().resolve(path)
  if (!info) throw nfspToUiError({ code: 'NOT_FOUND', message: 'folder gone' })
  return info.ref
}

async function run<T>(op: () => Promise<T>): Promise<T> {
  await ensureSession()
  try {
    return await op()
  } catch (err) {
    throw nfspToUiError(err)
  }
}

export function registerNfspFolderOps() {
  return registerFolderOps({
    nameExists(parentPath, name) {
      return run(async () => {
        const info = await nfspClient()
          .stat(parentPath, { name, cache: 'no-cache' })
          .catch(() => null)
        return info !== null
      })
    },

    createFolder(parentPath, name) {
      return run(async () => {
        const parentRef = await resolveDirRef(parentPath)
        await nfspClient().mkdir(parentRef, name)
        notifyDfsPath(parentPath)
      })
    },

    renameEntry(entry, name) {
      return run(async () => {
        const parentPath = parentPathOf(entry.path)
        const parentRef = await resolveDirRef(parentPath)
        await nfspClient().move(
          { parentRef, name: entry.name },
          { parentRef, name },
        )
        notifyDfsPath(parentPath)
      })
    },

    deleteEntries(entries: FileEntry[]) {
      return run(async () => {
        const touched = new Set<string>()
        for (const entry of entries) {
          const parentPath = parentPathOf(entry.path)
          await nfspClient().delete(parentPath, entry.name, { recursive: true })
          touched.add(parentPath)
        }
        for (const path of touched) notifyDfsPath(path)
      })
    },

    moveEntries(entries: FileEntry[], toParentPath) {
      return run(async () => {
        const toRef = await resolveDirRef(toParentPath)
        const touched = new Set<string>([toParentPath])
        for (const entry of entries) {
          const fromPath = parentPathOf(entry.path)
          const fromRef = await resolveDirRef(fromPath)
          await nfspClient().move(
            { parentRef: fromRef, name: entry.name },
            { parentRef: toRef, name: entry.name },
          )
          touched.add(fromPath)
        }
        for (const path of touched) notifyDfsPath(path)
      })
    },

    downloadUrl(entry) {
      if (entry.kind === 'folder') return null
      // FileEntry.id is the serialized live Ref identity (the node id) —
      // exactly what the data plane addresses (§8.2 read mapping).
      return nfspClient().raw.readUrl(entry.id, { download: true, name: entry.name })
    },
  })
}
