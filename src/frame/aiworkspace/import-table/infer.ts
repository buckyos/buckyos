/* Column types and value shaping (表格数据导入 TODO §3.3, §6.3). A column gets the first type of boolean → number →
 * decimal → date → datetime → currency → select → text that every one of its non-empty values passes; the shaper of
 * that type then turns each cell into the wire value the core stores (canonical decimals, `YYYY-MM-DD`, RFC 3339 UTC,
 * `{ amount, currency }`, option ids). Inference is lossless by construction — a value that does not fit makes the
 * column text — so loss only happens when the user picks another type in the wizard, where each cell that does not
 * fit is counted and reported. Raw values and hints from typed sources (HTML raw values, xlsx formats, SQLite types)
 * win over the display text. */

import type { Cell, ColumnHint } from './grid.ts'
import { languageCurrency, resolveAmbiguous, resolveMark, splitMoney, type Ambiguous } from './currency.ts'

export type ImportType = 'text' | 'boolean' | 'number' | 'decimal' | 'date' | 'datetime' | 'currency' | 'select' | 'multi_select'
export const IMPORT_TYPES: ImportType[] = ['text', 'number', 'decimal', 'currency', 'date', 'datetime', 'boolean', 'select', 'multi_select']

export type DateOrder = 'md' | 'dm'
export type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

/** Text values longer than this (UTF-16 units) do not fit a text field. */
export const TEXT_MAX = 65_536
/** Decimal places kept as a decimal; more: a number (a double), reported. */
export const MAX_SCALE = 6
const MAX_SAFE = 9007199254740991n
/** Select inference (Q5): at least 5 values, at most 20 distinct ones and half the values, each ≤ 64 characters. */
const SELECT_MIN_VALUES = 5
const SELECT_MAX_OPTIONS = 20
const SELECT_MAX_LABEL = 64
/** Options a column changed to select by hand may get. */
export const SELECT_OPTION_LIMIT = 200

// ---- numbers

export interface NumberText { /** Canonical plain digits, `-?\d+(\.\d+)?`, fraction as written. */ digits: string; frac: number; percent: boolean }

/** `1,234.50`, `-12`, `+3.5e2`, `12.5%` → plain decimal text; percentages stay as written (the shaper divides). */
export function parseNumber(text: string): NumberText | null {
  let s = text.trim().replace(/ /g, ' ')
  if (!s) return null
  let percent = false
  if (s.endsWith('%') || s.endsWith('％')) { percent = true; s = s.slice(0, -1).trimEnd() } // i18n-exempt: input vocabulary
  const m = /^([+\-−]?)\s*(\d{1,3}(?:,\d{3})+|\d+|(?=\.\d))(?:\.(\d+))?(?:[eE]([+-]?\d{1,4}))?$/.exec(s)
  if (!m) return null
  const neg = m[1] === '-' || m[1] === '−'
  const digits = shift(m[2].replace(/,/g, '') || '0', m[3] ?? '', m[4] ? parseInt(m[4], 10) : 0)
  if (!digits) return null
  const out = (neg && !/^0(\.0+)?$/.test(digits.text) ? '-' : '') + digits.text
  return { digits: out, frac: digits.frac, percent }
}

/** int.frac × 10^exp as plain text, the fraction kept as written (exponents move it). */
function shift(int: string, frac: string, exp: number): { text: string; frac: number } | null {
  if (Math.abs(exp) > 400) return null
  let i = int, f = frac
  if (exp > 0) { const take = f.slice(0, exp); i += take.padEnd(exp, '0'); f = f.slice(exp) }
  else if (exp < 0) { const n = -exp; const head = i.padStart(n + 1, '0'); i = head.slice(0, head.length - n); f = head.slice(head.length - n) + f }
  i = i.replace(/^0+(?=\d)/, '')
  return { text: f ? `${i}.${f}` : i, frac: f.length }
}

/** A JS number as plain text (no exponent). */
export function numberDigits(n: number): NumberText | null {
  if (!Number.isFinite(n)) return null
  const s = String(Number(n.toPrecision(15)))
  return parseNumber(s)
}

/** Divide plain decimal text by 100 (a percentage as a ratio). */
function percentRatio(n: NumberText): NumberText {
  const neg = n.digits.startsWith('-')
  const [int, frac = ''] = (neg ? n.digits.slice(1) : n.digits).split('.')
  const r = shift(int, frac, -2)!
  return { digits: (neg ? '-' : '') + r.text, frac: r.frac, percent: true }
}

