/* Display and input conversion of the value types of design §3.4.2 (table fields and record properties share them). */

import { parseNumber, resolveAmbiguous, resolveMark, splitMoney } from '@table/mod'
import type { FieldType, Json, OptionDef } from '../api/types'
import { aiwsT, aiwsText, localizedLabels } from '../i18n'

export interface ValueDef { type: FieldType; scale?: number; options?: OptionDef[]; required?: boolean; nullable?: boolean; currency?: string }

/** A stored currency value (`{ amount, currency }`, 表格数据导入 TODO §6.1). */
export function moneyParts(value: Json | undefined): { amount: string; currency: string } | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const { amount, currency } = value as { amount?: Json; currency?: Json }
  return typeof amount === 'string' && typeof currency === 'string' ? { amount, currency } : null
}

/** An amount in the reader's language with its currency's symbol (`¥500.00`, `US$12.00`), exact (no binary rounding). */
export function formatMoney(amount: string, currency: string, scale?: number): string {
  const digits = scale ?? amount.split('.')[1]?.length ?? 2
  try {
    const format = new Intl.NumberFormat(aiwsText().locale, { style: 'currency', currency, minimumFractionDigits: digits, maximumFractionDigits: digits })
    // a decimal string is formatted exactly (Intl.NumberFormat v3)
    return format.format(amount as unknown as number)
  } catch {
    return `${currency} ${amount}`
  }
}

/** The common currencies the field manager offers (any other ISO 4217 code can be typed). */
export const COMMON_CURRENCIES = ['CNY', 'USD', 'EUR', 'JPY', 'GBP', 'HKD', 'TWD', 'KRW', 'SGD', 'AUD', 'CAD', 'CHF', 'INR', 'RUB', 'BRL', 'MXN', 'THB', 'VND', 'MYR', 'IDR', 'PHP', 'NZD', 'SEK', 'NOK', 'DKK', 'PLN', 'TRY', 'ZAR', 'AED', 'SAR']

/** "Clear the cell": `table.unset_values` / `entity.unset_keys`. */
export const UNSET = Symbol('unset')
export type Input = Json | typeof UNSET

export function formatValue(def: ValueDef, value: Json | undefined): string {
  if (value === undefined) return ''
  if (value === null) return aiwsT('aiworkspace.data.value.empty')
  switch (def.type) {
    case 'boolean': return value === true ? aiwsT('aiworkspace.common.yes') : aiwsT('aiworkspace.common.no')
    case 'select': return def.options?.find((option) => option.option_id === value)?.label ?? aiwsT('aiworkspace.data.value.unknownOption', { id: String(value) })
    case 'multi_select': {
      const ids = Array.isArray(value) ? value.map(String) : []
      // display order follows the field's options, not the stored (byte) order
      const known = (def.options ?? []).filter((option) => ids.includes(option.option_id)).map((option) => option.label)
      const unknown = ids.filter((id) => !(def.options ?? []).some((option) => option.option_id === id)).map((id) => aiwsT('aiworkspace.data.value.unknownOption', { id }))
      return aiwsText().list([...known, ...unknown])
    }
    case 'datetime': {
      const date = new Date(String(value))
      return Number.isNaN(date.getTime()) ? String(value) : aiwsText().dateTime(date)
    }
    case 'object_ref': return typeof value === 'object' && value && 'entity_id' in value ? `→ ${String((value as { entity_id: Json }).entity_id)}` : JSON.stringify(value)
    case 'currency': {
      const money = moneyParts(value)
      return money ? formatMoney(money.amount, money.currency) : typeof value === 'string' || typeof value === 'number' ? String(value) : JSON.stringify(value)
    }
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
  if (def.type === 'currency') {
    // the field's own currency as a bare amount, any other with its code
    const money = moneyParts(value)
    if (money) return money.currency === def.currency ? money.amount : `${money.currency} ${money.amount}`
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
      return Number.isFinite(value) && text.trim() !== '' ? { ok: true, value } : { ok: false, error: aiwsT('aiworkspace.data.value.invalidNumber') }
    }
    case 'decimal': return { ok: true, value: text.trim() }
    case 'currency': {
      // `<code | symbol | name>? amount <code | name>?`: a mark gives the currency, none the field's default
      const { prefix, amount, suffix } = splitMoney(text)
      const number = parseNumber(amount)
      const marks = [prefix, suffix].filter(Boolean).map((m) => resolveMark(m))
      if (!number || number.percent || marks.some((m) => m === null)) return { ok: false, error: aiwsT('aiworkspace.data.value.invalidMoney') }
      const mark = marks[0]
      if (!mark) return { ok: true, value: number.digits }
      const locale = aiwsText().locale
      const currency = 'code' in mark ? mark.code : resolveAmbiguous(mark.ambiguous, def.currency ?? null, locale)
      return { ok: true, value: { amount: number.digits, currency } }
    }
    case 'datetime': {
      const date = new Date(text)
      return Number.isNaN(date.getTime()) ? { ok: false, error: aiwsT('aiworkspace.data.value.invalidDateTime') } : { ok: true, value: date.toISOString() }
    }
    default: return { ok: true, value: text }
  }
}

