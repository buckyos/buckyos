/* Display and input conversion of the value types of design §3.4.2 (table fields and record properties share them). */

import type { FieldType, Json, OptionDef } from '../api/types'

export interface ValueDef { type: FieldType; scale?: number; options?: OptionDef[]; required?: boolean; nullable?: boolean }

/** "Clear the cell": `table.unset_values` / `entity.unset_keys`. */
export const UNSET = Symbol('unset')
export type Input = Json | typeof UNSET

export function formatValue(def: ValueDef, value: Json | undefined): string {
  if (value === undefined) return ''
  if (value === null) return '（空）'
  switch (def.type) {
    case 'boolean': return value === true ? '是' : '否'
    case 'select': return def.options?.find((option) => option.option_id === value)?.label ?? `未知选项 ${String(value)}`
    case 'multi_select': {
      const ids = Array.isArray(value) ? value.map(String) : []
      // display order follows the field's options, not the stored (byte) order
      const known = (def.options ?? []).filter((option) => ids.includes(option.option_id)).map((option) => option.label)
      const unknown = ids.filter((id) => !(def.options ?? []).some((option) => option.option_id === id)).map((id) => `未知选项 ${id}`)
      return [...known, ...unknown].join('、')
    }
    case 'datetime': {
      const date = new Date(String(value))
      return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString()
    }
    case 'object_ref': return typeof value === 'object' && value && 'entity_id' in value ? `→ ${String((value as { entity_id: Json }).entity_id)}` : JSON.stringify(value)
    default: return typeof value === 'string' || typeof value === 'number' ? String(value) : JSON.stringify(value)
  }
}

/** Text the inline editor starts with. */
export function editText(def: ValueDef, value: Json | undefined): string {
  if (value === undefined || value === null) return ''
  if (def.type === 'datetime') {
    const date = new Date(String(value))
    if (Number.isNaN(date.getTime())) return ''
    const pad = (n: number) => String(n).padStart(2, '0')
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
  }
  return String(value)
}

/** Editor text → wire value. Only shape conversion happens here; validation and normalisation are the
 * backend's (the same rules as `normalize_value` of the core). Returns an error text for input that
 * cannot even be shaped (so nothing is sent and the input stays in the editor). */
export function parseInput(def: ValueDef, text: string): { ok: true; value: Input } | { ok: false; error: string } {
  if (text === '') return { ok: true, value: UNSET }
  switch (def.type) {
    case 'number': {
      const value = Number(text.trim())
      return Number.isFinite(value) && text.trim() !== '' ? { ok: true, value } : { ok: false, error: '不是有效的数字' }
    }
    case 'decimal': return { ok: true, value: text.trim() }
    case 'datetime': {
      const date = new Date(text)
      return Number.isNaN(date.getTime()) ? { ok: false, error: '不是有效的日期时间' } : { ok: true, value: date.toISOString() }
    }
    default: return { ok: true, value: text }
  }
}

export const FIELD_TYPE_LABEL: Record<FieldType, string> = {
  text: '文本', number: '数字', decimal: '定点小数', date: '日期', datetime: '日期时间', boolean: '布尔',
  select: '单选', multi_select: '多选', object_ref: '对象引用',
}

/** Filter operators per type (design §3.5.3 matrix). */
export function operatorsFor(type: FieldType): { id: string; label: string; needsValue: boolean }[] {
  const empty = [{ id: 'is_empty', label: '为空', needsValue: false }, { id: 'is_not_empty', label: '不为空', needsValue: false }]
  const eq = [{ id: 'eq', label: '等于', needsValue: true }, { id: 'ne', label: '不等于', needsValue: true }]
  const order = [{ id: 'lt', label: '小于', needsValue: true }, { id: 'lte', label: '不大于', needsValue: true }, { id: 'gt', label: '大于', needsValue: true }, { id: 'gte', label: '不小于', needsValue: true }]
  switch (type) {
    case 'text': return [...eq, { id: 'contains', label: '包含', needsValue: true }, { id: 'starts_with', label: '开头是', needsValue: true }, ...empty]
    case 'number': case 'decimal': case 'date': case 'datetime': return [...eq, ...order, ...empty]
    case 'boolean': case 'select': case 'object_ref': return [...eq, ...empty]
    case 'multi_select': return [{ id: 'has_any', label: '含任一', needsValue: true }, { id: 'has_all', label: '含全部', needsValue: true }, ...empty]
  }
}

export const SORTABLE: ReadonlySet<FieldType> = new Set<FieldType>(['text', 'number', 'decimal', 'date', 'datetime', 'boolean', 'select'])
