import type { ObjectAccess, ObjectInfo } from '../conversation/history/objectAccess'

interface MockObject {
  name: string
  mimeType: string
  make: () => Promise<Blob>
}

// Literal codes only; the table is reset before the code width would grow.
function lzwLiterals(indices: Uint8Array, minCodeSize: number): number[] {
  const clear = 1 << minCodeSize
  const width = minCodeSize + 1
  const maxRun = (1 << minCodeSize) - 2
  const out: number[] = []
  let acc = 0
  let bits = 0
  const emit = (code: number) => {
    acc |= code << bits
    bits += width
    while (bits >= 8) {
      out.push(acc & 0xff)
      acc >>= 8
      bits -= 8
    }
  }
  let run = 0
  emit(clear)
  for (const index of indices) {
    if (run === maxRun) {
      emit(clear)
      run = 0
    }
    emit(index)
    run += 1
  }
  emit(clear + 1)
  if (bits > 0) out.push(acc & 0xff)
  return out
}

function encodeGif(width: number, height: number, palette: Array<[number, number, number]>, frames: Uint8Array[], delayCs: number): Uint8Array {
  const bytes: number[] = []
  const push = (...values: number[]) => {
    for (const value of values) bytes.push(value)
  }
  const word = (value: number) => push(value & 0xff, (value >> 8) & 0xff)
  const ascii = (text: string) => push(...[...text].map(char => char.charCodeAt(0)))
  ascii('GIF89a')
  word(width)
  word(height)
  push(0xf2, 0, 0) // global colour table, 8 entries
  for (const [r, g, b] of palette) push(r, g, b)
  push(0x21, 0xff, 0x0b)
  ascii('NETSCAPE2.0')
  push(0x03, 0x01, 0x00, 0x00, 0x00) // loop forever
  for (const frame of frames) {
    push(0x21, 0xf9, 0x04, 0x00)
    word(delayCs)
    push(0x00, 0x00)
    push(0x2c)
    word(0)
    word(0)
    word(width)
    word(height)
    push(0x00)
    push(3)
    const data = lzwLiterals(frame, 3)
    for (let i = 0; i < data.length; i += 255) {
      const chunk = data.slice(i, i + 255)
      push(chunk.length, ...chunk)
    }
    push(0x00)
  }
  push(0x3b)
  return new Uint8Array(bytes)
}

function makeOrbitGif(): Blob {
  const width = 160
  const height = 100
  const palette: Array<[number, number, number]> = [
    [24, 32, 56], [52, 72, 120], [96, 165, 250], [250, 204, 21],
    [244, 114, 182], [255, 255, 255], [34, 197, 94], [15, 20, 36],
  ]
  const frames: Uint8Array[] = []
  const count = 12
  for (let f = 0; f < count; f += 1) {
    const pixels = new Uint8Array(width * height)
    const angle = (f / count) * Math.PI * 2
    const cx = width / 2 + Math.cos(angle) * 48
    const cy = height / 2 + Math.sin(angle) * 28
    for (let y = 0; y < height; y += 1) {
      for (let x = 0; x < width; x += 1) {
        const dx = x - width / 2
        const dy = (y - height / 2) * 1.7
        let color = (x + y) % 16 < 8 ? 0 : 7
        if (Math.abs(Math.hypot(dx, dy) - 50) < 1.2) color = 1
        if (Math.hypot(x - width / 2, y - height / 2) < 10) color = 3
        if (Math.hypot(x - cx, y - cy) < 8) color = f % 2 === 0 ? 2 : 4
        pixels[y * width + x] = color
      }
    }
    frames.push(pixels)
  }
  return new Blob([encodeGif(width, height, palette, frames, 8) as BlobPart], { type: 'image/gif' })
}

function makeFakeMov(): Blob {
  const head = [0, 0, 0, 0x14, ...[...'ftypqt  '].map(c => c.charCodeAt(0)), 0, 0, 0, 0, ...[...'qt  '].map(c => c.charCodeAt(0))]
  return new Blob([new Uint8Array([...head, ...new Array(8192).fill(0)])], { type: 'video/quicktime' })
}

const generators = () => import('../../../components/preview/mockProvider')

const MOCK_OBJECTS: Record<string, MockObject> = {
  'cyfile:mock-gif-orbit': { name: 'orbit-loader.gif', mimeType: 'image/gif', make: async () => makeOrbitGif() },
  'cyfile:mock-photo-harbor': { name: 'harbor-sunset.png', mimeType: 'image/png', make: async () => (await generators()).makeMockPng('Harbor sunset', 'harbor-sunset', 1200, 800) },
  'cyfile:mock-video-clip': {
    name: 'screen-recording.webm',
    mimeType: 'video/webm',
    make: async () => {
      const blob = await (await generators()).makeMockWebm()
      if (!blob) throw new Error('415 this runtime cannot record video')
      return blob
    },
  },
  'cyfile:mock-video-interview': { name: 'interview-cut.mov', mimeType: 'video/quicktime', make: async () => makeFakeMov() },
  'cyfile:mock-doc-brief': { name: 'design-brief.pdf', mimeType: 'application/pdf', make: async () => (await generators()).makeMockPdf('Design brief', ['Dashboard redesign — goals, scope and review dates.', 'Generated MessageHub mock attachment.']) },
}

const blobs = new Map<string, Promise<Blob>>()
const urls = new Map<string, Promise<string>>()

function blobOf(objId: string): Promise<Blob> {
  const spec = MOCK_OBJECTS[objId]
  if (!spec) return Promise.reject(new Error('404 object unavailable'))
  let pending = blobs.get(objId)
  if (!pending) {
    pending = spec.make()
    pending.catch(() => blobs.delete(objId))
    blobs.set(objId, pending)
  }
  return pending
}

export const mockObjectAccess: ObjectAccess = {
  async describe(objId) {
    const spec = MOCK_OBJECTS[objId]
    if (!spec) throw new Error('404 object unavailable')
    const blob = await blobOf(objId).catch(() => null)
    return { objId, isFile: true, name: spec.name, mimeType: spec.mimeType, size: blob?.size } satisfies ObjectInfo
  },
  contentUrl(objId) {
    let pending = urls.get(objId)
    if (!pending) {
      pending = blobOf(objId).then(blob => URL.createObjectURL(blob))
      pending.catch(() => urls.delete(objId))
      urls.set(objId, pending)
    }
    return pending
  },
}