/** A cell's number as the value to store: text percentages divided by 100, a source's ratio as it is. */
const asStored = (c: Cell, n: NumberText) => (n.percent && c.hint !== 'percent' ? percentRatio(n) : n)

/** The core's canonical decimal (design §3.4.2): no redundant zeros in front, exactly `scale` places, at most 38
 * significant digits, never rounded. Null when it does not fit. */
export function canonicalDecimal(digits: string, scale: number): string | null {
  const neg = digits.startsWith('-')
  const [int, frac = ''] = (neg ? digits.slice(1) : digits).split('.')
  const f = frac.replace(/0+$/, '')
  if (f.length > scale) return null
  const i = int.replace(/^0+/, '')
  if (i.length + f.length > 38) return null
  const zero = i === '' && f === ''
  return `${neg && !zero ? '-' : ''}${i || '0'}${scale > 0 ? `.${f.padEnd(scale, '0')}` : ''}`
}

const isInteger = (n: NumberText) => !/\.\d*[1-9]/.test(n.digits) && n.frac === 0
const withinSafe = (digits: string) => { const d = digits.split('.')[0]; return BigInt(d.replace(/^-/, '') || '0') <= MAX_SAFE }

/** A cell's number. `strict` (inference): a source's money is money, not a number; shaping takes its amount. */
function numericOf(c: Cell, strict = true): NumberText | null {
  if (strict && c.hint === 'currency') return null
  if (typeof c.raw === 'number') {
    const n = numberDigits(c.raw)
    return n ? { ...n, percent: c.hint === 'percent' } : null
  }
  if (typeof c.raw === 'string' && /^-?\d+$/.test(c.raw)) return { digits: c.raw, frac: 0, percent: false }
  if (typeof c.exact === 'number') return numberDigits(c.exact)
  if (typeof c.exact === 'string') return parseNumber(c.exact)
  if (c.hint === 'text' || c.hint === 'bool') return null
  const n = parseNumber(c.text)
  if (n || strict) return n
  const m = moneyOf(c)
  return m ? { digits: m.digits, frac: m.frac, percent: false } : null
}

// ---- booleans

// i18n-exempt: input vocabulary
const TRUE_WORDS = new Set(['true', 'yes', 'y', '是', '真', '✓', '✔', '☑'])
const FALSE_WORDS = new Set(['false', 'no', 'n', '否', '假', '×', '✗']) // i18n-exempt: input vocabulary

function booleanOf(c: Cell): boolean | null {
  if (typeof c.raw === 'boolean') return c.raw
  if (typeof c.exact === 'boolean') return c.exact
  const t = c.text.trim().toLowerCase()
  if (TRUE_WORDS.has(t)) return true
  if (FALSE_WORDS.has(t)) return false
  return null
}

// ---- dates and times

const MONTHS: Record<string, number> = { jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6, jul: 7, aug: 8, sep: 9, sept: 9, oct: 10, nov: 11, dec: 12 }
const monthOf = (name: string) => MONTHS[name.toLowerCase().slice(0, name.toLowerCase().startsWith('sept') ? 4 : 3)] ?? (MONTHS[name.toLowerCase()] ?? 0)
const FULL_MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december']
const isMonthName = (s: string) => { const l = s.toLowerCase().replace(/\.$/, ''); return l.length >= 3 && (FULL_MONTHS.some((m) => m.startsWith(l)) || l === 'sept') }

export interface DateParts {
  /** Year-first or named-month forms are certain; `ab` is `a/b/YYYY` (M/D or D/M, decided per column). */
  kind: 'ymd' | 'ab'
  y: number
  a: number
  b: number
  time: { h: number; mi: number; s: number; ms: number } | null
  /** Minutes east of UTC when the text says (`Z`, `+08:00`), else null (local time). */
  offset: number | null
}

const TIME = /^(?:[T\s]+)(\d{1,2}):(\d{2})(?::(\d{2})(?:[.,](\d{1,9}))?)?\s*([AaPp][Mm])?\s*(Z|[+-]\d{2}:?\d{2})?$/

