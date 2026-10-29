import { bucket, buckyos } from 'buckyos'
import { httpStatusToError } from './io'
import { extensionOf, mediaTypeFromExtension, stripMediaTypeParams } from './mediaTypes'
import { PreviewError, type BucketObjectRef, type ContentRef, type ResolvedPreviewSource } from './types'

export type BucketTarget = string | { bucketId: string; key: string }

function targetOf(target: BucketTarget): { bucketId: string; key: string } {
  if (typeof target !== 'string') return target
  const parsed = bucket.parseBucketObjUrl(target)
  if (!parsed) throw new PreviewError('INVALID_SOURCE', 'Invalid obj://bucket address')
  return parsed
}

/** Browser URL of a bucket binding (`/bucket/v1/...`); same-origin pages read it with the SSO cookie. */
export function bucketObjectUrl(target: BucketTarget, options: { version?: number; token?: string } = {}): string {
  const { bucketId, key } = targetOf(target)
  return buckyos.getBucketClient().objectUrl(bucketId, key, options)
}

export function bucketObjectRef(target: BucketTarget, options: { version?: number; token?: string } = {}): BucketObjectRef {
  const { bucketId, key } = targetOf(target)
  return { kind: 'bucket-object', url: bucket.bucketObjUrl(bucketId, key), ...options }
}

/** `object-id` sources may carry an `obj://bucket/...` address. */
export function asBucketObjectRef(source: ContentRef): BucketObjectRef | null {
  if (source.kind === 'bucket-object') return source as BucketObjectRef
  if (source.kind === 'object-id') {
    const id = (source as { objectId?: unknown }).objectId
    if (typeof id === 'string' && id.startsWith(bucket.BUCKET_OBJ_URL_PREFIX)) return { kind: 'bucket-object', url: id }
  }
  return null
}

function errorOf(error: unknown): PreviewError {
  if (error instanceof PreviewError) return error
  const code = (error as { code?: unknown })?.code
  if (code === 'no_such_key' || code === 'bucket_not_found' || code === 'content_evicted' || code === 'origin_unavailable') {
    return new PreviewError('NOT_FOUND', 'This content no longer exists')
  }
  if (code === 'permission_denied' || code === 'token_invalid' || code === 'not_held') return new PreviewError('PERMISSION_DENIED', 'You do not have access to this content')
  const status = (error as { status?: unknown })?.status
  if (typeof status === 'number') return httpStatusToError(status)
  return new PreviewError('NETWORK', 'Could not load the object from the content server', { retryable: true })
}

/** The bucket binding behind a resolved source: its own ref, or one a host resolver found for it. */
export function bucketRefOf(resolved: ResolvedPreviewSource): BucketObjectRef | null {
  const own = asBucketObjectRef(resolved.originalSource)
  if (own) return own
  const handle = resolved.providerHandle as { kind?: unknown; url?: unknown } | undefined
  return handle?.kind === 'bucket-object' && typeof handle.url === 'string' ? (handle as BucketObjectRef) : null
}

/**
 * Resolves a bucket binding to a URL read reference: `bucket.head` for name,
 * size and type (and `availability`, so ambient content that is gone fails
 * here instead of in `<img>`), then the data-plane URL itself.
 */
export async function resolveBucketSource(source: BucketObjectRef, signal?: AbortSignal): Promise<ResolvedPreviewSource> {
  try {
    signal?.throwIfAborted()
    const { bucketId, key } = targetOf(source.url)
    const client = buckyos.getBucketClient()
    const head = await client.call<{ binding: bucket.Binding; availability: bucket.Availability }>('bucket.head', {
      bucket_id: bucketId,
      key,
      version: source.version,
      ...(source.token ? { access_token: source.token } : {}),
    })
    signal?.throwIfAborted()
    if (head.availability !== 'local') throw new PreviewError('NOT_FOUND', 'This content is not available right now')
    const binding = head.binding
    if (binding.is_delete_marker) throw new PreviewError('NOT_FOUND', 'This content no longer exists')
    const name = key.split('/').pop() || key
    const mediaType = (binding.content_type ? stripMediaTypeParams(binding.content_type) : '') || mediaTypeFromExtension(extensionOf(name)) || ''
    const url = client.objectUrl(bucketId, key, { version: binding.version, token: source.token })
    return {
      originalSource: source,
      sourceObjectId: binding.obj_id,
      inputObjectId: binding.obj_id,
      versionToken: binding.etag,
      displayName: name,
      size: binding.size,
      objectType: 'file',
      mediaTypeHints: mediaType ? [mediaType] : [],
      readRef: { kind: 'url', url },
    }
  } catch (error) {
    if (signal?.aborted || (error instanceof DOMException && error.name === 'AbortError')) {
      throw new PreviewError('CANCELLED', 'Cancelled')
    }
    throw errorOf(error)
  }
}
