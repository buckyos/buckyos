import type { NodeInfo } from '../../../../api/nfsp_client'
import type { FileEntry } from '../../types'
import { registerFolderOps, runEntryBatch, operationError } from '../folderOps'
import { classifyFileKind } from '../fileKinds'
import { ensureSession, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { notifyDfsPath } from './invalidation'
import { refIdOf, unixToIso } from './mapping'

const parentOf = (path: string) => path.slice(0, path.lastIndexOf('/')) || '/'
async function resolveDirRef(path: string): Promise<NodeInfo> {
  const info = await nfspClient().stat(path, { cache: 'no-cache' })
  if (!info) throw operationError('NOT_FOUND', 'The destination folder no longer exists')
  if (!info.capabilities.accepts_content) throw operationError('PERMISSION_DENIED', 'The destination does not accept file content')
  return info
}
async function run<T>(op: () => Promise<T>): Promise<T> {
  try { await ensureSession(); return await op() }
  catch (err) { if (err instanceof Error && err.name === 'AbortError') throw err; throw nfspToUiError(err) }
}
async function statEntry(parent: string, name: string): Promise<FileEntry | null> {
  return run(async () => {
    try {
      const info = await nfspClient().stat(parent, { name, cache: 'no-cache', want: ['base'] })
      if (!info) return null
      return { id: refIdOf(info.ref), name, path: `${parent === '/' ? '' : parent}/${name}`, kind: info.kind === 'dir' ? 'folder' : classifyFileKind(name), modifiedAt: unixToIso(info.mtime), sizeBytes: info.size }
    } catch (err) {
      if (nfspToUiError(err).code === 'NOT_FOUND') return null
      throw err
    }
  })
}
async function validate(entry: FileEntry) {
  const parent = await nfspClient().stat(parentOf(entry.path), { cache: 'no-cache' })
  if (!parent) throw operationError('STALE', 'The original folder no longer exists')
  const live = await statEntry(parentOf(entry.path), entry.name)
  if (!live || live.id !== entry.id) throw operationError('STALE', 'This item moved or changed. Refresh before retrying.')
  return parent
}
export function registerNfspFolderOps() {
  return registerFolderOps({
    supportsCopy: false,
    nameExists: async (parent, name) => (await statEntry(parent, name)) !== null,
    statEntry,
    createFolder(parent, name) {
      return run(async () => { const info = await resolveDirRef(parent); if (await statEntry(parent, name)) throw operationError('CONFLICT', 'This name already exists here'); await nfspClient().mkdir(info.ref, name, { expectedRevision: info.revision }); notifyDfsPath(parent) })
    },
    renameEntry(entry, name) {
      return run(async () => {
        const info = await validate(entry)
        const parent = parentOf(entry.path)
        await nfspClient().move({ parentRef: info.ref, name: entry.name }, { parentRef: info.ref, name }, { expectedFromRevision: info.revision, expectedToRevision: info.revision })
        notifyDfsPath(parent)
      })
    },
    deleteEntries(entries, options) {
      return runEntryBatch(entries, (entry) => run(async () => {
        const info = await validate(entry)
        const parent = parentOf(entry.path)
        options?.signal?.throwIfAborted()
        await nfspClient().delete(parent, entry.name, { recursive: entry.kind === 'folder', expectedRevision: info.revision })
        notifyDfsPath(parent)
      }), undefined, options)
    },
    moveEntries(entries, target, options) {
      return runEntryBatch(entries, (entry, name) => run(async () => {
        const fromInfo = await validate(entry)
        const parent = parentOf(entry.path)
        const toInfo = await resolveDirRef(target)
        options?.signal?.throwIfAborted()
        await nfspClient().move({ parentRef: fromInfo.ref, name: entry.name }, { parentRef: toInfo.ref, name }, { expectedFromRevision: fromInfo.revision, expectedToRevision: toInfo.revision })
        notifyDfsPath(parent)
        notifyDfsPath(target)
      }), target, options)
    },
    downloadUrl(entry) {
      return entry.kind === 'folder' ? null : nfspClient().raw.readUrl(entry.id, { download: true, name: entry.name })
    },
  })
}
