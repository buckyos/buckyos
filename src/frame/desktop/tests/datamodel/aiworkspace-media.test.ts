/* AI Workspace media assets (doc/workspace/BuckyOS AI Workspace 多媒体对象.md): embedded covers and the kind helpers.
 *
 *   deno test --no-check --allow-env --allow-read --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/aiworkspace-media.test.ts */

import { embeddedCover } from '../../src/app/aiworkspace/ui/media/audioCover.ts'
import { assetMediaType, fileBadge, formatDuration, mediaKindOf } from '../../src/app/aiworkspace/ui/media/kinds.ts'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

const enc = new TextEncoder()
const bytes = (...parts: (string | number[] | Uint8Array)[]) => {
  const chunks = parts.map((p) => (typeof p === 'string' ? enc.encode(p) : p instanceof Uint8Array ? p : Uint8Array.from(p)))
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0))
  let at = 0
  for (const c of chunks) { out.set(c, at); at += c.length }
  return out
}
const be32 = (n: number) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255]
const syncsafe = (n: number) => [(n >> 21) & 127, (n >> 14) & 127, (n >> 7) & 127, n & 127]
const jpeg = (mark: number) => bytes([0xff, 0xd8, 0xff, 0xe0], new Array(20).fill(mark))
const png = bytes([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], new Array(20).fill(7))

function apic(type: number, data: Uint8Array, encoding = 0) {
  const description = encoding === 1 ? [0xff, 0xfe, 0x41, 0, 0, 0] : [0x41, 0]
  return bytes([encoding], 'image/jpeg', [0, type], description, data)
}
function id3(major: 3 | 4, frames: { id: string; body: Uint8Array; flags?: number }[]) {
  const body = bytes(...frames.map((f) => bytes(f.id, major === 4 ? syncsafe(f.body.length) : be32(f.body.length), [0, f.flags ?? 0], f.body)))
  return bytes('ID3', [major, 0, 0], syncsafe(body.length + 16), body, new Array(16).fill(0), [0xff, 0xfb, 0x90, 0x64])
}
const box = (type: string, ...body: Uint8Array[]) => { const inner = bytes(...body); return bytes(be32(inner.length + 8), type, inner) }

Deno.test('ID3v2.3: the front cover wins over an earlier picture', () => {
  const file = id3(3, [{ id: 'TIT2', body: bytes([0], 'Song') }, { id: 'APIC', body: apic(0, jpeg(1)) }, { id: 'APIC', body: apic(3, jpeg(2), 1) }])
  const cover = embeddedCover(file)
  equal(cover?.mediaType, 'image/jpeg')
  equal(cover?.data[4], 2)
})

Deno.test('ID3v2.4: a frame with a data length indicator', () => {
  const body = apic(3, png)
  const file = id3(4, [{ id: 'APIC', body: bytes(syncsafe(body.length), body), flags: 0x01 }])
  equal(embeddedCover(file)?.mediaType, 'image/png')
})

Deno.test('ID3v2.2: a PIC frame', () => {
  const pic = bytes([0], 'JPG', [3], 'x', [0], jpeg(5))
  const frames = bytes('PIC', [(pic.length >> 16) & 255, (pic.length >> 8) & 255, pic.length & 255], pic)
  const file = bytes('ID3', [2, 0, 0], syncsafe(frames.length), frames)
  equal(embeddedCover(file)?.data[4], 5)
})

Deno.test('FLAC: a PICTURE block after STREAMINFO', () => {
  const picture = bytes(be32(3), be32(9), 'image/png', be32(0), be32(4), be32(4), be32(24), be32(0), be32(png.length), png)
  const file = bytes('fLaC', [0, 0, 0, 34], new Array(34).fill(0), [0x80 | 6, (picture.length >> 16) & 255, (picture.length >> 8) & 255, picture.length & 255], picture)
  equal(embeddedCover(file)?.mediaType, 'image/png')
})

Deno.test('MP4: moov/udta/meta/ilst/covr/data', () => {
  const data = box('data', bytes(be32(13), be32(0)), jpeg(9))
  const meta = box('meta', bytes([0, 0, 0, 0]), box('hdlr', bytes(new Array(25).fill(0))), box('ilst', box('covr', data)))
  const file = bytes(box('ftyp', bytes('M4A ', be32(0), 'M4A mp42isom')), box('mdat', bytes(new Array(40).fill(1))), box('moov', box('mvhd', bytes(new Array(100).fill(0))), box('udta', meta)))
  equal(embeddedCover(file)?.data[4], 9)
})

Deno.test('no cover: plain audio, a damaged tag, other files', () => {
  equal(embeddedCover(id3(3, [{ id: 'TIT2', body: bytes([0], 'Song') }])), null)
  equal(embeddedCover(bytes('ID3', [3, 0, 0], syncsafe(1000), 'APIC', be32(5000))), null)
  equal(embeddedCover(bytes('RIFF', be32(0), 'WAVEfmt ')), null)
  equal(embeddedCover(new Uint8Array(0)), null)
})

Deno.test('kinds: the backend type first, the file name when the bytes said nothing', () => {
  equal(assetMediaType('video/mp4', 'clip.mov'), 'video/mp4')
  equal(assetMediaType('application/octet-stream', 'talk.mp3'), 'audio/mpeg')
  equal(assetMediaType('application/zip', 'report.docx'), 'application/vnd.openxmlformats-officedocument.wordprocessingml.document')
  equal(assetMediaType(undefined, undefined), 'application/octet-stream')
  equal(mediaKindOf('image/png'), 'image')
  equal(mediaKindOf('image/heic'), 'file')
  equal(mediaKindOf('video/webm'), 'video')
  equal(mediaKindOf('application/ogg'), 'audio')
  equal(mediaKindOf('application/pdf'), 'pdf')
  equal(mediaKindOf('application/octet-stream', 'notes.txt'), 'file')
})

Deno.test('labels: durations and file badges', () => {
  equal(formatDuration(65_000), '1:05')
  equal(formatDuration(3_723_000), '1:02:03')
  equal(formatDuration(400), '0:00')
  equal(fileBadge('application/pdf', 'a.pdf'), 'PDF')
  equal(fileBadge('application/x-tar', undefined), 'TAR')
})
