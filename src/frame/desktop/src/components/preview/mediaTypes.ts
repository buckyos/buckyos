/**
 * Media identification and the Runtime Adapter (PRD §8.1, §23.3).
 *
 * Type judgement combines object type, resolver hints, the response
 * Content-Type, the extension and a bounded magic-byte probe — the extension
 * is never the only basis, and security-sensitive text formats (HTML / SVG)
 * additionally require the bytes to look like text.
 *
 * `detectRuntimeProfile()` answers "can this Runtime directly consume X?" —
 * the answer feeds both the Direct decision and the Pipeline Target Profile.
 */

import type { PreviewRendererType, PreviewRuntimeProfile } from './types'

// ─── Extension → media type hints ───

import { extensionOf, mediaTypeFromExtension } from 'buckyos/content'
export { extensionOf, mediaTypeFromExtension } from 'buckyos/content'

const TEXT_APPLICATION_TYPES = new Set([
  'application/json',
  'application/ld+json',
  'application/xml',
  'application/yaml',
  'application/x-yaml',
  'application/toml',
  'application/javascript',
  'application/x-sh',
  'application/x-shellscript',
  'application/xhtml+xml',
])

/** Raster formats every supported Web Runtime decodes natively. */
const BASELINE_IMAGE_TYPES = [
  'image/png',
  'image/apng',
  'image/jpeg',
  'image/gif',
  'image/webp',
  'image/bmp',
  'image/x-icon',
  'image/vnd.microsoft.icon',
]

const GENERIC_CONTENT_TYPES = new Set(['', 'application/octet-stream', 'binary/octet-stream'])



export function stripMediaTypeParams(value: string | undefined | null): string {
  return (value ?? '').split(';')[0].trim().toLowerCase()
}

// ─── Bounded magic-byte probe ───

function ascii(bytes: Uint8Array, start: number, length: number): string {
  let out = ''
  for (let i = start; i < Math.min(bytes.length, start + length); i += 1) {
    out += String.fromCharCode(bytes[i])
  }
  return out
}

function textHead(bytes: Uint8Array, length = 512): string {
  // Skip a UTF-8 BOM and leading whitespace before looking at markup.
  let start = 0
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) start = 3
  return ascii(bytes, start, length).replace(/^\s+/, '')
}

/** HEVC-coded HEIF brands (image and sequence); `mif1`/`msf1` alone is generic HEIF. */
const HEIC_BRANDS = new Set(['heic', 'heix', 'heim', 'heis', 'hevc', 'hevx', 'hevm', 'hevs'])

function compatibleBrands(b: Uint8Array): string[] {
  const size = (b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]
  const end = Math.min(b.length, size > 16 ? size : 16)
  const brands: string[] = []
  for (let at = 16; at + 4 <= end; at += 4) brands.push(ascii(b, at, 4))
  return brands
}

