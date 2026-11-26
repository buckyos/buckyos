/* "Copy as table" (表格数据导入 TODO §4.1, T6; 剪贴板规范 §2.2): a table view's rows — its filter and order, up to
 * 20 000 — as the BuckyOS typed table: an HTML table carrying the field definitions (`data-buckyos-table`: names,
 * types, scales, currencies, option labels) and each cell's wire value (`data-buckyos-value`: option labels instead of
 * ids), with the tab-separated text spreadsheets read. A reference means nothing outside its workspace (its text is
 * copied as a text field); a formula is the reader's to evaluate (its cell stays empty). Pasting it into a workspace
 * prefills the wizard with exactly those types and options. */

import { typedTableRepresentations, type TableFieldType, type TypedTableCell, type TypedTableField } from 'buckyos/transfer'
import type { FieldDef, Json, QueryRow } from '../../api/types'
import { formatValue, moneyParts } from '../values'

export const COPY_LIMIT = 20_000

const isFormula = (v: Json | undefined) => typeof v === 'object' && v !== null && !Array.isArray(v) && '$formula' in v

function typedField(field: FieldDef): TypedTableField {
  const type: TableFieldType = field.type === 'object_ref' ? 'text' : field.type
  return {
    name: field.name, type,
    ...(field.scale !== undefined && (type === 'decimal' || type === 'currency') ? { scale: field.scale } : {}),
    ...(field.currency && type === 'currency' ? { currency: field.currency } : {}),
    ...(field.options && (type === 'select' || type === 'multi_select') ? { options: field.options.map((o) => o.label) } : {}),
  }
}

/** The wire value with option ids as labels (ids mean nothing in another table), a reference as its text. */
function wireValue(field: FieldDef, value: Json): unknown {
  switch (field.type) {
    case 'select': return field.options?.find((o) => o.option_id === value)?.label ?? String(value)
    case 'multi_select': return (Array.isArray(value) ? value : []).map((id) => field.options?.find((o) => o.option_id === id)?.label ?? String(id))
    case 'object_ref': return formatValue(field, value)
    case 'currency': return moneyParts(value) ?? undefined
    default: return value
  }
}

function typedCell(field: FieldDef, value: Json | undefined): TypedTableCell {
  if (value === undefined || value === null || isFormula(value)) return {}
  const wire = wireValue(field, value)
  return { ...(wire !== undefined ? { value: wire } : {}), text: formatValue(field, value) }
}

export function tableClipboard(fields: FieldDef[], rows: QueryRow[]): { text: string; html: string } {
  const typed = typedTableRepresentations({ fields: fields.map(typedField), rows: rows.map((row) => fields.map((f) => typedCell(f, row.values[f.field_id]))) })
  return { text: typed['text/plain'], html: typed['text/html'] }
}
