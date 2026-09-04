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

const TEXT_CODE_EXTENSIONS = [
  'txt', 'log', 'ini', 'conf', 'cfg', 'env', 'csv', 'tsv',
  'md', 'markdown', 'rst', 'adoc',
  'json', 'jsonc', 'json5', 'xml', 'yaml', 'yml', 'toml',
  'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'rs', 'py', 'go', 'c', 'h', 'cc', 'cpp', 'hpp',
  'java', 'kt', 'swift', 'rb', 'php', 'sql', 'sh', 'bash', 'zsh', 'ps1', 'bat',
  'css', 'scss', 'less', 'vue', 'svelte', 'lua', 'dart', 'r', 'scala', 'ex', 'exs',
  'dockerfile', 'makefile', 'gitignore', 'editorconfig', 'lock',
]

const MEDIA_TYPE_BY_EXTENSION: Record<string, string> = {
  png: 'image/png',
  apng: 'image/apng',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  jfif: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
  ico: 'image/x-icon',
  avif: 'image/avif',
  heic: 'image/heic',
  heif: 'image/heif',
  tif: 'image/tiff',
  tiff: 'image/tiff',
  psd: 'image/vnd.adobe.photoshop',
  svg: 'image/svg+xml',
  mp4: 'video/mp4',
  m4v: 'video/mp4',
  webm: 'video/webm',
  mov: 'video/quicktime',
  mkv: 'video/x-matroska',
  avi: 'video/x-msvideo',
  ogv: 'video/ogg',
  mp3: 'audio/mpeg',
  wav: 'audio/wav',
  flac: 'audio/flac',
  ogg: 'audio/ogg',
  oga: 'audio/ogg',
  opus: 'audio/ogg',
  m4a: 'audio/mp4',
  aac: 'audio/aac',
  pdf: 'application/pdf',
  html: 'text/html',
  htm: 'text/html',
  xhtml: 'application/xhtml+xml',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  doc: 'application/msword',
  xls: 'application/vnd.ms-excel',
  ppt: 'application/vnd.ms-powerpoint',
  odt: 'application/vnd.oasis.opendocument.text',
  zip: 'application/zip',
  tar: 'application/x-tar',
  gz: 'application/gzip',
  '7z': 'application/x-7z-compressed',
  rar: 'application/vnd.rar',
  epub: 'application/epub+zip',
  md: 'text/markdown',
  markdown: 'text/markdown',
  json: 'application/json',
  xml: 'application/xml',
  csv: 'text/csv',
  yaml: 'application/yaml',
  yml: 'application/yaml',
  toml: 'application/toml',
  js: 'text/javascript',
  mjs: 'text/javascript',
  cjs: 'text/javascript',
  css: 'text/css',
}

for (const ext of TEXT_CODE_EXTENSIONS) {
  if (!MEDIA_TYPE_BY_EXTENSION[ext]) MEDIA_TYPE_BY_EXTENSION[ext] = `text/x-${ext}`
}

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

export function extensionOf(name: string | undefined): string {
  if (!name) return ''
  const base = name.split('/').pop() ?? name
  const dot = base.lastIndexOf('.')
  if (dot <= 0) return ''
  return base.slice(dot + 1).toLowerCase()
}

export function mediaTypeFromExtension(ext: string): string | undefined {
  return MEDIA_TYPE_BY_EXTENSION[ext.toLowerCase()]
}

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
    if (brand === 'heic' || brand === 'heix' || brand === 'hevc' || brand === 'mif1' || brand === 'msf1') {
      return 'image/heic'
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
let avifProbe: Promise<boolean> | null = null

// 2×2 AVIF used for feature detection (same probe as common capability libs).
const AVIF_PROBE =
  'data:image/avif;base64,AAAAIGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZk1BMUIAAADybWV0YQAAAAAAAAAoaGRscgAAAAAAAAAAcGljdAAAAAAAAAAAAAAAAGxpYmF2aWYAAAAADnBpdG0AAAAAAAEAAAAeaWxvYwAAAABEAAABAAEAAAABAAABGgAAAB0AAAAoaWluZgAAAAAAAQAAABppbmZlAgAAAAABAABhdjAxQ29sb3IAAAAAamlwcnAAAABLaXBjbwAAABRpc3BlAAAAAAAAAAIAAAACAAAAEHBpeGkAAAAAAwgICAAAAAxhdjFDgQ0MAAAAABNjb2xybmNseAACAAIAAYAAAAAXaXBtYQAAAAAAAAABAAEEAQKDBAAAACVtZGF0EgAKCBgANogQEAwgMg8f8D///8WfhwB8+ErK42A='

function probeImageDecode(src: string): Promise<boolean> {
  if (typeof Image === 'undefined') return Promise.resolve(false)
  return new Promise((resolve) => {
    const img = new Image()
    img.onload = () => resolve(img.width > 0)
    img.onerror = () => resolve(false)
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

/** Synchronous profile (AVIF is filled in by `ensureRuntimeProfile`). */
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
    pdfInline: pdfInlineSupported(),
  }
  return runtimeProfile
}

/** Resolves once the async probes (AVIF decode) have settled. */
export function ensureRuntimeProfile(): Promise<PreviewRuntimeProfile> {
  const profile = detectRuntimeProfile()
  if (!avifProbe) {
    avifProbe = probeImageDecode(AVIF_PROBE).then((ok) => {
      if (ok && !profile.imageMediaTypes.includes('image/avif')) profile.imageMediaTypes.push('image/avif')
      return ok
    })
  }
  return avifProbe.then(() => profile)
}

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
  if (renderer === 'video' && !runtime.videoMediaTypes.includes(type)) {
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
