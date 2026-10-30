/* CSV / TSV files (表格数据导入 TODO §5.2): the encoding (a BOM; else strict UTF-8; else GB 18030; UTF-16 without a
 * BOM when every other byte is zero), the delimiter (the one of `, ; \t |` that splits the first 20 records into the
 * same number of fields), and RFC 4180 (quoted fields, `""` escapes, line breaks inside quotes, CRLF / LF / CR). */

import { textGrid, type Grid } from './grid.ts'

export type Encoding = 'utf-8' | 'gb18030' | 'utf-16le' | 'utf-16be' | 'windows-1252'
export const ENCODINGS: Encoding[] = ['utf-8', 'gb18030', 'utf-16le', 'utf-16be', 'windows-1252']
export type Delimiter = ',' | ';' | '\t' | '|'
export const DELIMITERS: Delimiter[] = [',', ';', '\t', '|']

/** Files larger than this are not parsed (memory; 表格数据导入 TODO §5.2). */
export const CSV_LIMIT = 64 * 1024 * 1024

export function sniffEncoding(bytes: Uint8Array): Encoding {
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) return 'utf-8'
  if (bytes[0] === 0xff && bytes[1] === 0xfe) return 'utf-16le'
  if (bytes[0] === 0xfe && bytes[1] === 0xff) return 'utf-16be'
  // UTF-16 text without a BOM: the high bytes of ASCII characters are zero
  const n = Math.min(bytes.length, 4096)
  if (n >= 4) {
    let odd = 0, even = 0
    for (let i = 0; i + 1 < n; i += 2) { if (bytes[i] === 0) even += 1; if (bytes[i + 1] === 0) odd += 1 }
    if (odd > n / 2 * 0.3 && even === 0) return 'utf-16le'
    if (even > n / 2 * 0.3 && odd === 0) return 'utf-16be'
  }
  try { new TextDecoder('utf-8', { fatal: true }).decode(bytes); return 'utf-8' } catch { return 'gb18030' }
}

export function decodeText(bytes: Uint8Array, encoding: Encoding | 'auto' = 'auto'): { text: string; encoding: Encoding } {
  const chosen = encoding === 'auto' ? sniffEncoding(bytes) : encoding
  const text = new TextDecoder(chosen).decode(bytes)
  // the decoder strips a matching BOM; a stray one (another encoding chosen) goes too
  return { text: text.charCodeAt(0) === 0xfeff ? text.slice(1) : text, encoding: chosen }
}

/** Split into records of fields (RFC 4180). `limit` stops after that many records (sniffing). */
export function parseDelimited(text: string, delimiter: string, limit = Infinity): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let quoted = false
  let i = 0
  const n = text.length
  const endRow = () => { row.push(field); field = ''; rows.push(row); row = [] }
  while (i < n) {
    const c = text[i]
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') { field += '"'; i += 2; continue }
        quoted = false
        i += 1
        continue
      }
      field += c
      i += 1
      continue
    }
    if (c === '"' && field.trim() === '') { field = ''; quoted = true; i += 1; continue }
    if (c === delimiter) { row.push(field); field = ''; i += 1; continue }
    if (c === '\r' || c === '\n') {
      endRow()
      i += c === '\r' && text[i + 1] === '\n' ? 2 : 1
      if (rows.length >= limit) return rows
      continue
    }
    field += c
    i += 1
  }
  if (field !== '' || row.length > 0 || quoted) endRow()
  return rows
}

/** The delimiter that splits the first 20 records into the same number (> 1) of fields; the largest such split,
 * `,` first on a tie. Without one: the delimiter seen most often, else `,`. */
export function sniffDelimiter(text: string): Delimiter {
  const head = text.slice(0, 256 * 1024)
  let best: { d: Delimiter; fields: number } | null = null
  const seen = new Map<Delimiter, number>()
  for (const d of DELIMITERS) {
    const rows = parseDelimited(head, d, 21).slice(0, 20).filter((r) => r.length > 1 || r[0] !== '')
    const sample = rows.length > 1 && head.length === 256 * 1024 ? rows.slice(0, -1) : rows
    seen.set(d, sample.reduce((a, r) => a + r.length - 1, 0))
    if (sample.length === 0) continue
    const fields = sample[0].length
    if (fields > 1 && sample.every((r) => r.length === fields) && (!best || fields > best.fields)) best = { d, fields }
  }
  if (best) return best.d
  let most: Delimiter = ','
  for (const d of DELIMITERS) if ((seen.get(d) ?? 0) > (seen.get(most) ?? 0)) most = d
  return most
}

export interface CsvOptions { encoding?: Encoding | 'auto'; delimiter?: Delimiter | 'auto' }
export interface CsvRead { grid: Grid; encoding: Encoding; delimiter: Delimiter }

export function readCsv(name: string, bytes: Uint8Array, options: CsvOptions = {}): CsvRead {
  const { text, encoding } = decodeText(bytes, options.encoding ?? 'auto')
  const tsv = /\.tsv$/i.test(name)
  const delimiter = options.delimiter && options.delimiter !== 'auto' ? options.delimiter : tsv ? '\t' : sniffDelimiter(text)
  const grid = textGrid(name.replace(/\.(csv|tsv|txt)$/i, '') || name, parseDelimited(text, delimiter))
  return { grid, encoding, delimiter }
}
