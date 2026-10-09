/* Surface operations and insertion choices (non-component helpers of the canvas tools). */

import { randomId } from '../../api/ids'
import type { EntityEnvelope, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'

export function surfacesOf(store: WorkspaceStore): EntityEnvelope[] {
  return store.outline.childrenOf('surfaces').filter((e) => e.kind === 'surface')
}

/** Create a Surface with its canvas content folder in one commit (§4.1), optionally with a preset icon. */
export function createSurfaceOps(store: WorkspaceStore, title: string, mode: 'free' | 'flow', icon?: string | null): { ops: Operation[]; surfaceId: string } {
  const core = store.core
  const surfaceId = randomId('sf')
  const folderId = `${surfaceId}-content`
  const lastSurface = store.outline.childrenOf('surfaces').at(-1)?.order_key
  const lastFolder = store.outline.childrenOf('canvas-content').at(-1)?.order_key
  return {
    surfaceId,
    ops: [
      { op: 'entity.create', entity_id: folderId, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: core.order_key_between(lastFolder, undefined), name: title, payload: { kind: 'folder', title, system: 'surface_content', surface_id: surfaceId } },
      { op: 'entity.create', entity_id: surfaceId, type_id: 'buckyos.container', parent_id: 'surfaces', order_key: core.order_key_between(lastSurface, undefined), name: title, payload: { kind: 'surface', layout: { mode }, title, content_folder_id: folderId, ...(icon ? { icon } : {}) } },
    ],
  }
}

