import { bucketObjectRef, resolveBucketSource } from '../../components/preview/bucketSource'
import { registerPreviewSourceResolver } from '../../components/preview/hostSources'
import { PreviewError, type ContentRef, type ResolvedPreviewSource } from '../../components/preview/types'
import { homeStationTransport } from './api/transport'
import { placeholderSvg } from './mock/media'
import type { FileObject, ObjId } from './protocol/feed'

export const HOMESTATION_FILE_SOURCE = 'homestation-file'

export interface HomeStationFileSourceValue {
  objId: ObjId
  name: string
  reference: string
  alt?: string
  file?: FileObject
  url?: string
}

export function mediaUrl(objId: ObjId, file: FileObject | undefined, label?: string) {
  const transport = homeStationTransport()
  return transport ? transport.contentUrl(objId) : `data:image/svg+xml;charset=utf-8,${encodeURIComponent(placeholderSvg(objId, file, label))}`
}

export function fileSourceRef(objId: ObjId, file: FileObject | undefined, alt?: string): ContentRef {
  const transport = homeStationTransport()
  const value: HomeStationFileSourceValue = transport
    ? { objId, name: file?.name ?? alt ?? objId, reference: objId, alt, file, url: transport.contentUrl(objId) }
    : { objId, name: `${alt ?? file?.name ?? 'image'}.svg`, reference: objId, alt, file }
  return { kind: HOMESTATION_FILE_SOURCE, value }
}

function cancelled(error: unknown, signal?: AbortSignal): boolean {
  return !!signal?.aborted || (error instanceof DOMException && error.name === 'AbortError')
}

/**
 * A HomeStation file reads from its bucket binding on the nfs-server data
 * plane (Range, a read token; the Pipeline can convert it), else from
 * `/home/<user>/objects/<id>/content`, which streams with Range as well.
 */
async function resolveFile(source: ContentRef, { signal }: { signal?: AbortSignal }): Promise<ResolvedPreviewSource> {
  const value = (source as { value?: HomeStationFileSourceValue }).value
  if (!value?.objId) throw new PreviewError('NOT_FOUND', 'Unknown HomeStation file')
  if (!value.url) {
    const blob = new Blob([placeholderSvg(value.objId, value.file, value.alt)], { type: 'image/svg+xml' })
    return { originalSource: source, sourceObjectId: value.objId, inputObjectId: value.objId, displayName: value.name, size: blob.size, objectType: 'file', mediaTypeHints: ['image/svg+xml'], readRef: { kind: 'blob', blob } }
  }
  const transport = homeStationTransport()
  let bound = null
  try {
    bound = transport ? await transport.contentRef(value.objId, signal) : null
  } catch (error) {
    if (cancelled(error, signal)) throw new PreviewError('CANCELLED', 'Cancelled')
  }
  const hints = [value.file?.meta.mime ?? bound?.mime ?? ''].filter(Boolean)
  if (bound) {
    const bucketRef = bucketObjectRef({ bucketId: bound.bucketId, key: bound.key }, { version: bound.version, token: bound.token })
    try {
      const resolved = await resolveBucketSource(bucketRef, signal)
      return { ...resolved, originalSource: source, sourceObjectId: value.objId, displayName: value.name, mediaTypeHints: hints.length ? hints : resolved.mediaTypeHints, providerHandle: bucketRef }
    } catch (error) {
      if (cancelled(error, signal) || (error instanceof PreviewError && error.code === 'CANCELLED')) throw new PreviewError('CANCELLED', 'Cancelled')
      // No bucket client here (a dev override on the mock shell): the content route serves it too.
    }
  }
  return {
    originalSource: source,
    sourceObjectId: value.objId,
    inputObjectId: value.objId,
    displayName: value.name,
    size: value.file?.meta.size,
    objectType: 'file',
    mediaTypeHints: hints,
    readRef: { kind: 'url', url: value.url },
  }
}

let installed = false

export function installHomeStationPreviewSource() {
  if (installed) return
  installed = true
  registerPreviewSourceResolver(HOMESTATION_FILE_SOURCE, { resolvePreviewSource: resolveFile })
}
