import { useCallback } from 'react'
import { useSelectContent } from '../sysdlg'
import { isMockRuntime } from '../runtime'
import { ensureSession, nfspClient } from '../app/filebrowser/data/nfsp/client'
import { nfspToUiError } from '../app/filebrowser/data/nfsp/errors'
import { notifyDfsPath } from '../app/filebrowser/data/nfsp/invalidation'

/** Something received (an attachment, a posted file) the viewer can keep in their own files. */
export interface ZoneSaveSource {
  /** The name suggested in the picker. */
  name: string
  kind: 'file' | 'dir'
  /**
   * Hands the object to the viewer's own session — the service that showed it decides whether the viewer reads it
   * (msg-center `msg.prepare_save_attachment`, HomeStation `…/objects/<id>/save`) — and answers its object ID.
   */
  grant(): Promise<string>
  /** Mock runtime: the bytes to place, by path below the saved item (`''` is the item itself, a file). */
  mockFiles(): Promise<Array<{ path: string; blob: Blob }>>
}

export type ZoneSaveResult = { status: 'saved'; path: string } | { status: 'cancelled' }

const join = (parent: string, name: string) => `${parent === '/' ? '' : parent}/${name}`

/**
 * Saving into the Zone, the user's main file system: the user picks a folder and a name in the system picker, then
 * the object is committed there by the file service from the named store (`commit_object`) with the user's own
 * session — the bytes never pass through the browser. A folder is never put over what is there: a taken name asks
 * for another one. `nameTaken` is the picker's title then.
 */
export function useSaveToZone(nameTaken: string): (source: ZoneSaveSource) => Promise<ZoneSaveResult> {
  const select = useSelectContent()
  return useCallback(async (source) => {
    let suggestedName = source.name
    let title: string | undefined
    let objId: string | undefined
    for (;;) {
      const result = await select({ mode: 'save', suggestedName, ...(title ? { title } : {}) }, { id: 'save-received' })
      if (result.action !== 'save') return { status: 'cancelled' }
      if (result.parent.kind !== 'cyfs-path') throw new Error('Only a Zone folder can be the destination')
      const parent = result.parent.path.slice('cyfs://'.length) || '/'
      const retry = () => { suggestedName = result.name; title = nameTaken }
      if (source.kind === 'dir' && result.overwrite) { retry(); continue }
      const path = join(parent, result.name)
      objId ??= await source.grant()
      if (isMockRuntime()) {
        const { mockPutFile } = await import('../app/filebrowser/mock/content')
        for (const file of await source.mockFiles()) mockPutFile(file.path ? join(path, file.path) : path, file.blob)
        return { status: 'saved', path }
      }
      try {
        await ensureSession()
        await nfspClient().commitObject(parent, result.name, objId, { overwrite: result.overwrite })
      } catch (error) {
        // Taken since the picker looked: ask again rather than put anything over it.
        const code = nfspToUiError(error).code
        if (code === 'NAMESPACE_CONFLICT' || (code === 'TARGET_MISMATCH' && !result.overwrite)) { retry(); continue }
        throw error
      }
      notifyDfsPath(parent)
      return { status: 'saved', path }
    }
  }, [select, nameTaken])
}
