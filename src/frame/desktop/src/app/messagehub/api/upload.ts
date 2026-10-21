/**
 * Attachment upload through the zone NDM gateway (`/ndm/v1/uploads`, served
 * by nfs-server). Files are materialized into FileObjects client-side and
 * uploaded with the SDK import session; the resulting object ids become
 * `refs[].target.obj_id` on the outgoing message.
 */
import { buckyos, ndm, ndn } from 'buckyos'
import { MAX_ATTACHMENT_BYTES, type ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import { jpegName, normalizeImageFile } from '../conversation/input/imageNormalize'
import { currentSessionToken } from '../datamodel/sessionApi'
import type { OutgoingUpload } from '../store/types'

export interface UploadedAttachment {
  objId: string
  name: string
  size: number
  mimeType?: string
  file: File
}

const UPLOAD_POLL_MS = 400
const UPLOAD_TIMEOUT_MS = 10 * 60_000

function ndmEndpoint(): string {
  return buckyos.getZoneServiceURL('control-panel').replace(/\/kapi\/control-panel\/?$/, '')
}

/**
 * The import session uploads chunk data only; the FileObject JSON (whose id
 * is the reference the message carries) stays client-side. Publish it to the
 * zone named store so the msg-center object route (and any other reader) can
 * resolve the reference. The SDK's structured store helper refuses the pure
 * browser runtime, so the gateway endpoint is called directly.
 */
async function putFileObject(endpoint: string, objId: string, fileObject: Record<string, unknown>): Promise<void> {
  const token = await currentSessionToken()
  const response = await fetch(`${endpoint}/ndm/v1/store/put_object`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', accept: 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    credentials: 'include',
    body: JSON.stringify({ obj_id: objId, obj_data: ndn.toCanonicalJsonString(fileObject) }),
  })
  if (!response.ok && response.status !== 204) {
    throw new Error(`attachment_upload_failed: put_object ${response.status} ${(await response.text().catch(() => '')).slice(0, 200)}`)
  }
}

export async function uploadAttachments(attachments: readonly ComposerAttachmentInput[], onProgress?: (progress: OutgoingUpload) => void): Promise<UploadedAttachment[]> {
  if (attachments.length === 0) return []
  const oversize = attachments.filter(item => item.file.size > MAX_ATTACHMENT_BYTES)
  if (oversize.length > 0) throw new Error(`attachment_too_large: ${oversize.map(item => item.relativePath || item.file.name).join(', ')}`)
  onProgress?.({ phase: 'preparing' })
  const files: File[] = []
  for (const item of attachments) files.push(await normalizeImageFile(item.file))
  ndm.setImportProvider({
    // Pure browser runtime: nothing is materialized locally, so every file
    // must go through the TUS upload (`canUseNDMStore` would mark the import
    // as already stored and skip the upload).
    getCapabilities: () => ({ canRevealRealPath: false, canUseNDMCache: false, canUseNDMStore: false, canPickDirectory: false, canPickMixed: false }),
    pickFiles: async () => files,
  })
  let snapshot
  try {
    snapshot = await ndm.pickupAndImport({ mode: 'multi_file', autoStartUpload: false })
  } catch (error) {
    throw new Error(`attachment_upload_unavailable: ${error instanceof Error ? error.message : String(error)}`)
  }
  const imported = snapshot.selection
  if (imported.length !== files.length) throw new Error('attachment_upload_failed: incomplete import')
  const total = files.reduce((sum, file) => sum + file.size, 0)
  onProgress?.({ phase: 'uploading', loaded: 0, total })
  const status = await ndm.startUpload(snapshot.sessionId, { endpoint: ndmEndpoint(), priority: 'foreground' })
  if (status.uploadStatus !== 'completed' && status.uploadStatus !== 'not_required') {
    const started = Date.now()
    for (;;) {
      await new Promise(resolve => setTimeout(resolve, UPLOAD_POLL_MS))
      const progress = await ndm.getUploadProgress(snapshot.sessionId)
      if (progress.uploadStatus === 'completed' || progress.uploadStatus === 'not_required') break
      onProgress?.({ phase: 'uploading', loaded: Math.min(progress.uploadedBytes, total), total })
      if (progress.uploadStatus === 'failed') throw new Error('attachment_upload_failed')
      if (Date.now() - started > UPLOAD_TIMEOUT_MS) throw new Error('attachment_upload_failed: timeout')
    }
  }
  const endpoint = ndmEndpoint()
  for (const object of imported) {
    if (!object._ndnFileObject) throw new Error('attachment_upload_failed: missing file object')
    await putFileObject(endpoint, object.objectId, object._ndnFileObject)
  }
  onProgress?.({ phase: 'uploading', loaded: total, total })
  return imported.map((object, index) => {
    const converted = files[index] !== attachments[index].file
    const relativePath = attachments[index].relativePath
    return {
      objId: object.objectId,
      name: (relativePath && (converted ? jpegName(relativePath) : relativePath)) || object.name || files[index].name,
      size: object.size,
      mimeType: object.mimeType || files[index].type || undefined,
      file: files[index],
    }
  })
}
