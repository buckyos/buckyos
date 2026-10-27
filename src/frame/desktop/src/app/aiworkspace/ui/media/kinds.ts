/* What an asset is to the canvas (多媒体对象 §2): a picture shown as itself, a video or a sound played in place,
 * a PDF shown by its first page, any other file shown by its icon. Only the Preview component's Runtime profile says
 * what this browser can show or play. */

import { detectRuntimeProfile, extensionOf, mediaTypeFromExtension, stripMediaTypeParams } from '../../../../components/preview/mediaTypes'

export type MediaKind = 'image' | 'video' | 'audio' | 'pdf' | 'file'

const GENERIC = new Set(['', 'application/octet-stream', 'binary/octet-stream'])

/** The asset's media type: the backend's (from the bytes), else the file name's — an older upload, or a format the
 * backend does not know. */
export function assetMediaType(mediaType: string | undefined, fileName: string | undefined): string {
  const sniffed = stripMediaTypeParams(mediaType)
  if (!GENERIC.has(sniffed) && !(sniffed === 'application/zip' && fileName)) return sniffed
  return stripMediaTypeParams(mediaTypeFromExtension(extensionOf(fileName))) || sniffed || 'application/octet-stream'
}

export function mediaKindOf(mediaType: string | undefined, fileName?: string): MediaKind {
  const type = assetMediaType(mediaType, fileName)
  if (type.startsWith('image/')) return detectRuntimeProfile().imageMediaTypes.includes(type) || type === 'image/avif' ? 'image' : 'file'
  if (type.startsWith('video/')) return 'video'
  if (type.startsWith('audio/') || type === 'application/ogg') return 'audio'
  if (type === 'application/pdf') return 'pdf'
  return 'file'
}

/** A video or a sound this browser plays: played in place on the canvas; anything else opens the preview. */
export function playsInPlace(kind: MediaKind, mediaType: string): boolean {
  const runtime = detectRuntimeProfile()
  if (kind === 'video') return runtime.videoMediaTypes.includes(mediaType)
  if (kind === 'audio') return runtime.audioMediaTypes.includes(mediaType === 'application/ogg' ? 'audio/ogg' : mediaType)
  return false
}

/** `1:05`, `1:02:03`. */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = String(total % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`
}

/** `PDF`, `DOCX`: the label of a file card. */
export function fileBadge(mediaType: string, fileName: string | undefined): string {
  const ext = extensionOf(fileName)
  if (ext) return ext.toUpperCase()
  const sub = mediaType.split('/')[1] ?? ''
  return (sub.replace(/^x-/, '').split(/[.+-]/)[0] || 'FILE').toUpperCase()
}