/** Returns the confirmed media type of a byte prefix, or null when unknown. */
export function sniffMagic(bytes: Uint8Array | null | undefined): string | null {
  if (!bytes || bytes.length < 4) return null
  const b = bytes
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return 'image/png'
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return 'image/jpeg'
  if (ascii(b, 0, 4) === 'GIF8') return 'image/gif'
  if (ascii(b, 0, 4) === 'RIFF' && b.length >= 12) {
    const form = ascii(b, 8, 4)
    if (form === 'WEBP') return 'image/webp'
    if (form === 'WAVE') return 'audio/wav'
    if (form === 'AVI ') return 'video/x-msvideo'
  }
  if (b[0] === 0x42 && b[1] === 0x4d) return 'image/bmp'
  if (b[0] === 0x00 && b[1] === 0x00 && b[2] === 0x01 && b[3] === 0x00) return 'image/x-icon'
  if (ascii(b, 0, 4) === 'II*\0' || ascii(b, 0, 4) === 'MM\0*') return 'image/tiff'
  if (ascii(b, 0, 4) === '8BPS') return 'image/vnd.adobe.photoshop'
  if (ascii(b, 0, 5) === '%PDF-') return 'application/pdf'
  if (b.length >= 12 && ascii(b, 4, 4) === 'ftyp') {
    const brand = ascii(b, 8, 4)
    if (brand === 'avif' || brand === 'avis') return 'image/avif'
    if (HEIC_BRANDS.has(brand)) return 'image/heic'
    if (brand === 'mif1' || brand === 'msf1') {
      const compatible = compatibleBrands(b)
      if (compatible.some((c) => c === 'avif' || c === 'avis')) return 'image/avif'
      return compatible.some((c) => HEIC_BRANDS.has(c)) ? 'image/heic' : 'image/heif'
    }
    if (brand === 'qt  ') return 'video/quicktime'
    if (brand === 'M4A ') return 'audio/mp4'
    return 'video/mp4'
  }
  if (b[0] === 0x1a && b[1] === 0x45 && b[2] === 0xdf && b[3] === 0xa3) {
    return ascii(b, 0, 64).includes('webm') ? 'video/webm' : 'video/x-matroska'
  }
  if (ascii(b, 0, 4) === 'OggS') {
    const head = ascii(b, 0, 96)
    return head.includes('theora') ? 'video/ogg' : 'audio/ogg'
  }
  if (ascii(b, 0, 3) === 'ID3') return 'audio/mpeg'
  if (b[0] === 0xff && (b[1] & 0xe6) === 0xe2 && (b[1] & 0x18) !== 0x08) return 'audio/mpeg'
  if (ascii(b, 0, 4) === 'fLaC') return 'audio/flac'
  if (b[0] === 0x50 && b[1] === 0x4b && (b[2] === 0x03 || b[2] === 0x05 || b[2] === 0x07)) {
    return 'application/zip'
  }
  if (b[0] === 0x1f && b[1] === 0x8b) return 'application/gzip'
  if (ascii(b, 0, 6) === '7z\xbc\xaf\x27\x1c') return 'application/x-7z-compressed'
  if (ascii(b, 0, 4) === 'Rar!') return 'application/vnd.rar'
  const head = textHead(b).toLowerCase()
  if (head.startsWith('<?xml') || head.startsWith('<svg') || head.startsWith('<!doctype svg')) {
    if (head.includes('<svg')) return 'image/svg+xml'
    if (head.startsWith('<?xml')) return 'application/xml'
  }
  if (head.startsWith('<!doctype html') || head.startsWith('<html') || head.startsWith('<head') || head.startsWith('<body')) {
    return 'text/html'
  }
  return null
}

/** Conservative text heuristic over a bounded sample (no NULs, valid UTF-8, few control chars). */
export function looksLikeText(bytes: Uint8Array | null | undefined): boolean {
  if (!bytes || bytes.length === 0) return false
  const sample = bytes.subarray(0, Math.min(bytes.length, 4096))
  let control = 0
  for (let i = 0; i < sample.length; i += 1) {
    const c = sample[i]
    if (c === 0) return false
    if (c < 0x20 && c !== 0x09 && c !== 0x0a && c !== 0x0d && c !== 0x0c) control += 1
  }
  if (control / sample.length > 0.05) return false
  // A cut-off multibyte sequence at the end must not fail the whole sample.
  const trimmed = sample.length > 8 ? sample.subarray(0, sample.length - 4) : sample
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(trimmed)
    return true
  } catch {
    return false
  }
}

// ─── Classification ───

export type MediaBasis = 'magic' | 'content-type' | 'extension' | 'text-heuristic' | 'unknown'

export interface MediaClassification {
  mediaType: string
  rendererType: PreviewRendererType | null
  basis: MediaBasis
  extension: string
  /** True when bytes or a trusted header confirmed the type (not extension only). */
  confirmed: boolean
}

export function rendererForMediaType(mediaType: string): PreviewRendererType | null {
  const type = stripMediaTypeParams(mediaType)
  if (!type) return null
  if (type === 'image/svg+xml') return 'svg'
  if (type.startsWith('image/')) return 'image'
  if (type === 'text/html' || type === 'application/xhtml+xml') return 'html'
  if (type.startsWith('text/') || TEXT_APPLICATION_TYPES.has(type)) return 'text'
  if (type === 'application/pdf') return 'pdf'
  if (type.startsWith('audio/') || type === 'application/ogg') return 'audio'
  if (type.startsWith('video/')) return 'video'
  return null
}

const SENSITIVE_TEXT_RENDERERS = new Set<PreviewRendererType>(['html', 'svg'])

