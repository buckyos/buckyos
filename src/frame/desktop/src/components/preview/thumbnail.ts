import { useEffect, useRef, useState } from 'react'
import { readProbe } from './io'
import { classifyMedia, decideDirect, ensureRuntimeProfile, type MediaClassification } from './mediaTypes'
import { ensurePreviewProvider } from './provider'
import { refIdentity } from './session'
import {
  PreviewError,
  type ContentRef,
  type PreviewReadRef,
  type PreviewRendererType,
  type PreviewResult,
  type PreviewWorkState,
  type ResolvedPreviewSource,
} from './types'

export interface PreviewThumbnailRequest {
  width: number
  height: number
}

export interface PreviewThumbnail {
  url: string
  mediaType: string
  contentRenderer: PreviewRendererType | null
  contentMediaType: string
  via: 'direct' | 'pipeline' | 'runtime-capture'
  width?: number
  height?: number
  durationSeconds?: number
}

const PROBE_BYTES = 512
const PIPELINE_TIMEOUT_MS = 60_000
const PIPELINE_ENSURE_TIMEOUT_MS = 8_000
const CAPTURE_TIMEOUT_MS = 20_000
const CAPTURE_CONCURRENCY = 2
const CACHE_MAX = 96
const CAPTURE_MAX_EDGE = 640

interface CacheEntry {
  promise: Promise<PreviewThumbnail>
  controller: AbortController
  consumers: number
  settled: boolean
  ownedUrl?: string
}

const cache = new Map<string, CacheEntry>()

function evict(key: string) {
  const entry = cache.get(key)
  if (!entry) return
  cache.delete(key)
  if (!entry.settled) entry.controller.abort()
  if (entry.ownedUrl) URL.revokeObjectURL(entry.ownedUrl)
}

function trimCache() {
  for (const [key, entry] of cache) {
    if (cache.size <= CACHE_MAX) return
    if (entry.consumers === 0) evict(key)
  }
}

function sizeBucket(request: PreviewThumbnailRequest): string {
  const dpr = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1
  const edge = Math.max(request.width, request.height) * dpr
  return String(edge <= 256 ? 256 : edge <= 512 ? 512 : 1024)
}

export function previewThumbnailKey(source: ContentRef, request: PreviewThumbnailRequest): string {
  return `${refIdentity(source)}|${sizeBucket(request)}`
}

export function ensurePreviewThumbnail(
  source: ContentRef,
  request: PreviewThumbnailRequest,
  signal?: AbortSignal,
): Promise<PreviewThumbnail> {
  const key = previewThumbnailKey(source, request)
  let entry = cache.get(key)
  if (!entry) {
    const controller = new AbortController()
    const created: CacheEntry = { controller, consumers: 0, settled: false, promise: Promise.resolve(null as never) }
    created.promise = buildThumbnail(source, request, controller.signal, (url) => {
      created.ownedUrl = url
    }).then(
      (thumbnail) => {
        created.settled = true
        return thumbnail
      },
      (err: unknown) => {
        created.settled = true
        if (cache.get(key) === created) cache.delete(key)
        if (created.ownedUrl) URL.revokeObjectURL(created.ownedUrl)
        throw err
      },
    )
    cache.set(key, created)
    trimCache()
    entry = created
  } else {
    cache.delete(key)
    cache.set(key, entry)
  }
  const joined = entry
  joined.consumers += 1
  let released = false
  const release = () => {
    if (released) return
    released = true
    joined.consumers -= 1
    if (joined.consumers === 0 && !joined.settled && cache.get(key) === joined) evict(key)
  }
  if (signal?.aborted) {
    release()
    return Promise.reject(new PreviewError('CANCELLED', 'Cancelled'))
  }
  return new Promise<PreviewThumbnail>((resolve, reject) => {
    const onAbort = () => {
      release()
      reject(new PreviewError('CANCELLED', 'Cancelled'))
    }
    signal?.addEventListener('abort', onAbort, { once: true })
    joined.promise.then(
      (thumbnail) => {
        signal?.removeEventListener('abort', onAbort)
        release()
        resolve(thumbnail)
      },
      (err: unknown) => {
        signal?.removeEventListener('abort', onAbort)
        release()
        reject(err)
      },
    )
  })
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
}

function urlOf(readRef: PreviewReadRef, own: (url: string) => void): string {
  if (readRef.kind === 'url') return readRef.url
  const url = URL.createObjectURL(readRef.blob)
  own(url)
  return url
}

