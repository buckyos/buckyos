import { ndn } from 'buckyos'
import type { AttachmentInput } from '../datamodel/inputs'

const probes = new WeakMap<File, Promise<ndn.FileMediaMeta>>()

/**
 * The common meta of a picked file, from its bytes: Chromium leaves `File.type`
 * empty for HEIC and reports `video/quicktime` only by extension. Picking and
 * uploading share one probe, and the upload hands it on as `fileMeta`.
 */
export function fileMetaOf(file: File): Promise<ndn.FileMediaMeta> {
  let probe = probes.get(file)
  if (!probe) {
    probe = ndn.probeFileMeta(file)
    probes.set(file, probe)
  }
  return probe
}

export function attachmentKindOf(mime: string | undefined): AttachmentInput['kind'] {
  if (mime?.startsWith('video/')) return 'video'
  if (mime?.startsWith('audio/')) return 'audio'
  if (mime?.startsWith('image/')) return 'image'
  return 'file'
}

export async function attachmentKindOfFile(file: File): Promise<AttachmentInput['kind']> {
  return attachmentKindOf((await fileMetaOf(file)).mime)
}
