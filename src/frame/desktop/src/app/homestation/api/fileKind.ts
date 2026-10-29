import { extensionOf, heifImageSize, mediaTypeFromExtension, sniffMagic, stripMediaTypeParams } from '../../../components/preview/mediaTypes'
import type { AttachmentInput } from '../datamodel/inputs'

// Enough for the `ftyp` and the `meta` box of an iPhone HEIC.
const HEAD_BYTES = 256 * 1024
const GENERIC = new Set(['', 'application/octet-stream', 'binary/octet-stream'])

export interface SniffedFile {
  mime: string
  head: Uint8Array
}

/**
 * Media type of a picked file from its bytes: Chromium leaves `File.type`
 * empty for HEIC and reports `video/quicktime` only by extension, so the
 * magic bytes decide for media; other files keep the declared or extension type.
 */
export async function sniffFile(file: File): Promise<SniffedFile> {
  const head = new Uint8Array(await file.slice(0, HEAD_BYTES).arrayBuffer())
  const magic = sniffMagic(head)
  const declared = stripMediaTypeParams(file.type)
  const media = magic && /^(image|video|audio)\//.test(magic) ? magic : undefined
  const mime = media ?? (GENERIC.has(declared) ? undefined : declared) ?? mediaTypeFromExtension(extensionOf(file.name)) ?? magic ?? 'application/octet-stream'
  return { mime, head }
}

export function attachmentKindOf(mime: string): AttachmentInput['kind'] {
  if (mime.startsWith('video/')) return 'video'
  if (mime.startsWith('audio/')) return 'audio'
  if (mime.startsWith('image/')) return 'image'
  return 'file'
}

export async function attachmentKindOfFile(file: File): Promise<AttachmentInput['kind']> {
  return attachmentKindOf((await sniffFile(file)).mime)
}

export function heifSizeOf(head: Uint8Array): { width: number; height: number } | undefined {
  return heifImageSize(head) ?? undefined
}
