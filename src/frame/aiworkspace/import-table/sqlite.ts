/* SQLite files (表格数据导入 TODO §5.4): the host opens the bytes (sqlite-wasm in a browser Worker, `node:sqlite` in
 * Deno) and hands in a read-only query function; this lists the tables (views on request), reads one with its
 * declared column types and maps them by SQLite's affinity rules: integers and reals are numbers (the values decide
 * number or decimal), NUMERIC / DECIMAL(p, s) a decimal of scale s, BOOL 0 / 1 booleans, DATE and DATETIME /
 * TIMESTAMP text or unix seconds, BLOB columns left out (reported), the rest inferred from the values. */

import { cell, note, type Cell, type ColumnHint, type Grid, type GridNote } from './grid.ts'
import { numberText } from './xlsx.ts'

/** A read-only query: column names and rows of values (number, bigint, string, bytes, null). */
export type SqliteQuery = (sql: string) => Promise<{ columns: string[]; rows: unknown[][] }> | { columns: string[]; rows: unknown[][] }

export interface SqliteTable { name: string; kind: 'table' | 'view'; rows: number }

const quote = (name: string) => `"${name.replace(/"/g, '""')}"`

export async function listSqliteTables(query: SqliteQuery, views = false): Promise<SqliteTable[]> {
  const { rows } = await query(`SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid`)
  const out: SqliteTable[] = []
  for (const [name, type] of rows as [string, string][]) {
    if (type === 'view' && !views) continue
    let count = 0
    try { count = Number((await query(`SELECT count(*) FROM ${quote(name)}`)).rows[0]?.[0] ?? 0) } catch { /* a broken view */ }
    out.push({ name, kind: type === 'view' ? 'view' : 'table', rows: count })
  }
  return out
}

/** A column's declared type → what the importer makes of it. */
export function affinityHint(declared: string): ColumnHint {
  const t = declared.toUpperCase()
  if (t.includes('BOOL')) return { type: 'boolean' }
  if (/^DATE\b|^DATE$/.test(t.trim())) return { type: 'date' }
  if (t.includes('DATETIME') || t.includes('TIMESTAMP')) return { type: 'datetime' }
  if (t.includes('INT')) return { type: 'numeric' }
  // text: the values decide (dates, money and the like are often stored as text)
  if (t.includes('CHAR') || t.includes('CLOB') || t.includes('TEXT')) return {}
  if (t.includes('BLOB')) return { skip: 'blob_column' }
  if (t.includes('REAL') || t.includes('FLOA') || t.includes('DOUB')) return { type: 'numeric' }
  const dec = /(NUMERIC|DECIMAL)\s*\(\s*\d+\s*,\s*(\d+)\s*\)/.exec(t)
  if (dec) return { type: 'decimal', scale: Math.min(18, Number(dec[2])) }
  if (t.includes('NUMERIC') || t.includes('DECIMAL')) return { type: 'numeric' }
  // no declared type: the values decide (an untyped column holding bytes is left out when it does)
  return {}
}

const pad = (n: number, w = 2) => String(n).padStart(w, '0')
function unixToIso(seconds: number, withTime: boolean): string {
  const d = new Date(seconds * 1000)
  const date = `${pad(d.getUTCFullYear(), 4)}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`
  return withTime ? `${d.toISOString().slice(0, 19)}Z` : date
}

function valueCell(v: unknown, hint: ColumnHint): Cell | null {
  if (v === null || v === undefined) return null
  if (v instanceof Uint8Array || v instanceof ArrayBuffer) return null
  if (typeof v === 'bigint') {
    const s = v.toString()
    return { text: s, raw: Math.abs(Number(v)) <= Number.MAX_SAFE_INTEGER ? Number(v) : s }
  }
  if (typeof v === 'number') {
    if (hint.type === 'boolean') return { text: v ? 'TRUE' : 'FALSE', raw: v !== 0, hint: 'bool' }
    if (hint.type === 'date' || hint.type === 'datetime') { const iso = unixToIso(v, hint.type === 'datetime'); return { text: iso, raw: iso, hint: hint.type } }
    return { text: numberText(v), raw: v }
  }
  const s = String(v)
  if (hint.type === 'boolean' && /^(0|1)$/.test(s.trim())) return { text: s, raw: s.trim() === '1', hint: 'bool' }
  return { text: s }
}

/** Read a table (or view) as a grid: the header row is the column names. */
export async function readSqliteTable(query: SqliteQuery, table: string, limit = Infinity): Promise<Grid> {
  const info = await query(`PRAGMA table_info(${quote(table)})`)
  const nameAt = info.columns.indexOf('name'), typeAt = info.columns.indexOf('type')
  const declared = info.rows.map((r) => ({ name: String(r[nameAt]), type: String(r[typeAt] ?? '') }))
  const notes: GridNote[] = []
  const lim = Number.isFinite(limit) ? ` LIMIT ${Math.floor(limit) + 1}` : ''
  const result = await query(`SELECT * FROM ${quote(table)}${lim}`)
  const names = declared.length ? declared.map((d) => d.name) : result.columns
  const hints: ColumnHint[] = names.map((_, i) => affinityHint(declared[i]?.type ?? ''))
  // an untyped column holding bytes is left out too
  result.rows.forEach((r) => r.forEach((v, i) => { if ((v instanceof Uint8Array || v instanceof ArrayBuffer) && !hints[i].skip) hints[i] = { skip: 'blob_column' } }))
  const rows = result.rows.slice(0, Number.isFinite(limit) ? limit : undefined)
  const truncated = Number.isFinite(limit) && result.rows.length > limit ? 1 : 0
  for (const [i, h] of hints.entries()) if (h.skip) note(notes, h.skip, `${table}.${names[i]}`)
  const header: Cell[] = names.map((n) => ({ text: n, hint: 'text' }))
  const body: Cell[][] = rows.map((r) => names.map((_, i) => valueCell(r[i], hints[i]) ?? cell('')))
  return { name: table, rows: [header, ...body], header: true, columns: hints, notes, ...(truncated ? { truncated } : {}) }
}

export function isSqlite(bytes: Uint8Array): boolean {
  const magic = 'SQLite format 3\0'
  if (bytes.length < 100) return false
  for (let i = 0; i < magic.length; i++) if (bytes[i] !== magic.charCodeAt(i)) return false
  return true
}
