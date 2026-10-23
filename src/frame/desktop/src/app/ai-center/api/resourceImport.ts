import { buckyos, ndm } from 'buckyos'

/** A resource field is a single typed file; directory semantics belong to batch fields. */
export async function importPlaygroundResource(file: File, signal: AbortSignal, onProgress?: (percent: number) => void): Promise<string> {
  const endpoint = buckyos.getZoneServiceURL('control-panel').replace(/\/kapi\/control-panel\/?$/, '')
  const sessionToken = (await buckyos.getAccountInfo())?.session_token ?? undefined
  const snapshot = await ndm.importFiles([file], { mode: 'single_file', endpoint, sessionToken, signal })
  try {
    await ndm.ensureUploaded(snapshot.sessionId, { endpoint, sessionToken, signal,
      onProgress: progress => onProgress?.(progress.totalBytes ? Math.round(progress.uploadedBytes * 100 / progress.totalBytes) : 100),
    })
    return snapshot.selection.objectId
  } finally {
    ndm.releaseImportSession(snapshot.sessionId)
  }
}
