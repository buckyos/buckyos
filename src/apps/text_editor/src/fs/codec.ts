export const FULL_FEATURE_LIMIT = 4 * 1024 * 1024
export const MAX_FILE_SIZE = 16 * 1024 * 1024
export interface TextFormat { encoding: string; bom: boolean; eol: '\n' | '\r\n' | '\r'; mixedEol: boolean }
export interface DecodedText extends TextFormat { text: string; readOnly: boolean }
export function decodeText(bytes: Uint8Array, fallback?: string): DecodedText {
  if (bytes.length > MAX_FILE_SIZE) throw new Error('too-large')
  let encoding = fallback ?? 'utf-8'; let offset = 0; let bom = false
  if (bytes[0] === 0xff && bytes[1] === 0xfe) { encoding = 'utf-16le'; offset = 2; bom = true }
  else if (bytes[0] === 0xfe && bytes[1] === 0xff) { encoding = 'utf-16be'; offset = 2; bom = true }
  else if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) { offset = 3; bom = true }
  if (!encoding.startsWith('utf-16') && bytes.subarray(0, 8192).includes(0)) throw new Error('binary')
  let text: string; let readOnly = !['utf-8', 'utf-16le', 'utf-16be'].includes(encoding)
  try { text = new TextDecoder(encoding, { fatal: true, ignoreBOM: true }).decode(bytes.subarray(offset)) }
  catch {
    if (fallback || encoding !== 'utf-8') throw new Error('invalid-encoding')
    text = new TextDecoder('utf-8').decode(bytes); readOnly = true
  }
  const counts = { '\n': 0, '\r\n': 0, '\r': 0 }
  for (const match of text.matchAll(/\r\n|\r|\n/g)) counts[match[0] as keyof typeof counts]++
  const eol = (Object.keys(counts) as Array<keyof typeof counts>).sort((a, b) => counts[b] - counts[a])[0]
  return { text: text.replace(/\r\n|\r/g, '\n'), encoding, bom, eol, mixedEol: Object.values(counts).filter(Boolean).length > 1, readOnly }
}
export function encodeText(text: string, format: TextFormat): Uint8Array {
  text = text.replace(/\r\n|\r|\n/g, format.eol)
  if (format.encoding === 'utf-8') {
    const body = new TextEncoder().encode(text)
    if (!format.bom) return body
    const result = new Uint8Array(body.length + 3); result.set([0xef, 0xbb, 0xbf]); result.set(body, 3); return result
  }
  if (format.encoding !== 'utf-16le' && format.encoding !== 'utf-16be') throw new Error('read-only-encoding')
  const result = new Uint8Array(text.length * 2 + (format.bom ? 2 : 0)); const view = new DataView(result.buffer)
  const little = format.encoding === 'utf-16le'; let at = 0
  if (format.bom) { view.setUint16(0, 0xfeff, little); at = 2 }
  for (let i = 0; i < text.length; i++) view.setUint16(at + i * 2, text.charCodeAt(i), little)
  return result
}
export const normalizeEtag = (etag: string | undefined): string => (etag ?? '').replace(/^W\//, '').replace(/^"|"$/g, '')
