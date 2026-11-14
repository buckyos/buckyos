/* The standard table editor's model (UI 框架 PRD §7.3): what a write of one value is, the columns a view shows, the
 * personal presentation, the conditions of a filter and the shape a value takes in an input control. Shared by the
 * grid, the cards, the record form and the field manager. */

import type { CapturedAnchor } from '../../anchors/registry'
import type { FieldDef, FilterNode, Json, Operation, QueryRow, SortSpec } from '../../api/types'
import { UNSET, editText, formatValue, operatorsFor, parseInput, type Input, type ValueDef } from '../values'

export type TableLayout = 'grid' | 'cards'
/** Where the editor sits (§7.1): a Block's box, a data-mode tab, a phone's whole screen. */
export type TableEditorSize = 'embedded' | 'tab' | 'fullscreen'

export const DEFAULT_WIDTH = 150
/** The three column widths a phone chooses from (§7.3, "列宽在字段管理里选窄、中、宽"). */
export const WIDTHS = { narrow: 100, medium: 150, wide: 240 } as const
export type WidthPreset = keyof typeof WIDTHS
export const MIN_WIDTH = 60
export const MAX_WIDTH = 640

export function widthPreset(width: number): WidthPreset | null {
  return (Object.keys(WIDTHS) as WidthPreset[]).find((key) => WIDTHS[key] === width) ?? null
}

export function cellEditId(sourceId: string, recordId: string, fieldId: string) {
  return `cell:${sourceId}:${recordId}:${fieldId}`
}

export function writeOperation(sourceId: string, field: FieldDef, recordId: string, value: Input, rev: number): Operation {
  return value === UNSET
    ? { op: 'table.unset_values', source_id: sourceId, values: [{ record_id: recordId, field_id: field.field_id, expect: { rev } }] }
    : { op: 'table.set_values', source_id: sourceId, field_type_revs: { [field.field_id]: field.type_rev }, values: [{ record_id: recordId, field_id: field.field_id, value, expect: { rev } }] }
}

/** The field a record is called by: the table's title field, else its first text field, else its first field. */
export function titleFieldOf(fields: readonly FieldDef[], titleFieldId: string | undefined): FieldDef | undefined {
  return fields.find((field) => field.field_id === titleFieldId) ?? fields.find((field) => field.type === 'text') ?? fields[0]
}

/** What a record is called: its title field's value, or `fallback`. */
export function recordTitle(titleField: FieldDef | undefined, row: QueryRow | undefined, fallback: string): string {
  const text = titleField && row ? formatValue(titleField, row.values[titleField.field_id]) : ''
  return text || fallback
}

/** What commenting on one cell anchors to. */
export function cellAnchor(sourceId: string, field: FieldDef, row: QueryRow): CapturedAnchor {
  const label = `${field.name} · ${row.record_id}`
  const exact = Array.from(formatValue(field, row.values[field.field_id])).slice(0, 200).join('')
  return {
    target: { entity_id: sourceId, selector: { kind: 'table_cell', record_id: row.record_id, field_id: field.field_id } },
    context: exact.trim() ? { quote: { exact }, label } : { label },
    label,
  }
}

/** A stable colour per option (eight soft tones). */
export function toneOf(id: string): string {
  let h = 0
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0
  return String(h % 8)
}

// ---- columns

export interface Column { field: FieldDef; width: number }
/** Every field in the order the view lists them, shown or not (the field manager's list). */
export interface ColumnEntry { field: FieldDef; shown: boolean; width: number }

/** A Block's explicit column list (`payload.fields`): listed = shown, in that order; none listed = every field. */
export function blockColumns(fields: readonly FieldDef[], listed: readonly { field_id: string; width?: number }[] | undefined): ColumnEntry[] {
  if (!listed || listed.length === 0) return fields.map((field) => ({ field, shown: true, width: DEFAULT_WIDTH }))
  const byId = new Map(fields.map((field) => [field.field_id, field]))
  const out: ColumnEntry[] = listed.flatMap((item) => { const field = byId.get(item.field_id); return field ? [{ field, shown: true, width: item.width ?? DEFAULT_WIDTH }] : [] })
  for (const field of fields) if (!listed.some((item) => item.field_id === field.field_id)) out.push({ field, shown: false, width: DEFAULT_WIDTH })
  return out
}

/** The table without a Block: this person's order, hidden fields and widths (user work state); new fields show at the end. */
export interface PersonalColumns { order?: string[]; hidden?: string[]; widths?: Record<string, number> }

export function personalColumns(fields: readonly FieldDef[], view: PersonalColumns): ColumnEntry[] {
  const order = view.order ?? []
  const rank = (id: string) => { const i = order.indexOf(id); return i < 0 ? order.length : i }
  return [...fields]
    .map((field, index) => ({ field, index }))
    .sort((a, b) => rank(a.field.field_id) - rank(b.field.field_id) || a.index - b.index)
    .map(({ field }) => ({ field, shown: !(view.hidden ?? []).includes(field.field_id), width: view.widths?.[field.field_id] ?? DEFAULT_WIDTH }))
}

