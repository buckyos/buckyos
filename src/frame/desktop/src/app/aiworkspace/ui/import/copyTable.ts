/* "Copy as table" (表格数据导入 TODO §4.1, T6): a table view's rows — its filter and order, up to 20 000 — as
 * tab-separated text (spreadsheets read it) and as an HTML table carrying aiws's own marks: the field definitions on
 * the table (`data-aiws-table`: names, types, scales, currencies, option labels) and each cell's wire value
 * (`data-aiws-value`, option labels instead of ids). Pasting it into a workspace prefills the wizard with exactly
 * those types and options. */

import type { FieldDef, Json, QueryRow } from '../../api/types'
import { formatValue, moneyParts } from '../values'

export const COPY_LIMIT = 20_000

const isFormula = (v: Json | undefined) => typeof v === 'object' && v !== null && !Array.isArray(v) && '$formula' in v
const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!)

/** A cell as spreadsheets take it: plain values (ISO dates, `CNY 500.00`), quoted when it holds a tab, quote or line break. */
function tsvCell(field: FieldDef, value: Json | undefined): string {
  if (value === undefined || value === null) return ''
  let text: string
  switch (field.type) {
    case 'boolean': text = value === true ? 'TRUE' : 'FALSE'; break
    case 'select': text = field.options?.find((o) => o.option_id === value)?.label ?? String(value); break
    case 'multi_select': text = (Array.isArray(value) ? value : []).map((id) => field.options?.find((o) => o.option_id === id)?.label ?? String(id)).join(', '); break
    case 'currency': { const m = moneyParts(value); text = m ? `${m.currency} ${m.amount}` : String(value); break }
    case 'object_ref': text = formatValue(field, value); break
    default: text = typeof value === 'string' || typeof value === 'number' ? String(value) : JSON.stringify(value)
  }
  return /[\t\n\r"]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
}

/** The wire value with option ids as labels (ids mean nothing in another table). */
function exactValue(field: FieldDef, value: Json): Json {
  if (field.type === 'select') return field.options?.find((o) => o.option_id === value)?.label ?? String(value)
  if (field.type === 'multi_select') return (Array.isArray(value) ? value : []).map((id) => field.options?.find((o) => o.option_id === id)?.label ?? String(id))
  // a reference means nothing in another workspace: its text
  if (field.type === 'object_ref') return formatValue(field, value)
  return value
}

export function tableClipboard(fields: FieldDef[], rows: QueryRow[]): { text: string; html: string } {
  const lines = [fields.map((f) => tsvCell({ ...f, type: 'text' }, f.name)).join('\t')]
  for (const row of rows) lines.push(fields.map((f) => (isFormula(row.values[f.field_id]) ? '' : tsvCell(f, row.values[f.field_id]))).join('\t'))
  const meta = { fields: fields.map((f) => ({ name: f.name, type: f.type === 'object_ref' ? 'text' : f.type, ...(f.scale !== undefined ? { scale: f.scale } : {}), ...(f.currency ? { currency: f.currency } : {}), ...(f.options ? { options: f.options.map((o) => o.label) } : {}) })) }
  const head = `<tr>${fields.map((f) => `<th>${escapeHtml(f.name)}</th>`).join('')}</tr>`
  const body = rows.map((row) => `<tr>${fields.map((f) => {
    const v = row.values[f.field_id]
    // a formula is the reader's to evaluate: not copied
    if (v === undefined || v === null || isFormula(v)) return '<td></td>'
    return `<td data-aiws-value="${escapeHtml(JSON.stringify(exactValue(f, v)))}">${escapeHtml(formatValue(f, v)).replace(/\n/g, '<br>')}</td>`
  }).join('')}</tr>`).join('')
  const html = `<meta charset="utf-8"><table data-aiws-table="${escapeHtml(JSON.stringify(meta))}"><thead>${head}</thead><tbody>${body}</tbody></table>`
  return { text: lines.join('\r\n') + '\r\n', html }
}
