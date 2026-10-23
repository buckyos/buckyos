import { buckyos, ndm } from 'buckyos'
import { attachmentSize, oversizedAttachmentNames, type ComposerAttachmentInput } from '../conversation/input/attachmentDraft'
import { normalizeImageFile } from '../conversation/input/imageNormalize'
import { currentSessionToken } from '../datamodel/sessionApi'
import type { OutgoingUpload } from '../store/types'

export interface UploadedAttachment {
  objId: string
  name: string
  size: number
  mimeType?: string
  file?: File
}

const sessions = new WeakMap<File, { sessionId: string; file?: File; object: { objectId: string; name: string; size: number } }>()

export async function uploadAttachments(attachments: readonly ComposerAttachmentInput[], onProgress?: (progress: OutgoingUpload) => void, signal?: AbortSignal): Promise<UploadedAttachment[]> {
  const endpoint = buckyos.getZoneServiceURL('control-panel').replace(/\/kapi\/control-panel\/?$/, '')
  const sessionToken = (await currentSessionToken()) ?? undefined
  const oversize = attachments.flatMap(oversizedAttachmentNames)
  if (oversize.length) throw new Error(`attachment_too_large: ${oversize.join(', ')}`)
  const total = attachments.reduce((sum, item) => sum + attachmentSize(item), 0)
  let completed = 0
  const uploaded: UploadedAttachment[] = []
  onProgress?.({ phase: 'preparing' })
  for (const item of attachments) {
    signal?.throwIfAborted()
    let session = sessions.get(item.file)
    if (!session) {
      const file = item.directory ? undefined : await normalizeImageFile(item.file)
      const options = { endpoint, sessionToken, signal, autoStartUpload: false }
      const snapshot = item.directory
        ? await ndm.importDirectory(item.directory, options)
        : await ndm.importFiles([file!], { ...options, mode: 'single_file' })
      const object = snapshot.items[0]
      if (!object) throw new Error('attachment_upload_failed: missing object')
      session = { sessionId: snapshot.sessionId, file, object: { objectId: object.objectId, name: object.name, size: file?.size ?? attachmentSize(item) } }
      sessions.set(item.file, session)
    }
    const progress = () => void ndm.getUploadProgress(session!.sessionId).then(value => {
      onProgress?.({ phase: 'uploading', loaded: Math.min(total, completed + value.uploadedBytes), total })
    }, () => undefined)
    const timer = setInterval(progress, 200)
    try {
      const status = await ndm.ensureUploaded(session.sessionId, { endpoint, sessionToken, signal, priority: 'foreground' })
      if (status.objectStatus !== 'ready') throw new Error('attachment_upload_failed: object not ready')
      const object = session.object
      if (!object) throw new Error('attachment_upload_failed: missing object')
      completed += attachmentSize(item)
      onProgress?.({ phase: 'uploading', loaded: completed, total })
      uploaded.push({ objId: object.objectId, name: object.name || item.file.name, size: object.size, mimeType: session.file?.type || undefined, file: session.file })
    } finally {
      clearInterval(timer)
    }
  }
  return uploaded
}

export function releaseAttachmentUploads(attachments: readonly ComposerAttachmentInput[]) {
  for (const item of attachments) {
    const session = sessions.get(item.file)
    if (session) ndm.releaseImportSession(session.sessionId)
    sessions.delete(item.file)
  }
}