export function parseDateText(text: string): DateParts | null {
  const s = text.trim()
  let m: RegExpExecArray | null
  let parts: Omit<DateParts, 'time' | 'offset'> | null = null
  let rest = ''
  if ((m = /^(\d{4})([-/.])(\d{1,2})\2(\d{1,2})/.exec(s))) { parts = { kind: 'ymd', y: +m[1], a: +m[3], b: +m[4] }; rest = s.slice(m[0].length) }
  else if ((m = /^(\d{4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日/.exec(s))) { parts = { kind: 'ymd', y: +m[1], a: +m[2], b: +m[3] }; rest = s.slice(m[0].length) } // i18n-exempt: input vocabulary
  else if ((m = /^(\d{1,2})([-/.])(\d{1,2})\2(\d{4})/.exec(s))) { parts = { kind: 'ab', y: +m[4], a: +m[1], b: +m[3] }; rest = s.slice(m[0].length) }
  else if ((m = /^(\d{1,2})[-\s]([A-Za-z]{3,9}\.?)[-\s,]+(\d{4})/.exec(s)) && isMonthName(m[2])) { parts = { kind: 'ymd', y: +m[3], a: monthOf(m[2].replace(/\.$/, '')), b: +m[1] }; rest = s.slice(m[0].length) }
  else if ((m = /^([A-Za-z]{3,9}\.?)\s+(\d{1,2}),?\s+(\d{4})/.exec(s)) && isMonthName(m[1])) { parts = { kind: 'ymd', y: +m[3], a: monthOf(m[1].replace(/\.$/, '')), b: +m[2] }; rest = s.slice(m[0].length) }
  if (!parts) return null
  if (parts.y < 1 || parts.y > 9999) return null
  if (rest.trim() === '') return { ...parts, time: null, offset: null }
  const t = TIME.exec(rest)
  if (!t) return null
  let h = +t[1]
  const mi = +t[2], sec = t[3] ? +t[3] : 0
  const ms = t[4] ? Math.floor(+(t[4] + '000').slice(0, 3)) : 0
  if (t[5]) { if (h < 1 || h > 12) return null; h = (h % 12) + (/p/i.test(t[5]) ? 12 : 0) }
  if (h > 23 || mi > 59 || sec > 59) return null
  let offset: number | null = null
  if (t[6]) offset = t[6] === 'Z' ? 0 : (t[6][0] === '-' ? -1 : 1) * (parseInt(t[6].slice(1, 3), 10) * 60 + parseInt(t[6].replace(':', '').slice(3, 5), 10))
  return { ...parts, time: { h, mi, s: sec, ms }, offset }
}

const daysIn = (y: number, m: number) => [31, (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0 ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1]
const validDate = (y: number, m: number, d: number) => m >= 1 && m <= 12 && d >= 1 && d <= daysIn(y, m)
const p2 = (n: number, w = 2) => String(n).padStart(w, '0')

/** Month and day of parsed parts in the column's order. */
function ymd(p: DateParts, order: DateOrder): [number, number, number] | null {
  const [m, d] = p.kind === 'ymd' || order === 'md' ? [p.a, p.b] : [p.b, p.a]
  return validDate(p.y, m, d) ? [p.y, m, d] : null
}

/** Minutes east of UTC of a wall-clock time in a time zone ('UTC', 'local' or an IANA name). */
function zoneOffset(utcMs: number, zone: string): number {
  if (zone === 'UTC') return 0
  if (zone === 'local') return -new Date(utcMs).getTimezoneOffset()
  const parts = new Intl.DateTimeFormat('en-US', { timeZone: zone, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' }).formatToParts(new Date(utcMs))
  const v = (t: string) => Number(parts.find((x) => x.type === t)?.value ?? 0)
  const asUtc = Date.UTC(v('year'), v('month') - 1, v('day'), v('hour') % 24, v('minute'), v('second'))
  return Math.round((asUtc - Math.floor(utcMs / 1000) * 1000) / 60_000)
}

/** A wall-clock time in `zone` → UTC milliseconds. */
export function zonedToUtc(y: number, m: number, d: number, h: number, mi: number, s: number, ms: number, zone: string): number {
  // Date.UTC reads years 0–99 as 19xx: set the year afterwards
  const wall = new Date(Date.UTC(2000, m - 1, d, h, mi, s, ms))
  wall.setUTCFullYear(y)
  const guess = wall.getTime()
  // the offset at the guess, then at the result (right across a daylight-saving change)
  const first = guess - zoneOffset(guess, zone) * 60_000
  return guess - zoneOffset(first, zone) * 60_000
}

function formatUtc(ms: number): string | null {
  const d = new Date(ms)
  if (Number.isNaN(d.getTime())) return null
  const y = d.getUTCFullYear()
  if (y < 1 || y > 9999) return null
  return `${p2(y, 4)}-${p2(d.getUTCMonth() + 1)}-${p2(d.getUTCDate())}T${p2(d.getUTCHours())}:${p2(d.getUTCMinutes())}:${p2(d.getUTCSeconds())}.${p2(d.getUTCMilliseconds(), 3)}Z`
}

function dateTextOf(c: Cell): string | null {
  if ((c.hint === 'date' || c.hint === 'datetime') && typeof c.raw === 'string') return c.raw
  if (typeof c.exact === 'string') return c.exact
  if (c.hint === 'bool' || typeof c.raw === 'number' || typeof c.raw === 'boolean') return null
  return c.text
}

// ---- money

export interface Money { digits: string; frac: number; mark: { code: string } | { ambiguous: Ambiguous } | null }

function moneyOf(c: Cell): Money | null {
  if (c.exact && typeof c.exact === 'object' && !Array.isArray(c.exact)) {
    const o = c.exact as { amount?: unknown; currency?: unknown }
    const n = typeof o.amount === 'number' ? numberDigits(o.amount) : typeof o.amount === 'string' ? parseNumber(o.amount) : null
    if (!n || n.percent || typeof o.currency !== 'string') return null
    return { digits: n.digits, frac: n.frac, mark: { code: o.currency.toUpperCase() } }
  }
  if (typeof c.raw === 'number' && (c.hint === 'currency' || c.hint === undefined)) {
    const n = numberDigits(c.raw)
    if (!n) return null
    return { digits: n.digits, frac: n.frac, mark: c.currency ? resolveMark(c.currency) : null }
  }
  if (c.hint === 'bool' || c.hint === 'percent' || c.hint === 'date' || c.hint === 'datetime') return null
  const { prefix, amount, suffix } = splitMoney(c.text)
  const n = parseNumber(amount)
  if (!n || n.percent) return null
  const a = prefix ? resolveMark(prefix) : null
  const b = suffix ? resolveMark(suffix) : null
  if ((prefix && !a) || (suffix && !b)) return null
  let mark = a ?? b ?? (c.currency ? resolveMark(c.currency) : null)
  if (a && b) {
    const same = 'code' in a && 'code' in b ? a.code === b.code : 'ambiguous' in a && 'ambiguous' in b ? a.ambiguous === b.ambiguous : true
    if (!same) return null
    mark = 'code' in a ? a : b
  }
  return { digits: n.digits, frac: n.frac, mark }
}

// ---- inference

export interface InferOptions { dateOrder: DateOrder; locale: string }

export interface ColumnGuess {
  type: ImportType
  scale?: number
  /** A currency column's default. */
  currency?: string
  /** Ambiguous symbols the column holds (the wizard offers a choice for each). */
  dollar?: boolean
  yen?: boolean
  /** Select options (labels in order of first appearance; a hint's options first). */
  options?: string[]
  /** Values written as percentages, imported as ratios. */
  percent?: boolean
  /** Decimal places beyond MAX_SCALE: imported as numbers. */
  precision?: boolean
  /** The values fixed the day / month order (some part above 12); null: the wizard's choice applies. */
  dateOrder?: DateOrder | null
  /** The type came from the source (SQLite declared type, aiws field definition). */
  fromSource?: boolean
}

const nonEmpty = (c: Cell | undefined): c is Cell => Boolean(c) && (c!.text.trim() !== '' || (c!.raw !== undefined && c!.raw !== '') || (c!.exact !== undefined && c!.exact !== null && c!.exact !== ''))

/** The day / month order the values allow: 'md', 'dm', 'either', or null (they contradict each other). */
function dateOrderOf(parts: DateParts[]): DateOrder | 'either' | null {
  let md = true, dm = true
  for (const p of parts) {
    if (p.kind !== 'ab') continue
    if (!validDate(p.y, p.a, p.b)) md = false
    if (!validDate(p.y, p.b, p.a)) dm = false
  }
  return md && dm ? 'either' : md ? 'md' : dm ? 'dm' : null
}

/** Default currency and ambiguous symbols of a money column (§6.3). */
function moneyColumn(values: Money[], locale: string): { currency: string; dollar: boolean; yen: boolean } {
  const counts = new Map<string, number>()
  let dollar = false, yen = false
  for (const v of values) {
    if (v.mark && 'code' in v.mark) counts.set(v.mark.code, (counts.get(v.mark.code) ?? 0) + 1)
    else if (v.mark) { if (v.mark.ambiguous === '$') dollar = true; else yen = true }
  }
  const top = (m: Map<string, number>) => [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]?.[0]
  let currency = top(counts)
  if (!currency && (dollar || yen)) {
    const lang = languageCurrency(locale)
    const resolved = new Map<string, number>()
    for (const v of values) if (v.mark && 'ambiguous' in v.mark) { const c = resolveAmbiguous(v.mark.ambiguous, lang, locale); resolved.set(c, (resolved.get(c) ?? 0) + 1) }
    currency = top(resolved)
  }
  return { currency: currency ?? languageCurrency(locale), dollar, yen }
}

/** The type of a column from its non-empty cells (§3.3), or from what the source declared. */
export function inferColumn(cells: Cell[], hint: ColumnHint | undefined, options: InferOptions, allowSelect: boolean): ColumnGuess {
  const values = cells.filter(nonEmpty)
  const declared = hint?.type
  if (declared && declared !== 'numeric') {
    const guess: ColumnGuess = { type: declared === 'number' ? 'number' : declared, fromSource: true }
    if (declared === 'decimal' || declared === 'currency') guess.scale = hint.scale ?? 2
    if (declared === 'currency') { const m = moneyColumn(values.map(moneyOf).filter((x): x is Money => x !== null), options.locale); guess.currency = hint.currency ?? m.currency; guess.dollar = m.dollar; guess.yen = m.yen }
    if (declared === 'select' || declared === 'multi_select') guess.options = [...(hint.options ?? [])]
    if (declared === 'date' || declared === 'datetime') { const order = dateOrderOf(values.map((c) => parseDateText(dateTextOf(c) ?? '')).filter((p): p is DateParts => p !== null)); guess.dateOrder = order === 'md' || order === 'dm' ? order : null }
    return guess
  }
  if (values.length === 0) return { type: 'text' }
  // 1 boolean (never 1 / 0)
  if (!declared && values.every((c) => booleanOf(c) !== null)) return { type: 'boolean' }
  // 2–3 number / decimal (percentages as ratios)
  const nums = values.map((c) => numericOf(c))
  if (nums.every((n): n is NumberText => n !== null)) {
    const percent = nums.every((n) => n.percent)
    if (percent || nums.every((n) => !n.percent)) {
      const shaped = nums.map((n, i) => asStored(values[i], n))
      const ints = shaped.every((n) => isInteger(n) && withinSafe(n.digits))
      if (ints && !percent) return { type: 'number', ...(declared ? { fromSource: true } : {}) }
      const scale = Math.max(0, ...shaped.map((n) => n.frac))
      if (scale > MAX_SCALE) return { type: 'number', precision: true, ...(percent ? { percent } : {}) }
      if (shaped.every((n) => canonicalDecimal(n.digits, scale) !== null)) return { type: 'decimal', scale, ...(percent ? { percent } : {}) }
    }
  }
  if (declared === 'numeric') return { type: 'text' }
  // 4–5 date, datetime
  const dates = values.map((c) => { const t = dateTextOf(c); return t === null ? null : parseDateText(t) })
  if (dates.every((d): d is DateParts => d !== null)) {
    const order = dateOrderOf(dates)
    if (order !== null) {
      const fixed = order === 'either' ? null : order
      const ok = dates.every((d) => ymd(d, fixed ?? options.dateOrder) !== null)
      if (ok && dates.every((d) => d.time === null)) return { type: 'date', dateOrder: fixed }
      if (ok) return { type: 'datetime', dateOrder: fixed }
    }
  }
  // 6 currency: every value an amount, at least one marked (or a currency format)
  const money = values.map(moneyOf)
  if (money.every((m): m is Money => m !== null) && (money.some((m) => m.mark !== null) || values.some((c) => c.hint === 'currency'))) {
    const scale = Math.max(2, ...money.map((m) => m.frac))
    if (scale <= MAX_SCALE) return { type: 'currency', scale, ...moneyColumn(money, options.locale) }
  }
  // 7 select
  if (allowSelect && values.length >= SELECT_MIN_VALUES) {
    const labels: string[] = []
    const seen = new Set<string>()
    let fits = true
    for (const c of values) {
      const l = c.text.trim()
      if (l.length > SELECT_MAX_LABEL || l.includes('\n')) { fits = false; break }
      if (!seen.has(l)) { seen.add(l); labels.push(l); if (labels.length > SELECT_MAX_OPTIONS) { fits = false; break } }
    }
    if (fits && labels.length <= values.length / 2) return { type: 'select', options: labels }
  }
  return { type: 'text' }
}

// ---- shaping

export interface ShapeSpec {
  type: ImportType
  scale?: number
  currency?: string
  /** What `$` and `¥` mean in this column. */
  dollar?: string
  yen?: string
  /** Select / multi-select: label → option id. */
  options?: Map<string, string>
  dateOrder: DateOrder
  timezone: string
}

export const INVALID = Symbol('invalid')
export type Shaped = Json | typeof INVALID | undefined

/** One cell as the wire value of the column's type: undefined for an empty cell (left unset, Q11), INVALID when it
 * does not fit. */
export function shapeCell(c: Cell | undefined, spec: ShapeSpec): Shaped {
  if (!c || !nonEmpty(c)) return undefined
  switch (spec.type) {
    case 'text': {
      const t = typeof c.exact === 'string' ? c.exact : c.text
      return t.length > TEXT_MAX ? INVALID : t
    }
    case 'boolean': { const b = booleanOf(c); return b === null ? INVALID : b }
    case 'number': {
      if (typeof c.exact === 'number') return c.exact
      const n = numericOf(c, false)
      if (!n) return INVALID
      const v = asStored(c, n)
      const num = Number(v.digits)
      if (!Number.isFinite(num) || (isInteger(v) && !withinSafe(v.digits))) return INVALID
      return num === 0 ? 0 : num
    }
    case 'decimal': {
      const n = numericOf(c, false)
      if (!n) return INVALID
      return canonicalDecimal(asStored(c, n).digits, spec.scale ?? 0) ?? INVALID
    }
    case 'date': {
      const t = dateTextOf(c)
      const p = t === null ? null : parseDateText(t)
      if (!p || p.time !== null && !(p.time.h === 0 && p.time.mi === 0 && p.time.s === 0 && p.time.ms === 0)) return INVALID
      const d = ymd(p, spec.dateOrder)
      return d ? `${p2(d[0], 4)}-${p2(d[1])}-${p2(d[2])}` : INVALID
    }
    case 'datetime': {
      const t = dateTextOf(c)
      const p = t === null ? null : parseDateText(t)
      if (!p) return INVALID
      const d = ymd(p, spec.dateOrder)
      if (!d) return INVALID
      const tm = p.time ?? { h: 0, mi: 0, s: 0, ms: 0 }
      const ms = p.offset !== null ? Date.UTC(d[0], d[1] - 1, d[2], tm.h, tm.mi, tm.s, tm.ms) - p.offset * 60_000 : zonedToUtc(d[0], d[1], d[2], tm.h, tm.mi, tm.s, tm.ms, spec.timezone)
      return formatUtc(ms) ?? INVALID
    }
    case 'currency': {
      const m = moneyOf(c)
      if (!m) return INVALID
      const amount = canonicalDecimal(m.digits, spec.scale ?? 2)
      if (amount === null) return INVALID
      const code = !m.mark ? spec.currency ?? 'USD' : 'code' in m.mark ? m.mark.code : m.mark.ambiguous === '$' ? spec.dollar ?? 'USD' : spec.yen ?? 'CNY'
      return { amount, currency: code }
    }
    case 'select': {
      const label = typeof c.exact === 'string' ? c.exact.trim() : c.text.trim()
      return spec.options?.get(label) ?? INVALID
    }
    case 'multi_select': {
      const labels = Array.isArray(c.exact) ? c.exact.map((x) => String(x).trim()) : c.text.split(/[,;，；、\n]/).map((x) => x.trim())
      const ids: string[] = []
      for (const l of labels) {
        if (!l) continue
        const id = spec.options?.get(l)
        if (!id) return INVALID
        if (!ids.includes(id)) ids.push(id)
      }
      return ids.sort()
    }
  }
}

/** The distinct labels of a column (in order of first appearance) for a select made by hand. */
export function labelsOf(cells: Cell[], multi: boolean, known: string[] = []): string[] {
  const out = [...known]
  const seen = new Set(out)
  for (const c of cells) {
    if (!nonEmpty(c)) continue
    const parts = Array.isArray(c.exact) ? c.exact.map(String) : multi ? c.text.split(/[,;，；、\n]/) : [typeof c.exact === 'string' ? c.exact : c.text] // i18n-exempt: input vocabulary
    for (const raw of parts) {
      const l = raw.trim()
      if (!l || seen.has(l) || l.length > 256) continue
      seen.add(l)
      out.push(l)
      if (out.length >= SELECT_OPTION_LIMIT) return out
    }
  }
  return out
}

export { nonEmpty }
