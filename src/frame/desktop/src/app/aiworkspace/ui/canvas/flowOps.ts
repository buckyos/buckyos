/* Structure of a flow page (流式页交互改进 §2): which children it shows, their reading order, moving one place
 * up or down, deleting with the subtree. */

import { isControlKind } from '@controls/contract'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import { orderKeyBetween } from '../../api/wasm'
import type { WorkspaceStore } from '../../state/store'

/** The Blocks, groups and controls a flow container shows (connectors keep their data but are not drawn, 连接线实现方案
 * §8.3; a control is one self-contained Block, 脑图方案 §6.3). */
export function flowChildren(store: WorkspaceStore, containerId: string): EntityEnvelope[] {
  return store.outline.childrenOf(containerId).filter((entity) => (entity.type_id === 'buckyos.cell' && entity.view_type !== 'connector') || (entity.type_id === 'buckyos.container' && (entity.kind === 'group' || isControlKind(entity.kind))))
}

/** Every Block and group of the page in reading order (keyboard navigation). */
export function flowOrder(store: WorkspaceStore, surfaceId: string): string[] {
  const out: string[] = []
  const walk = (id: string) => {
    for (const child of flowChildren(store, id)) {
      out.push(child.entity_id)
      if (child.type_id === 'buckyos.container' && child.kind === 'group') walk(child.entity_id)
    }
  }
  walk(surfaceId)
  return out
}

/** Move a Block one place up or down among the Blocks of its container. */
export function moveInFlow(store: WorkspaceStore, entity: EntityEnvelope, direction: -1 | 1) {
  if (!entity.capabilities.includes('structure') || !entity.parent_id) return
  const shown = flowChildren(store, entity.parent_id)
  const index = shown.findIndex((item) => item.entity_id === entity.entity_id)
  const target = shown[index + direction]
  if (!target) return
  const all = store.outline.childrenOf(entity.parent_id)
  const at = all.findIndex((item) => item.entity_id === target.entity_id)
  const before = direction === -1 ? all[at - 1]?.order_key : all[at]?.order_key
  const after = direction === -1 ? all[at]?.order_key : all[at + 1]?.order_key
  let key: string
  try { key = orderKeyBetween(store.core, before, after) } catch (error) { store.notify('error', aiwsT('aiworkspace.canvas.flow.orderKeyFailed', { reason: describeError(error) })); return }
  void store.submit({ editId: `entity:${entity.entity_id}`, label: aiwsT('aiworkspace.canvas.flow.reorderLabel', { name: entity.title ?? entity.name ?? entity.entity_id }), operations: [{ op: 'tree.place', entity_id: entity.entity_id, order_key: key }] })
}

export function deleteOps(store: WorkspaceStore, entity: EntityEnvelope): Operation[] {
  const subtree = entity.type_id === 'buckyos.container' ? store.outline.descendants(entity.entity_id).map((d) => d.entity_id) : []
  return [{ op: 'entity.delete', entity_id: entity.entity_id, ...(subtree.length ? { subtree: { delete: subtree } } : {}), expect: { rev: entity.life_rev } }]
}
