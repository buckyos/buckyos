/* A grid and the wizard's choices → the table to create (表格数据导入 TODO §3.1–§3.3): whether the first row is a
 * header, field names and ids, column types, every cell shaped (or counted as not fitting and handled by the
 * column's policy), and the report. The defaults are the product: `defaultChoices` is what an import without any
 * change in the wizard does. */

import { ImportReport } from '../import-drawio/convert.ts'
import { columnLetter, type Cell, type Grid } from './grid.ts'
import { INVALID, inferColumn, labelsOf, nonEmpty, shapeCell, TEXT_MAX, type ColumnGuess, type DateOrder, type ImportType, type Json, type ShapeSpec } from './infer.ts'
import { resolveAmbiguous } from './currency.ts'

/** Rows the wizard infers types from; more are checked when the import runs (§7). */
export const INFER_ROWS = 5000
/** One import at most (Q6). */
export const MAX_ROWS = 100_000
export const MAX_CELLS = 1_000_000
const NAME_MAX = 128

export interface PlanOptions {
  /** The interface language (`zh…` makes `¥` and bare amounts CNY, Q12). */
  locale: string
  /** A column's name when the grid has no header or the header cell is empty (`Column A`). */
  columnName?: (letter: string) => string
}

export interface ColumnChoice {
  include: boolean
  name: string
  type: ImportType
  scale?: number
  /** A currency column's default (bare amounts). */
  currency?: string
  /** What `$` and `¥` mean in a currency column. */
  dollar?: string
  yen?: string
  /** Select / multi-select option labels, in order. */
  options?: string[]
  /** Values that do not fit the type: left empty (default), or the whole column imported as text. */
  invalid: 'unset' | 'text'
  /** What inference said (the wizard marks the type "auto" while they agree). */
  guess: ColumnGuess
}

export interface TableChoices {
  title: string
  header: boolean
  columns: ColumnChoice[]
  dateOrder: DateOrder
  /** Times without an offset: 'local' (the browser's zone), 'UTC', or an IANA zone. */
  timezone: string
}

const isNumeric = (c: Cell) => typeof c.raw === 'number' || /^[+-]?[\d,.]+%?$/.test(c.text.trim())

/** The value rows of a grid. */
export const bodyOf = (grid: Grid, header: boolean) => (header ? grid.rows.slice(1) : grid.rows)
const width = (grid: Grid) => grid.rows.reduce((w, r) => Math.max(w, r.length), 0)
const columnOf = (rows: Cell[][], i: number, limit = Infinity) => {
  const out: Cell[] = []
  for (let r = 0; r < rows.length && r < limit; r++) out.push(rows[r][i] ?? { text: '' })
  return out
}

/** Column guesses: select is not offered to the first text column (the title column, §3.3 rule 7). */
export function guessColumns(grid: Grid, header: boolean, options: PlanOptions & { dateOrder: DateOrder }): ColumnGuess[] {
  const body = bodyOf(grid, header)
  const n = width(grid)
  const plain = Array.from({ length: n }, (_, i) => inferColumn(columnOf(body, i, INFER_ROWS), grid.columns[i], options, false))
  const titleAt = plain.findIndex((g) => g.type === 'text')
  return plain.map((g, i) => (g.type === 'text' && i !== titleAt && !grid.columns[i]?.type ? inferColumn(columnOf(body, i, INFER_ROWS), grid.columns[i], options, true) : g))
}

