/* Exporting a workspace package (UI framework PRD §9.3; the phone's main menu, §6.1): saved into the Zone by default —
 * the user's main file system, chosen in the system file picker's save mode — or downloaded to this device.
 *
 * Shared by the export dialog of the home's card menu and of a workspace's main menu:
 *
 *   const exporter = useWorkspacePackageExport()
 *   const saved = await exporter.toZone(client, { workspace_id, title }, { mode: 'share', selfContained: true, notes: false })
 *   if (saved.status === 'saved') … saved.path (the DFS path it was written to), saved.exported.result (the manifest)
 *   const pkg = await exporter.toDevice(client, workspace, options)   // the browser's download; pkg.blob, pkg.result
 *
 * `toZone` asks for the place first (folder and name, a taken name is asked about by the picker), then exports, then
 * writes the file there with the user's own session; the user cancelling the picker resolves `{ status: 'cancelled' }`
 * and exports nothing. Failures (the export refused, the write refused) are thrown.
 *
 * How the bytes get there: the service leaves the package in the caller's own `exports` bucket; nothing hands that
 * object to the user's files yet (msg-center's `prepare_save_attachment` does this for attachments through
 * nfs-server `grant_holdings`). Until the service has the same, the package is read into the browser and uploaded
 * into the Zone the way the Files app uploads (ndm import, then `commit_object` into the chosen folder). */

import { useMemo } from 'react'
import { getActiveSessionToken, ndm } from 'buckyos'
import { useSaveToZone } from '../../../../components/saveToZone'
import { isMockRuntime } from '../../../../runtime'
import { nfspBaseUrl } from '../../../filebrowser/data/nfsp/client'
import { unwrap, type AiwsClient } from '../../api/client'
import type { ExportResult } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { saveBlob } from '../shell/download'

export type PackageMode = 'share' | 'personal_backup'
export interface PackageOptions { mode: PackageMode; selfContained: boolean; notes: boolean }
export interface PackageWorkspace { workspace_id: string; title: string }
export interface ExportedPackage { result: ExportResult; blob: Blob; fileName: string }
export type ZoneSave = { status: 'saved'; path: string; exported: ExportedPackage } | { status: 'cancelled' }

export interface WorkspacePackageExport {
  /** Into the Zone at a place the user picks. */
  toZone(client: AiwsClient, workspace: PackageWorkspace, options: PackageOptions): Promise<ZoneSave>
  /** A download of this browser. */
  toDevice(client: AiwsClient, workspace: PackageWorkspace, options: PackageOptions): Promise<ExportedPackage>
}

/** `<title>-share.zip` / `<title>-backup.zip`, without the characters a file name cannot have. */
export function packageFileName(title: string, mode: PackageMode): string {
  // eslint-disable-next-line no-control-regex -- control characters are what is being removed
  const stem = title.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, '-').replace(/^[\s.-]+|[\s.]+$/g, '').slice(0, 120) || 'workspace'
  return `${stem}-${mode === 'share' ? 'share' : 'backup'}.zip`
}

/** `doc.export`, and the package it left, read with its token. */
export async function exportPackage(client: AiwsClient, workspace: PackageWorkspace, options: PackageOptions): Promise<ExportedPackage> {
  const result = unwrap(await client.export({ workspace_id: workspace.workspace_id }, options.mode, options.selfContained, options.notes))
  const blob = await client.transport.fetchDownload(result.download)
  return { result, blob, fileName: packageFileName(workspace.title, options.mode) }
}

/** The bytes as an object of the user's own session in the Zone's store, ready to be committed into a folder (also how
 * a directory's files are saved into the Zone, ui/directory/saveToZone.ts). */
export async function uploadToZoneStore(blob: Blob, name: string, type = blob.type || 'application/zip'): Promise<{ objectId: string; release: () => void }> {
  const endpoint = nfspBaseUrl()
  const sessionToken = new URL(endpoint).origin === window.location.origin ? await getActiveSessionToken() : null
  const options = { endpoint, sessionToken }
  const snapshot = await ndm.importFiles([new File([blob], name, { type })], { ...options, mode: 'single_file', autoStartUpload: false })
  const release = () => ndm.releaseImportSession(snapshot.sessionId)
  try {
    const status = await ndm.ensureUploaded(snapshot.sessionId, options)
    const object = snapshot.items[0]
    if (!object || status.objectStatus !== 'ready') throw new Error('The package was not stored in the Zone')
    return { objectId: object.objectId, release }
  } catch (error) {
    release()
    throw error
  }
}

export function useWorkspacePackageExport(): WorkspacePackageExport {
  const { t } = useAiwsText()
  const save = useSaveToZone(t('aiworkspace.home.export.nameTaken'))
  return useMemo<WorkspacePackageExport>(() => ({
    toZone: async (client, workspace, options) => {
      // made once, after the place is chosen (the picker asks again for a taken name without exporting again)
      const made: { pkg?: Promise<ExportedPackage>; release?: () => void } = {}
      const pkg = () => (made.pkg ??= exportPackage(client, workspace, options))
      try {
        const saved = await save({
          name: packageFileName(workspace.title, options.mode),
          kind: 'file',
          grant: async () => {
            const exported = await pkg()
            // the mock Zone takes the bytes themselves (mockFiles)
            if (isMockRuntime()) return 'mock'
            const stored = await uploadToZoneStore(exported.blob, exported.fileName)
            made.release = stored.release
            return stored.objectId
          },
          mockFiles: async () => [{ path: '', blob: (await pkg()).blob }],
        })
        if (saved.status !== 'saved' || !made.pkg) return { status: 'cancelled' }
        return { status: 'saved', path: saved.path, exported: await made.pkg }
      } finally {
        made.release?.()
      }
    },
    toDevice: async (client, workspace, options) => {
      const exported = await exportPackage(client, workspace, options)
      saveBlob(exported.blob, exported.fileName)
      return exported
    },
  }), [save])
}
