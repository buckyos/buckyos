/* The one shape every source becomes (表格数据导入 TODO §2): a rectangle of cells, each with its display text and,
 * when the source knew more (an HTML raw value, an xlsx cell type and number format, a SQLite column type), the raw
 * value and a hint. What reading found worth reporting (formulas, error cells, merged cells) travels with the grid.
 * Header detection, type inference, shaping and committing never look at where a grid came from. */

/** What a source says a cell is: `date` / `datetime` carry an ISO raw (`YYYY-MM-DD`, `YYYY-MM-DDTHH:MM:SS[.fff]`
 * local time), `currency` a numeric raw (and `currency` its code when known), `percent` a ratio (0.125 for 12.5 %). */
export type CellHint = 'date' | 'datetime' | 'currency' | 'percent' | 'bool' | 'text'

export interface Cell {
  text: string
  raw?: number | boolean | string
  hint?: CellHint
  /** An ISO 4217 code a source attached (xlsx `[$CNY]`, a Google currency format), or a symbol it showed (`$`, `¥`). */
  currency?: string
  /** An exact wire value (aiws's own copies, `data-aiws-value`). */
  exact?: unknown
  /** Covered by a merged cell that starts elsewhere (its value is in that cell). */
  covered?: boolean
}

/** What a source knows about a whole column (SQLite declared types, aiws field definitions). */
export interface ColumnHint {
  type?: 'text' | 'number' | 'numeric' | 'decimal' | 'boolean' | 'date' | 'datetime' | 'currency' | 'select' | 'multi_select'
  scale?: number
  currency?: string
  /** Option labels in order (select / multi_select). */
  options?: string[]
  /** A column that cannot be imported (a SQLite BLOB): left out, with the reason's report kind. */
  skip?: string
}

export interface GridNote { kind: string; count: number; samples: string[] }

export interface Grid {
  /** The table title it suggests (file, sheet or table name). */
  name: string
  rows: Cell[][]
  /** The source marks the first row as a header (`<thead>`, `<th>`, aiws's own copy); null: decide from the values. */
  header: boolean | null
  columns: (ColumnHint | undefined)[]
  notes: GridNote[]
  /** Rows the source had beyond what was read (a SQLite table over the limit). */
  truncated?: number
}

export function note(notes: GridNote[], kind: string, sample?: string, count = 1) {
  let n = notes.find((x) => x.kind === kind)
  if (!n) { n = { kind, count: 0, samples: [] }; notes.push(n) }
  n.count += count
  if (sample && n.samples.length < 5 && !n.samples.includes(sample)) n.samples.push(sample)
}

export const cell = (text: string): Cell => ({ text })
const blank = (c: Cell | undefined) => !c || (c.text.trim() === '' && (c.raw === undefined || c.raw === '') && c.exact === undefined)

/** Excel's column letters: 0 → A, 25 → Z, 26 → AA. */
export function columnLetter(index: number): string {
  let n = index + 1
  let out = ''
  while (n > 0) { const r = (n - 1) % 26; out = String.fromCharCode(65 + r) + out; n = Math.floor((n - 1) / 26) }
  return out
}

/** Drop empty leading and trailing rows and columns, drop (and count) empty rows in between, pad short rows. */
export function tidy(grid: Grid): Grid {
  const rows = grid.rows
  let width = 0
  for (const row of rows) width = Math.max(width, row.length)
  let left = width, right = -1
  for (const row of rows) {
    for (let i = 0; i < row.length; i++) if (!blank(row[i])) { if (i < left) left = i; if (i > right) right = i }
  }
  const notes = [...grid.notes.map((n) => ({ ...n, samples: [...n.samples] }))]
  if (right < 0) return { ...grid, rows: [], columns: [], notes }
  const kept: Cell[][] = []
  let skipped = 0
  let pending = 0
  for (const row of rows) {
    const empty = row.every((c) => blank(c))
    if (empty) { if (kept.length > 0) pending += 1; continue }
    skipped += pending
    pending = 0
    const out: Cell[] = []
    for (let i = left; i <= right; i++) out.push(row[i] ?? cell(''))
    kept.push(out)
  }
  if (skipped > 0) note(notes, 'empty_rows', undefined, skipped)
  return { ...grid, rows: kept, columns: grid.columns.slice(left, right + 1), notes }
}

/** A grid from plain strings (CSV, TSV). */
export function textGrid(name: string, rows: string[][], header: boolean | null = null): Grid {
  return { name, rows: rows.map((r) => r.map(cell)), header, columns: [], notes: [] }
}
