/* "保存到 Zone" of a directory's files (UI 框架 PRD §7.4; needs `export`): the Zone is the user's main file system, so
 * keeping a file is saving it there; downloading to this device stays as the other entry. One file: the system file
 * picker's save mode (folder and name; a taken name is asked about by the picker). Several: the picker's folder mode,
 * then each file under its own name (a taken name gets " (2)", " (3)"…, nothing is written over).
 *
 * How the bytes get there is the workspace package's way (ui/home/exportPackage.ts): the member is read with the
 * Workspace session, put into the Zone's store as an object of the user's own session, and committed into the folder. */

import { useCallback } from 'react'
import { useSelectContent } from '../../../../sysdlg'
import { useSaveToZone } from '../../../../components/saveToZone'
import { isMockRuntime } from '../../../../runtime'
import { folderOps } from '../../../filebrowser/data/folderOps'
import { ensureSession, nfspClient } from '../../../filebrowser/data/nfsp/client'
import { nfspToUiError } from '../../../filebrowser/data/nfsp/errors'
import { notifyDfsPath } from '../../../filebrowser/data/nfsp/invalidation'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { uploadToZoneStore } from '../home/exportPackage'
import { keepBothName } from './model'

export interface ZoneFile { name: string; objectId: string }
export type DirectoryZoneSave = { status: 'saved'; path: string; count: number } | { status: 'cancelled' }

const join = (parent: string, name: string) => `${parent === '/' ? '' : parent}/${name}`

/** A name not taken in the folder, as far as this page can tell (the file service refuses a taken one anyway). */
async function freeName(parent: string, name: string, used: Set<string>): Promise<string> {
  const taken = async (candidate: string) => used.has(candidate) || Boolean(await folderOps().statEntry(parent, candidate).catch(() => null))
  if (!await taken(name)) return name
  for (let n = 0; n < 50; n += 1) {
    const candidate = keepBothName(name, (c) => used.has(c))
    if (!await taken(candidate)) return candidate
    used.add(candidate)
  }
  return keepBothName(name, (c) => used.has(c))
}

/** Save `files` (members of a directory) into the Zone. */
export function useDirectorySaveToZone(store: WorkspaceStore): (files: ZoneFile[]) => Promise<DirectoryZoneSave> {
  const { t } = useAiwsText()
  const saveOne = useSaveToZone(t('aiworkspace.home.export.nameTaken'))
  const select = useSelectContent()
  return useCallback(async (files) => {
    if (files.length === 0) return { status: 'cancelled' }
    const blobOf = (file: ZoneFile) => store.session.fetchAsset(file.objectId)
    if (files.length === 1) {
      const [file] = files
      const releases: (() => void)[] = []
      try {
        const saved = await saveOne({
          name: file.name, kind: 'file',
          grant: async () => {
            const blob = await blobOf(file)
            if (isMockRuntime()) return 'mock'
            const stored = await uploadToZoneStore(blob, file.name, blob.type || 'application/octet-stream')
            releases.push(stored.release)
            return stored.objectId
          },
          mockFiles: async () => [{ path: '', blob: await blobOf(file) }],
        })
        return saved.status === 'saved' ? { status: 'saved', path: saved.path, count: 1 } : { status: 'cancelled' }
      } finally {
        for (const release of releases) release()
      }
    }
    const result = await select({ mode: 'folder', writable: true, title: t('aiworkspace.directory.saveToZone.folder', { count: files.length }) }, { id: 'save-received' })
    if (result.action !== 'selected') return { status: 'cancelled' }
    const target = result.items[0]
    if (!target || target.origin !== 'zone' || target.source.kind !== 'cyfs-path') throw new Error(t('aiworkspace.directory.saveToZone.zoneOnly'))
    const parent = target.source.path.slice('cyfs://'.length) || '/'
    const used = new Set<string>()
    for (const file of files) {
      const blob = await blobOf(file)
      let name = await freeName(parent, file.name, used)
      if (isMockRuntime()) {
        const { mockPutFile } = await import('../../../filebrowser/mock/content')
        mockPutFile(join(parent, name), blob)
        used.add(name)
        continue
      }
      const stored = await uploadToZoneStore(blob, name, blob.type || 'application/octet-stream')
      try {
        await ensureSession()
        for (let attempt = 0; ; attempt += 1) {
          try {
            await nfspClient().commitObject(parent, name, stored.objectId, { overwrite: false })
            break
          } catch (error) {
            const code = nfspToUiError(error).code
            if (attempt >= 20 || (code !== 'NAMESPACE_CONFLICT' && code !== 'TARGET_MISMATCH')) throw error
            used.add(name)
            name = keepBothName(file.name, (n) => used.has(n))
          }
        }
      } finally {
        stored.release()
      }
      used.add(name)
    }
    if (!isMockRuntime()) notifyDfsPath(parent)
    return { status: 'saved', path: parent, count: files.length }
  }, [store, saveOne, select, t])
}
