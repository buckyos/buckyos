/* xlsx workbooks (表格数据导入 TODO §5.3, Q3: our own reader, no SheetJS). A zip of XML parts: the central directory
 * locates each part, stored (0) and deflated (8) entries are read (`DecompressionStream('deflate-raw')`, in browsers
 * and Deno). From the workbook: sheet names, hidden sheets, the 1904 date system; shared strings (rich runs joined,
 * phonetic runs left out); the number format of each cell style; and per sheet the cells (`t` = s / str / inlineStr /
 * b / d / e / number), formulas (their cached value), merged ranges. Numbers become dates, money and percentages by
 * their format. `.xls`, `.numbers` and `.ods` are recognised and refused with advice (Q3). */

import { elements, parseXml, textOf, type XmlElement } from '../import-drawio/xml.ts'
import { classifyFormat, serialToIso, type FormatInfo } from './format.ts'
import { cell, note, type Cell, type Grid, type GridNote } from './grid.ts'

export class XlsxError extends Error {
  readonly code: 'not_zip' | 'not_xlsx' | 'encrypted' | 'unsupported' | 'corrupt'
  constructor(code: XlsxError['code'], message: string) { super(message); this.code = code }
}

interface ZipEntry { name: string; method: number; compressed: number; size: number; offset: number; flags: number }

