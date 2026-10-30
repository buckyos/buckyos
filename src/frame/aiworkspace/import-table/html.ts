/* A pasted HTML table (表格数据导入 TODO §4): Excel, Numbers, Google Sheets, Notion and web pages all put the selection
 * on the clipboard as `text/html` with a `<table>`. Clipboard HTML is not XML (unquoted attributes, unclosed cells,
 * `<br>`), so this reads it with a small tolerant tokenizer. Row and column spans are laid out (the value in the
 * top-left cell). Raw values win over display text where the source gives them: Excel `x:num` / `x:bool` with the
 * class's `mso-number-format`, Google Sheets `data-sheets-value` / `data-sheets-numberformat`, and aiws's own copies
 * (`data-aiws-table` field definitions, `data-aiws-value` wire values). */

import { classifyFormat, serialToIso, type FormatInfo } from './format.ts'
import { cell, note, type Cell, type ColumnHint, type Grid, type GridNote } from './grid.ts'

interface Tag { name: string; attrs: Record<string, string>; close: boolean; self: boolean }
type Token = Tag | string

const NAMED: Record<string, string> = {
  nbsp: ' ', amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", yen: '¥', euro: '€', pound: '£', cent: '¢', copy: '©', reg: '®', trade: '™',
  hellip: '…', mdash: '—', ndash: '–', laquo: '«', raquo: '»', times: '×', divide: '÷', deg: '°', middot: '·', bull: '•', lsquo: '‘', rsquo: '’', ldquo: '“', rdquo: '”',
}

export function decodeHtmlEntities(text: string): string {
  if (!text.includes('&')) return text
  return text.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);?/g, (all, ref: string) => {
    if (ref[0] === '#') {
      const code = ref[1] === 'x' || ref[1] === 'X' ? parseInt(ref.slice(2), 16) : parseInt(ref.slice(1), 10)
      return Number.isFinite(code) && code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : all
    }
    return NAMED[ref.toLowerCase()] ?? all
  })
}

/** Tags and text; comments, conditional comments, doctype and the content of script are dropped, style kept as text. */
function tokenize(html: string): Token[] {
  const out: Token[] = []
  let i = 0
  const n = html.length
  while (i < n) {
    const lt = html.indexOf('<', i)
    if (lt < 0) { out.push(html.slice(i)); break }
    if (lt > i) out.push(html.slice(i, lt))
    if (html.startsWith('<!--', lt)) { const end = html.indexOf('-->', lt + 4); i = end < 0 ? n : end + 3; continue }
    if (html[lt + 1] === '!' || html[lt + 1] === '?') { const end = html.indexOf('>', lt); i = end < 0 ? n : end + 1; continue }
    const close = html[lt + 1] === '/'
    let j = lt + (close ? 2 : 1)
    const nameStart = j
    while (j < n && /[A-Za-z0-9:_-]/.test(html[j])) j++
    const name = html.slice(nameStart, j).toLowerCase()
    if (!name) { out.push('<'); i = lt + 1; continue }
    const attrs: Record<string, string> = {}
    let self = false
    while (j < n && html[j] !== '>') {
      if (/\s/.test(html[j])) { j++; continue }
      if (html[j] === '/' ) { self = true; j++; continue }
      let k = j
      while (k < n && !/[\s=>/]/.test(html[k])) k++
      const attr = html.slice(j, k).toLowerCase()
      j = k
      while (j < n && /\s/.test(html[j])) j++
      if (html[j] !== '=') { if (attr) attrs[attr] = ''; continue }
      j++
      while (j < n && /\s/.test(html[j])) j++
      let value: string
      if (html[j] === '"' || html[j] === "'") {
        const q = html[j]
        const end = html.indexOf(q, j + 1)
        value = html.slice(j + 1, end < 0 ? n : end)
        j = end < 0 ? n : end + 1
      } else {
        let k2 = j
        while (k2 < n && !/[\s>]/.test(html[k2])) k2++
        value = html.slice(j, k2)
        j = k2
      }
      if (attr) attrs[attr] = decodeHtmlEntities(value)
    }
    i = j + 1
    out.push({ name, attrs, close, self })
    if (!close && (name === 'script' || name === 'style' || name === 'textarea' || name === 'title')) {
      const end = html.toLowerCase().indexOf(`</${name}`, i)
      const body = html.slice(i, end < 0 ? n : end)
      if (name === 'style') out.push({ name: '#style', attrs: { text: body }, close: false, self: true })
      i = end < 0 ? n : html.indexOf('>', end) + 1 || n
      out.push({ name, attrs: {}, close: true, self: false })
    }
  }
  return out
}

/** CSS escapes (`\0022` → `"`, `\#` → `#`) in a style value. */
function cssUnescape(s: string): string {
  return s.replace(/\\([0-9a-fA-F]{1,6})\s?|\\(.)/g, (_, hex: string | undefined, c: string | undefined) => (hex ? String.fromCodePoint(parseInt(hex, 16)) : c ?? ''))
}

