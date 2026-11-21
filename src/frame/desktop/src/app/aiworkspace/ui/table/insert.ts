import type { FieldDef, Json, Operation } from '../../api/types'
import type { AiwsCore } from '../../api/wasm'

export function missingRecordFields(core: AiwsCore, fields: readonly FieldDef[], schemaId?: string | null, values: Record<string, Json> = {}): string[] {
  return JSON.parse(core.missing_record_fields(JSON.stringify(fields), schemaId ?? undefined, JSON.stringify(values))) as string[]
}

export function insertRecordOp(sourceId: string, fields: readonly FieldDef[], recordId: string, draft: Record<string, Json> = {}): Operation {
  const values: Record<string, Json> = {}
  const revs: Record<string, number> = {}
  for (const field of fields) {
    if (field.submitter) continue
    const id = field.field_id
    if (Object.hasOwn(draft, id)) values[id] = draft[id]
    else if (field.required && field.nullable && field.default === undefined) values[id] = null
    else continue
    revs[id] = field.type_rev
  }
  return { op: 'table.insert_records', source_id: sourceId, field_type_revs: revs, records: [{ record_id: recordId, values }] }
}