/** §3.1: whether the first row names the columns, when the source does not say. */
export function detectHeader(grid: Grid, options: PlanOptions & { dateOrder: DateOrder }): boolean {
  if (grid.header !== null) return grid.header
  if (grid.rows.length < 2) return false
  const first = grid.rows[0]
  const n = width(grid)
  // every column named (a cell under a merged header counts as named: it gets a name of its own)
  if (first.length < n || first.some((c) => !c.covered && !nonEmpty(c))) return false
  const rest = guessColumns(grid, true, options)
  // (a) a column whose values have a type its first cell does not fit
  for (let i = 0; i < n; i++) {
    const g = rest[i]
    if (g.type === 'text' || g.type === 'select') continue
    const fits = shapeCell(first[i], specOf({ include: true, name: '', type: g.type, scale: g.scale, currency: g.currency, invalid: 'unset', guess: g }, options.dateOrder, 'UTC'))
    if (fits === INVALID) return true
  }
  // (b) all text below, and the first row distinct names that are not numbers
  if (rest.every((g) => g.type === 'text' || g.type === 'select')) {
    const names = first.map((c) => c.text.trim())
    return new Set(names).size === names.length && first.every((c) => !isNumeric(c))
  }
  return false
}

/** Field names: the header text (≤ 128 characters), else `Column A`; repeated names get ` (2)`, ` (3)`… */
export function columnNames(grid: Grid, header: boolean, options: PlanOptions): string[] {
  const n = width(grid)
  const fallback = options.columnName ?? ((letter: string) => `Column ${letter}`)
  const used = new Set<string>()
  const out: string[] = []
  for (let i = 0; i < n; i++) {
    const raw = header ? (grid.rows[0]?.[i]?.text ?? '').replace(/\s+/g, ' ').trim() : ''
    const base = (raw || fallback(columnLetter(i))).slice(0, NAME_MAX)
    let name = base
    for (let k = 2; used.has(name); k++) { const tail = ` (${k})`; name = base.slice(0, NAME_MAX - tail.length) + tail }
    used.add(name)
    out.push(name)
  }
  return out
}

function choiceOf(guess: ColumnGuess, name: string, locale: string): ColumnChoice {
  const c: ColumnChoice = { include: true, name, type: guess.type, invalid: 'unset', guess }
  if (guess.scale !== undefined) c.scale = guess.scale
  if (guess.type === 'currency') {
    c.currency = guess.currency
    c.dollar = resolveAmbiguous('$', guess.currency ?? null, locale)
    c.yen = resolveAmbiguous('¥', guess.currency ?? null, locale)
  }
  if (guess.options) c.options = [...guess.options]
  return c
}

/** What an import does when nothing is changed in the wizard. */
export function defaultChoices(grid: Grid, options: PlanOptions & { dateOrder?: DateOrder; timezone?: string; header?: boolean }): TableChoices {
  const dateOrder = options.dateOrder ?? 'md'
  const header = options.header ?? detectHeader(grid, { ...options, dateOrder })
  return { title: grid.name, header, dateOrder, timezone: options.timezone ?? 'local', columns: columnsFor(grid, header, { ...options, dateOrder }) }
}

/** Columns for a header choice (the wizard's header switch recomputes names and types; `keep` carries inclusion). */
export function columnsFor(grid: Grid, header: boolean, options: PlanOptions & { dateOrder: DateOrder }, keep?: ColumnChoice[]): ColumnChoice[] {
  const guesses = guessColumns(grid, header, options)
  const names = columnNames(grid, header, options)
  return guesses.map((g, i) => {
    const c = choiceOf(g, names[i], options.locale)
    if (grid.columns[i]?.skip) c.include = false
    if (keep?.[i]) c.include = keep[i].include
    return c
  })
}

/** A column changed to another type in the wizard: the type's settings (scale, currency, options from the values). */
export function retype(grid: Grid, header: boolean, index: number, choice: ColumnChoice, type: ImportType, locale: string): ColumnChoice {
  const next: ColumnChoice = { include: choice.include, name: choice.name, type, invalid: choice.invalid, guess: choice.guess }
  if (type === choice.guess.type) return { ...choiceOf(choice.guess, choice.name, locale), include: choice.include, invalid: choice.invalid }
  if (type === 'decimal') next.scale = 2
  if (type === 'currency') { next.scale = 2; next.currency = choice.currency ?? (locale.startsWith('zh') ? 'CNY' : 'USD'); next.dollar = resolveAmbiguous('$', next.currency, locale); next.yen = resolveAmbiguous('¥', next.currency, locale) }
  if (type === 'select' || type === 'multi_select') next.options = labelsOf(columnOf(bodyOf(grid, header), index), type === 'multi_select', grid.columns[index]?.options)
  return next
}

