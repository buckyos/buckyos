import { ndn } from 'buckyos'
import { httpStatusToError } from './io'
import { extensionOf, mediaTypeFromExtension, stripMediaTypeParams } from './mediaTypes'
import { PreviewError, type ObjectIdRef, type ResolvedPreviewSource } from './types'

export const NDM_PREVIEW_MAX_BYTES = 256 * 1024 * 1024

function checkSize(size: number) {
  if (size > NDM_PREVIEW_MAX_BYTES) throw new PreviewError('TOO_LARGE', 'Object previews are limited to 256 MiB')
}

async function readContent(response: Response, type: string, signal?: AbortSignal): Promise<Blob> {
  const length = Number(response.headers.get('content-length'))
  if (length > NDM_PREVIEW_MAX_BYTES) {
    await response.body?.cancel()
    checkSize(length)
  }
  if (!response.body) return new Blob([], { type })
  const reader = response.body.getReader()
  const parts: Uint8Array<ArrayBuffer>[] = []
  let size = 0
  const abort = () => { void reader.cancel().catch(() => {}) }
  signal?.addEventListener('abort', abort, { once: true })
  try {
    for (;;) {
      signal?.throwIfAborted()
      const { done, value } = await reader.read()
      signal?.throwIfAborted()
      if (done) return new Blob(parts, { type })
      size += value.byteLength
      checkSize(size)
      parts.push(value)
    }
  } finally {
    signal?.removeEventListener('abort', abort)
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

export async function resolveNdmSource(
  source: ObjectIdRef,
  endpoint: string,
  token: string | null,
  signal?: AbortSignal,
): Promise<ResolvedPreviewSource> {
  try {
    signal?.throwIfAborted()
    let id: ndn.ObjId
    try {
      id = ndn.ObjId.fromString(source.objectId)
    } catch {
      throw new PreviewError('INVALID_SOURCE', 'Invalid object ID')
    }
    if (!id.isFileObject() && !id.isChunk() && !id.isChunkList()) {
      throw new PreviewError('UNSUPPORTED', 'This object does not contain previewable file content')
    }
    const objId = id.toString()
    const request = async (path: string) => {
      signal?.throwIfAborted()
      const response = await fetch(`${endpoint}${path}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
        credentials: 'include',
        body: JSON.stringify({ obj_id: objId }),
        signal,
      })
      if (!response.ok) {
        await response.body?.cancel()
        if (response.status === 413) throw new PreviewError('TOO_LARGE', 'This object is too large to preview')
        if (response.status === 415) throw new PreviewError('UNSUPPORTED', 'This object has no readable content')
        throw httpStatusToError(response.status)
      }
      return response
    }
    let name = objId
    let size: number | undefined
    let contentId = objId
    let mediaType = ''
    if (id.isFileObject()) {
      const response = await request('/ndm/v1/store/get_object')
      try {
        const metadata = await response.json()
        const value = JSON.parse(metadata.obj_data)
        if (!value || typeof value !== 'object' || Array.isArray(value)
          || !Number.isSafeInteger(value.size ?? 0) || (value.size ?? 0) < 0 || typeof value.content !== 'string') {
          throw new Error('Invalid file metadata')
        }
        const file = ndn.FileObject.fromJSON(value)
        const content = ndn.ObjId.fromString(file.content)
        if (!content.isChunk() && !content.isChunkList()) throw new Error('Invalid file content')
        name = typeof file.content_obj.name === 'string' && file.content_obj.name ? file.content_obj.name : objId
        size = file.size
        contentId = content.toString()
        mediaType = typeof file.meta.mime_type === 'string' ? stripMediaTypeParams(file.meta.mime_type) : ''
      } catch {
        throw new PreviewError('CORRUPTED', 'The file object metadata is invalid')
      }
      checkSize(size)
    }
    mediaType ||= mediaTypeFromExtension(extensionOf(name)) ?? ''
    const response = await request('/ndm/proxy/v1/read/object/open')
    const blob = await readContent(response, mediaType, signal)
    if (size !== undefined && blob.size !== size) throw new PreviewError('CORRUPTED', 'The file content is incomplete')
    return {
      originalSource: source,
      sourceObjectId: objId,
      inputObjectId: contentId,
      displayName: name,
      size: blob.size,
      objectType: 'file',
      mediaTypeHints: mediaType ? [mediaType] : [],
      readRef: { kind: 'blob', blob },
    }
  } catch (error) {
    if (signal?.aborted || (error instanceof DOMException && error.name === 'AbortError')) {
      throw new PreviewError('CANCELLED', 'Cancelled')
    }
    if (error instanceof PreviewError) throw error
    throw new PreviewError('NETWORK', 'Could not load the object from the content server', { retryable: true })
  }
}
