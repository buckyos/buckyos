/* Shared helpers around entities (labels, sibling order, keys) and the annotation operation. Block
 * creation moved into the Block definitions (ui/blocks); data creation into the data tree (ui/sources). */
import type { CapturedAnchor } from '../anchors/registry'
import { randomId } from '../api/ids'
import type { EntityEnvelope, Json, Operation, Reference } from '../api/types'
import { orderKeyBetween, type AiwsCore } from '../api/wasm'

export function entityLabel(entity: EntityEnvelope): string {
  return entity.name ?? entity.title ?? entity.entity_id
}

export function sortedChildren(entities: EntityEnvelope[], parentId: string): EntityEnvelope[] {
  return entities
    .filter((entity) => entity.parent_id === parentId && !entity.deleted)
    .sort((a, b) => {
      const ka = a.order_key ?? ''
      const kb = b.order_key ?? ''
      return ka < kb ? -1 : ka > kb ? 1 : a.entity_id < b.entity_id ? -1 : a.entity_id > b.entity_id ? 1 : 0
    })
}

export function descendants(entities: EntityEnvelope[], entityId: string): string[] {
  const out: string[] = []
  const walk = (id: string) => {
    for (const child of entities) {
      if (child.parent_id === id && !child.deleted) { out.push(child.entity_id); walk(child.entity_id) }
    }
  }
  walk(entityId)
  return out
}

/** Keys for `count` new children appended after the last child of `parentId`. */
export function appendKeys(core: AiwsCore, entities: EntityEnvelope[], parentId: string, count: number): string[] {
  const siblings = sortedChildren(entities, parentId)
  let last = siblings.length > 0 ? siblings[siblings.length - 1].order_key ?? null : null
  const keys: string[] = []
  for (let i = 0; i < count; i++) {
    last = orderKeyBetween(core, last, null)
    keys.push(last)
  }
  return keys
}

/** An annotation placed under `parentId` (a folder of the data tree, phase two §4.5), anchored by `anchor`. */
export function annotationOp(core: AiwsCore, siblings: EntityEnvelope[], parentId: string, anchor: CapturedAnchor, body: string): Operation {
  const last = siblings.length > 0 ? siblings[siblings.length - 1].order_key ?? null : null
  const key = orderKeyBetween(core, last, null)
  const payload: Record<string, Json> = { target: anchor.target as unknown as Json, kind: 'note', body }
  if (anchor.range) payload.range = anchor.range
  if (anchor.context) payload.context = anchor.context as unknown as Json
  return { op: 'entity.create', entity_id: randomId('n'), type_id: 'buckyos.annotation', parent_id: parentId, order_key: key, payload }
}

export function describeTarget(target: Reference): string {
  const selector = target.selector
  if (!selector || selector.kind === 'entity') return target.entity_id
  if (selector.kind === 'table_cell') return `${target.entity_id} / ${selector.record_id} / ${selector.field_id}`
  if (selector.kind === 'table_record') return `${target.entity_id} / ${selector.record_id}`
  if (selector.kind === 'table_field') return `${target.entity_id} / 字段 ${selector.field_id}`
  if (selector.kind === 'richtext_block') return `${target.entity_id} / 块 ${selector.block_id}`
  return `${target.entity_id} / ${selector.kind}`
}