/** How a column's cells are shaped. */
export function specOf(choice: ColumnChoice, dateOrder: DateOrder, timezone: string): ShapeSpec {
  return {
    type: choice.type, scale: choice.scale, currency: choice.currency, dollar: choice.dollar, yen: choice.yen, timezone,
    dateOrder: choice.guess.dateOrder ?? dateOrder,
    options: choice.options ? new Map(choice.options.map((l, i) => [l, `o${i + 1}`])) : undefined,
  }
}

/** The field a column becomes (wire form of `table.add_field` / a TableSource's `fields`). */
export function fieldOf(choice: ColumnChoice, fieldId: string): Record<string, Json> {
  const f: Record<string, Json> = { field_id: fieldId, name: choice.name, type: choice.type }
  if (choice.type === 'decimal') f.scale = choice.scale ?? 0
  if (choice.type === 'currency') { f.currency = choice.currency ?? 'USD'; if ((choice.scale ?? 2) !== 2) f.scale = choice.scale ?? 2 }
  if (choice.type === 'select' || choice.type === 'multi_select') f.options = (choice.options ?? []).map((label, i) => ({ option_id: `o${i + 1}`, label }))
  return f
}

/** Whether a value is accepted for a field (the host's WASM `normalize_value`; the backend's own rules). */
export type Validate = (field: Record<string, Json>, value: Json) => boolean

export interface ColumnCheck { invalid: number; samples: string[] }

/** How many of a column's values (the first `limit` rows) do not fit its chosen type. */
export function checkColumn(grid: Grid, choices: TableChoices, index: number, validate?: Validate, limit = INFER_ROWS): ColumnCheck {
  const choice = choices.columns[index]
  const spec = specOf(choice, choices.dateOrder, choices.timezone)
  const field = fieldOf(choice, 'c1')
  const out: ColumnCheck = { invalid: 0, samples: [] }
  const body = bodyOf(grid, choices.header)
  for (let r = 0; r < body.length && r < limit; r++) {
    const c = body[r][index]
    const v = shapeCell(c, spec)
    if (v === undefined) continue
    if (v === INVALID ? !(choice.type === 'text') : validate && !validate(field, v)) {
      out.invalid += 1
      if (out.samples.length < 3 && c && !out.samples.includes(c.text)) out.samples.push(c.text.slice(0, 40))
    }
  }
  return out
}

export interface TablePlan {
  title: string
  /** Field definitions in column order (ids `c1…`). */
  fields: Record<string, Json>[]
  /** The first text field (none: no title field). */
  titleFieldId: string | null
  /** Values by field id, rows in grid order; empty cells are absent. */
  records: Record<string, Json>[]
  report: ImportReport
}

export class TableLimitError extends Error {
  readonly rows: number
  readonly cells: number
  constructor(rows: number, cells: number) {
    super(`${rows} rows / ${cells} cells: over the import limit of ${MAX_ROWS} rows / ${MAX_CELLS} cells`)
    this.rows = rows
    this.cells = cells
  }
}

const REPORT_MESSAGES: Record<string, string> = {
  empty_rows: 'Empty rows skipped',
  merged_header: 'Merged header cells (their columns were named)',
  merged_cells: 'Merged cells (the value kept in the first cell)',
  formula: 'Formulas imported as their calculated values',
  error_cells: 'Error cells (#N/A, #DIV/0!…) left empty',
  time_only: 'Times of day without a date imported as text',
  blob_column: 'Binary (BLOB) columns left out',
}

