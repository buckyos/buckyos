import { buckyos, ndm } from 'buckyos'
import { callRpc } from '../../api/rpc'

export interface PikgInspection {
  sourceObjId: string
  staging: { handle: string; pikg_digest: string; size: number; expires_at: number }
  inspection: {
    plan: { app: { show_name: string; version: string; did: string; object_id: string }; plan_fingerprint: string }
    status: { readiness: Record<string, unknown>; warnings?: string[] }
  }
}

async function rpc<T>(method: string, params: Record<string, unknown>): Promise<T> {
  const result = await callRpc<T>(method, params)
  if (result.error || !result.data) throw result.error ?? new Error(`${method} returned no result`)
  return result.data
}

export async function releasePikgInspection(handle: string): Promise<void> {
  await rpc('apps.staging.release', { staging_handle: handle })
}

/** Real package validation. Installation is a separate, explicitly approved operation. */
export async function inspectPikgFile(file: File, signal: AbortSignal, onProgress: (state: 'preparing' | 'importing', percent: number) => void): Promise<PikgInspection> {
  signal.throwIfAborted()
  const endpoint = buckyos.getZoneServiceURL('control-panel').replace(/\/kapi\/control-panel\/?$/, '')
  const sessionToken = (await buckyos.getAccountInfo())?.session_token ?? undefined
  onProgress('preparing', 0)
  const imported = await ndm.importFiles([file], { mode: 'single_file', endpoint, sessionToken, signal })
  let staged: PikgInspection['staging'] | undefined
  let complete = false
  try {
    await ndm.ensureUploaded(imported.sessionId, { endpoint, sessionToken, signal, onProgress: progress => onProgress('importing', progress.totalBytes ? 100 * progress.uploadedBytes / progress.totalBytes : 0) })
    signal.throwIfAborted()
    // The canonical FileObject binds the bytes. Staging computes and validates
    // the package digest and manifest; the browser does not hash the file again.
    staged = await rpc<PikgInspection['staging']>('apps.staging.finalize', { source_obj_id: imported.selection.objectId, size: file.size, purpose: 'inspect' })
    signal.throwIfAborted()
    const inspection = await rpc<PikgInspection['inspection']>('apps.inspect', { source: { kind: 'local_pikg', staging_handle: staged.handle } })
    signal.throwIfAborted()
    complete = true
    return { sourceObjId: imported.selection.objectId, staging: staged, inspection }
  } finally {
    ndm.releaseImportSession(imported.sessionId)
    if (staged && !complete) await releasePikgInspection(staged.handle).catch(error => console.warn('PIKG staging will expire after failed inspection:', error))
  }
}
