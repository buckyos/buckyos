/* The facts of a file about to become an asset (多媒体对象 §3): its kind, the size of its picture, its length or page
 * count, and a poster — a picture of the content, uploaded with it, so every viewer draws the Block without
 * downloading or decoding the content. Everything is best effort: a file whose poster cannot be made is still
 * inserted, shown by its icon. */

import { imageSize } from '@drawio/bytes'
import { embeddedCover } from './audioCover'
import { mediaKindOf, type MediaKind } from './kinds'

export interface MediaFacts {
  kind: MediaKind
  /** The natural size of the picture, the video frame or the poster. */
  image?: { width: number; height: number }
  poster?: Blob
  durationMs?: number
  pageCount?: number
}

const POSTER_EDGE = 1280
const POSTER_QUALITY = 0.85
const FACTS_TIMEOUT_MS = 15_000

export async function describeMediaFile(file: File): Promise<MediaFacts> {
  const kind = mediaKindOf(file.type, file.name)
  const control = new AbortController()
  const timer = window.setTimeout(() => control.abort(), FACTS_TIMEOUT_MS)
  try {
    switch (kind) {
      case 'image': return { kind, ...await pictureFacts(file) }
      case 'video': return { kind, ...await videoFacts(file, control.signal) }
      case 'audio': return { kind, ...await audioFacts(file, control.signal) }
      case 'pdf': return { kind, ...await pdfFacts(file, control.signal) }
      default: return { kind }
    }
  } catch {
    return { kind }
  } finally {
    window.clearTimeout(timer)
  }
}

async function pictureFacts(file: Blob): Promise<Pick<MediaFacts, 'image'>> {
  // decoded, so a photo's EXIF orientation is applied; an SVG has no bitmap size of its own
  if (file.type !== 'image/svg+xml') {
    try {
      const bitmap = await createImageBitmap(file)
      const image = { width: bitmap.width, height: bitmap.height }
      bitmap.close()
      return { image }
    } catch { /* fall through to the header */ }
  }
  const size = imageSize(new Uint8Array(await file.arrayBuffer()), file.type)
  return size ? { image: { width: Math.round(size.width), height: Math.round(size.height) } } : {}
}

function encode(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('the poster could not be encoded'))), 'image/jpeg', POSTER_QUALITY))
}

/** `source` drawn at most POSTER_EDGE on its long side. */
function drawScaled(source: CanvasImageSource, width: number, height: number): HTMLCanvasElement {
  const k = Math.min(1, POSTER_EDGE / Math.max(width, height, 1))
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(width * k))
  canvas.height = Math.max(1, Math.round(height * k))
  const context = canvas.getContext('2d')
  if (!context) throw new Error('no 2d context')
  context.drawImage(source, 0, 0, canvas.width, canvas.height)
  return canvas
}

function once(element: HTMLMediaElement, event: 'loadedmetadata' | 'loadeddata' | 'seeked', signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const done = () => { cleanup(); resolve() }
    const fail = () => { cleanup(); reject(new Error(signal.aborted ? 'timed out' : 'the media could not be read')) }
    const cleanup = () => { element.removeEventListener(event, done); element.removeEventListener('error', fail); signal.removeEventListener('abort', fail) }
    element.addEventListener(event, done)
    element.addEventListener('error', fail)
    signal.addEventListener('abort', fail)
  })
}

const durationOf = (element: HTMLMediaElement) => (Number.isFinite(element.duration) && element.duration > 0 ? Math.round(element.duration * 1000) : undefined)

/** A frame a little into the video (the first is often black), its size and length. */
async function videoFacts(file: File, signal: AbortSignal): Promise<Omit<MediaFacts, 'kind'>> {
  const url = URL.createObjectURL(file)
  const video = document.createElement('video')
  video.muted = true
  video.playsInline = true
  video.preload = 'auto'
  try {
    const metadata = once(video, 'loadedmetadata', signal)
    video.src = url
    await metadata
    const durationMs = durationOf(video)
    const at = durationMs ? Math.min(1, durationMs / 10000) : 0
    if (at > 0) {
      const seeked = once(video, 'seeked', signal)
      video.currentTime = at
      await seeked
    } else if (video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
      await once(video, 'loadeddata', signal)
    }
    if (!video.videoWidth || !video.videoHeight) return { durationMs }
    const image = { width: video.videoWidth, height: video.videoHeight }
    return { image, durationMs, poster: await encode(drawScaled(video, image.width, image.height)) }
  } finally {
    video.removeAttribute('src')
    video.load()
    URL.revokeObjectURL(url)
  }
}

/** The embedded cover (when there is one) and the length. */
async function audioFacts(file: File, signal: AbortSignal): Promise<Omit<MediaFacts, 'kind'>> {
  const url = URL.createObjectURL(file)
  const audio = document.createElement('audio')
  audio.preload = 'metadata'
  let durationMs: number | undefined
  try {
    const metadata = once(audio, 'loadedmetadata', signal)
    audio.src = url
    await metadata
    durationMs = durationOf(audio)
  } catch { /* the length stays unknown */ } finally {
    audio.removeAttribute('src')
    URL.revokeObjectURL(url)
  }
  const cover = embeddedCover(new Uint8Array(await file.arrayBuffer()))
  if (!cover) return { durationMs }
  try {
    const bitmap = await createImageBitmap(new Blob([cover.data as BlobPart], { type: cover.mediaType }))
    const image = { width: bitmap.width, height: bitmap.height }
    const poster = await encode(drawScaled(bitmap, image.width, image.height))
    bitmap.close()
    return { image, durationMs, poster }
  } catch {
    return { durationMs }
  }
}

async function pdfFacts(file: File, signal: AbortSignal): Promise<Omit<MediaFacts, 'kind'>> {
  const { pdfFirstPage } = await import('./pdfDoc')
  const { canvas, pageCount } = await pdfFirstPage(new Uint8Array(await file.arrayBuffer()), POSTER_EDGE, signal)
  return { image: { width: canvas.width, height: canvas.height }, pageCount, poster: await encode(canvas) }
}
