import type { ndn } from 'buckyos'
import type { ObjId } from '../protocol/feed'

function hueOf(objId: ObjId) {
  let hash = 0
  for (let index = 0; index < objId.length; index += 1) hash = (hash * 31 + objId.charCodeAt(index)) >>> 0
  return hash % 360
}

function escapeXml(text: string) {
  return text.replace(/[<>&"']/g, char => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[char] ?? char)
}

export function placeholderSvg(objId: ObjId, file: ndn.FileObjectJson | undefined, label: string | undefined) {
  const hue = hueOf(objId)
  const width = Math.min(file?.width ?? 1200, 1600)
  const height = Math.round(width * ((file?.height ?? 800) / (file?.width ?? 1200)))
  const caption = escapeXml(label ?? file?.name ?? objId)
  const fontSize = Math.round(width / 44)
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue} 26% 74%)"/><stop offset="1" stop-color="hsl(${hue} 22% 58%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="${width * 0.76}" cy="${height * 0.3}" r="${Math.min(width, height) * 0.09}" fill="hsl(${hue} 40% 92% / 0.7)"/><path d="M0 ${height * 0.8} L${width * 0.3} ${height * 0.54} L${width * 0.52} ${height * 0.72} L${width * 0.72} ${height * 0.58} L${width} ${height * 0.82} V${height} H0Z" fill="hsl(${hue} 20% 30% / 0.35)"/><text x="${fontSize * 1.5}" y="${height - fontSize * 1.5}" font-family="system-ui, sans-serif" font-size="${fontSize}" fill="white" fill-opacity="0.8">${caption}</text></svg>`
}
