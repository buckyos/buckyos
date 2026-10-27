/* The cover picture embedded in a sound file (多媒体对象 §3.2): ID3v2 APIC / PIC (MP3), a FLAC PICTURE block, an
 * MP4 `covr` item (M4A). A front cover wins over other pictures. Pure byte parsing: no decoding, no DOM. */

export interface EmbeddedPicture { data: Uint8Array; mediaType: string }

const FRONT_COVER = 3

function pictureType(data: Uint8Array): string | null {
  if (data[0] === 0xff && data[1] === 0xd8) return 'image/jpeg'
  if (data[0] === 0x89 && data[1] === 0x50 && data[2] === 0x4e && data[3] === 0x47) return 'image/png'
  if (data[0] === 0x47 && data[1] === 0x49 && data[2] === 0x46) return 'image/gif'
  if (data[0] === 0x52 && data[1] === 0x49 && data[2] === 0x46 && data[3] === 0x46 && data[8] === 0x57 && data[9] === 0x45) return 'image/webp'
  return null
}

function picture(data: Uint8Array): EmbeddedPicture | null {
  const mediaType = data.length > 16 ? pictureType(data) : null
  return mediaType ? { data, mediaType } : null
}

const ascii = (b: Uint8Array, at: number, n: number) => String.fromCharCode(...b.subarray(at, at + n))
const syncsafe = (b: Uint8Array, at: number) => ((b[at] & 0x7f) << 21) | ((b[at + 1] & 0x7f) << 14) | ((b[at + 2] & 0x7f) << 7) | (b[at + 3] & 0x7f)
const u32 = (b: Uint8Array, at: number) => ((b[at] << 24) >>> 0) + (b[at + 1] << 16) + (b[at + 2] << 8) + b[at + 3]

/** The end of a text terminated in `encoding` (0, 3: one zero byte; 1, 2: two, aligned), from `at`. */
function skipText(b: Uint8Array, at: number, end: number, encoding: number): number {
  if (encoding === 1 || encoding === 2) {
    for (let i = at; i + 1 < end; i += 2) if (b[i] === 0 && b[i + 1] === 0) return i + 2
  } else {
    for (let i = at; i < end; i += 1) if (b[i] === 0) return i + 1
  }
  return end
}

function removeUnsync(b: Uint8Array): Uint8Array {
  const out: number[] = []
  for (let i = 0; i < b.length; i += 1) {
    out.push(b[i])
    if (b[i] === 0xff && b[i + 1] === 0) i += 1
  }
  return Uint8Array.from(out)
}

function id3Cover(file: Uint8Array): EmbeddedPicture | null {
  const major = file[3]
  const flags = file[5]
  let tag = file.subarray(10, Math.min(file.length, 10 + syncsafe(file, 6)))
  if (flags & 0x80 && major < 4) tag = removeUnsync(tag)
  let at = 0
  if (flags & 0x40 && major >= 3) at = major === 4 ? syncsafe(tag, 0) : u32(tag, 0) + 4
  let best: EmbeddedPicture | null = null
  while (at + (major === 2 ? 6 : 10) <= tag.length) {
    const id = ascii(tag, at, major === 2 ? 3 : 4)
    if (!/^[A-Z0-9]{3,4}$/.test(id)) break
    const size = major === 2 ? (tag[at + 3] << 16) | (tag[at + 4] << 8) | tag[at + 5] : major === 4 ? syncsafe(tag, at + 4) : u32(tag, at + 4)
    const start = at + (major === 2 ? 6 : 10)
    const end = Math.min(tag.length, start + size)
    if (id === 'APIC' || id === 'PIC') {
      // ID3v2.4 format flags: a data length indicator (0x01) comes first, unsynchronisation (0x02) covers the rest
      let frame = tag.subarray(major === 4 && tag[at + 9] & 0x01 ? start + 4 : start, end)
      if (major === 4 && tag[at + 9] & 0x02) frame = removeUnsync(frame)
      const encoding = frame[0]
      const typeAt = id === 'PIC' ? 4 : skipText(frame, 1, frame.length, 0)
      const found = picture(frame.subarray(skipText(frame, typeAt + 1, frame.length, encoding)))
      if (found && (frame[typeAt] === FRONT_COVER || !best)) best = found
      if (found && frame[typeAt] === FRONT_COVER) return best
    }
    at = end
  }
  return best
}

function flacCover(file: Uint8Array): EmbeddedPicture | null {
  let at = 4
  let best: EmbeddedPicture | null = null
  for (;;) {
    if (at + 4 > file.length) return best
    const last = file[at] & 0x80
    const type = file[at] & 0x7f
    const length = (file[at + 1] << 16) | (file[at + 2] << 8) | file[at + 3]
    const start = at + 4
    if (type === 6 && start + 32 <= file.length) {
      const kind = u32(file, start)
      let p = start + 8 + u32(file, start + 4)
      p += 4 + u32(file, p) + 16
      const found = picture(file.subarray(p + 4, Math.min(file.length, p + 4 + u32(file, p))))
      if (found && (kind === FRONT_COVER || !best)) best = found
      if (found && kind === FRONT_COVER) return best
    }
    if (last) return best
    at = start + length
  }
}

/** The boxes inside `[start, end)` of an ISO media file. */
function* boxes(b: Uint8Array, start: number, end: number): Generator<{ type: string; body: number; end: number }> {
  let at = start
  while (at + 8 <= end) {
    let size = u32(b, at)
    let header = 8
    if (size === 1 && at + 16 <= end) { size = u32(b, at + 8) * 2 ** 32 + u32(b, at + 12); header = 16 }
    else if (size === 0) size = end - at
    if (size < header) return
    yield { type: ascii(b, at + 4, 4), body: at + header, end: Math.min(end, at + size) }
    at += size
  }
}

function child(b: Uint8Array, start: number, end: number, type: string) {
  for (const box of boxes(b, start, end)) if (box.type === type) return box
  return null
}

function mp4Cover(file: Uint8Array): EmbeddedPicture | null {
  const moov = child(file, 0, file.length, 'moov')
  const udta = moov && child(file, moov.body, moov.end, 'udta')
  const meta = udta && child(file, udta.body, udta.end, 'meta')
  // `meta` is a full box: version and flags come before its children
  const ilst = meta && child(file, meta.body + 4, meta.end, 'ilst')
  const covr = ilst && child(file, ilst.body, ilst.end, 'covr')
  const data = covr && child(file, covr.body, covr.end, 'data')
  return data ? picture(file.subarray(data.body + 8, data.end)) : null
}

export function embeddedCover(file: Uint8Array): EmbeddedPicture | null {
  try {
    if (ascii(file, 0, 3) === 'ID3') return id3Cover(file)
    if (ascii(file, 0, 4) === 'fLaC') return flacCover(file)
    if (ascii(file, 4, 4) === 'ftyp') return mp4Cover(file)
  } catch { /* a damaged tag has no cover */ }
  return null
}