/** The explicit list a Block stores for these entries. */
export function listedOf(entries: readonly ColumnEntry[]): { field_id: string; width?: number }[] {
  return entries.filter((entry) => entry.shown).map((entry) => (entry.width === DEFAULT_WIDTH ? { field_id: entry.field.field_id } : { field_id: entry.field.field_id, width: entry.width }))
}

export function personalOf(entries: readonly ColumnEntry[]): PersonalColumns {
  const widths: Record<string, number> = {}
  for (const entry of entries) if (entry.width !== DEFAULT_WIDTH) widths[entry.field.field_id] = entry.width
  return { order: entries.map((entry) => entry.field.field_id), hidden: entries.filter((entry) => !entry.shown).map((entry) => entry.field.field_id), widths }
}

export function moveEntry<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list]
  const [item] = next.splice(from, 1)
  next.splice(Math.max(0, Math.min(next.length, to)), 0, item)
  return next
}

// ---- conditions

export interface SessionView { filter: FilterNode | null; sorts: SortSpec[] | null }
export type Condition = Extract<FilterNode, { op: 'cmp' }>

/** The conditions of a filter combined by "and"; a filter of another shape is one opaque condition (null). */
export function conditionsOf(filter: FilterNode | null | undefined): (Condition | null)[] {
  if (!filter) return []
  if (filter.op === 'cmp') return [filter]
  if (filter.op === 'and') return filter.args.flatMap((arg) => (arg.op === 'and' ? conditionsOf(arg) : [arg.op === 'cmp' ? arg : null]))
  return [null]
}

export function filterOf(conditions: readonly FilterNode[]): FilterNode | null {
  if (conditions.length === 0) return null
  return conditions.length === 1 ? conditions[0] : { op: 'and', args: [...conditions] }
}

/** The filter without its `index`-th "and" condition. */
export function withoutCondition(filter: FilterNode, index: number): FilterNode | null {
  if (filter.op !== 'and') return null
  const flat: FilterNode[] = []
  const walk = (node: FilterNode) => { if (node.op === 'and') node.args.forEach(walk); else flat.push(node) }
  walk(filter)
  return filterOf(flat.filter((_, i) => i !== index))
}

// ---- a value in an input control

/** What a control holds: text for the typed kinds and single choices, the chosen ids for a multiple choice. */
export type Raw = string | string[]

export function rawOf(def: ValueDef, value: Json | undefined): Raw {
  if (def.type === 'multi_select') return Array.isArray(value) ? value.map(String) : []
  if (def.type === 'boolean') return value === true ? 'true' : value === false ? 'false' : ''
  if (def.type === 'select') return typeof value === 'string' ? value : ''
  return editText(def, value)
}

export function sameRaw(a: Raw, b: Raw): boolean {
  if (Array.isArray(a) || Array.isArray(b)) return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x) => b.includes(x))
  return a === b
}

/** Control → wire value (shape only, the core validates), or the text of an input that cannot even be shaped. */
export function inputOf(def: ValueDef, raw: Raw): { ok: true; value: Input } | { ok: false; error: string } {
  if (Array.isArray(raw)) return { ok: true, value: raw.length === 0 ? UNSET : raw }
  if (def.type === 'boolean') return { ok: true, value: raw === '' ? UNSET : raw === 'true' }
  if (def.type === 'select') return { ok: true, value: raw === '' ? UNSET : raw }
  return parseInput(def, raw)
}

export function isEmptyRaw(raw: Raw): boolean {
  return Array.isArray(raw) ? raw.length === 0 : raw.trim() === ''
}

/** One condition in words: `负责人 等于 林`, `预算 为空`. */
export function conditionText(fields: readonly FieldDef[], condition: Condition): string {
  const field = fields.find((item) => item.field_id === condition.field_id)
  const operator = field ? operatorsFor(field.type).find((item) => item.id === condition.operator) : undefined
  const value = condition.value === undefined ? '' : field
    ? (formatValue(field, condition.value))
    : JSON.stringify(condition.value)
  return [field?.name ?? condition.field_id, operator?.label ?? condition.operator, value].filter(Boolean).join(' ')
}

export function sortText(fields: readonly FieldDef[], sort: SortSpec, t: (key: string, vars?: Record<string, string | number>) => string): string {
  const name = fields.find((item) => item.field_id === sort.field_id)?.name ?? sort.field_id
  return t(sort.direction === 'desc' ? 'aiworkspace.table.chip.sortDesc' : 'aiworkspace.table.chip.sortAsc', { field: name })
}