async function buildThumbnail(
  source: ContentRef,
  request: PreviewThumbnailRequest,
  signal: AbortSignal,
  own: (url: string) => void,
): Promise<PreviewThumbnail> {
  const provider = await ensurePreviewProvider()
  let resolved = await provider.resolvePreviewSource(source, { signal })
  throwIfAborted(signal)

  let probeBytes: Uint8Array | null = null
  let contentType: string | null = null
  try {
    const probe = await readProbe(resolved.readRef, PROBE_BYTES, signal)
    probeBytes = probe.bytes
    contentType = probe.contentType
    if (resolved.size === undefined && probe.totalLength !== null) resolved = { ...resolved, size: probe.totalLength }
  } catch (err) {
    if (err instanceof PreviewError && (err.code === 'PERMISSION_DENIED' || err.code === 'NOT_FOUND' || err.code === 'CANCELLED')) throw err
  }
  throwIfAborted(signal)
  const classification = classifyMedia({
    name: resolved.displayName,
    hints: resolved.mediaTypeHints,
    contentType,
    magic: probeBytes,
    objectType: resolved.objectType,
  })
  const runtime = await ensureRuntimeProfile()
  const direct = decideDirect(classification, resolved.size, runtime)
  const renderer = classification.rendererType

  if (direct.ok && (renderer === 'image' || renderer === 'svg')) {
    return {
      url: urlOf(resolved.readRef, own),
      mediaType: classification.mediaType,
      contentRenderer: renderer,
      contentMediaType: classification.mediaType,
      via: 'direct',
    }
  }

  let pipelineError: unknown = null
  try {
    const result = await pipelineThumbnail(resolved, request, signal)
    if (result) {
      return {
        url: urlOf(result.readRef, own),
        mediaType: result.mediaType,
        contentRenderer: renderer,
        contentMediaType: classification.mediaType,
        via: 'pipeline',
        width: result.width,
        height: result.height,
        durationSeconds: result.durationSeconds,
      }
    }
  } catch (err) {
    if (signal.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
    pipelineError = err
  }

  if (renderer === 'video' && direct.ok) {
    const frame = await runCapture(() => captureVideoFrame(resolved.readRef, request, signal, own), signal)
    return { ...frame, contentRenderer: renderer, contentMediaType: classification.mediaType, via: 'runtime-capture' }
  }

  if (pipelineError instanceof PreviewError) throw pipelineError
  throw new PreviewError('UNSUPPORTED', 'No thumbnail is available for this content', { detail: contentLabel(classification) })
}

function contentLabel(classification: MediaClassification): string {
  return classification.extension ? `${classification.extension} · ${classification.mediaType}` : classification.mediaType
}

async function pipelineThumbnail(
  resolved: ResolvedPreviewSource,
  request: PreviewThumbnailRequest,
  signal: AbortSignal,
): Promise<PreviewResult | null> {
  const provider = await ensurePreviewProvider()
  const runtime = await ensureRuntimeProfile()
  const ensured = await withTimeout(provider.ensurePreviewWork({
    source: resolved,
    runtimeProfile: { ...runtime, acceptTypes: ['image'] },
    targetProfile: {
      purpose: 'thumbnail',
      viewport: { width: Math.max(1, Math.round(request.width)), height: Math.max(1, Math.round(request.height)), dpr: window.devicePixelRatio || 1 },
      quality: 'fast',
    },
    options: { signal },
  }), PIPELINE_ENSURE_TIMEOUT_MS)
  if (!ensured || 'kind' in ensured) return null
  let state: PreviewWorkState = ensured
  const startedAt = Date.now()
  while (state.state === 'processing') {
    if (Date.now() - startedAt > PIPELINE_TIMEOUT_MS) throw new PreviewError('TIMEOUT', 'Preparing the thumbnail took too long')
    await wait(Math.min(Math.max(state.retryAfterMs ?? 800, 300), 3000), signal)
    state = await provider.getPreviewWork(state.workKey, { signal })
  }
  if (state.state === 'failed') {
    throw new PreviewError('PIPELINE_FAILED', state.error.message, { retryable: state.error.retryable, detail: state.error.code })
  }
  const result = state.result
  return result.resultType === 'image' || result.resultType === 'svg' ? result : null
}

function withTimeout<T>(pending: Promise<T>, ms: number): Promise<T | null> {
  let timer = 0
  const timeout = new Promise<null>((resolve) => {
    timer = window.setTimeout(() => resolve(null), ms)
  })
  return Promise.race([pending, timeout]).finally(() => window.clearTimeout(timer))
}

function wait(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    const onAbort = () => {
      window.clearTimeout(timer)
      reject(new PreviewError('CANCELLED', 'Cancelled'))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

let activeCaptures = 0
const captureQueue: Array<() => void> = []

async function runCapture<T>(task: () => Promise<T>, signal: AbortSignal): Promise<T> {
  if (activeCaptures >= CAPTURE_CONCURRENCY) {
    await new Promise<void>((resolve) => captureQueue.push(resolve))
  }
  activeCaptures += 1
  try {
    throwIfAborted(signal)
    return await task()
  } finally {
    activeCaptures -= 1
    captureQueue.shift()?.()
  }
}

function mediaEvent(video: HTMLVideoElement, event: 'loadedmetadata' | 'loadeddata' | 'seeked', signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      window.clearTimeout(timer)
      video.removeEventListener(event, onEvent)
      video.removeEventListener('error', onError)
      signal.removeEventListener('abort', onAbort)
    }
    const onEvent = () => {
      cleanup()
      resolve()
    }
    const onError = () => {
      cleanup()
      reject(new PreviewError('CORRUPTED', 'The video could not be decoded'))
    }
    const onAbort = () => {
      cleanup()
      reject(new PreviewError('CANCELLED', 'Cancelled'))
    }
    const timer = window.setTimeout(() => {
      cleanup()
      reject(new PreviewError('TIMEOUT', 'The video did not load in time'))
    }, CAPTURE_TIMEOUT_MS)
    video.addEventListener(event, onEvent, { once: true })
    video.addEventListener('error', onError, { once: true })
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

async function captureVideoFrame(
  readRef: PreviewReadRef,
  request: PreviewThumbnailRequest,
  signal: AbortSignal,
  own: (url: string) => void,
): Promise<Omit<PreviewThumbnail, 'contentRenderer' | 'contentMediaType' | 'via'>> {
  const temporaryUrl = readRef.kind === 'blob' ? URL.createObjectURL(readRef.blob) : null
  const video = document.createElement('video')
  video.muted = true
  video.playsInline = true
  video.preload = 'auto'
  try {
    const metadata = mediaEvent(video, 'loadedmetadata', signal)
    video.src = temporaryUrl ?? (readRef.kind === 'url' ? readRef.url : '')
    await metadata
    const duration = Number.isFinite(video.duration) ? video.duration : undefined
    const at = duration !== undefined ? Math.min(1, duration * 0.1) : 0
    if (at > 0) {
      const seeked = mediaEvent(video, 'seeked', signal)
      video.currentTime = at
      await seeked
    } else if (video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
      await mediaEvent(video, 'loadeddata', signal)
    }
    const sourceWidth = video.videoWidth
    const sourceHeight = video.videoHeight
    if (!sourceWidth || !sourceHeight) throw new PreviewError('UNSUPPORTED', 'The media has no video track')
    const dpr = window.devicePixelRatio || 1
    const box = Math.min(Math.max(request.width, request.height) * dpr, CAPTURE_MAX_EDGE)
    const scale = Math.min(1, box / Math.max(sourceWidth, sourceHeight))
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(sourceWidth * scale))
    canvas.height = Math.max(1, Math.round(sourceHeight * scale))
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new PreviewError('INTERNAL', 'Canvas unavailable')
    ctx.drawImage(video, 0, 0, canvas.width, canvas.height)
    let blob: Blob | null
    try {
      blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.82))
    } catch {
      // A cross-origin video without CORS taints the canvas.
      throw new PreviewError('PERMISSION_DENIED', 'The video frame cannot be read')
    }
    if (!blob) throw new PreviewError('INTERNAL', 'The frame could not be encoded')
    throwIfAborted(signal)
    const url = URL.createObjectURL(blob)
    own(url)
    return { url, mediaType: 'image/jpeg', width: sourceWidth, height: sourceHeight, durationSeconds: duration }
  } finally {
    video.removeAttribute('src')
    video.load()
    if (temporaryUrl) URL.revokeObjectURL(temporaryUrl)
  }
}

