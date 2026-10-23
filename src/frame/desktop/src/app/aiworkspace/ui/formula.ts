/* `$formula` values (内核架构改进 D18, T5-3): the kernel keeps them verbatim; this window evaluates them for display
 * and never writes the result back by itself. "Materialize" turns the shown value into a plain value with a
 * derived record naming the formula. The language: numbers, field names or ids in the row, + - * / and parentheses. */

import type { FieldDef, Json } from '../api/types'

/** The kernel's shape (core/src/value.rs `check_formula`): `{ $formula: { lang: "aiws", version, expr } }`. */
export interface FormulaValue { $formula: { lang: string; version: string; expr: string } }

export function isFormula(value: Json | undefined): boolean {
  const f = value && typeof value === 'object' && !Array.isArray(value) ? (value as { $formula?: { expr?: unknown } }).$formula : undefined
  return Boolean(f) && typeof f === 'object' && typeof f?.expr === 'string'
}

export function asFormula(value: Json | undefined): FormulaValue | null {
  return isFormula(value) ? value as unknown as FormulaValue : null
}

type Token = { kind: 'num'; value: number } | { kind: 'id'; name: string } | { kind: 'op'; op: string }

function tokenize(src: string): Token[] {
  const out: Token[] = []
  let i = 0
  while (i < src.length) {
    const c = src[i]
    if (/\s/.test(c)) { i += 1; continue }
    if (/[0-9.]/.test(c)) { const m = /^[0-9]*\.?[0-9]+(?:e[+-]?[0-9]+)?/i.exec(src.slice(i)); if (!m) throw new Error(`bad number at ${i}`); out.push({ kind: 'num', value: Number(m[0]) }); i += m[0].length; continue }
    if ('+-*/()'.includes(c)) { out.push({ kind: 'op', op: c }); i += 1; continue }
    if (c === '[') { const end = src.indexOf(']', i); if (end < 0) throw new Error('unclosed ['); out.push({ kind: 'id', name: src.slice(i + 1, end) }); i = end + 1; continue }
    const m = /^[\p{L}_][\p{L}\p{N}_.-]*/u.exec(src.slice(i))
    if (!m) throw new Error(`unexpected ${c}`)
    out.push({ kind: 'id', name: m[0] })
    i += m[0].length
  }
  return out
}

/** Evaluate `expr` over a row: identifiers resolve by field name, then by field id. Throws on a reference to a non-number. */
export function evaluateFormula(expr: string, values: Record<string, Json>, fields: readonly FieldDef[]): number {
  const tokens = tokenize(expr)
  let pos = 0
  const peek = () => tokens[pos]
  const take = () => tokens[pos++]
  const lookup = (name: string): number => {
    const field = fields.find((f) => f.name === name) ?? fields.find((f) => f.field_id === name)
    const key = field ? field.field_id : name
    const v = values[key]
    if (typeof v === 'number') return v
    if (typeof v === 'string' && v.trim() !== '' && !Number.isNaN(Number(v))) return Number(v)
    if (v === undefined || v === null) return 0
    throw new Error(`${name} is not a number`)
  }
  const primary = (): number => {
    const t = take()
    if (!t) throw new Error('unexpected end')
    if (t.kind === 'num') return t.value
    if (t.kind === 'id') return lookup(t.name)
    if (t.op === '(') { const v = expression(); const close = take(); if (!close || close.kind !== 'op' || close.op !== ')') throw new Error('expected )'); return v }
    if (t.op === '-') return -primary()
    if (t.op === '+') return primary()
    throw new Error(`unexpected ${t.op}`)
  }
  const term = (): number => {
    let v = primary()
    for (;;) {
      const t = peek()
      if (!t || t.kind !== 'op' || (t.op !== '*' && t.op !== '/')) return v
      take()
      const r = primary()
      v = t.op === '*' ? v * r : v / r
    }
  }
  const expression = (): number => {
    let v = term()
    for (;;) {
      const t = peek()
      if (!t || t.kind !== 'op' || (t.op !== '+' && t.op !== '-')) return v
      take()
      const r = term()
      v = t.op === '+' ? v + r : v - r
    }
  }
  const result = expression()
  if (pos !== tokens.length) throw new Error('trailing input')
  return result
}

/** The number a formula shows in this row, or null when it cannot be evaluated here. */
export function formulaResult(value: FormulaValue, values: Record<string, Json>, fields: readonly FieldDef[]): number | null {
  try {
    const n = evaluateFormula(value.$formula.expr, values, fields)
    return Number.isFinite(n) ? Math.round(n * 1e6) / 1e6 : null
  } catch {
    return null
  }
}
