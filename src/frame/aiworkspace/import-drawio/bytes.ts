/* Byte helpers shared by the file readers: base64, raw deflate through the platform's (De)CompressionStream
 * (browsers, Deno and Node all have it), UTF-8, and the text chunks of a PNG. */

export function base64ToBytes(text: string): Uint8Array {
  const binary = atob(text.replace(/\s+/g, ''))
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
  return btoa(binary)
}

export const utf8 = new TextDecoder('utf-8')
export const latin1 = new TextDecoder('latin1')

async function pipe(bytes: Uint8Array, stream: TransformStream<Uint8Array, Uint8Array>): Promise<Uint8Array> {
  const writer = stream.writable.getWriter()
  void writer.write(bytes).catch(() => undefined)
  void writer.close().catch(() => undefined)
  const chunks: Uint8Array[] = []
  const reader = stream.readable.getReader()
  let total = 0
  for (;;) {
    const { value, done } = await reader.read()
    if (done) break
    chunks.push(value)
    total += value.length
  }
  const out = new Uint8Array(total)
  let at = 0
  for (const c of chunks) { out.set(c, at); at += c.length }
  return out
}

export function inflateRaw(bytes: Uint8Array): Promise<Uint8Array> {
  return pipe(bytes, new DecompressionStream('deflate-raw') as unknown as TransformStream<Uint8Array, Uint8Array>)
}

/** zlib-wrapped deflate (a PNG `zTXt` chunk). */
export function inflate(bytes: Uint8Array): Promise<Uint8Array> {
  return pipe(bytes, new DecompressionStream('deflate') as unknown as TransformStream<Uint8Array, Uint8Array>)
}

export function deflateRaw(bytes: Uint8Array): Promise<Uint8Array> {
  return pipe(bytes, new CompressionStream('deflate-raw') as unknown as TransformStream<Uint8Array, Uint8Array>)
}

const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10]

export function isPng(bytes: Uint8Array): boolean {
  return bytes.length > 8 && PNG_SIGNATURE.every((b, i) => bytes[i] === b)
}

/** The `tEXt` / `zTXt` / `iTXt` chunks of a PNG, keyword → text (Latin-1 for tEXt and zTXt, UTF-8 for iTXt). */
export async function pngTextChunks(bytes: Uint8Array): Promise<Map<string, string>> {
  const out = new Map<string, string>()
  if (!isPng(bytes)) return out
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let at = 8
  while (at + 8 <= bytes.length) {
    const length = view.getUint32(at)
    const type = latin1.decode(bytes.subarray(at + 4, at + 8))
    const data = bytes.subarray(at + 8, at + 8 + length)
    at += 12 + length
    if (type === 'IEND') break
    if (type !== 'tEXt' && type !== 'zTXt' && type !== 'iTXt') continue
    const zero = data.indexOf(0)
    if (zero < 0) continue
    const keyword = latin1.decode(data.subarray(0, zero))
    try {
      if (type === 'tEXt') out.set(keyword, latin1.decode(data.subarray(zero + 1)))
      else if (type === 'zTXt') out.set(keyword, latin1.decode(await inflate(data.subarray(zero + 2))))
      else {
        // keyword \0 compression-flag method language \0 translated \0 text
        const compressed = data[zero + 1] === 1
        let p = zero + 3
        const lang = data.indexOf(0, p); p = lang + 1
        const translated = data.indexOf(0, p); p = translated + 1
        const body = data.subarray(p)
        out.set(keyword, utf8.decode(compressed ? await inflate(body) : body))
      }
    } catch { /* a broken chunk is skipped */ }
  }
  return out
}

/** The pixel size of a PNG, JPEG, GIF, WebP or SVG image (null when unknown). */
export function imageSize(bytes: Uint8Array, mediaType: string): { width: number; height: number } | null {
  try {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    if (isPng(bytes)) return { width: view.getUint32(16), height: view.getUint32(20) }
    if (bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return { width: view.getUint16(6, true), height: view.getUint16(8, true) }
    if (bytes[0] === 0xff && bytes[1] === 0xd8) {
      let at = 2
      while (at + 9 < bytes.length) {
        if (bytes[at] !== 0xff) { at += 1; continue }
        const marker = bytes[at + 1]
        const length = view.getUint16(at + 2)
        if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) return { width: view.getUint16(at + 7), height: view.getUint16(at + 5) }
        at += 2 + length
      }
    }
    if (mediaType === 'image/svg+xml') {
      const text = utf8.decode(bytes.subarray(0, 4096))
      const w = /<svg[^>]*\swidth="([\d.]+)/.exec(text)
      const h = /<svg[^>]*\sheight="([\d.]+)/.exec(text)
      if (w && h) return { width: Number(w[1]), height: Number(h[1]) }
      const vb = /<svg[^>]*\sviewBox="[\d.-]+[\s,]+[\d.-]+[\s,]+([\d.]+)[\s,]+([\d.]+)/.exec(text)
      if (vb) return { width: Number(vb[1]), height: Number(vb[2]) }
    }
  } catch { /* unknown */ }
  return null
}