const u16 = (b: Uint8Array, o: number) => b[o] | (b[o + 1] << 8)
const u32 = (b: Uint8Array, o: number) => (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0

export function isZip(bytes: Uint8Array): boolean {
  return bytes.length > 4 && bytes[0] === 0x50 && bytes[1] === 0x4b && bytes[2] === 0x03 && bytes[3] === 0x04
}

export function zipEntries(bytes: Uint8Array): Map<string, ZipEntry> {
  // the end of central directory record: within the last 64 KiB + 22 bytes
  let eocd = -1
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 65_557); i--) {
    if (u32(bytes, i) === 0x06054b50) { eocd = i; break }
  }
  if (eocd < 0) throw new XlsxError('not_zip', 'no zip directory')
  const count = u16(bytes, eocd + 10)
  let p = u32(bytes, eocd + 16)
  const out = new Map<string, ZipEntry>()
  const dec = new TextDecoder()
  for (let k = 0; k < count; k++) {
    if (p + 46 > bytes.length || u32(bytes, p) !== 0x02014b50) throw new XlsxError('corrupt', 'bad zip directory')
    const flags = u16(bytes, p + 8)
    const method = u16(bytes, p + 10)
    const compressed = u32(bytes, p + 20)
    const size = u32(bytes, p + 24)
    const nameLen = u16(bytes, p + 28), extraLen = u16(bytes, p + 30), commentLen = u16(bytes, p + 32)
    const offset = u32(bytes, p + 42)
    const name = dec.decode(bytes.subarray(p + 46, p + 46 + nameLen))
    out.set(name.replace(/^\//, ''), { name, method, compressed, size, offset, flags })
    p += 46 + nameLen + extraLen + commentLen
  }
  return out
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate-raw'))
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

export async function zipRead(bytes: Uint8Array, entry: ZipEntry): Promise<Uint8Array> {
  if (entry.flags & 1) throw new XlsxError('encrypted', 'the workbook is encrypted')
  const p = entry.offset
  if (u32(bytes, p) !== 0x04034b50) throw new XlsxError('corrupt', `bad zip entry ${entry.name}`)
  const start = p + 30 + u16(bytes, p + 26) + u16(bytes, p + 28)
  const data = bytes.subarray(start, start + entry.compressed)
  if (entry.method === 0) return data
  if (entry.method === 8) return await inflateRaw(data)
  throw new XlsxError('unsupported', `zip compression method ${entry.method}`)
}

/** What a file is, by its bytes (for files without a telling name). */
export function sniffWorkbook(bytes: Uint8Array): 'xlsx' | 'ods' | 'numbers' | 'xls' | null {
  if (bytes.length > 8 && bytes[0] === 0xd0 && bytes[1] === 0xcf && bytes[2] === 0x11 && bytes[3] === 0xe0) return 'xls'
  if (!isZip(bytes)) return null
  try {
    const entries = zipEntries(bytes)
    if (entries.has('xl/workbook.xml')) return 'xlsx'
    if (entries.has('content.xml') && entries.has('mimetype')) return 'ods'
    if ([...entries.keys()].some((k) => k.startsWith('Index/') || k.endsWith('.iwa'))) return 'numbers'
  } catch { /* not a readable zip */ }
  return null
}

export interface SheetInfo { name: string; hidden: boolean; path: string }
export interface Workbook {
  sheets: SheetInfo[]
  date1904: boolean
  /** Read one sheet as a grid. */
  read(sheet: SheetInfo, limit?: number): Promise<Grid>
}

const local = (name: string) => name.slice(name.indexOf(':') + 1)
const child = (el: XmlElement, name: string) => el.children.find((c): c is XmlElement => typeof c !== 'string' && local(c.name) === name)
const kids = (el: XmlElement, name: string) => elements(el).filter((c) => local(c.name) === name)
const all = (el: XmlElement, name: string, out: XmlElement[] = []): XmlElement[] => {
  if (local(el.name) === name) out.push(el)
  for (const c of el.children) if (typeof c !== 'string') all(c, name, out)
  return out
}

/** `A1` → [row 0, column 0]. */
export function cellRef(ref: string): [number, number] | null {
  const m = /^\$?([A-Z]{1,3})\$?(\d+)$/.exec(ref.toUpperCase())
  if (!m) return null
  let col = 0
  for (const ch of m[1]) col = col * 26 + (ch.charCodeAt(0) - 64)
  return [parseInt(m[2], 10) - 1, col - 1]
}

/** A double as the text a spreadsheet shows (15 significant digits: no binary noise like 0.30000000000000004). */
export const numberText = (n: number) => String(Number(n.toPrecision(15)))

export async function openXlsx(bytes: Uint8Array): Promise<Workbook> {
  if (!isZip(bytes)) throw new XlsxError('not_zip', 'not a zip file')
  const entries = zipEntries(bytes)
  const dec = new TextDecoder()
  const text = async (path: string) => { const e = entries.get(path); return e ? dec.decode(await zipRead(bytes, e)) : null }
  const wbText = await text('xl/workbook.xml')
  if (wbText === null) throw new XlsxError('not_xlsx', 'no xl/workbook.xml')
  const wb = parseXml(wbText)
  const date1904 = all(wb, 'workbookPr').some((e) => /^(1|true)$/.test(e.attrs.date1904 ?? ''))
  const rels = new Map<string, string>()
  const relsText = await text('xl/_rels/workbook.xml.rels')
  if (relsText) for (const r of all(parseXml(relsText), 'Relationship')) rels.set(r.attrs.Id, r.attrs.Target)
  const target = (t: string) => (t.startsWith('/') ? t.slice(1) : `xl/${t}`).replace(/\/\.\//g, '/')
  const sheets: SheetInfo[] = []
  for (const s of all(wb, 'sheet')) {
    const rid = Object.entries(s.attrs).find(([k]) => local(k) === 'id')?.[1] ?? ''
    const t = rels.get(rid)
    if (!t) continue
    sheets.push({ name: s.attrs.name ?? `Sheet${sheets.length + 1}`, hidden: s.attrs.state === 'hidden' || s.attrs.state === 'veryHidden', path: target(t) })
  }
  // shared strings: the text of each <si>, phonetic guides left out
  let strings: string[] | null = null
  const sharedStrings = async () => {
    if (strings) return strings
    const st = await text('xl/sharedStrings.xml')
    strings = st ? kids(parseXml(st), 'si').map(siText) : []
    return strings
  }
  // the number format of each cell style
  let styles: FormatInfo[] | null = null
  const cellStyles = async () => {
    if (styles) return styles
    const st = await text('xl/styles.xml')
    styles = []
    if (st) {
      const doc = parseXml(st)
      const codes = new Map<number, string>()
      for (const f of all(doc, 'numFmt')) codes.set(Number(f.attrs.numFmtId), f.attrs.formatCode ?? '')
      const xfs = child(doc, 'cellXfs')
      for (const xf of xfs ? kids(xfs, 'xf') : []) {
        const id = Number(xf.attrs.numFmtId ?? 0)
        styles.push(classifyFormat(codes.get(id), id))
      }
    }
    return styles
  }
  return {
    sheets, date1904,
    read: async (sheet, limit = Infinity) => {
      const xml = await text(sheet.path)
      if (xml === null) throw new XlsxError('corrupt', `missing ${sheet.path}`)
      return readSheet(sheet.name, parseXml(xml), await sharedStrings(), await cellStyles(), date1904, limit)
    },
  }
}

function siText(si: XmlElement): string {
  const parts: string[] = []
  const walk = (el: XmlElement) => {
    for (const c of el.children) {
      if (typeof c === 'string') continue
      const n = local(c.name)
      if (n === 'rPh' || n === 'phoneticPr') continue
      if (n === 't') parts.push(textOf(c))
      else walk(c)
    }
  }
  walk(si)
  return parts.join('').replace(/_x([0-9A-Fa-f]{4})_/g, (_, h: string) => String.fromCharCode(parseInt(h, 16)))
}

function readSheet(name: string, doc: XmlElement, strings: string[], styles: FormatInfo[], date1904: boolean, limit: number): Grid {
  const notes: GridNote[] = []
  const rows: Cell[][] = []
  const data = child(doc, 'sheetData')
  let rowIndex = -1
  let truncated = 0
  for (const row of data ? kids(data, 'row') : []) {
    rowIndex = row.attrs.r ? parseInt(row.attrs.r, 10) - 1 : rowIndex + 1
    if (rowIndex >= limit) { truncated += 1; continue }
    let colIndex = -1
    for (const c of kids(row, 'c')) {
      const ref = c.attrs.r ? cellRef(c.attrs.r) : null
      colIndex = ref ? ref[1] : colIndex + 1
      const out = readCell(c, strings, styles, date1904, notes)
      if (!out) continue
      rows[rowIndex] ??= []
      rows[rowIndex][colIndex] = out
    }
  }
  for (let r = 0; r < rows.length; r++) { rows[r] ??= []; for (let k = 0; k < rows[r].length; k++) rows[r][k] ??= cell('') }
  // merged ranges keep the top-left value (the others are empty in the file already): mark them covered
  const mergeCells = child(doc, 'mergeCells')
  let first = Infinity
  for (const r of rows.keys()) if (rows[r].some((c) => c.text !== '')) { first = r; break }
  for (const m of mergeCells ? kids(mergeCells, 'mergeCell') : []) {
    const [a, b] = (m.attrs.ref ?? '').split(':')
    const at = cellRef(a ?? '')
    const to = cellRef(b ?? a ?? '')
    if (!at || !to) continue
    note(notes, at[0] === first ? 'merged_header' : 'merged_cells', m.attrs.ref)
    for (let r = at[0]; r <= Math.min(to[0], at[0] + 1000); r++) for (let k = at[1]; k <= Math.min(to[1], at[1] + 1000); k++) {
      if (r === at[0] && k === at[1]) continue
      if (r >= rows.length && r > first) continue
      rows[r] ??= []
      rows[r][k] = { text: '', covered: true }
    }
  }
  for (let r = 0; r < rows.length; r++) { rows[r] ??= []; for (let k = 0; k < rows[r].length; k++) rows[r][k] ??= cell('') }
  return { name, rows, header: null, columns: [], notes, ...(truncated ? { truncated } : {}) }
}

function readCell(c: XmlElement, strings: string[], styles: FormatInfo[], date1904: boolean, notes: GridNote[]): Cell | null {
  const t = c.attrs.t ?? 'n'
  const v = child(c, 'v')
  const f = child(c, 'f')
  const raw = v ? textOf(v) : ''
  if (f) note(notes, 'formula', c.attrs.r)
  switch (t) {
    case 's': { const s = strings[parseInt(raw, 10)] ?? ''; return { text: s, hint: 'text' } }
    case 'str': return { text: raw, hint: 'text' }
    case 'inlineStr': { const is = child(c, 'is'); return { text: is ? siText(is) : raw, hint: 'text' } }
    case 'b': return { text: raw === '1' ? 'TRUE' : 'FALSE', raw: raw === '1', hint: 'bool' }
    case 'e': note(notes, 'error_cells', `${c.attrs.r ?? ''} ${raw}`.trim()); return null
    case 'd': {
      const iso = raw.replace(/Z$/, '')
      const date = /^\d{4}-\d{2}-\d{2}$/.test(iso) || /T00:00:00(\.0+)?$/.test(iso)
      return { text: iso, raw: date ? iso.slice(0, 10) : iso, hint: date ? 'date' : 'datetime' }
    }
  }
  if (raw === '') return null
  const num = Number(raw)
  if (!Number.isFinite(num)) return { text: raw }
  const style = styles[parseInt(c.attrs.s ?? '0', 10)] ?? {}
  switch (style.hint) {
    case 'date': case 'datetime': {
      const iso = serialToIso(num, date1904, style.hint === 'datetime' || num % 1 !== 0)
      if (!iso) return { text: numberText(num), raw: num }
      return { text: iso.replace('T', ' '), raw: iso, hint: iso.length > 10 ? 'datetime' : 'date' }
    }
    case 'time': {
      note(notes, 'time_only', c.attrs.r)
      const iso = serialToIso(1 + (num % 1), false, true)
      return { text: iso ? iso.slice(11) : numberText(num), hint: 'text' }
    }
    case 'percent': return { text: `${numberText(num * 100)}%`, raw: num, hint: 'percent' }
    case 'currency': return { text: numberText(num), raw: num, hint: 'currency', ...(style.currency ? { currency: style.currency } : {}) }
    case 'text': return { text: numberText(num), hint: 'text' }
  }
  return { text: numberText(num), raw: num }
}
