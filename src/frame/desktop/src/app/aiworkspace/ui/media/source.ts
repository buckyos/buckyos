/* Workspace assets as Preview sources (多媒体对象 §4.3): the Preview component reads an asset through a host Source
 * Resolver (`aiworkspace-asset`). The source carries the reader of its Workspace's session, so it resolves wherever the
 * component is mounted; the bytes come from the asset route like a picture's. */

import { registerPreviewSourceResolver } from '../../../../components/preview/hostSources'
import { PreviewError, type ContentRef, type PreviewSessionContext, type PreviewSessionItemInput, type ResolvedPreviewSource } from '../../../../components/preview/types'
import type { ReadOk } from '../../api/session'
import type { AssetContent, EntityEnvelope } from '../../api/types'
import { TransportError } from '../../api/transport'
import type { WorkspaceStore } from '../../state/store'
import { assetMediaType } from './kinds'

export const AIWORKSPACE_ASSET_SOURCE = 'aiworkspace-asset'

export interface AssetSourceValue {
  workspace_id: string
  object_id: string
  media_type: string
  /** Display name and copyable reference (Preview's extension-ref convention). */
  name: string
  reference: string
  /** Not part of the source's identity (functions do not serialize). */
  read: () => Promise<Blob>
}

export function assetSource(store: WorkspaceStore, payload: AssetContent['payload'], name: string): ContentRef {
  if (!installed) {
    installed = true
    registerPreviewSourceResolver(AIWORKSPACE_ASSET_SOURCE, { resolvePreviewSource: resolveAsset })
  }
  const value: AssetSourceValue = {
    workspace_id: store.session.workspaceId, object_id: payload.object_id, media_type: assetMediaType(payload.media_type, payload.file_name),
    name, reference: payload.object_id, read: () => store.session.fetchAsset(payload.object_id),
  }
  return { kind: AIWORKSPACE_ASSET_SOURCE, value }
}
let installed = false

function failure(error: unknown): PreviewError {
  const detail = error instanceof Error ? error.message : String(error)
  if (error instanceof TransportError) {
    if (error.kind === 'network') return new PreviewError('NETWORK', 'The file could not be loaded', { detail })
    if (error.denial || error.status === 401 || error.status === 403) return new PreviewError('PERMISSION_DENIED', 'You do not have access to this file', { detail })
    if (error.status === 404 || error.status === 410) return new PreviewError('NOT_FOUND', 'This file is no longer available', { detail })
  }
  return new PreviewError('INTERNAL', 'The file could not be loaded', { detail, retryable: true })
}

async function resolveAsset(source: ContentRef): Promise<ResolvedPreviewSource> {
  const value = (source as { value?: AssetSourceValue }).value
  if (!value || typeof value.object_id !== 'string' || typeof value.read !== 'function') throw new PreviewError('INVALID_SOURCE', 'Invalid workspace file')
  let blob: Blob
  try { blob = await value.read() } catch (error) { throw failure(error) }
  // the asset route serves types it does not list as octet-stream
  const typed = blob.type === value.media_type ? blob : new Blob([blob], { type: value.media_type })
  return {
    originalSource: source, sourceObjectId: value.object_id, displayName: value.name, size: typed.size, objectType: 'file',
    mediaTypeHints: [value.media_type], readRef: { kind: 'blob', blob: typed },
  }
}

/** The media of a Surface in reading order (the outline's), for the preview's previous / next. */
export function surfaceMediaSession(store: WorkspaceStore, cellId: string, current: ContentRef): PreviewSessionContext {
  const surfaceId = store.outline.ancestors(cellId).find((id) => store.outline.get(id)?.kind === 'surface')
  return {
    kind: 'provider', sessionId: `aiworkspace:${store.session.workspaceId}:${surfaceId ?? cellId}:${cellId}`, currentItemId: cellId, navigation: 'bounded',
    provider: {
      listItems: async () => {
        const single = [{ id: cellId, source: current }]
        if (!surfaceId) return single
        const cells: EntityEnvelope[] = store.outline.descendants(surfaceId).filter((e) => e.view_type === 'asset' && e.source_id && !e.deleted)
        const items = await Promise.all(cells.map(async (cell): Promise<PreviewSessionItemInput | null> => {
          if (cell.entity_id === cellId) return { id: cellId, source: current }
          try {
            const read: ReadOk<AssetContent> = await store.readBatched<AssetContent>(cell.source_id as string)
            if (read.content.availability !== 'available') return null
            const name = read.content.payload.file_name ?? read.name ?? cell.source_id as string
            return { id: cell.entity_id, source: assetSource(store, read.content.payload, name), title: name }
          } catch { return null }
        }))
        const listed = items.filter((item): item is PreviewSessionItemInput => item !== null)
        return listed.some((item) => item.id === cellId) ? listed : single
      },
    },
  }
}
