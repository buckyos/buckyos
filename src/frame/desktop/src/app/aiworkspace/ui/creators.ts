/* Operations that create the built-in objects (design §3). A content entity and the cell that shows
 * it are created in one commit, so there is never a cell without a source or the reverse. */

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

function create(entityId: string, typeId: string, parentId: string, orderKey: string, payload: Record<string, Json>, name?: string): Operation {
  return { op: 'entity.create', entity_id: entityId, type_id: typeId, parent_id: parentId, order_key: orderKey, ...(name ? { name } : {}), payload }
}

function cell(parentId: string, orderKey: string, sourceId: string, view: string, title?: string): Operation {
  return create(randomId('c'), 'buckyos.cell', parentId, orderKey, { source_ref: { entity_id: sourceId }, view: { type: view }, ...(title ? { title } : {}) })
}

export type NewKind = 'group' | 'richtext' | 'table' | 'record' | 'view'

export function creationOps(core: AiwsCore, entities: EntityEnvelope[], parentId: string, kind: NewKind, title: string, existingSourceId?: string): Operation[] {
  const [first, second] = appendKeys(core, entities, parentId, 2)
  switch (kind) {
    case 'group':
      return [create(randomId('g'), 'buckyos.container', parentId, first, { kind: 'group', layout: { mode: 'flow' }, title })]
    case 'richtext': {
      const id = randomId('t')
      const content = { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') } }] }
      return [create(id, 'buckyos.richtext', parentId, first, { content }), cell(parentId, second, id, 'richtext', title)]
    }
    case 'table': {
      const id = randomId('s')
      const fields: Json = [
        { field_id: 'title', name: '标题', type: 'text', required: true },
        { field_id: 'done', name: '完成', type: 'boolean' },
      ]
      return [create(id, 'buckyos.table-source', parentId, first, { title_field_id: 'title', fields }), cell(parentId, second, id, 'table', title)]
    }
    case 'record': {
      const id = randomId('o')
      const schema: Json = { properties: [{ key: 'name', name: '名称', type: 'text' }, { key: 'date', name: '日期', type: 'date' }, { key: 'amount', name: '金额', type: 'decimal', scale: 2 }] }
      return [create(id, 'buckyos.record', parentId, first, { schema, props: {} }), cell(parentId, second, id, 'record', title)]
    }
    case 'view':
      return existingSourceId ? [cell(parentId, first, existingSourceId, 'table', title)] : []
  }
}

export function assetOps(core: AiwsCore, entities: EntityEnvelope[], parentId: string, objectId: string, fileName: string): Operation[] {
  const [first, second] = appendKeys(core, entities, parentId, 2)
  const id = randomId('a')
  return [
    create(id, 'buckyos.asset-ref', parentId, first, { object_id: objectId, file_name: fileName }),
    { ...cell(parentId, second, id, 'asset'), payload: { source_ref: { entity_id: id }, view: { type: 'asset' }, options: { fit: 'contain' } } },
  ]
}

export function annotationOp(core: AiwsCore, entities: EntityEnvelope[], pageId: string, anchor: CapturedAnchor, body: string): Operation {
  const [key] = appendKeys(core, entities, pageId, 1)
  const payload: Record<string, Json> = { target: anchor.target as unknown as Json, kind: 'note', body }
  if (anchor.range) payload.range = anchor.range
  if (anchor.context) payload.context = anchor.context as unknown as Json
  return create(randomId('n'), 'buckyos.annotation', pageId, key, payload)
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
