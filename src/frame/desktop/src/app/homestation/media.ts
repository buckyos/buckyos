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

async function fetchContent(value: HomeStationFileSourceValue, url: string): Promise<Blob> {
  let response: Response
  try {
    response = await fetch(url)
  } catch (error) {
    throw new PreviewError('NETWORK', error instanceof Error ? error.message : String(error))
  }
  if (response.status === 404) throw new PreviewError('NOT_FOUND', `${value.objId} is not held by your HomeStation`)
  if (response.status === 401 || response.status === 403) throw new PreviewError('PERMISSION_DENIED', `HTTP ${response.status}`)
  if (!response.ok) throw new PreviewError('NETWORK', `HTTP ${response.status}`)
  return response.blob()
}

async function resolveFile(source: ContentRef): Promise<ResolvedPreviewSource> {
  const value = (source as { value?: HomeStationFileSourceValue }).value
  if (!value?.objId) throw new PreviewError('NOT_FOUND', 'Unknown HomeStation file')
  const blob = value.url
    ? await fetchContent(value, value.url)
    : new Blob([placeholderSvg(value.objId, value.file, value.alt)], { type: 'image/svg+xml' })
  return {
    originalSource: source,
    sourceObjectId: value.objId,
    inputObjectId: value.objId,
    displayName: value.name,
    size: blob.size,
    objectType: 'file',
    mediaTypeHints: [value.url ? value.file?.meta.mime ?? blob.type : 'image/svg+xml'],
    readRef: { kind: 'blob', blob },
  }
}

let installed = false

export function installHomeStationPreviewSource() {
  if (installed) return
  installed = true
  registerPreviewSourceResolver(HOMESTATION_FILE_SOURCE, { resolvePreviewSource: resolveFile })
}
