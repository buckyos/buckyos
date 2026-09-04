/**
 * Bounded reads over a permission-bound read reference.
 *
 * Every read here is either a byte-range or a capped stream: the component
 * never pulls a whole large file just to classify it (PRD §17.1, §23.3).
 */

import { PreviewError, type PreviewReadRef } from './types'

export interface ProbeResult {
  bytes: Uint8Array
  contentType: string | null
  /** Total length when the server told us (Content-Range / Content-Length / blob size). */
  totalLength: number | null
  acceptsRanges: boolean
}

function totalFromHeaders(resp: Response): number | null {
  const range = resp.headers.get('Content-Range')
  if (range) {
    const total = range.split('/')[1]
    if (total && total !== '*') return Number(total)
  }
  if (resp.status === 200) {
    const length = resp.headers.get('Content-Length')
    if (length) return Number(length)
  }
  return null
}

export function httpStatusToError(status: number, detail?: string): PreviewError {
  if (status === 401 || status === 403) return new PreviewError('PERMISSION_DENIED', 'Access to this content is not allowed', { detail })
  if (status === 404 || status === 410) return new PreviewError('NOT_FOUND', 'This content no longer exists', { detail })
  if (status === 408 || status === 504) return new PreviewError('TIMEOUT', 'The content server timed out', { detail })
  if (status >= 500) return new PreviewError('NETWORK', 'The content server is unavailable', { detail, retryable: true })
  return new PreviewError('INTERNAL', `Unexpected response (${status})`, { detail })
}

/** Reads at most `length` bytes; a server without Range support is streamed and cut. */
export async function readProbe(
  readRef: PreviewReadRef,
  length: number,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  if (readRef.kind === 'blob') {
    const slice = readRef.blob.slice(0, length)
    const bytes = new Uint8Array(await slice.arrayBuffer())
    return { bytes, contentType: readRef.blob.type || null, totalLength: readRef.blob.size, acceptsRanges: true }
  }
  let resp: Response
  try {
    resp = await fetch(readRef.url, {
      headers: { Range: `bytes=0-${length - 1}` },
      signal,
      credentials: 'include',
      cache: 'no-store',
    })
  } catch (err) {
    if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
    throw new PreviewError('NETWORK', 'Could not reach the content server', { detail: String(err) })
  }
  if (!resp.ok) throw httpStatusToError(resp.status)
  const contentType = resp.headers.get('Content-Type')
  const totalLength = totalFromHeaders(resp)
  const acceptsRanges = resp.status === 206 || resp.headers.get('Accept-Ranges') === 'bytes'
  const bytes = await readCapped(resp, length)
  return { bytes, contentType, totalLength, acceptsRanges }
}

async function readCapped(resp: Response, cap: number): Promise<Uint8Array> {
  if (!resp.body) return new Uint8Array(await resp.arrayBuffer()).subarray(0, cap)
  const reader = resp.body.getReader()
  const chunks: Uint8Array[] = []
  let received = 0
  try {
    while (received < cap) {
      const { done, value } = await reader.read()
      if (done || !value) break
      chunks.push(value)
      received += value.byteLength
    }
  } finally {
    reader.cancel().catch(() => {})
  }
  const out = new Uint8Array(Math.min(received, cap))
  let offset = 0
  for (const chunk of chunks) {
    const take = Math.min(chunk.byteLength, out.length - offset)
    if (take <= 0) break
    out.set(chunk.subarray(0, take), offset)
    offset += take
  }
  return out
}

export interface TextReadResult {
  text: string
  truncated: boolean
  byteLength: number
  totalLength: number | null
}

/** Reads up to `maxBytes` as UTF-8 text (BOM stripped); reports truncation. */
export async function readText(
  readRef: PreviewReadRef,
  maxBytes: number,
  signal?: AbortSignal,
): Promise<TextReadResult> {
  const probe = await readProbe(readRef, maxBytes, signal)
  const total = probe.totalLength
  const truncated = total !== null ? total > probe.bytes.byteLength : probe.bytes.byteLength >= maxBytes
  let bytes = probe.bytes
  if (truncated) {
    // Do not end on a partial UTF-8 sequence.
    let cut = bytes.byteLength
    while (cut > 0 && cut > bytes.byteLength - 4 && (bytes[cut - 1] & 0xc0) === 0x80) cut -= 1
    if (cut > 0 && cut > bytes.byteLength - 4 && (bytes[cut - 1] & 0xc0) === 0xc0) cut -= 1
    bytes = bytes.subarray(0, cut)
  }
  const text = new TextDecoder('utf-8').decode(bytes)
  return { text, truncated, byteLength: bytes.byteLength, totalLength: total }
}

/** Reads the whole content (bounded by `maxBytes`, throws TOO_LARGE beyond). */
export async function readAll(readRef: PreviewReadRef, maxBytes: number, signal?: AbortSignal): Promise<Blob> {
  if (readRef.kind === 'blob') {
    if (readRef.blob.size > maxBytes) throw new PreviewError('TOO_LARGE', 'The content is too large to preview directly')
    return readRef.blob
  }
  let resp: Response
  try {
    resp = await fetch(readRef.url, { signal, credentials: 'include' })
  } catch (err) {
    if (signal?.aborted) throw new PreviewError('CANCELLED', 'Cancelled')
    throw new PreviewError('NETWORK', 'Could not reach the content server', { detail: String(err) })
  }
  if (!resp.ok) throw httpStatusToError(resp.status)
  const length = Number(resp.headers.get('Content-Length') ?? '0')
  if (length > maxBytes) throw new PreviewError('TOO_LARGE', 'The content is too large to preview directly')
  const blob = await resp.blob()
  if (blob.size > maxBytes) throw new PreviewError('TOO_LARGE', 'The content is too large to preview directly')
  return blob
}
