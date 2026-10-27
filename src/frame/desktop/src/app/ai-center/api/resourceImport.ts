import { bucket, buckyos, ndm } from 'buckyos'

/** AICC's per-user bucket for call inputs (`temp`, one day): the upload stays retained while the request uses it. */
const AICC_APP_ID = 'aicc'
const AICC_INPUTS_BUCKET = 'inputs'

/** A resource field is a single typed file; directory semantics belong to batch fields. */
export async function importPlaygroundResource(file: File, signal: AbortSignal, onProgress?: (percent: number) => void): Promise<string> {
  const endpoint = new URL(buckyos.getZoneServiceURL('control-panel'), window.location.origin).origin
  const account = await buckyos.getAccountInfo()
  if (!account?.user_id) throw new Error('Sign in to upload a resource')
  const sessionToken = account.session_token ?? undefined
  const snapshot = await ndm.importFiles([file], { mode: 'single_file', endpoint, sessionToken, signal })
  try {
    const objectId = snapshot.selection.objectId
    const status = await ndm.ensureUploaded(snapshot.sessionId, { endpoint, sessionToken, signal,
      bind: {
        bucketId: bucket.userBucketId(AICC_APP_ID, account.user_id, AICC_INPUTS_BUCKET),
        key: `${objectId.slice(objectId.indexOf(':') + 1, objectId.indexOf(':') + 17)}/${file.name.replace(/[/\\]/g, '_') || 'input'}`,
        ...(file.type ? { contentType: file.type } : {}),
      },
      onProgress: progress => onProgress?.(progress.totalBytes ? Math.round(progress.uploadedBytes * 100 / progress.totalBytes) : 100),
    })
    return status.binding?.obj_id ?? objectId
  } finally {
    ndm.releaseImportSession(snapshot.sessionId)
  }
}
