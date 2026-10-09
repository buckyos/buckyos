import { registerPreviewSourceResolver } from '../../../components/preview/hostSources'
import { PreviewError, type ContentRef, type ResolvedPreviewSource } from '../../../components/preview/types'
import type { FileObject, ObjId } from '../protocol/feed'

export const HOMESTATION_FILE_SOURCE = 'homestation-file'

export interface HomeStationFileSourceValue {
  objId: ObjId
  name: string
  reference: string
  alt?: string
  file?: FileObject
}

function hueOf(objId: ObjId) {
  let hash = 0
  for (let index = 0; index < objId.length; index += 1) hash = (hash * 31 + objId.charCodeAt(index)) >>> 0
  return hash % 360
}

function escapeXml(text: string) {
  return text.replace(/[<>&"']/g, char => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[char] ?? char)
}

export function placeholderSvg(objId: ObjId, file: FileObject | undefined, label: string | undefined) {
  const hue = hueOf(objId)
  const width = Math.min(file?.meta.width ?? 1200, 1600)
  const height = Math.round(width * ((file?.meta.height ?? 800) / (file?.meta.width ?? 1200)))
  const caption = escapeXml(label ?? file?.name ?? objId)
  const fontSize = Math.round(width / 26)
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 52% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 48) % 360} 46% 38%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="${width * 0.78}" cy="${height * 0.28}" r="${Math.min(width, height) * 0.12}" fill="hsl(${(hue + 180) % 360} 70% 86% / 0.55)"/><path d="M0 ${height * 0.78} L${width * 0.32} ${height * 0.5} L${width * 0.55} ${height * 0.7} L${width * 0.74} ${height * 0.56} L${width} ${height * 0.8} V${height} H0Z" fill="hsl(${hue} 40% 22% / 0.45)"/><text x="${width / 2}" y="${height - fontSize * 1.2}" font-family="system-ui, sans-serif" font-size="${fontSize}" fill="white" fill-opacity="0.92" text-anchor="middle">${caption}</text></svg>`
}

export function mediaUrl(objId: ObjId, file: FileObject | undefined, label?: string) {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(placeholderSvg(objId, file, label))}`
}

export function fileSourceRef(objId: ObjId, file: FileObject | undefined, alt?: string): ContentRef {
  const value: HomeStationFileSourceValue = { objId, name: `${alt ?? file?.name ?? 'image'}.svg`, reference: objId, alt, file }
  return { kind: HOMESTATION_FILE_SOURCE, value }
}

async function resolveFile(source: ContentRef): Promise<ResolvedPreviewSource> {
  const value = (source as { value?: HomeStationFileSourceValue }).value
  if (!value?.objId) throw new PreviewError('NOT_FOUND', 'Unknown HomeStation file')
  const svg = placeholderSvg(value.objId, value.file, value.alt)
  return {
    originalSource: source,
    sourceObjectId: value.objId,
    inputObjectId: value.objId,
    displayName: value.name,
    size: svg.length,
    objectType: 'file',
    mediaTypeHints: ['image/svg+xml'],
    readRef: { kind: 'blob', blob: new Blob([svg], { type: 'image/svg+xml' }) },
  }
}

let installed = false

export function installHomeStationPreviewSource() {
  if (installed) return
  installed = true
  registerPreviewSourceResolver(HOMESTATION_FILE_SOURCE, { resolvePreviewSource: resolveFile })
}
