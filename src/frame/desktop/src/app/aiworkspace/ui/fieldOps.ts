/* Field operations shared by the field manager and the table's own "+" column (流式页交互改进 §5). */

import { randomId } from '../api/ids'
import type { ReadOk } from '../api/session'
import type { CellPayload, FieldType, Json, KeyedContent, Operation } from '../api/types'

export const CREATABLE_FIELD_TYPES: FieldType[] = ['text', 'number', 'decimal', 'date', 'datetime', 'boolean', 'select', 'multi_select']

/** A new field, and — when the view lists its columns explicitly — the field added to this view in the same commit. */
export function addFieldOps(sourceId: string, cell: ReadOk<KeyedContent<CellPayload>>, name: string, type: FieldType, scale = 2): Operation[] {
  const fieldId = randomId('f')
  const field: Record<string, Json> = { field_id: fieldId, name, type }
  if (type === 'decimal') field.scale = scale
  if (type === 'select' || type === 'multi_select') field.options = []
  const operations: Operation[] = [{ op: 'table.add_field', source_id: sourceId, field }]
  const viewFields = cell.content.payload.fields
  if (viewFields && viewFields.length > 0 && !cell.entity_id.startsWith('source:')) {
    operations.push({ op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'fields', value: [...viewFields, { field_id: fieldId }] as unknown as Json, expect: { rev: cell.content.key_revs.fields ?? 0 } }] })
  }
  return operations
}
