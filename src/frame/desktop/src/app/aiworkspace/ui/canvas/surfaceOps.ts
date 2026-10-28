/* Surface operations and insertion choices (non-component helpers of the canvas tools). */

import { isControlKind } from '@controls/contract'
import { randomId } from '../../api/ids'
import type { EntityEnvelope, Json, KeyedContent, Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'

export function surfacesOf(store: WorkspaceStore): EntityEnvelope[] {
  return store.outline.childrenOf('surfaces').filter((e) => e.kind === 'surface')
}

/** Duplicate a canvas with same-canvas semantics (内核架构改进 D05, T5-4): the Blocks are created again on a new
 * Surface; data the canvas owns (created with it, not declared shared) is cloned through `entity.clone` into the
 * new content folder, everything else is referenced. Groups are flattened, controls copied whole; wishes and
 * definitions are referenced. */
export async function duplicateSurfaceOps(store: WorkspaceStore, surfaceId: string): Promise<{ ops: Operation[]; surfaceId: string }> {
  const surface = store.outline.get(surfaceId)
  if (!surface) throw new Error(surfaceId)
  const title = aiwsT('aiworkspace.shell.switcher.duplicateTitle', { title: surface.title ?? surface.name ?? surfaceId })
  const created = createSurfaceOps(store, title, surface.layout?.mode === 'flow' ? 'flow' : 'free', surface.icon ?? null)
  const folderId = `${created.surfaceId}-content`
  const sourceFolder = surface.content_folder_id ?? null
  const ops: Operation[] = [...created.ops]
  const clones = new Map<string, string>()
  const cells: EntityEnvelope[] = []
  const controls: EntityEnvelope[] = []
  const walk = (parentId: string) => {
    for (const child of store.outline.childrenOf(parentId)) {
      if (child.deleted) continue
      if (child.type_id === 'buckyos.cell') cells.push(child)
      else if (child.type_id === 'buckyos.container' && child.kind === 'group') walk(child.entity_id)
      else if (child.type_id === 'buckyos.container' && isControlKind(child.kind)) controls.push(child)
    }
  }
  walk(surfaceId)
  let order: string | undefined
  /** A Cell's payload for the copy: its own data cloned into the new content folder (D05). */
  const copyPayload = async (cell: EntityEnvelope): Promise<Record<string, Json>> => {
    const read = await store.session.read<KeyedContent<Record<string, Json>>>(cell.entity_id)
    const payload: Record<string, Json> = { ...read.content.payload }
    const ref = payload.source_ref as { entity_id?: string } | undefined
    const data = ref?.entity_id ? store.outline.get(ref.entity_id) : undefined
    const own = data && sourceFolder && data.parent_id === sourceFolder && payload.data_ownership !== 'shared' && ['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref'].includes(data.type_id)
    if (own && data) {
      let copy = clones.get(data.entity_id)
      if (!copy) {
        copy = randomId(data.entity_id.slice(0, 1))
        clones.set(data.entity_id, copy)
        ops.push({ op: 'entity.clone', entity_id: data.entity_id, new_entity_id: copy, parent_id: folderId, order_key: store.core.order_key_between(undefined, undefined), ...(data.name ? { name: data.name } : {}) })
      }
      payload.source_ref = { entity_id: copy }
    }
    return payload
  }
  for (const cell of cells) {
    const payload = await copyPayload(cell)
    order = store.core.order_key_between(order, undefined)
    ops.push({ op: 'entity.create', entity_id: randomId('b'), type_id: 'buckyos.cell', parent_id: created.surfaceId, order_key: order, ...(cell.placement ? { placement: cell.placement as unknown as Json } : {}), payload })
  }
  // a control (a mind map) is copied whole: its Blocks keep their tree, order and placements (脑图方案 §4.2)
  const copyTree = async (entity: EntityEnvelope, parentId: string, key: string) => {
    const id = randomId(entity.type_id === 'buckyos.cell' ? 'b' : 'g')
    const payload = entity.type_id === 'buckyos.cell' ? await copyPayload(entity) : { ...(await store.session.read<KeyedContent<Record<string, Json>>>(entity.entity_id)).content.payload }
    ops.push({ op: 'entity.create', entity_id: id, type_id: entity.type_id, parent_id: parentId, order_key: key, ...(entity.placement ? { placement: entity.placement as unknown as Json } : {}), payload })
    for (const child of store.outline.childrenOf(entity.entity_id)) if (!child.deleted && child.order_key) await copyTree(child, id, child.order_key)
  }
  for (const control of controls) {
    order = store.core.order_key_between(order, undefined)
    await copyTree(control, created.surfaceId, order)
  }
  return { ops, surfaceId: created.surfaceId }
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