export function classifyMedia(input: {
  name?: string
  hints?: string[]
  contentType?: string | null
  magic?: Uint8Array | null
  objectType?: string
}): MediaClassification {
  const extension = extensionOf(input.name)
  const magicType = sniffMagic(input.magic)
  const headerType = stripMediaTypeParams(input.contentType)
  const extType = extension ? mediaTypeFromExtension(extension) : undefined
  const hintType = input.hints?.map(stripMediaTypeParams).find((h) => h && !GENERIC_CONTENT_TYPES.has(h))
  const textLike = looksLikeText(input.magic)

  const finish = (mediaType: string, basis: MediaBasis, confirmed: boolean): MediaClassification => {
    let renderer = rendererForMediaType(mediaType)
    // HTML / SVG must at least be text — an extension alone never decides (§23.3 rule 5).
    if (renderer && SENSITIVE_TEXT_RENDERERS.has(renderer) && input.magic && !textLike) {
      renderer = null
    }
    return { mediaType, rendererType: renderer, basis, extension, confirmed }
  }

  if (magicType) {
    // ZIP containers are frequently office documents: keep the richer hint.
    if (magicType === 'application/zip' && extType && extType !== 'application/zip') {
      return finish(extType, 'extension', false)
    }
    return finish(magicType, 'magic', true)
  }
  if (headerType && !GENERIC_CONTENT_TYPES.has(headerType)) {
    // A generic text/plain header loses to a more specific extension hint.
    if (headerType === 'text/plain' && extType && rendererForMediaType(extType) === 'text') {
      return finish(extType, 'extension', true)
    }
    return finish(headerType, 'content-type', true)
  }
  if (hintType) {
    return finish(hintType, 'extension', false)
  }
  if (extType) {
    return finish(extType, 'extension', false)
  }
  if (textLike) {
    return finish('text/plain', 'text-heuristic', true)
  }
  return { mediaType: 'application/octet-stream', rendererType: null, basis: 'unknown', extension, confirmed: false }
}

/** "PNG · image/png" style label for the Unsupported / Info states. */
export function contentLabelOf(classification: MediaClassification): string {
  const ext = classification.extension ? classification.extension.toUpperCase() : ''
  const type = classification.mediaType
  if (ext && type && type !== 'application/octet-stream') return `${ext} · ${type}`
  return ext || type || 'unknown'
}

// ─── Runtime Adapter ───

let runtimeProfile: PreviewRuntimeProfile | null = null
let decodeProbes: Promise<void> | null = null

// 2×2 AVIF used for feature detection (same probe as common capability libs).
const AVIF_PROBE =
  'data:image/avif;base64,AAAAIGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZk1BMUIAAADybWV0YQAAAAAAAAAoaGRscgAAAAAAAAAAcGljdAAAAAAAAAAAAAAAAGxpYmF2aWYAAAAADnBpdG0AAAAAAAEAAAAeaWxvYwAAAABEAAABAAEAAAABAAABGgAAAB0AAAAoaWluZgAAAAAAAQAAABppbmZlAgAAAAABAABhdjAxQ29sb3IAAAAAamlwcnAAAABLaXBjbwAAABRpc3BlAAAAAAAAAAIAAAACAAAAEHBpeGkAAAAAAwgICAAAAAxhdjFDgQ0MAAAAABNjb2xybmNseAACAAIAAYAAAAAXaXBtYQAAAAAAAAABAAEEAQKDBAAAACVtZGF0EgAKCBgANogQEAwgMg8f8D///8WfhwB8+ErK42A='

// 2×2 HEVC-coded HEIC (libheif), the same kind of probe as AVIF.
const HEIC_PROBE =
  'data:image/heic;base64,AAAAHGZ0eXBoZWljAAAAAG1pZjFoZWljbWlhZgAAAXxtZXRhAAAAAAAAACFoZGxyAAAAAAAAAABwaWN0AAAAAAAAAAAAAAAAAAAAACJpbG9jAAAAAERAAAEAAQAAAAABoAABAAAAAAAAACsAAAAjaWluZgAAAAAAAQAAABVpbmZlAgAAAAABAABodmMxAAAAAA5waXRtAAAAAAABAAAA/GlwcnAAAADcaXBjbwAAAHVodmNDAQNwAAAAkAAAAAAAHvAA/P34+AAADwNgAAEAGEABDAH//wNwAAADAJAAAAMAAAMAHroCQGEAAQApQgEBA3AAAAMAkAAAAwAAAwAeoCCBBZbqrprm4CGgwIAAAAyAAAADAIRiAAEABkQBwXPBiQAAABNjb2xybmNseAABAA0ABoAAAAAUaXNwZQAAAAAAAABAAAAAQAAAAChjbGFwAAAAAgAAAAEAAAACAAAAAf///8IAAAAC////wgAAAAIAAAAQcGl4aQAAAAADCAgIAAAAGGlwbWEAAAAAAAAAAQABBYECAwWEAAAAM21kYXQAAAAnKAGvEyExlvhOUKeW/WMCzQyVTFq5T6Vz3QpQk3J+uP7yh5PFYoLg'

const DECODE_PROBE_TIMEOUT_MS = 3000