function msoFormat(style: string): string | null {
  const m = /mso-number-format\s*:\s*("([^"]*)"|'([^']*)'|[^;]+)/i.exec(style)
  if (!m) return null
  return cssUnescape((m[2] ?? m[3] ?? m[1]).trim())
}

/** `.xl65 { mso-number-format: … }` rules of Excel's style block, by class. */
function classFormats(css: string): Map<string, string> {
  const out = new Map<string, string>()
  for (const m of css.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const format = msoFormat(m[2])
    if (format === null) continue
    for (const sel of m[1].split(',')) {
      const cls = /\.([A-Za-z0-9_-]+)\s*$/.exec(sel.trim())
      if (cls) out.set(cls[1], format)
    }
  }
  return out
}

export function looksLikeHtmlTable(html: string): boolean {
  return /<table[\s>]/i.test(html) && /<t[dh][\s>]/i.test(html)
}

interface RawCell { th: boolean; text: string; attrs: Record<string, string>; row: number }
interface RawTable { rows: RawCell[][]; head: number; attrs: Record<string, string> }

const BLOCK = new Set(['p', 'div', 'li', 'ul', 'ol', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'pre', 'blockquote', 'section', 'article', 'header', 'footer'])

function collectTables(tokens: Token[]): { tables: RawTable[]; css: string } {
  const tables: RawTable[] = []
  let css = ''
  // the tables being read (a table inside a cell adds its text to that cell)
  const stack: { table: RawTable; row: RawCell[] | null; cell: RawCell | null; inHead: boolean; pre: number }[] = []
  const top = () => stack[stack.length - 1]
  const text = (s: string) => {
    const t = top()
    if (t?.cell) t.cell.text += t.pre > 0 ? s : s.replace(/[ \t\n\r\f]+/g, ' ')
  }
  const endCell = () => { const t = top(); if (t?.cell) { t.cell = null } }
  const endRow = () => { const t = top(); if (!t) return; endCell(); if (t.row) { t.table.rows.push(t.row); t.row = null } }
  for (const tok of tokens) {
    if (typeof tok === 'string') { text(decodeHtmlEntities(tok)); continue }
    if (tok.name === '#style') { css += tok.attrs.text + '\n'; continue }
    const t = top()
    if (tok.name === 'table') {
      if (tok.close) {
        if (!t) continue
        endRow()
        const done = stack.pop()!
        if (stack.length === 0) tables.push(done.table)
        else if (top().cell) top().cell!.text += ' '
      } else if (!tok.self) {
        if (t?.cell) t.cell.text += ' '
        stack.push({ table: { rows: [], head: 0, attrs: tok.attrs }, row: null, cell: null, inHead: false, pre: 0 })
      }
      continue
    }
    if (!t) continue
    switch (tok.name) {
      case 'thead': t.inHead = !tok.close; break
      case 'tbody': case 'tfoot': if (!tok.close) t.inHead = false; break
      case 'tr':
        endRow()
        if (!tok.close) { t.row = []; if (t.inHead) t.table.head = t.table.rows.length + 1 }
        break
      case 'td': case 'th':
        if (tok.close) { endCell(); break }
        endCell()
        if (!t.row) { t.row = []; if (t.inHead) t.table.head = t.table.rows.length + 1 }
        t.cell = { th: tok.name === 'th', text: '', attrs: tok.attrs, row: t.table.rows.length }
        t.row.push(t.cell)
        break
      case 'br': if (t.cell) t.cell.text += '\n'; break
      case 'pre': t.pre += tok.close ? -1 : 1; if (t.cell) t.cell.text += '\n'; break
      default:
        if (BLOCK.has(tok.name) && t.cell) t.cell.text += '\n'
    }
  }
  while (stack.length) { endRow(); const done = stack.pop()!; if (stack.length === 0) tables.push(done.table) }
  return { tables, css }
}

const cleanText = (s: string) => s.replace(/ /g, ' ').split('\n').map((l) => l.replace(/ +/g, ' ').trim()).join('\n').replace(/^\n+|\n+$/g, '').replace(/\n{3,}/g, '\n\n')

function parseJson(s: string | undefined): unknown {
  if (!s) return undefined
  try { return JSON.parse(s) } catch { return undefined }
}