export function planTable(grid: Grid, choices: TableChoices, options: { validate?: Validate } = {}): TablePlan {
  const body = bodyOf(grid, choices.header)
  const included = choices.columns.map((c, i) => ({ c, i })).filter(({ c }) => c.include)
  if (body.length > MAX_ROWS || body.length * included.length > MAX_CELLS) throw new TableLimitError(body.length, body.length * included.length)
  const report = new ImportReport()
  for (const n of grid.notes) {
    const message = REPORT_MESSAGES[n.kind] ?? n.kind
    report.notes.set(`table_${n.kind}`, { kind: `table_${n.kind}`, message, count: n.count, samples: [...n.samples] })
  }
  if (grid.truncated) report.add('table_rows_limit', 'Rows beyond the limit not imported')
  // policy "text": a column with values that do not fit becomes text before anything is shaped
  const columns = included.map(({ c, i }) => {
    if (c.type === 'text' || c.invalid !== 'text') return { c, i }
    const spec = specOf(c, choices.dateOrder, choices.timezone)
    const field = fieldOf(c, 'c1')
    const bad = body.some((row) => { const v = shapeCell(row[i], spec); return v === INVALID || (v !== undefined && options.validate !== undefined && !options.validate(field, v)) })
    if (!bad) return { c, i }
    report.add('table_as_text', 'Columns imported as text because some values did not fit the chosen type', c.name)
    return { c: { ...c, type: 'text' as ImportType, scale: undefined, currency: undefined, options: undefined }, i }
  })
  const fields = columns.map(({ c }, k) => fieldOf(c, `c${k + 1}`))
  const specs = columns.map(({ c }) => specOf(c, choices.dateOrder, choices.timezone))
  const titleAt = columns.findIndex(({ c }) => c.type === 'text')
  for (const { c } of columns) {
    if (c.type === c.guess.type && c.guess.percent) report.add('table_percent', 'Percentages imported as ratios (12.5% → 0.125)', c.name)
    if (c.type === c.guess.type && c.guess.precision) report.add('table_precision', 'Numbers with more than 6 decimal places imported as numbers (about 15 significant digits)', c.name)
  }
  // a validation per distinct value and column is enough
  const seen = columns.map(() => new Map<string, boolean>())
  const records: Record<string, Json>[] = []
  for (const row of body) {
    const values: Record<string, Json> = {}
    columns.forEach(({ c, i }, k) => {
      const cellAt = row[i]
      let v = shapeCell(cellAt, specs[k])
      if (v === undefined) return
      if (v === INVALID && c.type === 'text') {
        v = (cellAt?.text ?? '').slice(0, TEXT_MAX)
        report.add('table_long_text', 'Texts longer than 65 536 characters cut', c.name)
      }
      if (v !== INVALID && options.validate) {
        const key = typeof v === 'string' ? v : JSON.stringify(v)
        let ok = seen[k].get(key)
        if (ok === undefined) { ok = options.validate(fields[k], v); if (seen[k].size < 10_000) seen[k].set(key, ok) }
        if (!ok) v = INVALID
      }
      if (v === INVALID) { report.add('table_invalid', 'Values that did not fit the chosen type left empty', `${c.name}: ${(cellAt?.text ?? '').slice(0, 40)}`); return }
      values[fields[k].field_id as string] = v
    })
    records.push(values)
  }
  // how the shared symbols were read (Q12; the wizard offered the choice)
  for (const { c } of columns) {
    if (c.type !== 'currency') continue
    if (c.guess.dollar) report.add('table_currency_symbol', 'Currency symbols shared by several currencies, read as', `${c.name}: $ → ${c.dollar ?? 'USD'}`)
    if (c.guess.yen) report.add('table_currency_symbol', 'Currency symbols shared by several currencies, read as', `${c.name}: ¥ → ${c.yen ?? 'CNY'}`)
  }
  return { title: choices.title.trim() || grid.name, fields, titleFieldId: titleAt >= 0 ? `c${titleAt + 1}` : null, records, report }
}
