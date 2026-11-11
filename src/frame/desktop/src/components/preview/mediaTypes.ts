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
 *
 * Pipeline documents are never Direct: the renderer shows what a Pipeline
 * converts them to — a PDF (`preview-document`), an xlsx workbook
 * (`preview-sheet`) or a self-contained HTML page (`preview-html`). xlsx /
 * xlsm / csv / tsv are read by the sheet renderer itself.
 */

import type { PreviewRendererType, PreviewRuntimeProfile } from './types'

// ─── Extension → media type hints ───

import { extensionOf, mediaTypeFromExtension, sniffMagic, stripMediaTypeParams } from 'buckyos/content'
export { extensionOf, heifImageSize, mediaTypeFromExtension, sniffMagic, stripMediaTypeParams } from 'buckyos/content'

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

const XLSX_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'

/** Spreadsheets the sheet renderer reads itself. */
const SHEET_TYPES: Record<string, string> = {
  xlsx: XLSX_TYPE,
  xlsm: 'application/vnd.ms-excel.sheet.macroenabled.12',
  csv: 'text/csv',
  tsv: 'text/tab-separated-values',
}
const SHEET_MEDIA_TYPES = new Set(Object.values(SHEET_TYPES))

export type PipelineDocumentProfile = 'preview-document' | 'preview-sheet' | 'preview-html'

const OOXML = 'application/vnd.openxmlformats-officedocument'
const ODF = 'application/vnd.oasis.opendocument'

/** Documents shown as a Pipeline result, by extension: media type and the nfs_server profile that converts them. */
const PIPELINE_DOCUMENTS: Record<string, [string, PipelineDocumentProfile]> = {
  docx: [`${OOXML}.wordprocessingml.document`, 'preview-document'],
  docm: ['application/vnd.ms-word.document.macroenabled.12', 'preview-document'],
  dotx: [`${OOXML}.wordprocessingml.template`, 'preview-document'],
  dotm: ['application/vnd.ms-word.template.macroenabled.12', 'preview-document'],
  doc: ['application/msword', 'preview-document'],
  wps: ['application/kswps', 'preview-document'],
  wpt: ['application/kswpt', 'preview-document'],
  odt: [`${ODF}.text`, 'preview-document'],
  rtf: ['application/rtf', 'preview-document'],
  pptx: [`${OOXML}.presentationml.presentation`, 'preview-document'],
  pptm: ['application/vnd.ms-powerpoint.presentation.macroenabled.12', 'preview-document'],
  ppsx: [`${OOXML}.presentationml.slideshow`, 'preview-document'],
  potx: [`${OOXML}.presentationml.template`, 'preview-document'],
  ppt: ['application/vnd.ms-powerpoint', 'preview-document'],
  dps: ['application/ksdps', 'preview-document'],
  dpt: ['application/ksdpt', 'preview-document'],
  odp: [`${ODF}.presentation`, 'preview-document'],
  odg: [`${ODF}.graphics`, 'preview-document'],
  vsdx: ['application/vnd.ms-visio.drawing', 'preview-document'],
  pub: ['application/vnd.ms-publisher', 'preview-document'],
  hwp: ['application/x-hwp', 'preview-document'],
  wpd: ['application/vnd.wordperfect', 'preview-document'],
  pages: ['application/vnd.apple.pages', 'preview-document'],
  key: ['application/vnd.apple.keynote', 'preview-document'],
  xltx: [`${OOXML}.spreadsheetml.template`, 'preview-document'],
  xls: ['application/vnd.ms-excel', 'preview-sheet'],
  et: ['application/kset', 'preview-sheet'],
  ett: ['application/ksett', 'preview-sheet'],
  ods: [`${ODF}.spreadsheet`, 'preview-sheet'],
  numbers: ['application/vnd.apple.numbers', 'preview-sheet'],
  epub: ['application/epub+zip', 'preview-html'],
  ipynb: ['application/x-ipynb+json', 'preview-html'],
  md: ['text/markdown', 'preview-html'],
  markdown: ['text/markdown', 'preview-html'],
}

const PIPELINE_PROFILES = new Map<string, PipelineDocumentProfile>([...Object.values(PIPELINE_DOCUMENTS), ['text/rtf', 'preview-document']])

/** Text markup a Pipeline turns into a page; shown as plain text where no Pipeline does. */
const TEXT_FALLBACK_TYPES = new Set(['ipynb', 'md'].map((ext) => PIPELINE_DOCUMENTS[ext][0]))

/** The nfs_server profile that converts a Pipeline document; null for everything else. */
export function pipelineDocumentProfile(mediaType: string | undefined | null): PipelineDocumentProfile | null {
  return PIPELINE_PROFILES.get(stripMediaTypeParams(mediaType)) ?? null
}