export const FIELD_TYPE_LABEL: Readonly<Record<FieldType, string>> = localizedLabels({
  text: 'aiworkspace.data.fieldType.text', number: 'aiworkspace.data.fieldType.number', decimal: 'aiworkspace.data.fieldType.decimal',
  date: 'aiworkspace.data.fieldType.date', datetime: 'aiworkspace.data.fieldType.datetime', boolean: 'aiworkspace.data.fieldType.boolean',
  select: 'aiworkspace.data.fieldType.select', multi_select: 'aiworkspace.data.fieldType.multiSelect', object_ref: 'aiworkspace.data.fieldType.objectRef',
  currency: 'aiworkspace.data.fieldType.currency',
})

/** Filter operators per type (design §3.5.3 matrix). */
export function operatorsFor(type: FieldType): { id: string; label: string; needsValue: boolean }[] {
  const empty = [{ id: 'is_empty', label: aiwsT('aiworkspace.data.operator.isEmpty'), needsValue: false }, { id: 'is_not_empty', label: aiwsT('aiworkspace.data.operator.isNotEmpty'), needsValue: false }]
  const eq = [{ id: 'eq', label: aiwsT('aiworkspace.data.operator.eq'), needsValue: true }, { id: 'ne', label: aiwsT('aiworkspace.data.operator.ne'), needsValue: true }]
  const order = [
    { id: 'lt', label: aiwsT('aiworkspace.data.operator.lt'), needsValue: true }, { id: 'lte', label: aiwsT('aiworkspace.data.operator.lte'), needsValue: true },
    { id: 'gt', label: aiwsT('aiworkspace.data.operator.gt'), needsValue: true }, { id: 'gte', label: aiwsT('aiworkspace.data.operator.gte'), needsValue: true },
  ]
  switch (type) {
    case 'text': return [...eq, { id: 'contains', label: aiwsT('aiworkspace.data.operator.contains'), needsValue: true }, { id: 'starts_with', label: aiwsT('aiworkspace.data.operator.startsWith'), needsValue: true }, ...empty]
    case 'number': case 'decimal': case 'currency': case 'date': case 'datetime': return [...eq, ...order, ...empty]
    case 'boolean': case 'select': case 'object_ref': return [...eq, ...empty]
    case 'multi_select': return [{ id: 'has_any', label: aiwsT('aiworkspace.data.operator.hasAny'), needsValue: true }, { id: 'has_all', label: aiwsT('aiworkspace.data.operator.hasAll'), needsValue: true }, ...empty]
  }
}

export const SORTABLE: ReadonlySet<FieldType> = new Set<FieldType>(['text', 'number', 'decimal', 'currency', 'date', 'datetime', 'boolean', 'select'])