function probeImageDecode(src: string): Promise<boolean> {
  if (typeof Image === 'undefined') return Promise.resolve(false)
  return new Promise((resolve) => {
    const img = new Image()
    const finish = (ok: boolean) => {
      window.clearTimeout(timer)
      img.onload = null
      img.onerror = null
      img.removeAttribute('src')
      resolve(ok)
    }
    const timer = window.setTimeout(() => finish(false), DECODE_PROBE_TIMEOUT_MS)
    img.onload = () => finish(img.naturalWidth > 0)
    img.onerror = () => finish(false)
    img.src = src
  })
}

function canPlay(element: HTMLMediaElement | null, type: string): boolean {
  if (!element) return false
  try {
    return element.canPlayType(type) !== ''
  } catch {
    return false
  }
}

function pdfInlineSupported(): boolean {
  if (typeof navigator === 'undefined') return false
  const nav = navigator as Navigator & { pdfViewerEnabled?: boolean }
  if (typeof nav.pdfViewerEnabled === 'boolean') return nav.pdfViewerEnabled
  try {
    return Boolean(navigator.mimeTypes?.namedItem?.('application/pdf'))
  } catch {
    return false
  }
}

/** Synchronous profile (AVIF / HEIC decoding is filled in by `ensureRuntimeProfile`). */
export function detectRuntimeProfile(): PreviewRuntimeProfile {
  if (runtimeProfile) return runtimeProfile
  const video = typeof document !== 'undefined' ? document.createElement('video') : null
  const audio = typeof document !== 'undefined' ? document.createElement('audio') : null
  const videoCandidates = ['video/mp4', 'video/webm', 'video/ogg', 'video/quicktime', 'video/x-matroska']
  const audioCandidates = ['audio/mpeg', 'audio/mp4', 'audio/aac', 'audio/wav', 'audio/ogg', 'audio/flac', 'audio/webm']
  runtimeProfile = {
    acceptTypes: ['image', 'svg', 'text', 'html', 'audio', 'video', 'pdf'],
    imageMediaTypes: [...BASELINE_IMAGE_TYPES, 'image/svg+xml'],
    videoMediaTypes: videoCandidates.filter((t) => canPlay(video, t)),
    audioMediaTypes: audioCandidates.filter((t) => canPlay(audio, t)),
    videoCodecs: {
      h264: canPlay(video, 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'),
      hevc: canPlay(video, 'video/mp4; codecs="hvc1.1.6.L93.B0"') || canPlay(video, 'video/mp4; codecs="hev1.1.6.L93.B0"'),
    },
    heic: false,
    pdfInline: pdfInlineSupported(),
  }
  return runtimeProfile
}

/** Resolves once the async decode probes (AVIF, HEIC) have settled. */
export function ensureRuntimeProfile(): Promise<PreviewRuntimeProfile> {
  const profile = detectRuntimeProfile()
  if (!decodeProbes) {
    decodeProbes = Promise.all([probeImageDecode(AVIF_PROBE), probeImageDecode(HEIC_PROBE)]).then(([avif, heic]) => {
      if (avif && !profile.imageMediaTypes.includes('image/avif')) profile.imageMediaTypes.push('image/avif')
      profile.heic = heic
      if (heic) {
        for (const type of ['image/heic', 'image/heif']) {
          if (!profile.imageMediaTypes.includes(type)) profile.imageMediaTypes.push(type)
        }
      }
    })
  }
  return decodeProbes.then(() => profile)
}

/**
 * Video containers every Runtime is asked to play directly: `canPlayType`
 * only hints (Chromium answers "" for `video/quicktime` yet plays H.264 MOV),
 * a `MediaError` from `<video>` is what sends the item to the Pipeline.
 */
const DIRECT_VIDEO_CONTAINERS = new Set(['video/mp4', 'video/quicktime', 'video/webm', 'video/ogg', 'video/x-matroska', 'video/x-m4v'])

export type DirectDecision =
  | { ok: true }
  | { ok: false; reason: 'no-renderer' | 'runtime-unsupported' | 'too-large' }

/** Direct-render budgets (bytes). Text is range-read, media streams. */
export const DIRECT_SIZE_BUDGET: Record<PreviewRendererType, number> = {
  image: 256 * 1024 * 1024,
  svg: 24 * 1024 * 1024,
  text: Number.POSITIVE_INFINITY,
  html: 8 * 1024 * 1024,
  audio: Number.POSITIVE_INFINITY,
  video: Number.POSITIVE_INFINITY,
  pdf: Number.POSITIVE_INFINITY,
}

/** How much of a text file the text renderer reads before showing a truncation notice. */
export const TEXT_READ_BUDGET = 2 * 1024 * 1024

export function decideDirect(
  classification: MediaClassification,
  size: number | undefined,
  runtime: PreviewRuntimeProfile,
): DirectDecision {
  const renderer = classification.rendererType
  if (!renderer) return { ok: false, reason: 'no-renderer' }
  const type = classification.mediaType
  if (renderer === 'image' && !runtime.imageMediaTypes.includes(type)) {
    return { ok: false, reason: 'runtime-unsupported' }
  }
  if (renderer === 'video' && !DIRECT_VIDEO_CONTAINERS.has(type) && !runtime.videoMediaTypes.includes(type)) {
    return { ok: false, reason: 'runtime-unsupported' }
  }
  if (renderer === 'audio') {
    const normalized = type === 'application/ogg' ? 'audio/ogg' : type
    if (!runtime.audioMediaTypes.includes(normalized)) return { ok: false, reason: 'runtime-unsupported' }
  }
  if (renderer === 'pdf' && !runtime.pdfInline) {
    return { ok: false, reason: 'runtime-unsupported' }
  }
  if (typeof size === 'number' && size > DIRECT_SIZE_BUDGET[renderer]) {
    return { ok: false, reason: 'too-large' }
  }
  return { ok: true }
}

// ─── HEIF primary image size (`ispe` + `irot`, no decoding) ───

interface IsoBox {
  type: string
  start: number
  end: number
}

function isoBoxes(view: DataView, start: number, end: number): IsoBox[] {
  const boxes: IsoBox[] = []
  let at = start
  while (at + 8 <= end) {
    let size = view.getUint32(at)
    const type = String.fromCharCode(view.getUint8(at + 4), view.getUint8(at + 5), view.getUint8(at + 6), view.getUint8(at + 7))
    let header = 8
    if (size === 1) {
      if (at + 16 > end) break
      size = Number(view.getBigUint64(at + 8))
      header = 16
    } else if (size === 0) {
      size = end - at
    }
    if (size < header) break
    boxes.push({ type, start: at + header, end: Math.min(end, at + size) })
    at += size
  }
  return boxes
}

/**
 * Displayed size of a HEIF / HEIC primary image from the `meta` box of a
 * bounded prefix: the primary item's `ispe`, swapped for a 90°/270° `irot`.
 * Null when the prefix does not hold the box (or the file is no HEIF).
 */
export function heifImageSize(bytes: Uint8Array): { width: number; height: number } | null {
  try {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    const meta = isoBoxes(view, 0, bytes.length).find((b) => b.type === 'meta')
    if (!meta) return null
    const children = isoBoxes(view, meta.start + 4, meta.end)
    const pitm = children.find((b) => b.type === 'pitm')
    const iprp = children.find((b) => b.type === 'iprp')
    if (!pitm || !iprp) return null
    const primary = view.getUint8(pitm.start) === 0 ? view.getUint16(pitm.start + 4) : view.getUint32(pitm.start + 4)
    const props = isoBoxes(view, iprp.start, iprp.end)
    const ipco = props.find((b) => b.type === 'ipco')
    if (!ipco) return null
    const properties = isoBoxes(view, ipco.start, ipco.end)
    let width = 0
    let height = 0
    let quarterTurns = 0
    for (const ipma of props.filter((b) => b.type === 'ipma')) {
      const version = view.getUint8(ipma.start)
      const wideIndex = (view.getUint8(ipma.start + 3) & 1) === 1
      let at = ipma.start + 4
      const count = view.getUint32(at)
      at += 4
      for (let i = 0; i < count && at < ipma.end; i += 1) {
        const item = version < 1 ? view.getUint16(at) : view.getUint32(at)
        at += version < 1 ? 2 : 4
        const associations = view.getUint8(at)
        at += 1
        for (let j = 0; j < associations; j += 1) {
          const index = wideIndex ? view.getUint16(at) & 0x7fff : view.getUint8(at) & 0x7f
          at += wideIndex ? 2 : 1
          if (item !== primary || index === 0) continue
          const property = properties[index - 1]
          if (property?.type === 'ispe') {
            width = view.getUint32(property.start + 4)
            height = view.getUint32(property.start + 8)
          } else if (property?.type === 'irot') {
            quarterTurns = view.getUint8(property.start) & 3
          }
        }
      }
    }
    if (!width || !height) return null
    return quarterTurns % 2 === 1 ? { width: height, height: width } : { width, height }
  } catch {
    return null
  }
}