/** A document rendered only as what a Pipeline converts it to. */
export function isPipelineDocument(mediaType: string | undefined | null): boolean {
  return pipelineDocumentProfile(mediaType) !== null
}

/** Extensions that also name plain-text files (a PEM `.key`, an SSH `.pub`): a document only when the bytes are not text. */
const BINARY_ONLY_DOCUMENTS = new Set(['key', 'pub', 'pages', 'numbers'])

function typeFromExtension(extension: string, textLike: boolean): string | undefined {
  if (!extension) return undefined
  if (textLike && BINARY_ONLY_DOCUMENTS.has(extension)) return mediaTypeFromExtension(extension)
  return SHEET_TYPES[extension] ?? PIPELINE_DOCUMENTS[extension]?.[0] ?? mediaTypeFromExtension(extension)
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
  /** The renderer takes the original bytes; false = only a Pipeline result (an Office document's PDF). */
  direct: boolean
  /** A Pipeline document that is text: shown by the text renderer where no Pipeline converts it. */
  textFallback: boolean
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
  if (SHEET_MEDIA_TYPES.has(type)) return 'sheet'
  const profile = pipelineDocumentProfile(type)
  if (profile) return profile === 'preview-sheet' ? 'sheet' : profile === 'preview-html' ? 'html' : 'pdf'
  if (type === 'application/pdf') return 'pdf'
  if (type.startsWith('text/') || TEXT_APPLICATION_TYPES.has(type)) return 'text'
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
  const textLike = looksLikeText(input.magic)
  const extType = typeFromExtension(extension, textLike)
  const hintType = input.hints?.map(stripMediaTypeParams).find((h) => h && !GENERIC_CONTENT_TYPES.has(h))

  const finish = (mediaType: string, basis: MediaBasis, confirmed: boolean): MediaClassification => {
    let renderer = rendererForMediaType(mediaType)
    const pipelineOnly = isPipelineDocument(mediaType)
    // HTML / SVG must at least be text — an extension alone never decides (§23.3 rule 5).
    if (renderer && !pipelineOnly && SENSITIVE_TEXT_RENDERERS.has(renderer) && input.magic && !textLike) {
      renderer = null
    }
    return { mediaType, rendererType: renderer, direct: renderer !== null && !pipelineOnly, textFallback: textLike && TEXT_FALLBACK_TYPES.has(mediaType), basis, extension, confirmed }
  }

  if (magicType) {
    // ZIP containers are frequently office documents: keep the richer hint.
    if (magicType === 'application/zip' && extType && extType !== 'application/zip') {
      return finish(extType, 'extension', false)
    }
    return finish(magicType, 'magic', true)
  }
  if (headerType && !GENERIC_CONTENT_TYPES.has(headerType)) {
    // A generic text header loses to a more specific extension hint.
    const extRenderer = extType ? rendererForMediaType(extType) : null
    if (extType && headerType !== extType && rendererForMediaType(headerType) === 'text' && (extRenderer === 'text' || extRenderer === 'sheet' || isPipelineDocument(extType))) {
      return finish(extType, 'extension', !isPipelineDocument(extType))
    }
    return finish(headerType, 'content-type', true)
  }
  // A sheet or Pipeline document named by its extension: the resolver's hint table does not know them all.
  if (extType && (SHEET_MEDIA_TYPES.has(extType) || isPipelineDocument(extType))) {
    return finish(extType, 'extension', false)
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
  return { mediaType: 'application/octet-stream', rendererType: null, direct: false, textFallback: false, basis: 'unknown', extension, confirmed: false }
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

/** PDFs are drawn by pdf.js where the Runtime has no built-in viewer and on touch devices; else by the viewer in an iframe. */
export function pdfUsesPdfjs(runtime: PreviewRuntimeProfile = detectRuntimeProfile()): boolean {
  if (!runtime.pdfInline) return true
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(pointer: coarse)').matches
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
    acceptTypes: ['image', 'svg', 'text', 'html', 'audio', 'video', 'pdf', 'sheet'],
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
  sheet: 30 * 1024 * 1024,
}

/** Rows of one sheet the sheet renderer shows before a truncation notice. */
export const SHEET_ROW_LIMIT = 5000

/** How much of a text file the text renderer reads before showing a truncation notice. */
export const TEXT_READ_BUDGET = 2 * 1024 * 1024

export function decideDirect(
  classification: MediaClassification,
  size: number | undefined,
  runtime: PreviewRuntimeProfile,
): DirectDecision {
  const renderer = classification.rendererType
  if (!renderer || !classification.direct) return { ok: false, reason: 'no-renderer' }
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
  if (typeof size === 'number' && size > DIRECT_SIZE_BUDGET[renderer]) {
    return { ok: false, reason: 'too-large' }
  }
  return { ok: true }
}