export type PreviewThumbnailState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; thumbnail: PreviewThumbnail }
  | { status: 'error'; error: PreviewError }

function asPreviewError(err: unknown): PreviewError {
  if (err instanceof PreviewError) return err
  return new PreviewError('INTERNAL', err instanceof Error ? err.message : String(err))
}

export function usePreviewThumbnail(
  source: ContentRef | null,
  request: PreviewThumbnailRequest,
): PreviewThumbnailState & { retry: () => void } {
  const [attempt, setAttempt] = useState(0)
  const key = source ? `${previewThumbnailKey(source, request)}#${attempt}` : null
  const [entry, setEntry] = useState<{ key: string; state: PreviewThumbnailState } | null>(null)
  const latest = useRef({ source, request })
  useEffect(() => {
    latest.current = { source, request }
  })
  useEffect(() => {
    const current = latest.current.source
    if (!key || !current) return
    const controller = new AbortController()
    ensurePreviewThumbnail(current, latest.current.request, controller.signal).then(
      (thumbnail) => setEntry({ key, state: { status: 'ready', thumbnail } }),
      (err: unknown) => {
        if (!controller.signal.aborted) setEntry({ key, state: { status: 'error', error: asPreviewError(err) } })
      },
    )
    return () => controller.abort()
  }, [key])
  const state: PreviewThumbnailState = !key ? { status: 'idle' } : entry?.key === key ? entry.state : { status: 'loading' }
  return { ...state, retry: () => setAttempt((value) => value + 1) }
}
