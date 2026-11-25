/* Field operations shared by the field manager and the table's own "+" column (流式页交互改进 §5). */

import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import { aiwsT } from '../i18n'
import type { AiwsCore } from '../api/wasm'
import type { CellPayload, FieldDef, FieldType, Json, KeyedContent, Operation } from '../api/types'

export type CreatableField = FieldType | 'image' | 'canvas'

export const CREATABLE_FIELD_TYPES: CreatableField[] = ['text', 'number', 'decimal', 'currency', 'date', 'datetime', 'boolean', 'select', 'multi_select', 'json', 'image', 'canvas']

function columnName(index: number): string {
  let letters = ''
  for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) {
    letters = String.fromCharCode(65 + (n - 1) % 26) + letters
  }
  return aiwsT('aiworkspace.table.column.defaultName', { letters })
}

export function ordinaryTableOps(sourceId: string, parentId: string, orderKey: string, name?: string): Operation[] {
  const fields = Array.from({ length: 3 }, (_, i) => ({ field_id: randomId('f'), name: columnName(i), type: 'text' }))
  return [
    { op: 'entity.create', entity_id: sourceId, parent_id: parentId, order_key: orderKey, type_id: 'buckyos.table-source',
      ...(name ? { name } : {}), payload: { title_field_id: fields[0].field_id, fields } },
    { op: 'table.insert_records', source_id: sourceId, records: Array.from({ length: 2 }, () => ({ record_id: randomId('r'), values: {} })) },
  ]
}

type Side = 'before' | 'after'

function insertionIndex(order: readonly string[], anchorId?: string, side: Side = 'after') {
  const index = anchorId === undefined ? -1 : order.indexOf(anchorId)
  return index < 0 ? order.length : index + (side === 'after' ? 1 : 0)
}

export function insertColumnId(order: readonly string[], fieldId: string, anchorId?: string, side?: Side): string[] {
  const next = [...order]
  next.splice(insertionIndex(order, anchorId, side), 0, fieldId)
  return next
}

export function quickColumnOps(core: AiwsCore, sourceId: string, cell: ReadOk<KeyedContent<CellPayload>>, fields: readonly FieldDef[], anchorId?: string, side?: Side) {
  const existing = new Set(fields.map((field) => field.name))
  let index = 0
  let name = columnName(index)
  while (existing.has(name)) name = columnName(++index)
  const fieldId = randomId('f')
  const position = insertionIndex(fields.map((f) => f.field_id), anchorId, side)
  const orderKey = core.order_key_between(fields[position - 1]?.order_key, fields[position]?.order_key)
  const operations = addFieldOps(sourceId, cell, name, 'text', 2, 'USD', { fieldId, orderKey, anchorId, side })
  return { fieldId, operations }
}

/** A new field, and — when the view lists its columns explicitly — the field added to this view in the same commit. */
export function addFieldOps(sourceId: string, cell: ReadOk<KeyedContent<CellPayload>>, name: string, type: CreatableField, scale = 2, currency = 'USD', position?: { fieldId: string; orderKey: string; anchorId?: string; side?: Side }): Operation[] {
  const fieldId = position?.fieldId ?? randomId('f')
  const field: Record<string, Json> = { field_id: fieldId, name, type }
  if (type === 'image' || type === 'canvas') {
    field.type = 'object_ref'
    field.ref_purpose = type
    field.target_types = [type === 'image' ? 'buckyos.asset-ref' : 'buckyos.container']
  }
  if (type === 'decimal') field.scale = scale
  // a currency field: its default currency, and the amount's scale when not the usual 2
  if (type === 'currency') { field.currency = currency; if (scale !== 2) field.scale = scale }
  if (type === 'select' || type === 'multi_select') field.options = []
  const operations: Operation[] = [{ op: 'table.add_field', source_id: sourceId, field, ...(position ? { order_key: position.orderKey } : {}) }]
  const viewFields = cell.content.payload.fields
  if (viewFields && viewFields.length > 0 && !cell.entity_id.startsWith('source:')) {
    const listed = [...viewFields]
    listed.splice(insertionIndex(viewFields.map((entry) => entry.field_id), position?.anchorId, position?.side), 0, { field_id: fieldId })
    operations.push({ op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'fields', value: listed as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }] })
  }
  return operations
}