/** Raw value and hint of one cell from what its source attached. */
function typedCell(raw: RawCell, formats: Map<string, string>, notes: GridNote[]): Cell {
  const text = cleanText(raw.text)
  const out: Cell = { text }
  const a = raw.attrs
  // aiws's own copy: the exact wire value
  if ('data-aiws-value' in a) {
    const exact = parseJson(a['data-aiws-value'])
    if (exact !== undefined) out.exact = exact
    return out
  }
  let format: FormatInfo = {}
  // Google Sheets
  const gValue = parseJson(a['data-sheets-value']) as Record<string, unknown> | undefined
  const gFormat = parseJson(a['data-sheets-numberformat']) as Record<string, unknown> | undefined
  if (gFormat) {
    const pattern = typeof gFormat['2'] === 'string' ? gFormat['2'] : null
    const kind = Number(gFormat['1'])
    format = pattern ? classifyFormat(pattern) : kind === 3 ? { hint: 'percent' } : kind === 4 ? { hint: 'currency' } : kind === 5 ? { hint: 'date' } : kind === 6 ? { hint: 'time' } : kind === 7 ? { hint: 'datetime' } : {}
  }
  let num: number | undefined
  if (gValue) {
    const kind = Number(gValue['1'])
    if (kind === 3 && typeof gValue['3'] === 'number') num = gValue['3']
    else if (kind === 4 && typeof gValue['4'] === 'boolean') { out.raw = gValue['4']; out.hint = 'bool'; return out }
    else if (kind === 2) { out.hint = 'text'; return out }
  }
  // Excel
  if ('x:bool' in a) { out.raw = /^true$/i.test(a['x:bool'] || text); out.hint = 'bool'; return out }
  if ('x:fmla' in a) note(notes, 'formula')
  if ('x:num' in a) {
    const v = Number(a['x:num'] !== '' ? a['x:num'] : text.replace(/[,\s]/g, ''))
    if (Number.isFinite(v)) num = v
  }
  if ('x:str' in a) { out.hint = 'text'; return out }
  const cls = (a.class ?? '').split(/\s+/).find((c) => formats.has(c))
  const msoInline = a.style ? msoFormat(a.style) : null
  const mso = msoInline ?? (cls ? formats.get(cls)! : null)
  if (mso !== null && !gFormat) format = classifyFormat(mso)
  if (format.hint === 'text') { out.hint = 'text'; return out }
  if (num === undefined) {
    // a format without a raw value still says what the text is (the text is parsed later)
    if (format.hint === 'currency') { out.hint = 'currency'; if (format.currency) out.currency = format.currency }
    return out
  }
  switch (format.hint) {
    case 'date': case 'datetime': {
      const iso = serialToIso(num, false, format.hint === 'datetime' || num % 1 !== 0)
      if (iso) { out.raw = iso; out.hint = iso.length > 10 ? 'datetime' : 'date' } else out.raw = num
      break
    }
    case 'time': out.hint = 'text'; break
    case 'percent': out.raw = num; out.hint = 'percent'; break
    case 'currency': out.raw = num; out.hint = 'currency'; if (format.currency) out.currency = format.currency; break
    default: out.raw = num
  }
  return out
}

/** The aiws field definitions a copy carries. */
function aiwsColumns(attr: string | undefined): ColumnHint[] | null {
  const meta = parseJson(attr) as { fields?: { name?: string; type?: string; scale?: number; currency?: string; options?: string[] }[] } | undefined
  if (!meta?.fields || !Array.isArray(meta.fields)) return null
  return meta.fields.map((f) => ({ type: f.type as ColumnHint['type'], ...(typeof f.scale === 'number' ? { scale: f.scale } : {}), ...(f.currency ? { currency: f.currency } : {}), ...(Array.isArray(f.options) ? { options: f.options.map(String) } : {}) }))
}

const MAX_SPAN = 1000

/** The largest table of a clipboard HTML fragment as a grid; null when there is none. */
export function readHtmlTable(name: string, html: string): Grid | null {
  const { tables, css } = collectTables(tokenize(html))
  const size = (t: RawTable) => t.rows.reduce((a, r) => a + r.length, 0)
  const table = tables.filter((t) => size(t) > 0).sort((a, b) => size(b) - size(a))[0]
  if (!table) return null
  const formats = classFormats(css)
  const notes: GridNote[] = []
  const rows: Cell[][] = []
  // laid out: a span takes its slots; the value stays in the top-left one
  const taken = new Set<string>()
  let mergedHeader = 0
  let merged = 0
  table.rows.forEach((row, r) => {
    rows[r] ??= []
    let c = 0
    for (const raw of row) {
      while (taken.has(`${r},${c}`)) c++
      const colspan = Math.min(MAX_SPAN, Math.max(1, parseInt(raw.attrs.colspan ?? '1', 10) || 1))
      const rowspan = Math.min(MAX_SPAN, Math.max(1, parseInt(raw.attrs.rowspan ?? '1', 10) || 1))
      rows[r][c] = typedCell(raw, formats, notes)
      if (colspan > 1 || rowspan > 1) {
        if (r === 0) mergedHeader += 1; else merged += 1
        for (let dr = 0; dr < rowspan; dr++) for (let dc = 0; dc < colspan; dc++) {
          if (dr === 0 && dc === 0) continue
          taken.add(`${r + dr},${c + dc}`)
          rows[r + dr] ??= []
          rows[r + dr][c + dc] = { text: '', covered: true }
        }
      }
      c += colspan
    }
  })
  for (let r = 0; r < rows.length; r++) { rows[r] ??= []; for (let c = 0; c < rows[r].length; c++) rows[r][c] ??= cell('') }
  if (mergedHeader) note(notes, 'merged_header', undefined, mergedHeader)
  if (merged) note(notes, 'merged_cells', undefined, merged)
  const columns = aiwsColumns(table.attrs['data-aiws-table'])
  const header = columns ? true : table.head > 0 || (table.rows[0]?.length > 0 && table.rows[0].every((c) => c.th)) ? true : null
  return { name, rows, header, columns: columns ?? [], notes }
}
